//! Inspectable Markdown exports. Escape database-controlled text before rendering it.
use crate::{
    compare::{BlockingEdge, Comparison, SessionChange, StatementChange, StatementDelta},
    model::{Observation, Session, Snapshot, Statement},
};

pub(crate) fn snapshot_markdown(snapshot: &Snapshot) -> String {
    let mut output = String::from("# PostgreSQL snapshot\n\n");
    output.push_str(&format!(
        "- Source: {} / {} (database OID {})\n- PostgreSQL: {}\n- Server started: {}\n- System identifier: {}\n- Collection started: {}\n- Collection completed: {}\n- Collection duration: {} ms\n- Coverage: {}\n- Snapshot format: {}\n\n",
        escape(&snapshot.source.endpoint), escape(&snapshot.source.database), snapshot.source.database_oid,
        escape(&snapshot.source.server_version), snapshot.source.server_started_at.to_rfc3339(),
        optional(snapshot.source.system_identifier.as_deref()), snapshot.started_at.to_rfc3339(), snapshot.completed_at.to_rfc3339(),
        (snapshot.completed_at - snapshot.started_at).num_milliseconds(),
        if snapshot.is_complete() { "complete" } else { "partial; inspect unavailable metrics and warnings" }, snapshot.schema_version,
    ));
    warnings(&mut output, &snapshot.warnings);
    output.push_str("## Sessions\n\nQuery age is elapsed time for a currently active query. Transaction age includes idle time in an open transaction. Missing fields are unavailable or NULL, never measured zero.\n\n");
    match &snapshot.activity {
        Observation::Available(sessions) => {
            output.push_str(&format!("Observed sessions: {}.\n\n", sessions.len()));
            session_table(&mut output, sessions);
            output.push_str("## Blocking relationships\n\nRelationships come from PostgreSQL blocking PIDs. A blocker may disappear or be outside the visible activity sample. PID 0 can represent a prepared transaction.\n\n");
            let edges: Vec<_> = sessions
                .iter()
                .flat_map(|session| {
                    session
                        .blockers
                        .iter()
                        .map(move |blocker| (session, blocker))
                })
                .collect();
            if edges.is_empty() {
                output.push_str("No blocking relationships observed in this capture.\n\n");
            } else {
                output.push_str("| Waiter PID | Blocker PID | Waiter transaction age (ms) | Wait event |\n| --- | --- | --- | --- |\n");
                for (session, blocker) in edges {
                    output.push_str(&format!(
                        "| {} | {} | {} | {} |\n",
                        session.pid,
                        blocker,
                        number(session.transaction_age_ms),
                        wait(session)
                    ));
                }
                output.push('\n');
            }
        }
        Observation::Unavailable(reason) => {
            unavailable(&mut output, reason);
            output.push_str("## Blocking relationships\n\n");
            unavailable(
                &mut output,
                &format!("Activity collection unavailable: {reason}"),
            );
        }
    }
    output.push_str("## Cumulative statement statistics\n\nThese counters describe executions since statistics began or were reset. Mean execution time is cumulative; it is separate from a currently active query's age. Block counts use PostgreSQL blocks, not bytes.\n\n");
    match &snapshot.statements {
        Observation::Unavailable(reason) => unavailable(&mut output, reason),
        Observation::Available(statistics) => {
            output.push_str(&format!("- Statistics reset: {}\n- Deallocated entries: {}\n- Captured statements: {}\n- Collection truncated: {}\n\n",
                statistics.reset_at.map(|time| time.to_rfc3339()).unwrap_or_else(|| "Unavailable".into()),
                number(statistics.dealloc), statistics.entries.len(), if statistics.truncated { "yes; coverage is incomplete" } else { "no" }));
            if statistics.entries.is_empty() {
                output.push_str(
                    "No statement rows observed. This is distinct from unavailable statistics.\n\n",
                );
            } else {
                output.push_str("| Query ID | User OID | Database OID | Top level | Calls | Total exec (ms) | Mean exec (ms) | Rows | Shared hits (blocks) | Shared reads (blocks) | Temp written (blocks) | Statistics since |\n| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |\n");
                let mut entries: Vec<_> = statistics.entries.iter().collect();
                entries
                    .sort_by_key(|entry| (entry.userid, entry.dbid, entry.queryid, entry.toplevel));
                for statement in entries {
                    output.push_str(&format!(
                        "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
                        statement.queryid,
                        statement.userid,
                        statement.dbid,
                        statement.toplevel,
                        statement.calls,
                        decimal(statement.total_exec_ms),
                        decimal(statement.mean_exec_ms),
                        statement.rows,
                        statement.shared_blks_hit,
                        statement.shared_blks_read,
                        statement.temp_blks_written,
                        statement
                            .stats_since
                            .map(|time| time.to_rfc3339())
                            .unwrap_or_else(|| "Unavailable".into())
                    ));
                }
                output.push('\n');
            }
            for statement in &statistics.entries {
                if let Some(query) = &statement.query {
                    output.push_str(&format!(
                        "Statement SQL (query {}, user {}, database {}, top level {}): {}\n\n",
                        statement.queryid,
                        statement.userid,
                        statement.dbid,
                        statement.toplevel,
                        escape(query)
                    ));
                }
            }
        }
    }
    output.push_str("SQL text appears only when it was explicitly included during collection and was visible to the monitoring role. Captures are observations taken over an interval, not an atomic view of the server.\n");
    output
}

