//! Conservative, deterministic comparison of two observations; no database access.
use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::model::{
    IndexStats, Observation, Replica, ReplicationSlot, Session, Snapshot, Source, Statement,
    StatementStats, TableStats,
};
use crate::{
    diagnostics::{self, Analysis},
    metrics::{self, IntervalMetrics},
};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Comparison {
    pub(crate) health: HealthComparison,
    pub(crate) analysis: Analysis,
    pub(crate) before: DateTime<Utc>,
    pub(crate) after: DateTime<Utc>,
    pub(crate) source: Source,
    pub(crate) after_source: Source,
    pub(crate) source_compatible: bool,
    pub(crate) interval_ms: Option<i64>,
    pub(crate) warnings: Vec<String>,
    pub(crate) sessions: Observation<SessionComparison>,
    pub(crate) blocking: Observation<BlockingComparison>,
    pub(crate) statements: Observation<StatementComparison>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) struct SessionIdentity {
    pub(crate) pid: i32,
    pub(crate) backend_start: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SessionChange {
    pub(crate) identity: SessionIdentity,
    pub(crate) before: Session,
    pub(crate) after: Session,
    pub(crate) fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SessionComparison {
    pub(crate) added: Vec<Session>,
    pub(crate) removed: Vec<Session>,
    pub(crate) changed: Vec<SessionChange>,
    pub(crate) unchanged: usize,
    pub(crate) unidentifiable_before: Vec<Session>,
    pub(crate) unidentifiable_after: Vec<Session>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) struct BlockingEdge {
    pub(crate) waiter: SessionIdentity,
    pub(crate) blocker: SessionIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) struct UnresolvedBlockingEdge {
    pub(crate) waiter_pid: i32,
    pub(crate) blocker_pid: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct BlockingComparison {
    pub(crate) added: Vec<BlockingEdge>,
    pub(crate) removed: Vec<BlockingEdge>,
    pub(crate) unchanged: usize,
    pub(crate) unresolved_before: Vec<UnresolvedBlockingEdge>,
    pub(crate) unresolved_after: Vec<UnresolvedBlockingEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub(crate) struct StatementIdentity {
    pub(crate) userid: i64,
    pub(crate) dbid: i64,
    pub(crate) queryid: i64,
    pub(crate) toplevel: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct StatementDelta {
    pub(crate) calls: i64,
    pub(crate) total_exec_ms: f64,
    /// The interval mean is delta(total execution time) / delta(calls).
    pub(crate) mean_exec_ms: Option<f64>,
    pub(crate) rows: i64,
    pub(crate) shared_blks_hit: i64,
    pub(crate) shared_blks_read: i64,
    pub(crate) temp_blks_written: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct StatementChange {
    pub(crate) identity: StatementIdentity,
    pub(crate) before: Option<Statement>,
    pub(crate) after: Option<Statement>,
    pub(crate) delta: Observation<StatementDelta>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct StatementComparison {
    pub(crate) entries: Vec<StatementChange>,
    pub(crate) total: Observation<StatementDelta>,
    pub(crate) before_truncated: bool,
    pub(crate) after_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct HealthComparison {
    pub(crate) database: Observation<DatabaseChange>,
    pub(crate) relations: Observation<RelationComparison>,
    pub(crate) replication: Observation<ReplicationComparison>,
    pub(crate) rates: IntervalMetrics,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct DatabaseChange {
    pub(crate) size_bytes_before: i64,
    pub(crate) size_bytes_after: i64,
    pub(crate) size_delta_bytes: Observation<i64>,
    pub(crate) connections_before: i64,
    pub(crate) connections_after: i64,
    pub(crate) cluster_connections_before: i64,
    pub(crate) cluster_connections_after: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct RelationComparison {
    pub(crate) tables: Vec<TableChange>,
    pub(crate) indexes: Vec<IndexChange>,
    pub(crate) before_truncated: bool,
    pub(crate) after_truncated: bool,
    pub(crate) caveats: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct TableChange {
    pub(crate) oid: i64,
    pub(crate) before: Option<TableStats>,
    pub(crate) after: Option<TableStats>,
    pub(crate) size_delta_bytes: Observation<i64>,
    pub(crate) estimated_live_tuple_change: Observation<i64>,
    pub(crate) estimated_dead_tuple_change: Observation<i64>,
    pub(crate) sequential_scans: Observation<i64>,
    pub(crate) index_scans: Observation<i64>,
    pub(crate) inserts: Observation<i64>,
    pub(crate) updates: Observation<i64>,
    pub(crate) deletes: Observation<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct IndexChange {
    pub(crate) oid: i64,
    pub(crate) before: Option<IndexStats>,
    pub(crate) after: Option<IndexStats>,
    pub(crate) size_delta_bytes: Observation<i64>,
    pub(crate) scans: Observation<i64>,
    pub(crate) tuples_read: Observation<i64>,
    pub(crate) tuples_fetched: Observation<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct ReplicationComparison {
    pub(crate) in_recovery_before: bool,
    pub(crate) in_recovery_after: bool,
    pub(crate) received_replay_backlog_delta_bytes: Observation<i64>,
    pub(crate) senders_before: Vec<Replica>,
    pub(crate) senders_after: Vec<Replica>,
    pub(crate) slots: Vec<SlotChange>,
    pub(crate) caveats: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SlotChange {
    pub(crate) name: String,
    pub(crate) before: Option<ReplicationSlot>,
    pub(crate) after: Option<ReplicationSlot>,
    pub(crate) retained_delta_bytes: Observation<i64>,
}

fn compare_health(before: &Snapshot, after: &Snapshot) -> HealthComparison {
    let rates = metrics::between(before, after);
    let mut result = HealthComparison {
        database: Observation::Unavailable("Not compared".into()),
        relations: Observation::Unavailable("Not compared".into()),
        replication: Observation::Unavailable("Not compared".into()),
        rates,
    };
    if let Err(reason) = metrics::interval_seconds(before, after) {
        result.database = Observation::Unavailable(reason.clone());
        result.relations = Observation::Unavailable(reason.clone());
        result.replication = Observation::Unavailable(reason);
        return result;
    }
    result.database = match (&before.health.database, &after.health.database) {
        (Observation::Available(left), Observation::Available(right)) => {
            Observation::Available(DatabaseChange {
                size_bytes_before: left.size_bytes,
                size_bytes_after: right.size_bytes,
                size_delta_bytes: gauge_delta(left.size_bytes, right.size_bytes),
                connections_before: left.num_backends,
                connections_after: right.num_backends,
                cluster_connections_before: left.cluster_backends,
                cluster_connections_after: right.cluster_backends,
            })
        }
        (left, right) => Observation::Unavailable(unavailable_reason("Database", left, right)),
    };
    let counter_problem = match (&before.health.database, &after.health.database) {
        (Observation::Available(left), Observation::Available(right)) => {
            metrics::database_baseline_problem(left, right)
        }
        _ => Some(
            "Database statistics/reset metadata unavailable; relation counter continuity unknown"
                .into(),
        ),
    };
    result.relations = match (&before.health.tables, &after.health.tables) {
        (Observation::Available(left), Observation::Available(right)) => {
            let before_tables: BTreeMap<_, _> =
                left.tables.iter().map(|table| (table.oid, table)).collect();
            let after_tables: BTreeMap<_, _> = right
                .tables
                .iter()
                .map(|table| (table.oid, table))
                .collect();
            let before_indexes: BTreeMap<_, _> = left
                .indexes
                .iter()
                .map(|index| (index.oid, index))
                .collect();
            let after_indexes: BTreeMap<_, _> = right
                .indexes
                .iter()
                .map(|index| (index.oid, index))
                .collect();
            if before_tables.len() != left.tables.len()
                || after_tables.len() != right.tables.len()
                || before_indexes.len() != left.indexes.len()
                || after_indexes.len() != right.indexes.len()
            {
                Observation::Unavailable("Duplicate relation OIDs make matching ambiguous".into())
            } else {
                let table_keys: BTreeSet<_> = before_tables
                    .keys()
                    .chain(after_tables.keys())
                    .copied()
                    .collect();
                let index_keys: BTreeSet<_> = before_indexes
                    .keys()
                    .chain(after_indexes.keys())
                    .copied()
                    .collect();
                let tables = table_keys
                    .into_iter()
                    .map(|oid| {
                        table_change(
                            oid,
                            before_tables.get(&oid).copied(),
                            after_tables.get(&oid).copied(),
                            counter_problem.as_deref(),
                        )
                    })
                    .collect();
                let indexes = index_keys
                    .into_iter()
                    .map(|oid| {
                        index_change(
                            oid,
                            before_indexes.get(&oid).copied(),
                            after_indexes.get(&oid).copied(),
                            counter_problem.as_deref(),
                        )
                    })
                    .collect();
                Observation::Available(RelationComparison { tables, indexes, before_truncated: left.truncated, after_truncated: right.truncated,
                    caveats: vec!["Relations are matched by OID and observed names. Truncated rankings cannot prove creation or removal. OID reuse between captures cannot be fully excluded.".into(), "Tuple counts are estimates, not exact row counts or bloat measurements. Size and estimate changes are gauges and may be negative.".into(), "Usage deltas require track_counts, matching database reset epochs (including two observed SQL NULL values before any recorded reset), and non-regressing counters. PostgreSQL single-relation resets also update the database reset timestamp. Zero index scans is not a recommendation to drop an index.".into()],
                })
            }
        }
        (left, right) => {
            Observation::Unavailable(unavailable_reason("Tables and indexes", left, right))
        }
    };
    result.replication = match (&before.health.replication, &after.health.replication) {
        (Observation::Available(left), Observation::Available(right)) => {
            let before_slots: BTreeMap<_, _> = left
                .slots
                .iter()
                .map(|slot| (slot.name.clone(), slot))
                .collect();
            let after_slots: BTreeMap<_, _> = right
                .slots
                .iter()
                .map(|slot| (slot.name.clone(), slot))
                .collect();
            if before_slots.len() != left.slots.len() || after_slots.len() != right.slots.len() {
                Observation::Unavailable("Duplicate slot names make matching ambiguous".into())
            } else {
                let names: BTreeSet<_> = before_slots
                    .keys()
                    .chain(after_slots.keys())
                    .cloned()
                    .collect();
                let slots = names
                    .into_iter()
                    .map(|name| {
                        let previous = before_slots.get(&name).copied();
                        let current = after_slots.get(&name).copied();
                        let retained_delta_bytes = match (previous, current) {
                            (Some(a), Some(b))
                                if a.slot_type == b.slot_type && a.database == b.database =>
                            {
                                optional_gauge_delta(a.retained_bytes, b.retained_bytes)
                            }
                            _ => Observation::Unavailable(
                                "Slot has no compatible observed counterpart".into(),
                            ),
                        };
                        SlotChange {
                            name,
                            before: previous.cloned(),
                            after: current.cloned(),
                            retained_delta_bytes,
                        }
                    })
                    .collect();
                Observation::Available(ReplicationComparison {
                    in_recovery_before: left.in_recovery, in_recovery_after: right.in_recovery,
                    received_replay_backlog_delta_bytes: if left.in_recovery && right.in_recovery {
                        optional_gauge_delta(left.receive_replay_lag_bytes, right.receive_replay_lag_bytes)
                    } else { Observation::Unavailable("Received-to-replay backlog is only comparable on a standby".into()) },
                    senders_before: left.senders.clone(), senders_after: right.senders.clone(), slots,
                    caveats: vec!["Backlog and retained-WAL changes compare observed gauges, not generated or transmitted bytes. Slot names may be recreated between captures.".into(), "Sender observations are shown separately: sender PID alone cannot establish backend identity across captures. Time since last replay is not a reliable lag measure on an idle primary.".into()],
                })
            }
        }
        (left, right) => Observation::Unavailable(unavailable_reason("Replication", left, right)),
    };
    result
}

fn gauge_delta(before: i64, after: i64) -> Observation<i64> {
    if before < 0 || after < 0 {
        return Observation::Unavailable("Observed gauge is invalid (negative)".into());
    }
    match after.checked_sub(before) {
        Some(delta) => Observation::Available(delta),
        None => Observation::Unavailable("Gauge change exceeds supported integer range".into()),
    }
}

fn optional_gauge_delta(before: Option<i64>, after: Option<i64>) -> Observation<i64> {
    match (before, after) {
        (Some(before), Some(after)) => gauge_delta(before, after),
        _ => Observation::Unavailable("At least one gauge is unavailable / NULL".into()),
    }
}

fn table_change(
    oid: i64,
    before: Option<&TableStats>,
    after: Option<&TableStats>,
    counter_problem: Option<&str>,
) -> TableChange {
    let pair = before
        .zip(after)
        .filter(|(a, b)| a.schema == b.schema && a.name == b.name);
    let gauge = |field: fn(&TableStats) -> i64| {
        pair.map_or_else(
            || {
                Observation::Unavailable(
                    "No same-named relation with a shared OID in both captures".into(),
                )
            },
            |(a, b)| gauge_delta(field(a), field(b)),
        )
    };
    let counter = |field: fn(&TableStats) -> i64| {
        if let Some(reason) = counter_problem {
            return Observation::Unavailable(reason.into());
        }
        pair.map_or_else(
            || Observation::Unavailable("No compatible relation baseline".into()),
            |(a, b)| metrics::counter_delta(field(a), field(b), "Relation"),
        )
    };
    let index_scans = if let Some(reason) = counter_problem {
        Observation::Unavailable(reason.into())
    } else {
        match pair.and_then(|(a, b)| a.idx_scan.zip(b.idx_scan)) {
            Some((a, b)) => metrics::counter_delta(a, b, "Relation index scans"),
            None => Observation::Unavailable("Relation index scans are unavailable / NULL".into()),
        }
    };
    TableChange {
        oid,
        before: before.cloned(),
        after: after.cloned(),
        size_delta_bytes: gauge(|t| t.total_bytes),
        estimated_live_tuple_change: gauge(|t| t.live_tuples),
        estimated_dead_tuple_change: gauge(|t| t.dead_tuples),
        sequential_scans: counter(|t| t.seq_scan),
        index_scans,
        inserts: counter(|t| t.inserts),
        updates: counter(|t| t.updates),
        deletes: counter(|t| t.deletes),
    }
}

fn index_change(
    oid: i64,
    before: Option<&IndexStats>,
    after: Option<&IndexStats>,
    counter_problem: Option<&str>,
) -> IndexChange {
    let pair = before
        .zip(after)
        .filter(|(a, b)| a.schema == b.schema && a.name == b.name && a.table_oid == b.table_oid);
    let counter = |field: fn(&IndexStats) -> i64| {
        if let Some(reason) = counter_problem {
            return Observation::Unavailable(reason.into());
        }
        pair.map_or_else(
            || Observation::Unavailable("No compatible index baseline".into()),
            |(a, b)| metrics::counter_delta(field(a), field(b), "Index"),
        )
    };
    IndexChange {
        oid,
        before: before.cloned(),
        after: after.cloned(),
        size_delta_bytes: pair.map_or_else(
            || Observation::Unavailable("No compatible index baseline".into()),
            |(a, b)| gauge_delta(a.size_bytes, b.size_bytes),
        ),
        scans: counter(|t| t.scans),
        tuples_read: counter(|t| t.tuples_read),
        tuples_fetched: counter(|t| t.tuples_fetched),
    }
}

pub(crate) fn compare(before: &Snapshot, after: &Snapshot) -> Comparison {
    let mut result = Comparison {
        health: compare_health(before, after),
        analysis: diagnostics::analyze(after, Some(before)),
        before: before.completed_at,
        after: after.completed_at,
        source: before.source.clone(),
        after_source: after.source.clone(),
        source_compatible: true,
        interval_ms: None,
        warnings: Vec::new(),
        sessions: Observation::Unavailable("Not compared".into()),
        blocking: Observation::Unavailable("Not compared".into()),
        statements: Observation::Unavailable("Not compared".into()),
    };
    if !sources_compatible(&before.source, &after.source) {
        result.source_compatible = false;
        let reason = "Source identity differs: captures must use the same endpoint, database, database OID, and uninterrupted PostgreSQL server instance".to_owned();
        result.warnings.push(reason.clone());
        result.sessions = Observation::Unavailable(reason.clone());
        result.blocking = Observation::Unavailable(reason.clone());
        result.statements = Observation::Unavailable(reason);
        return result;
    }
    if !(1..=crate::model::SNAPSHOT_VERSION).contains(&before.schema_version)
        || !(1..=crate::model::SNAPSHOT_VERSION).contains(&after.schema_version)
    {
        let reason = "Snapshot format versions are incompatible".to_owned();
        result.sessions = Observation::Unavailable(reason.clone());
        result.blocking = Observation::Unavailable(reason.clone());
        result.statements = Observation::Unavailable(reason.clone());
        result.warnings.push(reason);
        return result;
    }
    if before.source.system_identifier.is_none() || after.source.system_identifier.is_none() {
        result.warnings.push("PostgreSQL system identifier unavailable; source compatibility uses endpoint, database OID, and server start time. This cannot prove identity across endpoint reassignment.".into());
    }
    if before.completed_at < before.started_at
        || after.completed_at < after.started_at
        || after.started_at < before.completed_at
        || after.completed_at <= before.completed_at
    {
        result.warnings.push("Capture intervals overlap, are reversed, or have no positive elapsed time; interval statement deltas are unavailable.".into());
    } else {
        result.interval_ms = Some((after.completed_at - before.completed_at).num_milliseconds());
    }
    if !before.is_complete() || !after.is_complete() {
        result.warnings.push("At least one capture is incomplete; inspect availability and capture warnings before interpreting changes.".into());
    }
    for warning in &before.warnings {
        result.warnings.push(format!("Before capture: {warning}"));
    }
    for warning in &after.warnings {
        result.warnings.push(format!("After capture: {warning}"));
    }
    match (&before.activity, &after.activity) {
        (Observation::Available(previous), Observation::Available(current)) => {
            result.sessions = compare_sessions(previous, current);
            result.blocking = compare_blocking(previous, current);
        }
        _ => {
            let reason = unavailable_reason("Activity", &before.activity, &after.activity);
            result.sessions = Observation::Unavailable(reason.clone());
            result.blocking = Observation::Unavailable(reason);
        }
    }
    match (&before.statements, &after.statements) {
        (Observation::Available(previous), Observation::Available(current)) => {
            result.statements =
                Observation::Available(compare_statements(previous, current, result.interval_ms));
        }
        _ => {
            result.statements = Observation::Unavailable(unavailable_reason(
                "Statement statistics",
                &before.statements,
                &after.statements,
            ))
        }
    }
    result
}

pub(crate) fn sources_compatible(before: &Source, after: &Source) -> bool {
    !before.endpoint.is_empty()
        && !before.database.is_empty()
        && before.database_oid > 0
        && before.endpoint == after.endpoint
        && before.database == after.database
        && before.database_oid == after.database_oid
        && before.server_started_at == after.server_started_at
        && before.server_version == after.server_version
        && match (&before.system_identifier, &after.system_identifier) {
            (Some(left), Some(right)) => left == right,
            _ => true,
        }
}

fn unavailable_reason<T>(section: &str, before: &Observation<T>, after: &Observation<T>) -> String {
    let mut reasons = Vec::new();
    if let Observation::Unavailable(reason) = before {
        reasons.push(format!("before: {reason}"));
    }
    if let Observation::Unavailable(reason) = after {
        reasons.push(format!("after: {reason}"));
    }
    format!("{section} unavailable ({})", reasons.join("; "))
}

fn session_identity(session: &Session) -> Option<SessionIdentity> {
    session.backend_start.map(|backend_start| SessionIdentity {
        pid: session.pid,
        backend_start,
    })
}

type SessionMap<'a> = BTreeMap<SessionIdentity, &'a Session>;

fn indexed_sessions<'a>(
    sessions: &'a [Session],
    unknown_pids: &BTreeSet<i32>,
) -> Option<(SessionMap<'a>, Vec<Session>)> {
    let mut indexed = BTreeMap::new();
    let mut unknown = Vec::new();
    let mut seen_pids = BTreeSet::new();
    for session in sessions {
        if !seen_pids.insert(session.pid) {
            return None;
        }
        if unknown_pids.contains(&session.pid) {
            unknown.push(session.clone());
            continue;
        }
        if let Some(identity) = session_identity(session) {
            if indexed.insert(identity, session).is_some() {
                return None;
            }
        } else {
            unknown.push(session.clone());
        }
    }
    unknown.sort_by_key(|session| session.pid);
    Some((indexed, unknown))
}

fn compare_sessions(before: &[Session], after: &[Session]) -> Observation<SessionComparison> {
    // If one capture hides a backend's identity, the other observation of that PID
    // also has an unknown match. Do not invent an addition or removal.
    let unknown_pids = before
        .iter()
        .chain(after)
        .filter(|session| session.backend_start.is_none())
        .map(|session| session.pid)
        .collect();
    let Some((left, unidentifiable_before)) = indexed_sessions(before, &unknown_pids) else {
        return Observation::Unavailable("Duplicate session identities in before capture".into());
    };
    let Some((right, unidentifiable_after)) = indexed_sessions(after, &unknown_pids) else {
        return Observation::Unavailable("Duplicate session identities in after capture".into());
    };
    let mut comparison = SessionComparison {
        added: Vec::new(),
        removed: Vec::new(),
        changed: Vec::new(),
        unchanged: 0,
        unidentifiable_before,
        unidentifiable_after,
    };
    for (key, current) in &right {
        if let Some(previous) = left.get(key) {
            let fields = changed_session_fields(previous, current);
            if fields.is_empty() {
                comparison.unchanged += 1;
            } else {
                comparison.changed.push(SessionChange {
                    identity: key.clone(),
                    before: (*previous).clone(),
                    after: (*current).clone(),
                    fields,
                });
            }
        } else {
            comparison.added.push((*current).clone());
        }
    }
    for (key, previous) in &left {
        if !right.contains_key(key) {
            comparison.removed.push((*previous).clone());
        }
    }
    Observation::Available(comparison)
}

fn changed_session_fields(before: &Session, after: &Session) -> Vec<String> {
    let mut fields = Vec::new();
    macro_rules! changed {
        ($($field:ident),+ $(,)?) => {
            $(if before.$field != after.$field { fields.push(stringify!($field).to_owned()); })+
        };
    }
    changed!(
        user,
        database,
        application,
        client,
        state,
        query_age_ms,
        transaction_age_ms,
        wait_event_type,
        wait_event,
        query
    );
    let before_blockers: BTreeSet<_> = before.blockers.iter().collect();
    let after_blockers: BTreeSet<_> = after.blockers.iter().collect();
    if before_blockers != after_blockers {
        fields.push("blockers".into());
    }
    fields
}

fn blocking_edges(
    sessions: &[Session],
) -> (BTreeSet<BlockingEdge>, BTreeSet<UnresolvedBlockingEdge>) {
    let mut identities = BTreeMap::new();
    let mut duplicates = BTreeSet::new();
    for session in sessions {
        if identities
            .insert(session.pid, session_identity(session))
            .is_some()
        {
            duplicates.insert(session.pid);
        }
    }
    let mut edges = BTreeSet::new();
    let mut unknown = BTreeSet::new();
    for session in sessions {
        for blocker_pid in &session.blockers {
            match (
                session_identity(session),
                identities.get(blocker_pid).cloned().flatten(),
            ) {
                (Some(waiter), Some(blocker))
                    if !duplicates.contains(&session.pid) && !duplicates.contains(blocker_pid) =>
                {
                    edges.insert(BlockingEdge { waiter, blocker });
                }
                _ => {
                    unknown.insert(UnresolvedBlockingEdge {
                        waiter_pid: session.pid,
                        blocker_pid: *blocker_pid,
                    });
                }
            }
        }
    }
    (edges, unknown)
}

fn compare_blocking(before: &[Session], after: &[Session]) -> Observation<BlockingComparison> {
    let (mut left, unknown_left) = blocking_edges(before);
    let (mut right, unknown_right) = blocking_edges(after);
    let unresolved: BTreeSet<_> = unknown_left
        .iter()
        .chain(&unknown_right)
        .map(|edge| (edge.waiter_pid, edge.blocker_pid))
        .collect();
    left.retain(|edge| !unresolved.contains(&(edge.waiter.pid, edge.blocker.pid)));
    right.retain(|edge| !unresolved.contains(&(edge.waiter.pid, edge.blocker.pid)));
    Observation::Available(BlockingComparison {
        added: right.difference(&left).cloned().collect(),
        removed: left.difference(&right).cloned().collect(),
        unchanged: left.intersection(&right).count(),
        unresolved_before: unknown_left.into_iter().collect(),
        unresolved_after: unknown_right.into_iter().collect(),
    })
}

fn statement_identity(statement: &Statement) -> StatementIdentity {
    StatementIdentity {
        userid: statement.userid,
        dbid: statement.dbid,
        queryid: statement.queryid,
        toplevel: statement.toplevel,
    }
}

fn global_delta_problem(
    before: &StatementStats,
    after: &StatementStats,
    interval_ms: Option<i64>,
) -> Option<String> {
    if !interval_ms.is_some_and(|interval| interval > 0) {
        return Some(
            "Capture intervals overlap, are reversed, or have no positive elapsed time".into(),
        );
    }
    if before.truncated || after.truncated {
        return Some(
            "Statement collection was truncated; a complete counter baseline is unavailable".into(),
        );
    }
    match (before.reset_at, after.reset_at) {
        (Some(left), Some(right)) if left == right => {}
        (Some(_), Some(_)) => return Some("pg_stat_statements was reset between captures".into()),
        _ => return Some("Statement reset metadata is unavailable".into()),
    }
    match (before.dealloc, after.dealloc) {
        (Some(left), Some(right)) if left == right && left >= 0 => {}
        (Some(_), Some(_)) => {
            return Some(
                "Statement deallocation counter changed; entries may have been evicted or reset"
                    .into(),
            );
        }
        _ => return Some("Statement deallocation metadata is unavailable".into()),
    }
    None
}

pub(crate) fn compare_statements(
    before: &StatementStats,
    after: &StatementStats,
    interval_ms: Option<i64>,
) -> StatementComparison {
    let left: BTreeMap<_, _> = before
        .entries
        .iter()
        .map(|entry| (statement_identity(entry), entry))
        .collect();
    let right: BTreeMap<_, _> = after
        .entries
        .iter()
        .map(|entry| (statement_identity(entry), entry))
        .collect();
    let global_problem = if left.len() != before.entries.len() || right.len() != after.entries.len()
    {
        Some("Duplicate statement identities make the counter baseline ambiguous".into())
    } else {
        global_delta_problem(before, after, interval_ms)
    };
    let keys: BTreeSet<_> = left.keys().chain(right.keys()).cloned().collect();
    let entries: Vec<_> = keys
        .into_iter()
        .map(|identity| {
            let previous = left.get(&identity).copied();
            let current = right.get(&identity).copied();
            let delta = if let Some(reason) = &global_problem {
                Observation::Unavailable(reason.clone())
            } else {
                match (previous, current) {
                    (Some(previous), Some(current)) => statement_delta(previous, current),
                    (None, Some(_)) => Observation::Unavailable(
                        "First observed statement; previous counter baseline unavailable".into(),
                    ),
                    (Some(_), None) => Observation::Unavailable(
                        "Statement no longer observed; final counters unavailable".into(),
                    ),
                    (None, None) => {
                        Observation::Unavailable("Statement observations unavailable".into())
                    }
                }
            };
            StatementChange {
                identity,
                before: previous.cloned(),
                after: current.cloned(),
                delta,
            }
        })
        .collect();
    let total = match global_problem {
        Some(reason) => Observation::Unavailable(reason),
        None => sum_deltas(&entries),
    };
    StatementComparison {
        entries,
        total,
        before_truncated: before.truncated,
        after_truncated: after.truncated,
    }
}

fn statement_delta(before: &Statement, after: &Statement) -> Observation<StatementDelta> {
    match (before.stats_since, after.stats_since) {
        (Some(left), Some(right)) if left == right => {}
        (Some(_), Some(_)) => return Observation::Unavailable(
            "Statement statistics start time changed; counters were reset or entry was recreated"
                .into(),
        ),
        _ => {
            return Observation::Unavailable(
                "Statement statistics start time is unavailable".into(),
            );
        }
    }
    let counters_before = [
        before.calls,
        before.rows,
        before.shared_blks_hit,
        before.shared_blks_read,
        before.temp_blks_written,
    ];
    let counters_after = [
        after.calls,
        after.rows,
        after.shared_blks_hit,
        after.shared_blks_read,
        after.temp_blks_written,
    ];
    if counters_before
        .iter()
        .zip(counters_after)
        .any(|(left, right)| *left < 0 || right < *left)
        || !before.total_exec_ms.is_finite()
        || !after.total_exec_ms.is_finite()
        || before.total_exec_ms < 0.0
        || after.total_exec_ms < before.total_exec_ms
    {
        return Observation::Unavailable("Statement counters regressed or are invalid; a reset or inconsistent baseline is possible".into());
    }
    let calls = after.calls - before.calls;
    let total_exec_ms = after.total_exec_ms - before.total_exec_ms;
    Observation::Available(StatementDelta {
        calls,
        total_exec_ms,
        mean_exec_ms: (calls > 0).then(|| total_exec_ms / calls as f64),
        rows: after.rows - before.rows,
        shared_blks_hit: after.shared_blks_hit - before.shared_blks_hit,
        shared_blks_read: after.shared_blks_read - before.shared_blks_read,
        temp_blks_written: after.temp_blks_written - before.temp_blks_written,
    })
}

fn sum_deltas(entries: &[StatementChange]) -> Observation<StatementDelta> {
    let mut total = StatementDelta {
        calls: 0,
        total_exec_ms: 0.0,
        mean_exec_ms: None,
        rows: 0,
        shared_blks_hit: 0,
        shared_blks_read: 0,
        temp_blks_written: 0,
    };
    for entry in entries {
        let Observation::Available(delta) = &entry.delta else {
            return Observation::Unavailable("Aggregate interval metrics require every statement to have a valid before/after baseline; inspect individual statements".into());
        };
        macro_rules! add {
            ($($field:ident),+ $(,)?) => {
                $(match total.$field.checked_add(delta.$field) {
                    Some(value) => total.$field = value,
                    None => return Observation::Unavailable("Aggregate statement counters exceed the supported integer range".into()),
                })+
            };
        }
        add!(
            calls,
            rows,
            shared_blks_hit,
            shared_blks_read,
            temp_blks_written
        );
        total.total_exec_ms += delta.total_exec_ms;
    }
    if !total.total_exec_ms.is_finite() {
        return Observation::Unavailable(
            "Aggregate execution time exceeds the supported numeric range".into(),
        );
    }
    total.mean_exec_ms = (total.calls > 0).then(|| total.total_exec_ms / total.calls as f64);
    Observation::Available(total)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::{SNAPSHOT_VERSION, StatementStats};
    use chrono::Duration;

    pub(crate) fn snapshot() -> Snapshot {
        let time = DateTime::parse_from_rfc3339("2026-01-02T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        Snapshot {
            schema_version: SNAPSHOT_VERSION,
            health: crate::demo::snapshot(0, false).health,
            started_at: time,
            completed_at: time + Duration::milliseconds(50),
            source: Source {
                endpoint: "localhost:5432".into(),
                database: "app".into(),
                database_oid: 16384,
                server_version: "18.0".into(),
                server_started_at: time - Duration::hours(1),
                system_identifier: Some("123456789".into()),
            },
            activity: Observation::Available(vec![Session {
                pid: 123,
                backend_start: Some(time - Duration::minutes(5)),
                user: Some("app".into()),
                database: Some("app".into()),
                application: "service".into(),
                client: None,
                state: Some("active".into()),
                query_age_ms: Some(100),
                transaction_age_ms: None,
                wait_event_type: None,
                wait_event: None,
                blockers: vec![],
                query: None,
            }]),
            statements: Observation::Available(StatementStats {
                reset_at: Some(time - Duration::hours(1)),
                dealloc: Some(0),
                truncated: false,
                entries: vec![Statement {
                    userid: 42,
                    dbid: 16384,
                    queryid: 7,
                    toplevel: true,
                    stats_since: Some(time - Duration::hours(1)),
                    calls: 10,
                    total_exec_ms: 200.0,
                    mean_exec_ms: 20.0,
                    rows: 50,
                    shared_blks_hit: 100,
                    shared_blks_read: 4,
                    temp_blks_written: 0,
                    query: None,
                }],
            }),
            warnings: vec![],
        }
    }

    fn later(before: &Snapshot) -> Snapshot {
        let mut after = before.clone();
        after.started_at += Duration::seconds(10);
        after.completed_at += Duration::seconds(10);
        after
    }

    #[test]
    fn identical_observations_have_zero_deltas_and_no_changes() {
        let before = snapshot();
        let result = compare(&before, &later(&before));
        let sessions = result.sessions.available().unwrap();
        assert!(
            sessions.added.is_empty() && sessions.removed.is_empty() && sessions.changed.is_empty()
        );
        assert_eq!(sessions.unchanged, 1);
        let delta = result
            .statements
            .available()
            .unwrap()
            .total
            .available()
            .unwrap();
        assert_eq!(delta.calls, 0);
        assert_eq!(delta.total_exec_ms, 0.0);
        assert_eq!(delta.mean_exec_ms, None);
    }

    #[test]
    fn reused_pids_and_missing_backend_identity_do_not_become_changed_sessions() {
        let before = snapshot();
        let mut after = later(&before);
        if let Observation::Available(sessions) = &mut after.activity {
            sessions[0].backend_start = Some(after.started_at);
        }
        let result = compare(&before, &after);
        let sessions = result.sessions.available().unwrap();
        assert_eq!(sessions.added.len(), 1);
        assert_eq!(sessions.removed.len(), 1);
        assert!(sessions.changed.is_empty());
        if let Observation::Available(sessions) = &mut after.activity {
            sessions[0].backend_start = None;
        }
        let result = compare(&before, &after);
        assert_eq!(
            result
                .sessions
                .available()
                .unwrap()
                .unidentifiable_after
                .len(),
            1
        );
        assert!(result.sessions.available().unwrap().added.is_empty());
        assert!(result.sessions.available().unwrap().removed.is_empty());
        assert_eq!(
            result
                .sessions
                .available()
                .unwrap()
                .unidentifiable_before
                .len(),
            1
        );
    }

    #[test]
    fn interval_mean_uses_delta_totals_and_calls() {
        let before = snapshot();
        let mut after = later(&before);
        if let Observation::Available(stats) = &mut after.statements {
            stats.entries[0].calls = 12;
            stats.entries[0].total_exec_ms = 300.0;
            stats.entries[0].mean_exec_ms = 25.0;
        }
        let result = compare(&before, &after);
        let delta = result
            .statements
            .available()
            .unwrap()
            .total
            .available()
            .unwrap();
        assert_eq!(delta.calls, 2);
        assert_eq!(delta.total_exec_ms, 100.0);
        assert_eq!(delta.mean_exec_ms, Some(50.0));
    }

    #[test]
    fn incompatible_sources_and_restarts_cannot_produce_deltas() {
        let before = snapshot();
        for kind in 0..5 {
            let mut after = later(&before);
            match kind {
                0 => after.source.endpoint = "other:5432".into(),
                1 => after.source.database_oid += 1,
                2 => after.source.server_started_at += Duration::seconds(1),
                3 => after.source.system_identifier = Some("another".into()),
                _ => after.source.database = "other".into(),
            }
            let comparison = compare(&before, &after);
            assert!(!comparison.source_compatible);
            assert!(comparison.statements.available().is_none());
            assert!(comparison.sessions.available().is_none());
        }
    }

    #[test]
    fn inaccessible_cluster_id_uses_explicit_conservative_fallback() {
        let mut before = snapshot();
        before.source.system_identifier = None;
        let comparison = compare(&before, &later(&before));
        assert!(comparison.source_compatible);
        assert!(
            comparison
                .warnings
                .iter()
                .any(|warning| warning.contains("system identifier unavailable"))
        );
    }

    #[test]
    fn reset_eviction_missing_metadata_regression_and_truncation_invalidate_deltas() {
        let before = snapshot();
        for kind in 0..9 {
            let mut after = later(&before);
            if let Observation::Available(stats) = &mut after.statements {
                match kind {
                    0 => stats.reset_at = Some(after.started_at),
                    1 => stats.reset_at = None,
                    2 => stats.dealloc = Some(1),
                    3 => stats.dealloc = None,
                    4 => stats.entries[0].stats_since = Some(after.started_at),
                    5 => stats.entries[0].stats_since = None,
                    6 => stats.entries[0].calls = 1,
                    7 => stats.entries[0].shared_blks_hit = 1,
                    _ => stats.truncated = true,
                }
            }
            let comparison = compare(&before, &after);
            let statements = comparison.statements.available().unwrap();
            assert!(statements.total.available().is_none(), "case {kind}");
            assert!(
                statements.entries[0].delta.available().is_none(),
                "case {kind}"
            );
        }
    }

    #[test]
    fn reversed_and_overlapping_captures_cannot_produce_interval_metrics() {
        let before = snapshot();
        for after in [before.clone(), {
            let mut capture = before.clone();
            capture.started_at -= Duration::minutes(1);
            capture.completed_at += Duration::minutes(1);
            capture
        }] {
            let comparison = compare(&before, &after);
            assert_eq!(comparison.interval_ms, None);
            assert!(
                comparison
                    .statements
                    .available()
                    .unwrap()
                    .total
                    .available()
                    .is_none()
            );
        }
        assert_eq!(compare(&later(&before), &before).interval_ms, None);
    }

    #[test]
    fn missing_sections_are_not_empty_or_healthy() {
        let before = snapshot();
        let mut after = later(&before);
        after.activity = Observation::Unavailable("permission denied".into());
        after.statements = Observation::Unavailable("extension missing".into());
        let result = compare(&before, &after);
        assert!(
            matches!(result.sessions, Observation::Unavailable(reason) if reason.contains("permission denied"))
        );
        assert!(result.blocking.available().is_none());
        assert!(
            matches!(result.statements, Observation::Unavailable(reason) if reason.contains("extension missing"))
        );
    }

    #[test]
    fn blocking_edges_use_backend_identity_and_distinguish_unresolved_blockers() {
        let before = snapshot();
        let mut after = later(&before);
        if let Observation::Available(sessions) = &mut after.activity {
            let mut waiter = sessions[0].clone();
            waiter.pid = 456;
            waiter.blockers = vec![123, 999, 123];
            sessions.push(waiter);
        }
        let result = compare(&before, &after);
        let blocking = result.blocking.available().unwrap();
        assert_eq!(blocking.added.len(), 1);
        assert_eq!(blocking.added[0].waiter.pid, 456);
        assert_eq!(blocking.added[0].blocker.pid, 123);
        assert_eq!(blocking.unresolved_after.len(), 1);
        assert_eq!(blocking.unresolved_after[0].blocker_pid, 999);
        let cleared = compare(&after, &later(&before));
        assert_eq!(cleared.blocking.available().unwrap().removed.len(), 1);
    }

    #[test]
    fn hidden_blocker_identity_does_not_claim_a_resolved_block() {
        let mut before = snapshot();
        if let Observation::Available(sessions) = &mut before.activity {
            let mut waiter = sessions[0].clone();
            waiter.pid = 456;
            waiter.blockers = vec![123];
            sessions.push(waiter);
        }
        let mut after = later(&before);
        if let Observation::Available(sessions) = &mut after.activity {
            sessions[0].backend_start = None;
        }
        let result = compare(&before, &after);
        let blocking = result.blocking.available().unwrap();
        assert!(blocking.added.is_empty());
        assert!(blocking.removed.is_empty());
        assert_eq!(blocking.unresolved_after.len(), 1);
    }

    #[test]
    fn aggregate_overflow_is_unavailable_without_panicking() {
        let mut before = snapshot();
        if let Observation::Available(statistics) = &mut before.statements {
            statistics.entries[0].calls = 0;
            let mut second = statistics.entries[0].clone();
            second.queryid = 8;
            statistics.entries.push(second);
        }
        let mut after = later(&before);
        if let Observation::Available(statistics) = &mut after.statements {
            for entry in &mut statistics.entries {
                entry.calls = i64::MAX;
            }
        }
        let result = compare(&before, &after);
        let statistics = result.statements.available().unwrap();
        assert!(
            statistics
                .entries
                .iter()
                .all(|entry| entry.delta.available().is_some())
        );
        assert!(
            matches!(&statistics.total, Observation::Unavailable(reason) if reason.contains("integer range"))
        );
    }

    #[test]
    fn statement_additions_and_removals_do_not_invent_interval_counters() {
        let before = snapshot();
        let mut after = later(&before);
        if let Observation::Available(stats) = &mut after.statements {
            stats.entries[0].queryid = 99;
        }
        let result = compare(&before, &after);
        let stats = result.statements.available().unwrap();
        assert_eq!(stats.entries.len(), 2);
        assert!(stats.entries[0].after.is_none());
        assert!(stats.entries[1].before.is_none());
        assert!(stats.total.available().is_none());
    }

    #[test]
    fn comparison_order_is_stable_under_input_reordering() {
        let before = snapshot();
        let mut after = later(&before);
        if let Observation::Available(sessions) = &mut after.activity {
            let mut extra = sessions[0].clone();
            extra.pid = 12;
            sessions.push(extra);
            sessions[0].state = Some("idle".into());
        }
        let original = compare(&before, &after);
        if let Observation::Available(sessions) = &mut after.activity {
            sessions.reverse();
        }
        assert_eq!(compare(&before, &after), original);
    }
}

#[cfg(test)]
mod health_tests {
    use super::*;
    use chrono::Duration;

    fn later(before: &Snapshot) -> Snapshot {
        let mut after = before.clone();
        after.started_at += Duration::seconds(10);
        after.completed_at += Duration::seconds(10);
        after
    }

    #[test]
    fn relation_gauges_can_shrink_while_regressed_usage_remains_unknown() {
        let before = tests::snapshot();
        let mut after = later(&before);
        if let Observation::Available(relations) = &mut after.health.tables {
            relations.tables[0].total_bytes -= 8192;
            relations.tables[0].dead_tuples -= 100;
            relations.tables[0].seq_scan = 0;
            relations.tables[0].updates += 50;
            relations.indexes[0].scans += 10;
        }
        let comparison = compare(&before, &after);
        let relations = comparison.health.relations.available().unwrap();
        assert_eq!(
            relations.tables[0].size_delta_bytes.available(),
            Some(&-8192)
        );
        assert_eq!(
            relations.tables[0].estimated_dead_tuple_change.available(),
            Some(&-100)
        );
        assert!(relations.tables[0].sequential_scans.available().is_none());
        assert_eq!(relations.tables[0].updates.available(), Some(&50));
        assert_eq!(relations.indexes[0].scans.available(), Some(&10));
    }

    #[test]
    fn database_reset_does_not_erase_observed_size_or_fabricate_usage() {
        let before = tests::snapshot();
        let mut after = later(&before);
        if let Observation::Available(db) = &mut after.health.database {
            db.stats_reset = None;
        }
        if let Observation::Available(relations) = &mut after.health.tables {
            relations.tables[0].total_bytes += 8192;
        }
        let comparison = compare(&before, &after);
        let relations = comparison.health.relations.available().unwrap();
        assert_eq!(
            relations.tables[0].size_delta_bytes.available(),
            Some(&8192)
        );
        assert!(relations.tables[0].updates.available().is_none());
        assert!(comparison.health.rates.database.available().is_none());
    }

    #[test]
    fn missing_truncated_or_renamed_relation_never_gets_an_invented_delta() {
        let before = tests::snapshot();
        let mut after = later(&before);
        if let Observation::Available(relations) = &mut after.health.tables {
            relations.truncated = true;
            relations.tables.remove(1);
            relations.tables[0].name = "renamed_orders".into();
        }
        let comparison = compare(&before, &after);
        let relations = comparison.health.relations.available().unwrap();
        assert!(relations.after_truncated);
        assert_eq!(relations.tables.len(), 2);
        assert!(
            relations
                .tables
                .iter()
                .all(|table| table.size_delta_bytes.available().is_none())
        );
        assert!(relations.tables[1].after.is_none());
        assert!(
            relations
                .caveats
                .iter()
                .any(|reason| reason.contains("cannot prove creation or removal"))
        );
    }

    #[test]
    fn replica_sender_pid_is_not_treated_as_a_durable_backend_identity() {
        let before = tests::snapshot();
        let mut after = later(&before);
        if let Observation::Available(replication) = &mut after.health.replication {
            replication.slots[0].retained_bytes = Some(8_000_000);
            replication.senders[0].application = "different-replica".into();
        }
        let comparison = compare(&before, &after);
        let replication = comparison.health.replication.available().unwrap();
        assert_eq!(
            replication.slots[0].retained_delta_bytes.available(),
            Some(&-8_000_000)
        );
        assert_ne!(
            replication.senders_before[0].application,
            replication.senders_after[0].application
        );
        assert!(
            replication
                .caveats
                .iter()
                .any(|reason| reason.contains("PID alone cannot establish"))
        );
    }

    #[test]
    fn original_snapshot_format_still_compares_sessions_and_statements() {
        let mut before = tests::snapshot();
        before.schema_version = 1;
        before.health = Default::default();
        let mut after = later(&before);
        after.schema_version = 2;
        after.health = crate::demo::snapshot(0, false).health;
        let comparison = compare(&before, &after);
        assert!(comparison.sessions.available().is_some());
        assert!(
            comparison
                .statements
                .available()
                .unwrap()
                .total
                .available()
                .is_some()
        );
        assert!(comparison.health.database.available().is_none());
        assert!(comparison.health.relations.available().is_none());
    }
}
