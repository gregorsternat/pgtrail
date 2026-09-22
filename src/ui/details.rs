//! Complete, frozen detail text. Rendering and navigation share this evidence.
use chrono::{DateTime, Utc};

use crate::{
    app::{App, Tab},
    model::{Observation, Session, Statement},
};

use super::{duration, unique_session, unresolved_blocker, wait};

pub(crate) fn detail_report(app: &App) -> Option<(&'static str, String)> {
    let snapshot = app.snapshot()?;
    let (title, body) = match app.tab {
        Tab::Activity => {
            let sessions = app.filtered_sessions();
            let session = sessions.get(app.selected())?;
            ("Session details", session_text(session))
        }
        Tab::Blocking => {
            let edges = app.blocking_edges();
            let (waiter, blocker_pid) = edges.get(app.selected())?;
            let blocker = snapshot
                .activity
                .available()
                .and_then(|sessions| unique_session(sessions, *blocker_pid));
            let body = format!(
                "# Blocking relationship\nWaiter {} → blocker {blocker_pid}\nRelationships are observed at collection time; unresolved identities are not inferred.\nUse b to inspect the blocker or w to inspect the waiter.\n\n# Blocker\n{}\n\n# Waiter\n{}",
                waiter.pid,
                blocker.map(session_text).unwrap_or_else(|| format!(
                    "PID {blocker_pid}: {}. Identity unavailable or ambiguous; no backend is inferred.",
                    unresolved_blocker(*blocker_pid)
                )),
                session_text(waiter)
            );
            ("Blocking details", body)
        }
        Tab::Statements => ("Statement details", statement_text(app)?),
        Tab::Relations if app.show_indexes => {
            let indexes = app.filtered_indexes();
            let index = indexes.get(app.selected())?;
            (
                "Index details",
                format!(
                    "# Index identity\n{}.{}\nIndex OID: {}\nTable: {}.{} (OID {})\n\n# Size and cumulative usage\nSize: {} bytes\nScans: {}\nIndex tuples read: {}\nHeap tuples fetched: {}\n\n# Validity and constraints\nValid: {}\nUnique: {}\nPrimary: {}\n\n# Interpretation\nScan counts are cumulative and can reset. Low usage alone does not justify removal.\nCheck constraints, workload coverage and query plans before changing an index.",
                    index.schema,
                    index.name,
                    index.oid,
                    index.schema,
                    index.table,
                    index.table_oid,
                    index.size_bytes,
                    index.scans,
                    index.tuples_read,
                    index.tuples_fetched,
                    index.valid,
                    index.unique,
                    index.primary
                ),
            )
        }
        Tab::Relations => {
            let tables = app.filtered_tables();
            let table = tables.get(app.selected())?;
            (
                "Relation details",
                format!(
                    "# Table identity\n{}.{}\nRelation OID: {}\n\n# Size\nTotal: {} bytes\nTable: {} bytes\nIndexes: {} bytes\n\n# Cumulative counters\nSequential scans: {}\nIndex scans: {}\nInserts: {}\nUpdates: {}\nDeletes: {}\n\n# Estimates and maintenance\nEstimated live tuples: {}\nEstimated dead tuples: {}\nEstimated modifications since analyze: {}\nFrozen transaction ID age: {}\nLast vacuum: {}\nLast autovacuum: {}\nLast analyze: {}\nLast autoanalyze: {}\n\n# Interpretation\nTuple counts are estimates, not measured bloat. Scans and writes are cumulative and can reset.\nA missing maintenance timestamp means none was recorded; maintenance may have run before a reset.",
                    table.schema,
                    table.name,
                    table.oid,
                    table.total_bytes,
                    table.table_bytes,
                    table.index_bytes,
                    table.seq_scan,
                    table
                        .idx_scan
                        .map_or_else(|| "unavailable".into(), |n| n.to_string()),
                    table.inserts,
                    table.updates,
                    table.deletes,
                    table.live_tuples,
                    table.dead_tuples,
                    table.modified_since_analyze,
                    table.frozen_xid_age,
                    timestamp(table.last_vacuum),
                    timestamp(table.last_autovacuum),
                    timestamp(table.last_analyze),
                    timestamp(table.last_autoanalyze)
                ),
            )
        }
        _ => return None,
    };
    let capture = app.capture.as_ref().map_or_else(String::new, |capture| {
        format!("Capture #{}: {}\n", capture.id, capture.label)
    });
    let provenance = if snapshot.is_synthetic() {
        "Synthetic demo evidence"
    } else {
        "Collected observation"
    };
    let context = format!(
        "# Observation\n{capture}{provenance}\nSource: {} / {} (database OID {})\nPostgreSQL {}\nObserved: {} UTC\nServer started: {} UTC\n\n",
        snapshot.source.endpoint,
        snapshot.source.database,
        snapshot.source.database_oid,
        snapshot.source.server_version,
        snapshot.completed_at.format("%Y-%m-%d %H:%M:%S%.3f"),
        snapshot
            .source
            .server_started_at
            .format("%Y-%m-%d %H:%M:%S%.3f"),
    );
    Some((title, format!("{context}{body}")))
}

