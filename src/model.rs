//! Serializable observations, independent of database drivers and the terminal.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub(crate) const SNAPSHOT_VERSION: u32 = 2;

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
    #[serde(default)]
    pub(crate) health: Health,
}

impl Snapshot {
    pub(crate) fn is_complete(&self) -> bool {
        self.activity.available().is_some()
            && self.statements.available().is_some_and(|s| !s.truncated)
            && self.warnings.is_empty()
            && (self.schema_version < 2 || self.health.is_complete())
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

/// Additional observations are optional for older captures and restricted roles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub(crate) struct Health {
    pub(crate) database: Observation<DatabaseStats>,
    pub(crate) tables: Observation<RelationStats>,
    pub(crate) replication: Observation<ReplicationStats>,
    pub(crate) wal: Observation<WalStats>,
    pub(crate) io: Observation<Vec<IoStats>>,
    pub(crate) vacuum: Observation<Vec<VacuumProgress>>,
}

impl<T> Default for Observation<T> {
    fn default() -> Self {
        Self::Unavailable("Not collected in this snapshot".into())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct DatabaseStats {
    #[serde(deserialize_with = "required_optional_timestamp")]
    pub(crate) stats_reset: Option<DateTime<Utc>>,
    pub(crate) size_bytes: i64,
    pub(crate) num_backends: i64,
    pub(crate) cluster_backends: i64,
    pub(crate) max_connections: i64,
    pub(crate) reserved_connections: i64,
    pub(crate) xact_commit: i64,
    pub(crate) xact_rollback: i64,
    pub(crate) blks_read: i64,
    pub(crate) blks_hit: i64,
    pub(crate) tup_returned: i64,
    pub(crate) tup_fetched: i64,
    pub(crate) tup_inserted: i64,
    pub(crate) tup_updated: i64,
    pub(crate) tup_deleted: i64,
    pub(crate) conflicts: i64,
    pub(crate) deadlocks: i64,
    pub(crate) temp_files: i64,
    pub(crate) temp_bytes: i64,
    pub(crate) blk_read_time_ms: f64,
    pub(crate) blk_write_time_ms: f64,
    pub(crate) frozen_xid_age: i64,
    pub(crate) autovacuum: bool,
    pub(crate) track_counts: bool,
    pub(crate) track_io_timing: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct RelationStats {
    pub(crate) tables: Vec<TableStats>,
    pub(crate) indexes: Vec<IndexStats>,
    pub(crate) truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct TableStats {
    pub(crate) oid: i64,
    pub(crate) schema: String,
    pub(crate) name: String,
    pub(crate) total_bytes: i64,
    pub(crate) table_bytes: i64,
    pub(crate) index_bytes: i64,
    pub(crate) seq_scan: i64,
    pub(crate) idx_scan: Option<i64>,
    pub(crate) live_tuples: i64,
    pub(crate) dead_tuples: i64,
    pub(crate) modified_since_analyze: i64,
    pub(crate) inserts: i64,
    pub(crate) updates: i64,
    pub(crate) deletes: i64,
    pub(crate) last_vacuum: Option<DateTime<Utc>>,
    pub(crate) last_autovacuum: Option<DateTime<Utc>>,
    pub(crate) last_analyze: Option<DateTime<Utc>>,
    pub(crate) last_autoanalyze: Option<DateTime<Utc>>,
    pub(crate) frozen_xid_age: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct IndexStats {
    pub(crate) oid: i64,
    pub(crate) table_oid: i64,
    pub(crate) schema: String,
    pub(crate) table: String,
    pub(crate) name: String,
    pub(crate) size_bytes: i64,
    pub(crate) scans: i64,
    pub(crate) tuples_read: i64,
    pub(crate) tuples_fetched: i64,
    pub(crate) unique: bool,
    pub(crate) primary: bool,
    pub(crate) valid: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ReplicationStats {
    pub(crate) in_recovery: bool,
    pub(crate) replay_delay_seconds: Option<f64>,
    pub(crate) receive_replay_lag_bytes: Option<i64>,
    pub(crate) senders: Vec<Replica>,
    pub(crate) slots: Vec<ReplicationSlot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Replica {
    pub(crate) pid: i32,
    pub(crate) application: String,
    pub(crate) client: Option<String>,
    pub(crate) state: Option<String>,
    pub(crate) sync_state: Option<String>,
    pub(crate) sent_replay_lag_bytes: Option<i64>,
    pub(crate) replay_lag_ms: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ReplicationSlot {
    pub(crate) name: String,
    pub(crate) slot_type: String,
    pub(crate) database: Option<String>,
    pub(crate) active: bool,
    pub(crate) retained_bytes: Option<i64>,
    pub(crate) wal_status: Option<String>,
    pub(crate) safe_wal_size: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct WalStats {
    pub(crate) stats_reset: Option<DateTime<Utc>>,
    pub(crate) records: i64,
    pub(crate) full_page_images: i64,
    pub(crate) bytes: f64,
    pub(crate) buffers_full: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct IoStats {
    pub(crate) backend_type: String,
    pub(crate) object: String,
    pub(crate) context: String,
    pub(crate) reads: Option<i64>,
    pub(crate) writes: Option<i64>,
    pub(crate) read_time_ms: Option<f64>,
    pub(crate) write_time_ms: Option<f64>,
    pub(crate) hits: Option<i64>,
    pub(crate) evictions: Option<i64>,
    pub(crate) fsyncs: Option<i64>,
    pub(crate) stats_reset: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct VacuumProgress {
    pub(crate) pid: i32,
    pub(crate) table_oid: i64,
    pub(crate) phase: String,
    pub(crate) heap_blocks_total: i64,
    pub(crate) heap_blocks_scanned: i64,
    pub(crate) heap_blocks_vacuumed: i64,
}

impl Health {
    pub(crate) fn is_complete(&self) -> bool {
        self.database.available().is_some()
            && self.tables.available().is_some_and(|v| !v.truncated)
            && self.replication.available().is_some()
            && self.wal.available().is_some()
            && self.io.available().is_some()
            && self.vacuum.available().is_some()
    }
}

// A present SQL NULL denotes the initial, unreset database epoch; a missing JSON
// field must not silently become that evidence when loading a capture.
fn required_optional_timestamp<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<DateTime<Utc>>::deserialize(deserializer)
}
