//! Conservative, deterministic comparison of two observations; no database access.
use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::model::{Observation, Session, Snapshot, Source, Statement, StatementStats};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Comparison {
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

pub(crate) fn compare(before: &Snapshot, after: &Snapshot) -> Comparison {
    let mut result = Comparison {
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
    if before.schema_version != after.schema_version
        || before.schema_version != crate::model::SNAPSHOT_VERSION
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

fn sources_compatible(before: &Source, after: &Source) -> bool {
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

fn compare_statements(
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
