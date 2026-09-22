//! Private connection profiles contain connection coordinates, never passwords or URLs.
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

const VERSION: u32 = 1;
const MAX_FILE_BYTES: u64 = 128 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Profile {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) database: String,
    pub(crate) user: String,
    pub(crate) sslmode: String,
    pub(crate) sslrootcert: Option<PathBuf>,
    pub(crate) password_env: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Profiles {
    version: u32,
    profiles: BTreeMap<String, Profile>,
}

impl Default for Profiles {
    fn default() -> Self {
        Self {
            version: VERSION,
            profiles: BTreeMap::new(),
        }
    }
}

impl Profiles {
    pub(crate) fn get(&self, name: &str) -> Result<&Profile> {
        validate_name(name)?;
        self.profiles
            .get(name)
            .context("connection profile does not exist")
    }

    pub(crate) fn add(&mut self, name: &str, profile: Profile) -> Result<()> {
        validate_name(name)?;
        profile.validate()?;
        if self.profiles.contains_key(name) {
            bail!("connection profile already exists; remove it before replacing it");
        }
        if self.profiles.len() >= 100 {
            bail!("at most 100 connection profiles can be saved");
        }
        self.profiles.insert(name.to_owned(), profile);
        Ok(())
    }

    pub(crate) fn remove(&mut self, name: &str) -> Result<()> {
        validate_name(name)?;
        if self.profiles.remove(name).is_none() {
            bail!("connection profile does not exist");
        }
        Ok(())
    }

    pub(crate) fn list(&self) -> &BTreeMap<String, Profile> {
        &self.profiles
    }

    fn validate(&self) -> Result<()> {
        if self.version != VERSION {
            bail!("connection profile file has an unsupported version");
        }
        if self.profiles.len() > 100 {
            bail!("connection profile file contains too many profiles");
        }
        for (name, profile) in &self.profiles {
            validate_name(name)?;
            profile.validate()?;
        }
        Ok(())
    }
}

impl Profile {
    fn validate(&self) -> Result<()> {
        if self.host.is_empty()
            || self.host.len() > 1024
            || self
                .host
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '@' | '?' | '#' | '\\'))
        {
            bail!("profile host must be a hostname, IP address, or absolute Unix socket directory");
        }
        if self.host.contains('/') && !self.host.starts_with('/') {
            bail!("profile host must not contain a connection URL or relative path");
        }
        if !self.host.starts_with('/') {
            let host = if self.host.contains(':') && !self.host.starts_with('[') {
                format!("[{}]", self.host)
            } else {
                self.host.clone()
            };
            url::Host::parse(&host).map_err(|_| anyhow::anyhow!("profile host is invalid"))?;
        }
        if self.port == 0 {
            bail!("profile port must be between 1 and 65535");
        }
        for (name, value) in [("database", &self.database), ("user", &self.user)] {
            if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
                bail!("profile {name} must contain 1 to 256 bytes without control characters");
            }
        }
        if !matches!(
            self.sslmode.as_str(),
            "disable" | "allow" | "prefer" | "require" | "verify-ca" | "verify-full"
        ) {
            bail!(
                "profile sslmode must be disable, allow, prefer, require, verify-ca, or verify-full"
            );
        }
        if let Some(path) = &self.sslrootcert {
            let value = path
                .to_str()
                .context("profile certificate path must be valid UTF-8")?;
            if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
                bail!("profile certificate path is invalid");
            }
        }
        if let Some(variable) = &self.password_env {
            let mut characters = variable.chars();
            if variable.len() > 128
                || !characters
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                || !characters.all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                bail!("profile password environment variable name is invalid");
            }
        }
        Ok(())
    }

    pub(crate) fn connection_url(&self) -> Result<String> {
        self.connection_url_with(|name| std::env::var(name).ok())
    }

    fn connection_url_with(&self, lookup: impl FnOnce(&str) -> Option<String>) -> Result<String> {
        self.validate()?;
        let mut url = url::Url::parse("postgresql://localhost")
            .context("could not initialize profile connection")?;
        if self.host.starts_with('/') {
            url.query_pairs_mut().append_pair("host", &self.host);
        } else {
            let host = if self.host.contains(':') && !self.host.starts_with('[') {
                format!("[{}]", self.host)
            } else {
                self.host.clone()
            };
            url.set_host(Some(&host))
                .map_err(|_| anyhow::anyhow!("profile host is invalid"))?;
        }
        url.set_port(Some(self.port))
            .map_err(|_| anyhow::anyhow!("profile port is invalid"))?;
        url.set_username(&self.user)
            .map_err(|_| anyhow::anyhow!("profile user is invalid"))?;
        // A path segment setter escapes '/', '?' and '#' in database names correctly.
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("profile database is invalid"))?
            .clear()
            .push(&self.database);
        url.query_pairs_mut().append_pair("sslmode", &self.sslmode);
        if let Some(path) = &self.sslrootcert {
            url.query_pairs_mut().append_pair(
                "sslrootcert",
                path.to_str()
                    .context("profile certificate path must be valid UTF-8")?,
            );
        }
        if let Some(name) = &self.password_env {
            let password = lookup(name).with_context(|| {
                format!(
                    "profile password environment variable {name} is not set or is not valid UTF-8"
                )
            })?;
            url.set_password(Some(&password))
                .map_err(|_| anyhow::anyhow!("could not apply profile password"))?;
        }
        Ok(url.into())
    }
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        bail!("profile name must contain 1 to 64 ASCII letters, digits, '.', '_' or '-'");
    }
    Ok(())
}

