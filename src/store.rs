//! Private, local SQLite history. Stored observations never contain a connection URL.
use std::{collections::BTreeMap, path::Path, time::Duration};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::{
    ConnectOptions, Row, SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions, SqliteRow},
};

use crate::model::{SNAPSHOT_VERSION, Snapshot};

const STORE_VERSION: i64 = 2;
const APPLICATION_ID: i64 = 0x5047_5452;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SnapshotSummary {
    pub(crate) id: i64,
    pub(crate) label: String,
    pub(crate) captured_at: DateTime<Utc>,
    pub(crate) source: String,
    pub(crate) complete: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct CaptureMetadata {
    pub(crate) label: String,
    pub(crate) note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct IncidentSummary {
    pub(crate) id: i64,
    pub(crate) title: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) closed_at: Option<DateTime<Utc>>,
    pub(crate) capture_count: i64,
    pub(crate) note_count: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct IncidentNote {
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Incident {
    pub(crate) summary: IncidentSummary,
    pub(crate) notes: Vec<IncidentNote>,
    pub(crate) captures: Vec<SnapshotSummary>,
    pub(crate) capture_notes: BTreeMap<i64, String>,
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
        if version < 2 {
            for statement in [
                "CREATE TABLE incidents (id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 200), created_at TEXT NOT NULL, closed_at TEXT)",
                "CREATE TABLE incident_notes (id INTEGER PRIMARY KEY AUTOINCREMENT, incident_id INTEGER NOT NULL REFERENCES incidents(id) ON DELETE CASCADE, created_at TEXT NOT NULL, text TEXT NOT NULL CHECK (length(text) BETWEEN 1 AND 10000))",
                "CREATE TABLE incident_snapshots (incident_id INTEGER NOT NULL REFERENCES incidents(id) ON DELETE CASCADE, snapshot_id INTEGER NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE, PRIMARY KEY (incident_id, snapshot_id))",
                "CREATE INDEX incident_notes_order ON incident_notes(incident_id, created_at, id)",
                "CREATE TABLE snapshot_annotations (snapshot_id INTEGER PRIMARY KEY REFERENCES snapshots(id) ON DELETE CASCADE, note TEXT NOT NULL CHECK (length(note) <= 10000))",
                "PRAGMA user_version = 2",
            ] {
                sqlx::query(statement)
                    .execute(&mut *transaction)
                    .await
                    .context("could not migrate incident history")?;
            }
        }
        for statement in [
            "SELECT id, title, created_at, closed_at FROM incidents LIMIT 0",
            "SELECT id, incident_id, created_at, text FROM incident_notes LIMIT 0",
            "SELECT incident_id, snapshot_id FROM incident_snapshots LIMIT 0",
            "SELECT snapshot_id, note FROM snapshot_annotations LIMIT 0",
        ] {
            sqlx::query(statement)
                .execute(&mut *transaction)
                .await
                .context("incident history schema is damaged or incompatible")?;
        }
        transaction
            .commit()
            .await
            .context("could not commit history migration")?;
        Ok(())
    }

    pub(crate) async fn save(&self, snapshot: &Snapshot, label: &str) -> Result<i64> {
        self.save_for_incident(snapshot, label, None).await
    }

    pub(crate) async fn save_for_incident(
        &self,
        snapshot: &Snapshot,
        label: &str,
        incident: Option<i64>,
    ) -> Result<i64> {
        validate_snapshot(snapshot)?;
        validate_text(label.trim(), "capture label", 200, true)?;
        let payload =
            serde_json::to_string(snapshot).context("could not serialize the snapshot")?;
        let source = format!(
            "{} / {}",
            snapshot.source.endpoint, snapshot.source.database
        );
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let id: i64 = sqlx::query_scalar("INSERT INTO snapshots (label, captured_at, source, complete, payload) VALUES (?, ?, ?, ?, ?) RETURNING id")
            .bind(label.trim()).bind(snapshot.completed_at.to_rfc3339()).bind(source)
            .bind(snapshot.is_complete()).bind(payload).fetch_one(&mut *transaction).await
            .context("could not save the snapshot to local history")?;
        if let Some(incident) = incident {
            attach_in_transaction(&mut transaction, incident, id).await?;
        }
        transaction
            .commit()
            .await
            .context("could not commit saved snapshot")?;
        Ok(id)
    }

    pub(crate) async fn list(&self) -> Result<Vec<SnapshotSummary>> {
        let rows = sqlx::query(
            "SELECT id, label, captured_at, source, complete FROM snapshots ORDER BY id DESC",
        )
        .fetch_all(&self.pool)
        .await
        .context("could not list snapshot history")?;
        rows.into_iter().map(snapshot_summary).collect()
    }

    pub(crate) async fn load(&self, id: i64) -> Result<Snapshot> {
        let payload: Option<String> =
            sqlx::query_scalar("SELECT payload FROM snapshots WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .context("could not load the snapshot")?;
        let payload = payload.with_context(|| format!("snapshot {id} does not exist"))?;
        decode_snapshot(&payload)
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

    pub(crate) async fn annotate_capture(
        &self,
        id: i64,
        label: Option<&str>,
        note: Option<&str>,
    ) -> Result<()> {
        if let Some(label) = label {
            validate_text(label.trim(), "capture label", 200, true)?;
        }
        if let Some(note) = note {
            validate_text(note.trim(), "capture note", 10_000, true)?;
        }
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM snapshots WHERE id = ?)")
                .bind(id)
                .fetch_one(&mut *transaction)
                .await?;
        if !exists {
            bail!("snapshot {id} does not exist");
        }
        if let Some(label) = label {
            sqlx::query("UPDATE snapshots SET label = ? WHERE id = ?")
                .bind(label.trim())
                .bind(id)
                .execute(&mut *transaction)
                .await
                .context("could not update capture label")?;
        }
        if let Some(note) = note {
            sqlx::query("INSERT INTO snapshot_annotations(snapshot_id, note) VALUES (?, ?) ON CONFLICT(snapshot_id) DO UPDATE SET note = excluded.note")
                .bind(id).bind(note.trim()).execute(&mut *transaction).await
                .context("could not annotate capture")?;
        }
        transaction
            .commit()
            .await
            .context("could not save capture annotation")
    }

    pub(crate) async fn capture_metadata(&self, id: i64) -> Result<CaptureMetadata> {
        let row = sqlx::query("SELECT s.label, COALESCE(a.note, '') AS note FROM snapshots s LEFT JOIN snapshot_annotations a ON a.snapshot_id = s.id WHERE s.id = ?")
            .bind(id).fetch_optional(&self.pool).await.context("could not read capture annotation")?
            .with_context(|| format!("snapshot {id} does not exist"))?;
        Ok(CaptureMetadata {
            label: row.try_get("label")?,
            note: row.try_get("note")?,
        })
    }

    pub(crate) async fn create_incident(&self, title: &str) -> Result<i64> {
        validate_text(title.trim(), "incident title", 200, false)?;
        sqlx::query_scalar("INSERT INTO incidents(title, created_at) VALUES (?, ?) RETURNING id")
            .bind(title.trim())
            .bind(Utc::now().to_rfc3339())
            .fetch_one(&self.pool)
            .await
            .context("could not create incident")
    }

    pub(crate) async fn incidents(&self) -> Result<Vec<IncidentSummary>> {
        let rows = sqlx::query("SELECT i.id, i.title, i.created_at, i.closed_at, (SELECT COUNT(*) FROM incident_snapshots s WHERE s.incident_id = i.id) AS capture_count, (SELECT COUNT(*) FROM incident_notes n WHERE n.incident_id = i.id) AS note_count FROM incidents i ORDER BY i.id DESC")
            .fetch_all(&self.pool).await.context("could not list incidents")?;
        rows.into_iter().map(incident_summary).collect()
    }

    pub(crate) async fn incident(&self, id: i64) -> Result<Incident> {
        // One read transaction keeps the counts, notes and attachments consistent.
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query("SELECT i.id, i.title, i.created_at, i.closed_at, (SELECT COUNT(*) FROM incident_snapshots s WHERE s.incident_id = i.id) AS capture_count, (SELECT COUNT(*) FROM incident_notes n WHERE n.incident_id = i.id) AS note_count FROM incidents i WHERE i.id = ?")
            .bind(id).fetch_optional(&mut *transaction).await.context("could not load incident")?
            .with_context(|| format!("incident {id} does not exist"))?;
        let summary = incident_summary(row)?;
        let mut notes = sqlx::query(
            "SELECT created_at, text FROM incident_notes WHERE incident_id = ? ORDER BY id",
        )
        .bind(id)
        .fetch_all(&mut *transaction)
        .await
        .context("could not load incident notes")?
        .into_iter()
        .map(|row| {
            Ok(IncidentNote {
                created_at: parse_timestamp(row.try_get("created_at")?)?,
                text: row.try_get("text")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
        let mut captures = sqlx::query("SELECT s.id, s.label, s.captured_at, s.source, s.complete FROM snapshots s JOIN incident_snapshots a ON a.snapshot_id = s.id WHERE a.incident_id = ? ORDER BY s.id")
            .bind(id).fetch_all(&mut *transaction).await.context("could not load incident captures")?
            .into_iter().map(snapshot_summary).collect::<Result<Vec<_>>>()?;
        let capture_notes = sqlx::query("SELECT n.snapshot_id, n.note FROM snapshot_annotations n JOIN incident_snapshots a ON a.snapshot_id = n.snapshot_id WHERE a.incident_id = ? AND n.note <> '' ORDER BY n.snapshot_id")
            .bind(id).fetch_all(&mut *transaction).await.context("could not load annotated incident evidence")?
            .into_iter().map(|row| Ok((row.try_get("snapshot_id")?, row.try_get("note")?))).collect::<Result<BTreeMap<_, _>>>()?;
        // RFC3339 text can use different offsets and fractional precision; order parsed times.
        notes.sort_by_key(|note| note.created_at);
        captures.sort_by_key(|capture| (capture.captured_at, capture.id));
        transaction.commit().await?;
        Ok(Incident {
            summary,
            notes,
            captures,
            capture_notes,
        })
    }

    pub(crate) async fn add_note(&self, id: i64, text: &str) -> Result<()> {
        validate_text(text.trim(), "incident note", 10_000, false)?;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        ensure_open(&mut transaction, id).await?;
        sqlx::query("INSERT INTO incident_notes(incident_id, created_at, text) VALUES (?, ?, ?)")
            .bind(id)
            .bind(Utc::now().to_rfc3339())
            .bind(text.trim())
            .execute(&mut *transaction)
            .await
            .context("could not save incident note")?;
        transaction
            .commit()
            .await
            .context("could not commit incident note")
    }

    pub(crate) async fn attach_capture(&self, incident: i64, capture: i64) -> Result<()> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        attach_in_transaction(&mut transaction, incident, capture).await?;
        transaction
            .commit()
            .await
            .context("could not commit incident capture")
    }

    pub(crate) async fn detach_capture(&self, incident: i64, capture: i64) -> Result<()> {
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        ensure_open(&mut transaction, incident).await?;
        let result =
            sqlx::query("DELETE FROM incident_snapshots WHERE incident_id = ? AND snapshot_id = ?")
                .bind(incident)
                .bind(capture)
                .execute(&mut *transaction)
                .await
                .context("could not detach capture")?;
        if result.rows_affected() == 0 {
            bail!("snapshot {capture} is not attached to incident {incident}");
        }
        transaction
            .commit()
            .await
            .context("could not commit incident update")
    }

    pub(crate) async fn set_incident_closed(&self, id: i64, closed: bool) -> Result<()> {
        let result = sqlx::query("UPDATE incidents SET closed_at = CASE WHEN ? THEN COALESCE(closed_at, ?) ELSE NULL END WHERE id = ?")
            .bind(closed).bind(Utc::now().to_rfc3339()).bind(id).execute(&self.pool).await
            .context("could not update incident status")?;
        if result.rows_affected() == 0 {
            bail!("incident {id} does not exist");
        }
        Ok(())
    }
}

async fn attach_in_transaction(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    incident: i64,
    capture: i64,
) -> Result<()> {
    ensure_open(transaction, incident).await?;
    let payload: String = sqlx::query_scalar("SELECT payload FROM snapshots WHERE id = ?")
        .bind(capture)
        .fetch_optional(&mut **transaction)
        .await?
        .with_context(|| format!("snapshot {capture} does not exist"))?;
    let snapshot = decode_snapshot(&payload)?;
    // Project only source metadata: a long recording must not load every prior
    // activity/table/statement payload into application memory on every save.
    let sources: Vec<Option<String>> = sqlx::query_scalar("SELECT DISTINCT json_extract(s.payload, '$.source') FROM snapshots s JOIN incident_snapshots a ON a.snapshot_id = s.id WHERE a.incident_id = ?")
        .bind(incident).fetch_all(&mut **transaction).await.context("could not validate incident capture sources")?;
    for source in sources {
        let source = source.context("incident contains a capture without source metadata")?;
        let previous: crate::model::Source = serde_json::from_str(&source)
            .map_err(|_| anyhow::anyhow!("incident contains damaged capture source metadata"))?;
        if !incident_sources_compatible(&previous, &snapshot.source) {
            bail!(
                "capture source is incompatible with this incident; use a separate incident for a different PostgreSQL target"
            );
        }
    }
    sqlx::query("INSERT OR IGNORE INTO incident_snapshots(incident_id, snapshot_id) VALUES (?, ?)")
        .bind(incident)
        .bind(capture)
        .execute(&mut **transaction)
        .await
        .context("could not attach capture")?;
    Ok(())
}

async fn ensure_open(transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>, id: i64) -> Result<()> {
    let closed: Option<String> = sqlx::query_scalar("SELECT closed_at FROM incidents WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut **transaction)
        .await
        .context("could not inspect incident status")?
        .with_context(|| format!("incident {id} does not exist"))?;
    if closed.is_some() {
        bail!("incident {id} is closed; reopen it before changing notes or attachments");
    }
    Ok(())
}

fn parse_timestamp(timestamp: String) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&timestamp)
        .map(|time| time.with_timezone(&Utc))
        .context("local history contains an invalid timestamp")
}

fn snapshot_summary(row: SqliteRow) -> Result<SnapshotSummary> {
    Ok(SnapshotSummary {
        id: row.try_get("id")?,
        label: row.try_get("label")?,
        captured_at: parse_timestamp(row.try_get("captured_at")?)?,
        source: row.try_get("source")?,
        complete: row.try_get("complete")?,
    })
}

fn incident_summary(row: SqliteRow) -> Result<IncidentSummary> {
    let closed: Option<String> = row.try_get("closed_at")?;
    Ok(IncidentSummary {
        id: row.try_get("id")?,
        title: row.try_get("title")?,
        created_at: parse_timestamp(row.try_get("created_at")?)?,
        closed_at: closed.map(parse_timestamp).transpose()?,
        capture_count: row.try_get("capture_count")?,
        note_count: row.try_get("note_count")?,
    })
}

fn validate_text(text: &str, name: &str, maximum: usize, allow_empty: bool) -> Result<()> {
    if (!allow_empty && text.is_empty()) || text.chars().count() > maximum {
        bail!(
            "{name} must contain {} to {maximum} characters",
            usize::from(!allow_empty)
        );
    }
    if text
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
    {
        bail!("{name} contains unsupported control characters");
    }
    Ok(())
}

fn decode_snapshot(payload: &str) -> Result<Snapshot> {
    let snapshot: Snapshot = serde_json::from_str(payload)
        .map_err(|_| anyhow::anyhow!("saved snapshot is damaged or incompatible"))?;
    validate_snapshot(&snapshot)?;
    Ok(snapshot)
}

fn incident_sources_compatible(left: &crate::model::Source, right: &crate::model::Source) -> bool {
    !left.endpoint.is_empty()
        && !left.database.is_empty()
        && left.database_oid > 0
        && left.endpoint == right.endpoint
        && left.database == right.database
        && left.database_oid == right.database_oid
        && match (&left.system_identifier, &right.system_identifier) {
            (Some(left), Some(right)) => left == right,
            _ => true,
        }
}

fn validate_snapshot(snapshot: &Snapshot) -> Result<()> {
    if !(1..=SNAPSHOT_VERSION).contains(&snapshot.schema_version) {
        bail!(
            "unsupported snapshot format version {} (supported 1 through {SNAPSHOT_VERSION})",
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
    let mut measurements = Vec::new();
    if let Some(database) = snapshot.health.database.available() {
        measurements.extend([database.blk_read_time_ms, database.blk_write_time_ms]);
    }
    if let Some(wal) = snapshot.health.wal.available() {
        measurements.push(wal.bytes);
    }
    if let Some(replication) = snapshot.health.replication.available() {
        measurements.extend(replication.replay_delay_seconds);
        measurements.extend(
            replication
                .senders
                .iter()
                .filter_map(|sender| sender.replay_lag_ms),
        );
    }
    if let Some(io) = snapshot.health.io.available() {
        for entry in io {
            measurements.extend(entry.read_time_ms);
            measurements.extend(entry.write_time_ms);
        }
    }
    if measurements.iter().any(|value| !value.is_finite()) {
        bail!("snapshot contains non-finite health statistics");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::tests::snapshot;

    #[tokio::test]
    async fn version_one_database_and_payload_migrate_without_rewriting_capture() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("v1.sqlite3");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true),
            )
            .await?;
        sqlx::query("CREATE TABLE snapshots (id INTEGER PRIMARY KEY AUTOINCREMENT, label TEXT NOT NULL, captured_at TEXT NOT NULL, source TEXT NOT NULL, complete INTEGER NOT NULL CHECK (complete IN (0, 1)), payload TEXT NOT NULL)")
            .execute(&pool).await?;
        sqlx::query("PRAGMA application_id = 1346851922")
            .execute(&pool)
            .await?;
        sqlx::query("PRAGMA user_version = 1")
            .execute(&pool)
            .await?;
        let mut value = serde_json::to_value(snapshot())?;
        value["schema_version"] = 1.into();
        value.as_object_mut().unwrap().remove("health");
        let payload = serde_json::to_string(&value)?;
        sqlx::query("INSERT INTO snapshots(id, label, captured_at, source, complete, payload) VALUES (42, 'old baseline', ?, 'localhost:5432 / app', 1, ?)")
            .bind(snapshot().completed_at.to_rfc3339()).bind(&payload).execute(&pool).await?;
        pool.close().await;
        let (left, right) = tokio::join!(Store::open(&path), Store::open(&path));
        let (store, other) = (left?, right?);
        assert_eq!(store.list().await?[0].id, 42);
        assert_eq!(store.list().await?[0].label, "old baseline");
        assert_eq!(store.load(42).await?.schema_version, 1);
        assert!(store.load(42).await?.health.database.available().is_none());
        let persisted: String = sqlx::query_scalar("SELECT payload FROM snapshots WHERE id = 42")
            .fetch_one(&other.pool)
            .await?;
        assert_eq!(persisted, payload);
        let incident = store.create_incident("legacy follow-up").await?;
        store.attach_capture(incident, 42).await?;
        assert_eq!(other.incident(incident).await?.summary.capture_count, 1);
        Ok(())
    }

    #[tokio::test]
    async fn incident_lifecycle_preserves_chronology_annotations_and_atomic_failures() -> Result<()>
    {
        let directory = tempfile::tempdir()?;
        let store = Store::open(&directory.path().join("history.sqlite3")).await?;
        let incident = store.create_incident(" Checkout latency ").await?;
        let mut before = snapshot();
        let mut after = before.clone();
        after.started_at += chrono::Duration::seconds(10);
        after.completed_at += chrono::Duration::seconds(10);
        let later = store
            .save_for_incident(&after, "later", Some(incident))
            .await?;
        let earlier = store
            .save_for_incident(&before, "earlier", Some(incident))
            .await?;
        store.attach_capture(incident, earlier).await?; // Attaching an existing member is idempotent.
        store.add_note(incident, " Raised pool limit ").await?;
        store
            .add_note(incident, "Latency returned to baseline")
            .await?;
        let loaded = store.incident(incident).await?;
        assert_eq!(loaded.summary.title, "Checkout latency");
        assert_eq!(loaded.summary.capture_count, 2);
        assert_eq!(loaded.summary.note_count, 2);
        assert_eq!(
            loaded
                .captures
                .iter()
                .map(|capture| capture.id)
                .collect::<Vec<_>>(),
            [earlier, later]
        );
        assert!(loaded.notes[0].created_at <= loaded.notes[1].created_at);
        assert_eq!(loaded.notes[0].text, "Raised pool limit");
        store
            .annotate_capture(earlier, Some(" baseline "), Some(" before mitigation "))
            .await?;
        assert_eq!(
            store.capture_metadata(earlier).await?,
            CaptureMetadata {
                label: "baseline".into(),
                note: "before mitigation".into()
            }
        );
        assert_eq!(
            store
                .incident(incident)
                .await?
                .capture_notes
                .get(&earlier)
                .map(String::as_str),
            Some("before mitigation")
        );
        store.annotate_capture(earlier, None, Some("")).await?;
        assert_eq!(store.capture_metadata(earlier).await?.note, "");
        store.set_incident_closed(incident, true).await?;
        let closed = store.incident(incident).await?.summary.closed_at;
        store.set_incident_closed(incident, true).await?;
        assert_eq!(store.incident(incident).await?.summary.closed_at, closed);
        assert!(store.add_note(incident, "must reopen").await.is_err());
        assert!(store.detach_capture(incident, earlier).await.is_err());
        assert!(
            store
                .save_for_incident(&before, "must not persist", Some(incident))
                .await
                .is_err()
        );
        assert_eq!(store.list().await?.len(), 2);
        store.set_incident_closed(incident, false).await?;
        assert!(store.incident(incident).await?.summary.closed_at.is_none());
        store.detach_capture(incident, earlier).await?;
        assert_eq!(store.incident(incident).await?.summary.capture_count, 1);
        before.source.database = "different-target".into();
        assert!(
            store
                .save_for_incident(&before, "must not persist", Some(incident))
                .await
                .is_err()
        );
        assert_eq!(store.list().await?.len(), 2);
        store.delete(later).await?;
        assert_eq!(store.incident(incident).await?.summary.capture_count, 0);
        assert_eq!(store.incidents().await?[0].note_count, 2);
        Ok(())
    }

    #[tokio::test]
    async fn incident_targets_allow_restart_but_reject_different_cluster_and_invalid_metadata()
    -> Result<()> {
        let directory = tempfile::tempdir()?;
        let store = Store::open(&directory.path().join("history.sqlite3")).await?;
        let incident = store.create_incident("server restart").await?;
        let mut observation = snapshot();
        let capture = store
            .save_for_incident(&observation, "baseline", Some(incident))
            .await?;
        observation.source.server_started_at += chrono::Duration::minutes(1);
        store
            .save_for_incident(&observation, "restarted", Some(incident))
            .await?;
        observation.source.system_identifier = Some("a different physical cluster".into());
        assert!(
            store
                .save_for_incident(&observation, "wrong cluster", Some(incident))
                .await
                .is_err()
        );
        observation.source.system_identifier = None;
        observation.source.database_oid += 1;
        assert!(
            store
                .save_for_incident(&observation, "recreated database", Some(incident))
                .await
                .is_err()
        );
        assert_eq!(store.incident(incident).await?.summary.capture_count, 2);
        assert!(store.create_incident("  ").await.is_err());
        assert!(store.create_incident(&"x".repeat(201)).await.is_err());
        assert!(store.add_note(incident, "\u{1b}[31m").await.is_err());
        assert!(store.add_note(incident, &"x".repeat(10_001)).await.is_err());
        assert!(
            store
                .annotate_capture(capture, Some(&"x".repeat(201)), None)
                .await
                .is_err()
        );
        assert!(
            store
                .annotate_capture(999, Some("unknown"), None)
                .await
                .is_err()
        );
        assert!(store.add_note(999, "unknown").await.is_err());
        assert!(store.set_incident_closed(999, true).await.is_err());
        assert!(store.incident(999).await.is_err());
        Ok(())
    }

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
        let mut observation = crate::demo::snapshot(0, false);
        if let crate::model::Observation::Available(wal) = &mut observation.health.wal {
            wal.bytes = f64::NAN;
        }
        assert!(
            store
                .save(&observation, "invalid numeric metric")
                .await
                .is_err()
        );
        assert_eq!(store.list().await?.len(), 1);
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
