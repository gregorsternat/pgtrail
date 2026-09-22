use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    version,
    about,
    long_about = "Read-only PostgreSQL investigation, local snapshots, and offline comparisons.\n\nRun with --demo to explore without a database. Set PGTRAIL_DATABASE_URL for a live connection. SQL text is excluded unless explicitly enabled."
)]
pub(crate) struct Cli {
    /// PostgreSQL URL; prefer PGTRAIL_DATABASE_URL to keep credentials out of shell history
    #[arg(long, global = true, conflicts_with = "demo")]
    pub(crate) database_url: Option<String>,
    /// Named connection profile; passwords are resolved from its environment variable
    #[arg(long, global = true, conflicts_with_all = ["demo", "database_url"])]
    pub(crate) profile: Option<String>,
    /// Private JSON connection profile file
    #[arg(long, global = true)]
    pub(crate) profiles_file: Option<PathBuf>,
    /// Attach new captures to an existing open incident
    #[arg(long, global = true, value_parser = clap::value_parser!(i64).range(1..))]
    pub(crate) incident: Option<i64>,
    /// Explore synthetic incident data without connecting to PostgreSQL
    #[arg(long, global = true)]
    pub(crate) demo: bool,
    /// SQLite history path (default: platform user data directory)
    #[arg(long, global = true, value_name = "PATH")]
    pub(crate) store: Option<PathBuf>,
    /// Seconds between live refreshes
    #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u64).range(1..=3600), global = true)]
    pub(crate) refresh: u64,
    /// Maximum seconds for a connection or collection
    #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u64).range(1..=120), global = true)]
    pub(crate) timeout: u64,
    /// Include potentially sensitive SQL in the UI, captures, and exports
    #[arg(long, global = true)]
    pub(crate) include_query_text: bool,
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Collect once and print a diagnostic report without storing a capture
    Check {
        #[arg(long, value_enum, default_value_t = Format::Markdown)]
        format: Format,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Collect a fresh snapshot and save it locally
    Capture {
        #[arg(long, default_value = "Manual capture")]
        label: String,
    },
    /// List saved captures without a database connection
    Snapshots {
        #[arg(long)]
        json: bool,
    },
    /// Inspect or export one saved capture offline
    Show {
        #[arg(value_parser = clap::value_parser!(i64).range(1..))]
        id: i64,
        #[arg(long, value_enum, default_value_t = Format::Markdown)]
        format: Format,
        /// Create an export file; refuses to overwrite an existing file
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Compare an earlier capture with a later capture offline
    Compare {
        #[arg(value_parser = clap::value_parser!(i64).range(1..))]
        before: i64,
        #[arg(value_parser = clap::value_parser!(i64).range(1..))]
        after: i64,
        #[arg(long, value_enum, default_value_t = Format::Markdown)]
        format: Format,
        /// Create an export file; refuses to overwrite an existing file
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Explain current findings using two samples and valid interval metrics
    Diagnose {
        #[arg(long, default_value_t = 2, value_parser = clap::value_parser!(u64).range(1..=120))]
        sample_seconds: u64,
        #[arg(long, value_enum, default_value_t = Format::Markdown)]
        format: Format,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long, value_parser = ["warning", "critical"])]
        fail_on: Option<String>,
    },
    /// Collect a bounded investigation session in the foreground
    Record {
        #[arg(long, default_value_t = 12, value_parser = clap::value_parser!(u32).range(1..=3600))]
        count: u32,
        #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u64).range(1..=3600))]
        interval: u64,
        #[arg(long, default_value = "Recorded observation")]
        label: String,
    },
    /// Add a note or rename a saved capture
    Annotate {
        #[arg(value_parser = clap::value_parser!(i64).range(1..))]
        id: i64,
        #[arg(long, required_unless_present = "note")]
        label: Option<String>,
        #[arg(long, required_unless_present = "label")]
        note: Option<String>,
    },
    /// Organize captures and timestamped notes into local incident dossiers
    Incident {
        #[command(subcommand)]
        command: IncidentCommand,
    },
    /// Manage named connection profiles without storing passwords
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
    /// Permanently remove a saved capture by ID
    Delete {
        #[arg(value_parser = clap::value_parser!(i64).range(1..))]
        id: i64,
    },
}

#[derive(Subcommand)]
pub(crate) enum IncidentCommand {
    Create {
        title: String,
    },
    List {
        #[arg(long)]
        json: bool,
    },
    Show {
        #[arg(value_parser = clap::value_parser!(i64).range(1..))]
        id: i64,
        #[arg(long, value_enum, default_value_t = Format::Markdown)]
        format: Format,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Note {
        #[arg(value_parser = clap::value_parser!(i64).range(1..))]
        id: i64,
        text: String,
    },
    Attach {
        #[arg(value_parser = clap::value_parser!(i64).range(1..))]
        id: i64,
        #[arg(value_parser = clap::value_parser!(i64).range(1..))]
        capture: i64,
    },
    Detach {
        #[arg(value_parser = clap::value_parser!(i64).range(1..))]
        id: i64,
        #[arg(value_parser = clap::value_parser!(i64).range(1..))]
        capture: i64,
    },
    Close {
        #[arg(value_parser = clap::value_parser!(i64).range(1..))]
        id: i64,
    },
    Reopen {
        #[arg(value_parser = clap::value_parser!(i64).range(1..))]
        id: i64,
    },
}

#[derive(Subcommand)]
pub(crate) enum ProfileCommand {
    List,
    Add {
        name: String,
        #[arg(long)]
        host: String,
        #[arg(long, default_value_t = 5432, value_parser = clap::value_parser!(u16).range(1..))]
        port: u16,
        #[arg(long)]
        database: String,
        #[arg(long)]
        user: String,
        #[arg(long, default_value = "verify-full", value_parser = ["disable", "allow", "prefer", "require", "verify-ca", "verify-full"])]
        sslmode: String,
        #[arg(long)]
        sslrootcert: Option<PathBuf>,
        #[arg(long)]
        password_env: Option<String>,
    },
    Remove {
        name: String,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum Format {
    Markdown,
    Json,
}

impl Cli {
    pub(crate) fn database_url(&self) -> Option<String> {
        if self.demo {
            return None;
        }
        self.database_url.clone().or_else(|| {
            std::env::var("PGTRAIL_DATABASE_URL")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
    }

    pub(crate) fn profiles_path(&self) -> anyhow::Result<PathBuf> {
        if let Some(path) = &self.profiles_file {
            return Ok(path.clone());
        }
        if let Some(path) = std::env::var_os("PGTRAIL_PROFILES").filter(|v| !v.is_empty()) {
            return Ok(path.into());
        }
        if let Some(path) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
            let path = PathBuf::from(path);
            if path.is_absolute() {
                return Ok(path.join("pgtrail/profiles.json"));
            }
        }
        let home = std::env::var_os("HOME")
            .filter(|v| !v.is_empty())
            .ok_or_else(|| anyhow::anyhow!("use --profiles-file to select a profile file"))?;
        Ok(PathBuf::from(home).join(".config/pgtrail/profiles.json"))
    }

    pub(crate) fn store_path(&self) -> anyhow::Result<PathBuf> {
        if let Some(path) = &self.store {
            return Ok(path.clone());
        }
        if let Some(path) = std::env::var_os("PGTRAIL_STORE").filter(|s| !s.is_empty()) {
            return Ok(PathBuf::from(path));
        }
        if let Some(path) = std::env::var_os("XDG_DATA_HOME").filter(|s| !s.is_empty()) {
            let path = PathBuf::from(path);
            if path.is_absolute() {
                return Ok(path.join("pgtrail/history.sqlite3"));
            }
        }
        let home = std::env::var_os("HOME")
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!("could not locate the user data directory; pass --store PATH")
            })?;
        let base = if cfg!(target_os = "macos") {
            "Library/Application Support"
        } else {
            ".local/share"
        };
        Ok(PathBuf::from(home)
            .join(base)
            .join("pgtrail/history.sqlite3"))
    }
}
