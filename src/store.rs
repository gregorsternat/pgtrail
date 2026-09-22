//! Private, local SQLite history. Stored observations never contain a connection URL.
use std::{path::Path, time::Duration};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{
    ConnectOptions, Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};

use crate::model::{SNAPSHOT_VERSION, Snapshot};

const STORE_VERSION: i64 = 1;
const APPLICATION_ID: i64 = 0x5047_5452;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SnapshotSummary {
    pub(crate) id: i64,
    pub(crate) label: String,
    pub(crate) captured_at: DateTime<Utc>,
    pub(crate) source: String,
    pub(crate) complete: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct Store {
    pool: SqlitePool,
}

impl Store {
    pub(crate) async fn open(path: &Path) -> Result<Self> {
        if path.as_os_str().is_empty() || path.file_name().is_none() {
            bail!("snapshot history requires a file path");
        }
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            let mut builder = tokio::fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            builder.mode(0o700);
            builder
                .create(parent)
                .await
                .context("could not create the snapshot history directory")?;
        }
        // Do not follow a link to a file outside the selected history location.
        match tokio::fs::symlink_metadata(path).await {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                bail!("snapshot history must be a regular file, not a directory or symbolic link");
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut options = tokio::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                options.mode(0o600);
                match options.open(path).await {
                    Ok(file) => drop(file),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                        let metadata = tokio::fs::symlink_metadata(path)
                            .await
                            .context("could not inspect the snapshot history file")?;
                        if metadata.file_type().is_symlink() || !metadata.is_file() {
                            bail!("snapshot history must be a regular file");
                        }
                    }
                    Err(error) => {
                        return Err(error).context("could not create the snapshot history file");
                    }
                }
            }
            Err(error) => return Err(error).context("could not inspect the snapshot history file"),
        }
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(false)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5))
            .disable_statement_logging();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .context("could not open snapshot history")?;
        let store = Self { pool };
        store.migrate().await?;
        // Only tighten an existing file's permissions after proving it belongs to pgtrail.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .await
                .context("could not secure the snapshot history file")?;
        }
        Ok(store)
    }

    async fn migrate(&self) -> Result<()> {
        let mut transaction = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .context("could not start history migration")?;
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&mut *transaction)
            .await
            .context("could not read history schema version")?;
        if !(0..=STORE_VERSION).contains(&version) {
            bail!(
                "snapshot history uses schema version {version}; this pgtrail supports version {STORE_VERSION}"
            );
        }
        let application_id: i64 = sqlx::query_scalar("PRAGMA application_id")
            .fetch_one(&mut *transaction)
            .await?;
        if version == 0 {
            let existing_tables: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            )
            .fetch_one(&mut *transaction)
            .await?;
            if existing_tables != 0 || (application_id != 0 && application_id != APPLICATION_ID) {
                bail!("refusing to initialize a non-pgtrail SQLite database");
            }
            sqlx::query("CREATE TABLE snapshots (id INTEGER PRIMARY KEY AUTOINCREMENT, label TEXT NOT NULL, captured_at TEXT NOT NULL, source TEXT NOT NULL, complete INTEGER NOT NULL CHECK (complete IN (0, 1)), payload TEXT NOT NULL)")
                .execute(&mut *transaction).await.context("could not create the snapshot history schema")?;
            sqlx::query("PRAGMA application_id = 1346851922")
                .execute(&mut *transaction)
                .await?;
            sqlx::query("PRAGMA user_version = 1")
                .execute(&mut *transaction)
                .await?;
        } else if application_id != APPLICATION_ID {
            bail!("the selected SQLite file is not a pgtrail snapshot history");
        }
        sqlx::query(
            "SELECT id, label, captured_at, source, complete, payload FROM snapshots LIMIT 0",
        )
        .execute(&mut *transaction)
        .await
        .context("snapshot history schema is damaged or incompatible")?;
        transaction
            .commit()
            .await
            .context("could not commit history migration")?;
        Ok(())
    }

    pub(crate) async fn save(&self, snapshot: &Snapshot, label: &str) -> Result<i64> {
        validate_snapshot(snapshot)?;
        let payload =
            serde_json::to_string(snapshot).context("could not serialize the snapshot")?;
        let source = format!(
            "{} / {}",
            snapshot.source.endpoint, snapshot.source.database
        );
        let row = sqlx::query("INSERT INTO snapshots (label, captured_at, source, complete, payload) VALUES (?, ?, ?, ?, ?) RETURNING id")
            .bind(label.trim())
            .bind(snapshot.completed_at.to_rfc3339())
            .bind(source)
            .bind(snapshot.is_complete())
            .bind(payload)
            .fetch_one(&self.pool).await.context("could not save the snapshot to local history")?;
        row.try_get("id")
            .context("could not read the saved snapshot ID")
    }

    pub(crate) async fn list(&self) -> Result<Vec<SnapshotSummary>> {
        let rows = sqlx::query(
            "SELECT id, label, captured_at, source, complete FROM snapshots ORDER BY id DESC",
        )
        .fetch_all(&self.pool)
        .await
        .context("could not list snapshot history")?;
        rows.into_iter()
            .map(|row| {
                let captured_at: String = row.try_get("captured_at")?;
                Ok(SnapshotSummary {
                    id: row.try_get("id")?,
                    label: row.try_get("label")?,
                    captured_at: DateTime::parse_from_rfc3339(&captured_at)
                        .context("snapshot history contains an invalid capture timestamp")?
                        .with_timezone(&Utc),
                    source: row.try_get("source")?,
                    complete: row.try_get("complete")?,
                })
            })
            .collect()
    }

    pub(crate) async fn load(&self, id: i64) -> Result<Snapshot> {
        let payload: Option<String> =
            sqlx::query_scalar("SELECT payload FROM snapshots WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .context("could not load the snapshot")?;
        let payload = payload.with_context(|| format!("snapshot {id} does not exist"))?;
        let snapshot: Snapshot =
            serde_json::from_str(&payload).context("saved snapshot is damaged or incompatible")?;
        validate_snapshot(&snapshot)?;
        Ok(snapshot)
    }

    pub(crate) async fn delete(&self, id: i64) -> Result<()> {
        let result = sqlx::query("DELETE FROM snapshots WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .context("could not delete the saved snapshot")?;
        if result.rows_affected() == 0 {
            bail!("snapshot {id} does not exist");
        }
        Ok(())
    }
}

fn validate_snapshot(snapshot: &Snapshot) -> Result<()> {
    if snapshot.schema_version != SNAPSHOT_VERSION {
        bail!(
            "unsupported snapshot format version {} (expected {SNAPSHOT_VERSION})",
            snapshot.schema_version
        );
    }
    if snapshot.completed_at < snapshot.started_at {
        bail!("snapshot collection completed before it started");
    }
    if let Some(statistics) = snapshot.statements.available() {
        for statement in &statistics.entries {
            if !statement.total_exec_ms.is_finite() || !statement.mean_exec_ms.is_finite() {
                bail!("snapshot contains non-finite statement statistics");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::tests::snapshot;

    #[tokio::test]
    async fn captures_survive_restart_and_delete_is_explicit() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("private/history.sqlite3");
        let store = Store::open(&path).await?;
        let first = snapshot();
        let id = store.save(&first, " baseline ").await?;
        assert_eq!(store.list().await?[0].label, "baseline");
        store.pool.close().await;
        let reopened = Store::open(&path).await?;
        assert_eq!(reopened.load(id).await?, first);
        assert!(reopened.list().await?[0].complete);
        reopened.delete(id).await?;
        assert!(reopened.list().await?.is_empty());
        assert!(reopened.load(id).await.is_err());
        assert!(reopened.delete(id).await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn migration_is_versioned_and_rejects_future_versions() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("history.sqlite3");
        let store = Store::open(&path).await?;
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&store.pool)
            .await?;
        assert_eq!(version, STORE_VERSION);
        sqlx::query("PRAGMA user_version = 99")
            .execute(&store.pool)
            .await?;
        store.pool.close().await;
        assert!(
            Store::open(&path)
                .await
                .unwrap_err()
                .to_string()
                .contains("version 99")
        );
        Ok(())
    }

    #[tokio::test]
    async fn simultaneous_first_opens_serialize_migration_and_preserve_both_captures() -> Result<()>
    {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("history.sqlite3");
        let (left, right) = tokio::join!(Store::open(&path), Store::open(&path));
        let (left, right) = (left?, right?);
        let observation = snapshot();
        let (first, second) = tokio::join!(
            left.save(&observation, "left"),
            right.save(&observation, "right")
        );
        assert_ne!(first?, second?);
        assert_eq!(left.list().await?.len(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn unknown_sqlite_database_is_not_migrated() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("other.sqlite3");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true),
            )
            .await?;
        sqlx::query("CREATE TABLE unrelated (value TEXT)")
            .execute(&pool)
            .await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640))?;
        }
        assert!(Store::open(&path).await.is_err());
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(&pool)
            .await?;
        assert_eq!(version, 0);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path)?.permissions().mode() & 0o777,
                0o640
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn incomplete_captures_preserve_availability_and_unknown_versions_fail() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let store = Store::open(&directory.path().join("history.sqlite3")).await?;
        let mut observation = snapshot();
        observation.statements =
            crate::model::Observation::Unavailable("extension is not installed".into());
        let id = store.save(&observation, "partial").await?;
        assert!(!store.list().await?[0].complete);
        assert_eq!(store.load(id).await?, observation);
        observation.schema_version = SNAPSHOT_VERSION + 1;
        assert!(store.save(&observation, "unsupported").await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn invalid_files_and_payloads_fail_visibly() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let bad_path = directory.path().join("not-a-database");
        tokio::fs::write(&bad_path, b"corrupt database").await?;
        assert!(Store::open(&bad_path).await.is_err());
        assert!(Store::open(&bad_path.join("child")).await.is_err());
        let store = Store::open(&directory.path().join("valid.sqlite3")).await?;
        let id = store.save(&snapshot(), "capture").await?;
        sqlx::query("UPDATE snapshots SET payload = '{}' WHERE id = ?")
            .bind(id)
            .execute(&store.pool)
            .await?;
        assert!(store.load(id).await.is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn private_permissions_and_symbolic_link_rejection() -> Result<()> {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("private/history.sqlite3");
        let _store = Store::open(&path).await?;
        assert_eq!(
            std::fs::metadata(&path)?.permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(path.parent().unwrap())?
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        let link = directory.path().join("link.sqlite3");
        symlink(path, &link)?;
        assert!(Store::open(&link).await.is_err());
        Ok(())
    }
}