pub(crate) async fn load(path: &Path) -> Result<Profiles> {
    check_path(path).await?;
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Profiles::default());
        }
        Err(_) => bail!("could not inspect connection profile file"),
    };
    validate_file(&metadata)?;
    if metadata.len() > MAX_FILE_BYTES {
        bail!("connection profile file is too large");
    }
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|_| anyhow::anyhow!("could not read connection profiles"))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        bail!("connection profile file is too large");
    }
    // Deserializer errors may repeat arbitrary private values. Keep these errors generic.
    let profiles: Profiles = serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("connection profile file is invalid; expected versioned profiles without passwords, URLs, or unknown fields"))?;
    profiles.validate()?;
    Ok(profiles)
}

pub(crate) async fn save(path: &Path, profiles: &Profiles) -> Result<()> {
    profiles.validate()?;
    check_path(path).await?;
    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => validate_file(&metadata)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => bail!("could not inspect connection profile file"),
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut directories = tokio::fs::DirBuilder::new();
    directories.recursive(true);
    #[cfg(unix)]
    directories.mode(0o700);
    directories
        .create(parent)
        .await
        .context("could not create profile directory")?;
    check_path(path).await?;
    let bytes =
        serde_json::to_vec_pretty(profiles).context("could not serialize connection profiles")?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        bail!("connection profile file is too large");
    }
    // Exclusive creation and atomic rename prevent partially written config files.
    let mut temporary = None;
    for attempt in 0..10 {
        let candidate = parent.join(format!(
            ".pgtrail-profiles-{}-{}-{attempt}.tmp",
            std::process::id(),
            chrono::Utc::now().timestamp_micros()
        ));
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        match options.open(&candidate).await {
            Ok(file) => {
                temporary = Some((candidate, file));
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => bail!("could not create private profile file"),
        }
    }
    let (temporary_path, file) = temporary.context("could not create private profile file")?;
    // The std file supports write_all without adding Tokio's optional io-util dependency.
    let mut file = file.into_std().await;
    let outcome = (|| -> Result<()> {
        use std::io::Write;
        file.write_all(&bytes)
            .context("could not write connection profiles")?;
        file.sync_all()
            .context("could not synchronize connection profiles")?;
        Ok(())
    })();
    drop(file);
    if outcome.is_err() {
        let _ = tokio::fs::remove_file(&temporary_path).await;
        return outcome;
    }
    if tokio::fs::rename(&temporary_path, path).await.is_err() {
        let _ = tokio::fs::remove_file(&temporary_path).await;
        bail!("could not replace connection profile file");
    }
    Ok(())
}