pub(crate) fn comparison_markdown(comparison: &Comparison) -> String {
    let mut output = String::from("# PostgreSQL snapshot comparison\n\n");
    output.push_str(&format!("- Source: {} / {}\n- Before capture completed: {}\n- After capture completed: {}\n- Source compatibility: {}\n- Completion-to-completion interval: {} ms\n\n",
        escape(&comparison.source.endpoint), escape(&comparison.source.database), comparison.before.to_rfc3339(), comparison.after.to_rfc3339(),
        if comparison.source_compatible { "compatible (see identity warnings)" } else { "incompatible" }, number(comparison.interval_ms)));
    output.push_str(&format!("- Before identity: database OID {}, server started {}, system identifier {}\n- After source: {} / {}\n- After identity: database OID {}, server started {}, system identifier {}\n\n",
        comparison.source.database_oid, comparison.source.server_started_at.to_rfc3339(), optional(comparison.source.system_identifier.as_deref()),
        escape(&comparison.after_source.endpoint), escape(&comparison.after_source.database), comparison.after_source.database_oid,
        comparison.after_source.server_started_at.to_rfc3339(), optional(comparison.after_source.system_identifier.as_deref())));
    warnings(&mut output, &comparison.warnings);
    output.push_str("## Session changes\n\nSessions are matched by PID and backend start time. Reused PIDs are separate sessions. Missing backend identity is reported as unknown.\n\n");
    match &comparison.sessions {
        Observation::Unavailable(reason) => unavailable(&mut output, reason),
        Observation::Available(sessions) => {
            output.push_str(&format!("Added: {}. Removed: {}. Changed: {}. Unchanged: {}. Unknown identity before: {}. Unknown identity after: {}.\n\n",
                sessions.added.len(), sessions.removed.len(), sessions.changed.len(), sessions.unchanged, sessions.unidentifiable_before.len(), sessions.unidentifiable_after.len()));
            if !sessions.added.is_empty() {
                output.push_str("### Added sessions\n\n");
                session_table(&mut output, &sessions.added);
            }
            if !sessions.removed.is_empty() {
                output.push_str("### Removed sessions\n\n");
                session_table(&mut output, &sessions.removed);
            }
            if !sessions.changed.is_empty() {
                output.push_str("### Changed sessions\n\n| PID | Backend started | Field | Before | After |\n| --- | --- | --- | --- | --- |\n");
                for change in &sessions.changed {
                    changed_session_table(&mut output, change);
                }
                output.push('\n');
            }
            if !sessions.unidentifiable_before.is_empty() {
                output.push_str("### Unknown session identities before\n\nThese observations cannot be reliably matched to another capture.\n\n");
                session_table(&mut output, &sessions.unidentifiable_before);
            }
            if !sessions.unidentifiable_after.is_empty() {
                output.push_str("### Unknown session identities after\n\nThese observations cannot be reliably matched to another capture.\n\n");
                session_table(&mut output, &sessions.unidentifiable_after);
            }
        }
    }
    output.push_str("## Blocking changes\n\n");
    match &comparison.blocking {
        Observation::Unavailable(reason) => unavailable(&mut output, reason),
        Observation::Available(blocking) => {
            output.push_str(&format!(
                "Added relationships: {}. Removed relationships: {}. Unchanged: {}.\n\n",
                blocking.added.len(),
                blocking.removed.len(),
                blocking.unchanged
            ));
            if !blocking.added.is_empty() || !blocking.removed.is_empty() {
                output.push_str("| Change | Waiter PID | Waiter backend started | Blocker PID | Blocker backend started |\n| --- | --- | --- | --- | --- |\n");
                for edge in &blocking.added {
                    blocking_row(&mut output, "Added", edge);
                }
                for edge in &blocking.removed {
                    blocking_row(&mut output, "Removed", edge);
                }
                output.push('\n');
            }
            if !blocking.unresolved_before.is_empty() || !blocking.unresolved_after.is_empty() {
                output.push_str("Unresolved relationships have unavailable backend identity or a blocker outside the visible sample. They cannot establish an added or removed backend relationship. PID 0 can represent a prepared transaction.\n\n| Capture | Waiter PID | Blocker PID |\n| --- | --- | --- |\n");
                for edge in &blocking.unresolved_before {
                    output.push_str(&format!(
                        "| Before | {} | {} |\n",
                        edge.waiter_pid, edge.blocker_pid
                    ));
                }
                for edge in &blocking.unresolved_after {
                    output.push_str(&format!(
                        "| After | {} | {} |\n",
                        edge.waiter_pid, edge.blocker_pid
                    ));
                }
                output.push('\n');
            }
        }
    }
    output.push_str("## Statement interval metrics\n\nInterval values are after minus before. Interval mean is delta total execution time divided by delta calls; it is unavailable when no executions completed. Statements observed in only one capture have no shared baseline. Absence from a truncated ranking does not establish whether a statement appeared or disappeared on the server. Missing/reset/truncated statistics never become numeric zero.\n\n");
    match &comparison.statements {
        Observation::Unavailable(reason) => unavailable(&mut output, reason),
        Observation::Available(statements) => {
            output.push_str(&format!("Statements across both captures: {}. Truncated before: {}. Truncated after: {}.\n\n### Aggregate interval\n\n",
                statements.entries.len(), statements.before_truncated, statements.after_truncated));
            match &statements.total {
                Observation::Unavailable(reason) => unavailable(&mut output, reason),
                Observation::Available(delta) => delta_table(&mut output, delta),
            }
            for statement in &statements.entries {
                statement_comparison(&mut output, statement);
            }
        }
    }
    output
}