fn session_text(session: &Session) -> String {
    let blockers = if session.blockers.is_empty() {
        "none observed".into()
    } else {
        session
            .blockers
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "## Session identity\nPID: {}\nBackend started: {}\nUser: {}\nDatabase: {}\nApplication: {}\nClient: {}\n\n## Current activity\nState: {}\nCurrent query elapsed: {}\nTransaction elapsed: {}\nWait event: {}\nBlocking PIDs: {blockers}\nElapsed durations describe this observation; they are not cumulative execution counters.\n\n## Captured SQL\n{}",
        session.pid,
        timestamp(session.backend_start),
        optional(session.user.as_deref()),
        optional(session.database.as_deref()),
        session.application,
        optional(session.client.as_deref()),
        optional(session.state.as_deref()),
        duration(session.query_age_ms),
        duration(session.transaction_age_ms),
        wait(session),
        sql(session.query.as_deref())
    )
}

fn statement_text(app: &App) -> Option<String> {
    if app.statement_interval {
        let rates = app.filtered_statement_rates();
        let rate = rates.get(app.selected())?;
        let identity = &rate.identity;
        let statement = app
            .snapshot()?
            .statements
            .available()?
            .entries
            .iter()
            .find(|statement| {
                statement.userid == identity.userid
                    && statement.dbid == identity.dbid
                    && statement.queryid == identity.queryid
                    && statement.toplevel == identity.toplevel
            });
        let elapsed = app.analysis.as_ref()?.rates.elapsed_seconds.map_or_else(
            || "unavailable".into(),
            |value| format!("{value:.3} seconds"),
        );
        Some(format!(
            "# Statement identity\nUser ID: {}\nDatabase ID: {}\nQuery ID: {}\nTop level: {}\n\n# Interval metrics\nElapsed interval: {elapsed}\nCalls: {}\nExecution: {}\nMean completed execution: {}\nShared reads: {}\nTemporary writes: {}\nRates require compatible sources and valid counter baselines; each unavailable value keeps its reason.\n\n# Captured SQL\n{}",
            identity.userid,
            identity.dbid,
            identity.queryid,
            identity.toplevel,
            metric(&rate.calls_per_second, "/s"),
            metric(&rate.exec_ms_per_second, " ms/s"),
            metric(&rate.mean_exec_ms, " ms"),
            metric(&rate.shared_reads_per_second, " blocks/s"),
            metric(&rate.temp_blocks_per_second, " blocks/s"),
            sql(statement.and_then(|statement| statement.query.as_deref()))
        ))
    } else {
        let statements = app.filtered_statements();
        let statement = statements.get(app.selected())?;
        Some(cumulative_statement(statement))
    }
}

fn cumulative_statement(statement: &Statement) -> String {
    format!(
        "# Statement identity\nUser ID: {}\nDatabase ID: {}\nQuery ID: {}\nTop level: {}\nStatistics since: {}\n\n# Cumulative execution statistics\nCalls: {}\nTotal execution: {:.3} ms\nMean execution: {:.3} ms\nRows: {}\nShared block hits: {}\nShared block reads: {}\nTemporary blocks written: {}\nThese counters aggregate completed executions. Current query age belongs to Activity.\n\n# Captured SQL\n{}",
        statement.userid,
        statement.dbid,
        statement.queryid,
        statement.toplevel,
        timestamp(statement.stats_since),
        statement.calls,
        statement.total_exec_ms,
        statement.mean_exec_ms,
        statement.rows,
        statement.shared_blks_hit,
        statement.shared_blks_read,
        statement.temp_blks_written,
        sql(statement.query.as_deref())
    )
}

fn timestamp(timestamp: Option<DateTime<Utc>>) -> String {
    timestamp.map_or_else(
        || "unavailable / not recorded".into(),
        |value| format!("{} UTC", value.format("%Y-%m-%d %H:%M:%S%.6f")),
    )
}

fn optional(value: Option<&str>) -> &str {
    value.unwrap_or("unavailable")
}

fn sql(value: Option<&str>) -> &str {
    value.unwrap_or("Not captured or unavailable. Text requires --include-query-text and sufficient visibility.")
}

fn metric(value: &Observation<f64>, suffix: &str) -> String {
    match value {
        Observation::Available(value) => format!("{value:.3}{suffix}"),
        Observation::Unavailable(reason) => format!("unavailable: {reason}"),
    }
}