async fn check_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() || path.file_name().is_none() {
        bail!("connection profiles require a file path");
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        if matches!(component, Component::ParentDir) {
            bail!("connection profile path must not contain '..'");
        }
        current.push(component.as_os_str());
        match tokio::fs::symlink_metadata(&current).await {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!("connection profile path must not contain symbolic links")
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => bail!("could not inspect connection profile path"),
        }
    }
    Ok(())
}

fn validate_file(metadata: &std::fs::Metadata) -> Result<()> {
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        bail!("connection profiles must be a regular file");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            bail!("connection profile file must be private; restrict its permissions to 0600");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> Profile {
        Profile {
            host: "db.example.test".into(),
            port: 5432,
            database: "app".into(),
            user: "monitor".into(),
            sslmode: "verify-full".into(),
            sslrootcert: None,
            password_env: Some("TEST_PASSWORD".into()),
        }
    }

    #[test]
    fn password_is_resolved_only_at_connection_time_and_escaped() -> Result<()> {
        let profile = profile();
        let json = serde_json::to_string(&profile)?;
        let secret = "secret@/#? with spaces";
        assert!(!json.contains(secret));
        let url = url::Url::parse(&profile.connection_url_with(|name| {
            assert_eq!(name, "TEST_PASSWORD");
            Some(secret.into())
        })?)?;
        assert_eq!(url.host_str(), Some("db.example.test"));
        assert_eq!(url.username(), "monitor");
        assert!(url.password().unwrap().contains("%40"));
        let error = profile
            .connection_url_with(|_| None)
            .unwrap_err()
            .to_string();
        assert!(!error.contains(secret));
        Ok(())
    }

    #[test]
    fn profiles_support_socket_and_ipv6_without_url_injection() -> Result<()> {
        let mut profile = profile();
        profile.password_env = None;
        profile.host = "::1".into();
        profile.database = "app/blue?test#x".into();
        let url = profile.connection_url_with(|_| unreachable!())?;
        assert!(url.contains("[::1]"));
        assert!(url.contains("app%2Fblue%3Ftest%23x"));
        profile.host = "/var/run/postgresql".into();
        let url = url::Url::parse(&profile.connection_url_with(|_| unreachable!())?)?;
        assert!(
            url.query_pairs()
                .any(|(key, value)| key == "host" && value == "/var/run/postgresql")
        );
        for host in [
            "postgresql://user:secret@host",
            "db?password=secret",
            "user@host",
        ] {
            profile.host = host.into();
            assert!(profile.validate().is_err());
        }
        Ok(())
    }

    #[tokio::test]
    async fn profiles_round_trip_privately_and_unknown_secret_fields_fail_without_echo()
    -> Result<()> {
        let directory = tempfile::tempdir()?;
        let directory = directory.path().canonicalize()?;
        let path = directory.join("settings/profiles.json");
        let mut profiles = load(&path).await?;
        profiles.add("production", profile())?;
        assert!(profiles.add("production", profile()).is_err());
        save(&path, &profiles).await?;
        assert_eq!(load(&path).await?, profiles);
        assert_eq!(profiles.list().len(), 1);
        profiles.remove("production")?;
        assert!(profiles.get("production").is_err());
        save(&path, &profiles).await?;
        assert!(load(&path).await?.list().is_empty());
        tokio::fs::write(
            &path,
            r#"{"version":1,"profiles":{},"password":"DO_NOT_ECHO"}"#,
        )
        .await?;
        let error = load(&path).await.unwrap_err().to_string();
        assert!(!error.contains("DO_NOT_ECHO"));
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn links_and_public_permissions_are_rejected() -> Result<()> {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let directory = tempfile::tempdir()?;
        let directory = directory.path().canonicalize()?;
        let path = directory.join("profiles.json");
        save(&path, &Profiles::default()).await?;
        assert_eq!(
            std::fs::metadata(&path)?.permissions().mode() & 0o777,
            0o600
        );
        let link = directory.join("link.json");
        symlink(&path, &link)?;
        assert!(load(&link).await.is_err());
        assert!(save(&link, &Profiles::default()).await.is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))?;
        assert!(load(&path).await.is_err());
        assert!(save(&path, &Profiles::default()).await.is_err());
        Ok(())
    }
}