fn warnings(output: &mut String, warnings: &[String]) {
    if !warnings.is_empty() {
        output.push_str("## Warnings\n\n");
        for warning in warnings {
            output.push_str(&format!("- {}\n", escape(warning)));
        }
        output.push('\n');
    }
}

fn unavailable(output: &mut String, reason: &str) {
    output.push_str(&format!("Unavailable: {}.\n\n", escape(reason)));
}

fn session_table(output: &mut String, sessions: &[Session]) {
    if sessions.is_empty() {
        return;
    }
    output.push_str("| PID | Backend started | User | Database | Application | Client | State | Active query age (ms) | Transaction age (ms) | Wait event | Blocker PIDs |\n| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |\n");
    let mut sorted: Vec<_> = sessions.iter().collect();
    sorted.sort_by_key(|session| (session.pid, session.backend_start));
    for session in sorted {
        output.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            session.pid,
            session
                .backend_start
                .map(|time| time.to_rfc3339())
                .unwrap_or_else(|| "Unavailable / NULL".into()),
            optional(session.user.as_deref()),
            optional(session.database.as_deref()),
            escape(&session.application),
            optional(session.client.as_deref()),
            optional(session.state.as_deref()),
            number(session.query_age_ms),
            number(session.transaction_age_ms),
            wait(session),
            blockers(session)
        ));
    }
    output.push('\n');
    for session in sessions {
        if let Some(query) = &session.query {
            output.push_str(&format!(
                "Session SQL (PID {}): {}\n\n",
                session.pid,
                escape(query)
            ));
        }
    }
}

fn changed_session_table(output: &mut String, change: &SessionChange) {
    for field in &change.fields {
        output.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            change.identity.pid,
            change.identity.backend_start.to_rfc3339(),
            escape(field),
            session_field(&change.before, field),
            session_field(&change.after, field)
        ));
    }
}

fn session_field(session: &Session, field: &str) -> String {
    match field {
        "user" => optional(session.user.as_deref()),
        "database" => optional(session.database.as_deref()),
        "application" => escape(&session.application),
        "client" => optional(session.client.as_deref()),
        "state" => optional(session.state.as_deref()),
        "query_age_ms" => number(session.query_age_ms),
        "transaction_age_ms" => number(session.transaction_age_ms),
        "wait_event_type" => optional(session.wait_event_type.as_deref()),
        "wait_event" => optional(session.wait_event.as_deref()),
        "query" => session
            .query
            .as_deref()
            .map(escape)
            .unwrap_or_else(|| "Not collected / unavailable".into()),
        "blockers" => blockers(session),
        _ => "Unavailable".into(),
    }
}

