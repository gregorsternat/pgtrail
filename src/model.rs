//! Serializable observations, independent of database drivers and the terminal.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub(crate) const SNAPSHOT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", content = "data", rename_all = "snake_case")]
pub(crate) enum Observation<T> {
    Available(T),
    Unavailable(String),
}

impl<T> Observation<T> {
    pub(crate) fn available(&self) -> Option<&T> {
        match self {
            Self::Available(value) => Some(value),
            Self::Unavailable(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Source {
    /// Host/socket and port only, never a URL, username, or password.
    pub(crate) endpoint: String,
    pub(crate) database: String,
    pub(crate) database_oid: i64,
    pub(crate) server_version: String,
    pub(crate) server_started_at: DateTime<Utc>,
    pub(crate) system_identifier: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Snapshot {
    pub(crate) schema_version: u32,
    pub(crate) started_at: DateTime<Utc>,
    pub(crate) completed_at: DateTime<Utc>,
    pub(crate) source: Source,
    pub(crate) activity: Observation<Vec<Session>>,
    pub(crate) statements: Observation<StatementStats>,
    pub(crate) warnings: Vec<String>,
}

impl Snapshot {
    pub(crate) fn is_complete(&self) -> bool {
        self.activity.available().is_some()
            && self.statements.available().is_some_and(|s| !s.truncated)
            && self.warnings.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Session {
    pub(crate) pid: i32,
    pub(crate) backend_start: Option<DateTime<Utc>>,
    pub(crate) user: Option<String>,
    pub(crate) database: Option<String>,
    pub(crate) application: String,
    pub(crate) client: Option<String>,
    pub(crate) state: Option<String>,
    /// Elapsed age only for a currently active query; never cumulative time.
    pub(crate) query_age_ms: Option<i64>,
    pub(crate) transaction_age_ms: Option<i64>,
    pub(crate) wait_event_type: Option<String>,
    pub(crate) wait_event: Option<String>,
    pub(crate) blockers: Vec<i32>,
    /// SQL text is collected only by explicit opt-in.
    pub(crate) query: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct StatementStats {
    pub(crate) reset_at: Option<DateTime<Utc>>,
    pub(crate) dealloc: Option<i64>,
    pub(crate) truncated: bool,
    pub(crate) entries: Vec<Statement>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Statement {
    pub(crate) userid: i64,
    pub(crate) dbid: i64,
    pub(crate) queryid: i64,
    pub(crate) toplevel: bool,
    pub(crate) stats_since: Option<DateTime<Utc>>,
    pub(crate) calls: i64,
    pub(crate) total_exec_ms: f64,
    pub(crate) mean_exec_ms: f64,
    pub(crate) rows: i64,
    pub(crate) shared_blks_hit: i64,
    pub(crate) shared_blks_read: i64,
    pub(crate) temp_blks_written: i64,
    pub(crate) query: Option<String>,
}