fn blocking_row(output: &mut String, kind: &str, edge: &BlockingEdge) {
    output.push_str(&format!(
        "| {kind} | {} | {} | {} | {} |\n",
        edge.waiter.pid,
        edge.waiter.backend_start.to_rfc3339(),
        edge.blocker.pid,
        edge.blocker.backend_start.to_rfc3339()
    ));
}

fn delta_table(output: &mut String, delta: &StatementDelta) {
    output.push_str("| Metric | Interval value |\n| --- | --- |\n");
    output.push_str(&format!("| Calls | {} |\n| Total execution (ms) | {} |\n| Mean execution (ms/call) | {} |\n| Rows | {} |\n| Shared hits (blocks) | {} |\n| Shared reads (blocks) | {} |\n| Temp written (blocks) | {} |\n\n",
        delta.calls, decimal(delta.total_exec_ms), delta.mean_exec_ms.map(decimal).unwrap_or_else(|| "Unavailable (no calls)".into()),
        delta.rows, delta.shared_blks_hit, delta.shared_blks_read, delta.temp_blks_written));
}

fn statement_comparison(output: &mut String, change: &StatementChange) {
    let identity = &change.identity;
    let kind = match (&change.before, &change.after) {
        (None, Some(_)) => "Only observed after",
        (Some(_), None) => "Only observed before",
        _ => "Matched",
    };
    output.push_str(&format!(
        "### Statement {}: {kind}\n\nUser OID: {}. Database OID: {}. Top level: {}.\n\n",
        identity.queryid, identity.userid, identity.dbid, identity.toplevel
    ));
    if let Observation::Unavailable(reason) = &change.delta {
        unavailable(output, reason);
    }
    output.push_str("| Metric | Before (cumulative) | After (cumulative) | Interval |\n| --- | --- | --- | --- |\n");
    for metric in [
        "Calls",
        "Total execution (ms)",
        "Mean execution (ms/call)",
        "Rows",
        "Shared hits (blocks)",
        "Shared reads (blocks)",
        "Temp written (blocks)",
        "Statistics since",
    ] {
        output.push_str(&format!(
            "| {metric} | {} | {} | {} |\n",
            cumulative_metric(change.before.as_ref(), metric),
            cumulative_metric(change.after.as_ref(), metric),
            delta_metric(change.delta.available(), metric)
        ));
    }
    output.push('\n');
    if let Some(query) = change
        .after
        .as_ref()
        .and_then(|s| s.query.as_deref())
        .or_else(|| change.before.as_ref().and_then(|s| s.query.as_deref()))
    {
        output.push_str(&format!("SQL: {}\n\n", escape(query)));
    }
}

fn cumulative_metric(statement: Option<&Statement>, metric: &str) -> String {
    let Some(statement) = statement else {
        return "Not observed".into();
    };
    match metric {
        "Calls" => statement.calls.to_string(),
        "Total execution (ms)" => decimal(statement.total_exec_ms),
        "Mean execution (ms/call)" => decimal(statement.mean_exec_ms),
        "Rows" => statement.rows.to_string(),
        "Shared hits (blocks)" => statement.shared_blks_hit.to_string(),
        "Shared reads (blocks)" => statement.shared_blks_read.to_string(),
        "Temp written (blocks)" => statement.temp_blks_written.to_string(),
        "Statistics since" => statement
            .stats_since
            .map(|time| time.to_rfc3339())
            .unwrap_or_else(|| "Unavailable".into()),
        _ => "Unavailable".into(),
    }
}

fn delta_metric(delta: Option<&StatementDelta>, metric: &str) -> String {
    if metric == "Statistics since" {
        return "Not an interval metric".into();
    }
    let Some(delta) = delta else {
        return "Unavailable".into();
    };
    match metric {
        "Calls" => delta.calls.to_string(),
        "Total execution (ms)" => decimal(delta.total_exec_ms),
        "Mean execution (ms/call)" => delta
            .mean_exec_ms
            .map(decimal)
            .unwrap_or_else(|| "Unavailable (no calls)".into()),
        "Rows" => delta.rows.to_string(),
        "Shared hits (blocks)" => delta.shared_blks_hit.to_string(),
        "Shared reads (blocks)" => delta.shared_blks_read.to_string(),
        "Temp written (blocks)" => delta.temp_blks_written.to_string(),
        _ => "Unavailable".into(),
    }
}

fn wait(session: &Session) -> String {
    match (&session.wait_event_type, &session.wait_event) {
        (Some(kind), Some(event)) => format!("{} / {}", escape(kind), escape(event)),
        (Some(kind), None) => format!("{} / unavailable", escape(kind)),
        (None, Some(event)) => format!("unavailable / {}", escape(event)),
        (None, None) => "No wait observed / unavailable".into(),
    }
}

fn blockers(session: &Session) -> String {
    if session.blockers.is_empty() {
        return "None observed".into();
    }
    let mut pids = session.blockers.clone();
    pids.sort_unstable();
    pids.dedup();
    pids.iter()
        .map(i32::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

fn optional(value: Option<&str>) -> String {
    value
        .map(escape)
        .unwrap_or_else(|| "Unavailable / NULL".into())
}
fn number(value: Option<i64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "Unavailable / NULL".into())
}
fn decimal(value: f64) -> String {
    if value.is_finite() {
        format!("{value:.3}")
    } else {
        "Unavailable (invalid number)".into()
    }
}

fn escape(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '\\' | '|' | '*' | '_' | '`' | '[' | ']' | '#' => {
                result.push('\\');
                result.push(character);
            }
            '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}' => {
                result.push(' ')
            }
            c if c.is_control() => result.push(' '),
            c => result.push(c),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compare::{compare, tests::snapshot};
    use chrono::Duration;

    #[test]
    fn unavailable_is_distinct_from_zero_and_sql_is_not_fabricated() {
        let mut snapshot = snapshot();
        snapshot.statements =
            Observation::Unavailable("pg_stat_statements is not installed".into());
        let report = snapshot_markdown(&snapshot);
        assert!(report.contains("Coverage: partial"));
        assert!(report.contains("Unavailable: pg\\_stat\\_statements is not installed"));
        assert!(!report.contains("Session SQL ("));
        assert!(report.contains("Active query age (ms)"));
    }

    #[test]
    fn database_text_cannot_inject_terminal_controls_html_or_markdown() {
        let mut snapshot = snapshot();
        snapshot.source.database = "a|b\n# forged\u{1b}[31m<script>".into();
        if let Observation::Available(sessions) = &mut snapshot.activity {
            sessions[0].query = Some("SELECT '[x](https://evil)'\u{202e}".into());
        }
        let report = snapshot_markdown(&snapshot);
        assert!(!report.contains('\u{1b}'));
        assert!(!report.contains('\u{202e}'));
        assert!(!report.contains("<script>"));
        assert!(!report.contains("\n# forged"));
        assert!(report.contains("a\\|b"));
        assert!(report.contains("&lt;script&gt;"));
        assert!(report.contains("\\[x\\]"));
    }

    #[test]
    fn comparison_report_contains_inspectable_cumulative_and_interval_values() {
        let before = snapshot();
        let mut after = before.clone();
        after.started_at += Duration::seconds(10);
        after.completed_at += Duration::seconds(10);
        if let Observation::Available(stats) = &mut after.statements {
            stats.entries[0].calls = 12;
            stats.entries[0].total_exec_ms = 300.0;
            stats.entries[0].mean_exec_ms = 25.0;
        }
        let report = comparison_markdown(&compare(&before, &after));
        assert!(report.contains("| Calls | 10 | 12 | 2 |"));
        assert!(report.contains("| Mean execution (ms/call) | 20.000 | 25.000 | 50.000 |"));
        assert!(report.contains("Unchanged: 1"));
    }

    #[test]
    fn truncated_rankings_report_observation_changes_without_claiming_removal() {
        let mut before = snapshot();
        if let Observation::Available(stats) = &mut before.statements {
            stats.truncated = true;
        }
        let mut after = before.clone();
        after.started_at += Duration::seconds(10);
        after.completed_at += Duration::seconds(10);
        if let Observation::Available(stats) = &mut after.statements {
            stats.entries[0].queryid = 99;
        }
        let report = comparison_markdown(&compare(&before, &after));
        let statements = report
            .split("## Statement interval metrics")
            .nth(1)
            .unwrap();
        assert!(statements.contains("### Statement 7: Only observed before"));
        assert!(statements.contains("### Statement 99: Only observed after"));
        assert!(statements.contains("Absence from a truncated ranking does not establish"));
        assert!(statements.contains("Unavailable"));
        assert!(!statements.contains("Added"));
        assert!(!statements.contains("Removed"));
    }
}
