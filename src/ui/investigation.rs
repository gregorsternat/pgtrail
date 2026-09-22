//! Investigation views consume state only; collection and persistence live elsewhere.
use chrono::{DateTime, Utc};

use super::*;
use crate::{app::PromptKind, diagnostics::Severity, model::Snapshot};

pub(super) fn overview(frame: &mut Frame, area: Rect, app: &App) {
    let Some(snapshot) = app.snapshot() else {
        render_empty(
            frame,
            area,
            " Investigation overview ",
            "Waiting for an observation.\n\nConnect with --profile NAME or PGTRAIL_DATABASE_URL.\nUse --demo to explore synthetic data, or 5 to open local history.",
        );
        return;
    };
    let Some(analysis) = &app.analysis else {
        return;
    };
    let details_height = if area.height >= 17 {
        10
    } else if area.height >= 10 {
        5
    } else {
        0
    };
    let [summary, list, details] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(details_height),
    ])
    .areas(area);
    let activity = snapshot.activity.available().map_or_else(
        || "Activity unavailable".into(),
        |sessions| {
            format!(
                "{} visible sessions · {} waiting on blockers",
                sessions.len(),
                sessions.iter().filter(|s| !s.blockers.is_empty()).count()
            )
        },
    );
    let mut lines = vec![Line::raw(format!(
        "{} · {} · {} findings",
        if snapshot.is_complete() {
            "Complete observation"
        } else {
            "INCOMPLETE observation"
        },
        activity,
        analysis.findings.len()
    ))];
    if let Observation::Unavailable(reason) = &snapshot.statements {
        lines.push(
            Line::raw(format!("Statement metrics unavailable: {}", clean(reason))).fg(WARNING),
        );
    } else {
        lines.push(Line::raw("Findings are investigation hints. Select one and press Enter for full evidence and next steps.").fg(MUTED));
    }
    lines.push(
        Line::raw(format!(
            "Coverage: {}",
            if analysis.coverage.is_empty() {
                "No missing sections reported".into()
            } else {
                analysis
                    .coverage
                    .iter()
                    .map(|s| clean(s))
                    .collect::<Vec<_>>()
                    .join(" · ")
            }
        ))
        .fg(WARNING),
    );
    frame.render_widget(Paragraph::new(lines), summary);
    let findings = app.filtered_findings();
    if findings.is_empty() {
        render_empty(
            frame,
            list,
            " Findings ",
            if app.filter.is_empty() {
                "No configured findings triggered in the available evidence.\nThis does not prove database health; review coverage and workload context."
            } else {
                "No findings match this filter. Press Esc to clear it."
            },
        );
    } else {
        let rows = findings.iter().map(|finding| {
            Row::new(vec![
                severity(finding.severity).0.to_owned(),
                clean(&finding.title),
            ])
            .style(Style::new().fg(severity(finding.severity).1))
        });
        let table = Table::new(rows, [Constraint::Length(9), Constraint::Min(10)])
            .header(table_header(vec!["Severity", "Investigation finding"]))
            .block(panel(" Findings · Enter: full details "));
        render_table(frame, list, table, app.selected());
    }
    if details_height > 0 {
        let mut lines = Vec::new();
        if let Some(finding) = findings.get(app.selected()) {
            lines.push(Line::raw(clean(&finding.interpretation)));
            lines.push(Line::raw("Evidence").fg(ACCENT).bold());
            lines.extend(
                finding
                    .evidence
                    .iter()
                    .map(|e| Line::raw(format!("• {}", clean(e)))),
            );
            lines.push(Line::raw("Next steps").fg(ACCENT).bold());
            lines.extend(
                finding
                    .next_steps
                    .iter()
                    .map(|step| Line::raw(format!("• {}", clean(step)))),
            );
        } else {
            lines.push(Line::raw("Coverage and interpretation limits").fg(ACCENT));
            lines.extend(
                analysis
                    .coverage
                    .iter()
                    .map(|reason| Line::raw(clean(reason))),
            );
        }
        frame.render_widget(
            Paragraph::new(lines)
                .block(panel(" Evidence · Enter expands all details "))
                .wrap(Wrap { trim: false }),
            details,
        );
    }
}

pub(super) fn database(frame: &mut Frame, area: Rect, app: &App) {
    let Some(snapshot) = observed(frame, area, app, " Database ") else {
        return;
    };
    let mut lines = Vec::new();
    heading(&mut lines, "CURRENT DATABASE AND CLUSTER CAPACITY");
    match &snapshot.health.database {
        Observation::Unavailable(reason) => unavailable(&mut lines, "Database", reason),
        Observation::Available(db) => {
            lines.push(Line::raw(format!(
                "Database size {} · connections {} in this database",
                bytes(db.size_bytes as f64),
                db.num_backends
            )));
            lines.push(Line::raw(format!(
                "Cluster client connections {} / {} max · {} reserved",
                db.cluster_backends, db.max_connections, db.reserved_connections
            )));
            lines.push(Line::raw(format!(
                "Autovacuum {} · track_counts {} · I/O timing {} · database XID age {}",
                on(db.autovacuum),
                on(db.track_counts),
                on(db.track_io_timing),
                db.frozen_xid_age
            )));
            lines.push(Line::raw(format!(
                "Database counter reset: {}",
                timestamp(db.stats_reset)
            )));
        }
    }
    heading(
        &mut lines,
        "INTERVAL RATES · PREVIOUS TO CURRENT OBSERVATION",
    );
    if let Some(analysis) = &app.analysis {
        lines.push(Line::raw(format!(
            "Completion-to-completion interval: {}",
            analysis
                .rates
                .elapsed_seconds
                .map_or_else(|| "unavailable".into(), |s| format!("{s:.3} s"))
        )));
        match &analysis.rates.database {
            Observation::Unavailable(reason) => unavailable(&mut lines, "Rates", reason),
            Observation::Available(rates) => {
                lines.push(Line::raw(clean(&rates.baseline)).fg(MUTED));
                for (label, metric, unit) in [
                    ("Transactions", &rates.transactions_per_second, "/s"),
                    ("Commits", &rates.commits_per_second, "/s"),
                    ("Rollbacks", &rates.rollbacks_per_second, "/s"),
                    ("Rollback share", &rates.rollback_percent, "%"),
                    ("Shared block reads", &rates.reads_per_second, "/s"),
                    ("Shared block hits", &rates.hits_per_second, "/s"),
                    ("Shared-buffer hit share", &rates.cache_hit_percent, "%"),
                    ("Temporary writes", &rates.temp_bytes_per_second, " B/s"),
                    ("Deadlocks", &rates.deadlocks_per_second, "/s"),
                    ("Rows inserted", &rates.inserted_per_second, "/s"),
                    ("Rows updated", &rates.updated_per_second, "/s"),
                    ("Rows deleted", &rates.deleted_per_second, "/s"),
                    ("Block read time", &rates.read_ms_per_second, " ms/s"),
                    ("Block write time", &rates.write_ms_per_second, " ms/s"),
                ] {
                    lines.push(Line::raw(format!(
                        "{label}: {}",
                        metric_with_reason(metric, unit)
                    )));
                }
            }
        }
    }
    heading(
        &mut lines,
        "RECENT LIVE SAMPLES · EACH GRAPH SCALES INDEPENDENTLY",
    );
    if app.is_offline() {
        lines.push(
            Line::raw("Live trends are hidden while inspecting an offline capture.").fg(MUTED),
        );
    } else {
        let width = usize::from(area.width.saturating_sub(24));
        for (label, values) in [
            (
                "Transactions/s",
                app.trend.iter().map(|p| p.transactions).collect::<Vec<_>>(),
            ),
            (
                "WAL bytes/s",
                app.trend.iter().map(|p| p.wal_bytes).collect(),
            ),
            (
                "Active sessions",
                app.trend.iter().map(|p| p.active).collect(),
            ),
            (
                "Blocked sessions",
                app.trend.iter().map(|p| p.blocked).collect(),
            ),
        ] {
            lines.push(Line::raw(format!(
                "{label:17} {}",
                sparkline(&values, width)
            )));
        }
        if let (Some(first), Some(last)) = (
            app.trend.iter().rev().take(width).next_back(),
            app.trend.back(),
        ) {
            lines.push(
                Line::raw(format!(
                    "{}–{} UTC · {} visible samples · × = unavailable, ▁ = zero",
                    first.at.format("%H:%M:%S"),
                    last.at.format("%H:%M:%S"),
                    app.trend.len().min(width)
                ))
                .fg(MUTED),
            );
        }
    }
    heading(
        &mut lines,
        "CUMULATIVE DATABASE COUNTERS · SINCE THEIR RESET",
    );
    if let Observation::Available(db) = &snapshot.health.database {
        lines.push(Line::raw(format!(
            "Commits {} · rollbacks {} · deadlocks {} · recovery conflicts {}",
            db.xact_commit, db.xact_rollback, db.deadlocks, db.conflicts
        )));
        lines.push(Line::raw(format!(
            "Temporary files {} · written {} · shared reads {} · hits {}",
            db.temp_files,
            bytes(db.temp_bytes as f64),
            db.blks_read,
            db.blks_hit
        )));
        lines.push(Line::raw(format!(
            "Tuples returned {} · fetched {} · inserted {} · updated {} · deleted {}",
            db.tup_returned, db.tup_fetched, db.tup_inserted, db.tup_updated, db.tup_deleted
        )));
    }
    lines.push(
        Line::raw(
            "Shared-buffer hits do not measure the OS cache. Statistics can lag current activity.",
        )
        .fg(MUTED),
    );
    scroll(frame, area, app, " Database · j/k: scroll ", lines);
}

pub(super) fn relations(frame: &mut Frame, area: Rect, app: &App) {
    let Some(snapshot) = observed(frame, area, app, " Relations ") else {
        return;
    };
    let stats = match &snapshot.health.tables {
        Observation::Unavailable(reason) => {
            render_empty(frame, area, " Relations unavailable ", &clean(reason));
            return;
        }
        Observation::Available(stats) => stats,
    };
    let detail_height = if area.height >= 13 { 8 } else { 0 };
    let [table_area, details] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(detail_height)]).areas(area);
    let limited = if stats.truncated {
        " · LIMITED coverage"
    } else {
        ""
    };
    let mut detail_lines = Vec::new();
    if app.show_indexes {
        let indexes = app.filtered_indexes();
        let sort = ["size ↓", "invalid first", "scans ↓"][app.relation_sort.min(2)];
        let rows = indexes.iter().map(|i| {
            Row::new(vec![
                format!("{}.{}", clean(&i.schema), clean(&i.name)),
                bytes(i.size_bytes as f64),
                i.scans.to_string(),
                if i.valid {
                    "valid".into()
                } else {
                    "INVALID".into()
                },
                if i.primary {
                    "primary".into()
                } else if i.unique {
                    "unique".into()
                } else {
                    "ordinary".into()
                },
            ])
            .style(Style::new().fg(if i.valid { Color::Reset } else { WARNING }))
        });
        let table = Table::new(
            rows,
            [
                Constraint::Min(20),
                Constraint::Length(12),
                Constraint::Length(12),
                Constraint::Length(9),
                Constraint::Length(10),
            ],
        )
        .header(table_header(vec![
            "Index",
            "Size",
            "Scans",
            "Validity",
            "Constraint",
        ]))
        .block(panel(format!(
            " Indexes · sort: {sort} (s) · v: tables{limited} "
        )));
        render_table(frame, table_area, table, app.selected());
        if let Some(i) = indexes.get(app.selected()) {
            detail_lines.push(Line::raw(format!(
                "{}.{} on {} · index OID {} · table OID {}",
                clean(&i.schema),
                clean(&i.name),
                clean(&i.table),
                i.oid,
                i.table_oid
            )));
            detail_lines.push(Line::raw(format!(
                "Scans {} · index tuples read {} · heap tuples fetched {}",
                i.scans, i.tuples_read, i.tuples_fetched
            )));
            detail_lines.push(Line::raw("Scan counts are cumulative and can reset; low usage alone does not justify removal.").fg(MUTED));
            detail_lines.push(Line::raw("Check constraints, workload coverage and query plans before changing an index.").fg(MUTED));
        } else {
            detail_lines.push(Line::raw("No indexes match this observation and filter."));
        }
    } else {
        let tables = app.filtered_tables();
        let wide = area.width >= 100;
        let sort =
            ["size ↓", "estimated dead tuples ↓", "sequential scans ↓"][app.relation_sort.min(2)];
        let rows = tables.iter().map(|t| {
            let mut cells = vec![
                format!("{}.{}", clean(&t.schema), clean(&t.name)),
                bytes(t.total_bytes as f64),
                t.live_tuples.to_string(),
                t.dead_tuples.to_string(),
                t.seq_scan.to_string(),
            ];
            if wide {
                cells.push(optional_integer(t.idx_scan));
            }
            Row::new(cells)
        });
        let mut widths = vec![
            Constraint::Min(18),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(10),
        ];
        let mut headers = vec!["Table", "Total size", "Est. live", "Est. dead", "Seq scans"];
        if wide {
            widths.push(Constraint::Length(12));
            headers.push("Idx scans");
        }
        let table = Table::new(rows, widths)
            .header(table_header(headers))
            .block(panel(format!(
                " Tables · sort: {sort} (s) · v: indexes{limited} "
            )));
        render_table(frame, table_area, table, app.selected());
        if let Some(t) = tables.get(app.selected()) {
            detail_lines.push(Line::raw(format!(
                "{}.{} · OID {} · table {} · indexes {} · XID age {}",
                clean(&t.schema),
                clean(&t.name),
                t.oid,
                bytes(t.table_bytes as f64),
                bytes(t.index_bytes as f64),
                t.frozen_xid_age
            )));
            detail_lines.push(Line::raw(format!(
                "Modifications since analyze (est.) {} · scans seq {} / index {}",
                t.modified_since_analyze,
                t.seq_scan,
                optional_integer(t.idx_scan)
            )));
            detail_lines.push(Line::raw(format!(
                "Vacuum {} · autovacuum {} (UTC)",
                maintenance_time(t.last_vacuum),
                maintenance_time(t.last_autovacuum)
            )));
            detail_lines.push(Line::raw(format!(
                "Analyze {} · autoanalyze {} (UTC)",
                maintenance_time(t.last_analyze),
                maintenance_time(t.last_autoanalyze)
            )));
            detail_lines.push(
                Line::raw("Tuples are estimates, not measured bloat. Scans are cumulative.")
                    .fg(MUTED),
            );
            detail_lines.push(
                Line::raw("— = no timestamp recorded; maintenance may have run before reset.")
                    .fg(MUTED),
            );
        } else {
            detail_lines.push(Line::raw("No tables match this observation and filter."));
        }
    }
    if detail_height > 0 {
        frame.render_widget(
            Paragraph::new(detail_lines)
                .block(panel(" Selected relation · maintenance context "))
                .wrap(Wrap { trim: false }),
            details,
        );
    }
}

pub(super) fn replication(frame: &mut Frame, area: Rect, app: &App) {
    let Some(snapshot) = observed(frame, area, app, " Replication ") else {
        return;
    };
    let stats = match &snapshot.health.replication {
        Observation::Unavailable(reason) => {
            render_empty(frame, area, " Replication unavailable ", &clean(reason));
            return;
        }
        Observation::Available(stats) => stats,
    };
    let mut lines = Vec::new();
    heading(
        &mut lines,
        if stats.in_recovery {
            "STANDBY · RECOVERY IN PROGRESS"
        } else {
            "PRIMARY · NOT IN RECOVERY"
        },
    );
    if stats.in_recovery {
        lines.push(Line::raw(format!(
            "Receive-to-replay backlog: {}",
            optional_bytes(stats.receive_replay_lag_bytes)
        )));
        lines.push(Line::raw(format!(
            "Time since last replayed transaction: {}",
            optional_float(stats.replay_delay_seconds, " s")
        )));
        lines.push(Line::raw("Time since replay can grow on an idle primary; it is not a measurement of pending work.").fg(MUTED));
    }
    heading(&mut lines, "WAL SENDERS · CLUSTER SCOPE");
    if stats.senders.is_empty() {
        lines.push(Line::raw("No WAL senders observed."));
    }
    for sender in &stats.senders {
        lines.push(
            Line::raw(format!(
                "PID {} · {} · client {} · state {} · sync {}",
                sender.pid,
                clean(&sender.application),
                value(sender.client.as_deref()),
                value(sender.state.as_deref()),
                value(sender.sync_state.as_deref())
            ))
            .fg(ACCENT),
        );
        lines.push(Line::raw(format!(
            "  Sent-to-replay backlog {} · reported replay lag {}",
            optional_bytes(sender.sent_replay_lag_bytes),
            optional_float(sender.replay_lag_ms, " ms")
        )));
        lines.push(
            Line::raw(
                "  Lag may be NULL on idle or disconnected senders; it is not catch-up time.",
            )
            .fg(MUTED),
        );
    }
    heading(&mut lines, "REPLICATION SLOTS · CLUSTER SCOPE");
    if stats.slots.is_empty() {
        lines.push(Line::raw("No replication slots observed."));
    }
    for slot in &stats.slots {
        lines.push(
            Line::raw(format!(
                "{} · {} · {} · database {}",
                clean(&slot.name),
                clean(&slot.slot_type),
                if slot.active { "active" } else { "inactive" },
                value(slot.database.as_deref())
            ))
            .fg(if slot.active { ACCENT } else { WARNING }),
        );
        lines.push(Line::raw(format!(
            "  Retained WAL {} · WAL status {} · safe WAL headroom {}",
            optional_bytes(slot.retained_bytes),
            value(slot.wal_status.as_deref()),
            optional_bytes(slot.safe_wal_size)
        )));
        lines.push(Line::raw("  NULL headroom can mean unlimited retention or an unusable slot; inspect WAL status.").fg(MUTED));
    }
    lines.push(Line::raw("Retention is a potential disk-pressure signal. pgtrail never drops slots or changes replication.").fg(MUTED));
    scroll(frame, area, app, " Replication · j/k: scroll ", lines);
}

pub(super) fn io(frame: &mut Frame, area: Rect, app: &App) {
    let Some(snapshot) = observed(frame, area, app, " I/O and maintenance ") else {
        return;
    };
    let mut lines = Vec::new();
    heading(&mut lines, "WAL GENERATION · CLUSTER SCOPE");
    match &snapshot.health.wal {
        Observation::Unavailable(reason) => unavailable(&mut lines, "WAL", reason),
        Observation::Available(wal) => {
            lines.push(Line::raw(format!(
                "Cumulative {} · records {} · full-page images {} · buffers full {}",
                bytes(wal.bytes),
                wal.records,
                wal.full_page_images,
                wal.buffers_full
            )));
            lines.push(Line::raw(format!(
                "Counter reset: {}",
                timestamp(wal.stats_reset)
            )));
        }
    }
    if let Some(analysis) = &app.analysis {
        match &analysis.rates.wal {
            Observation::Unavailable(reason) => unavailable(&mut lines, "WAL interval", reason),
            Observation::Available(wal) => {
                lines.push(Line::raw(format!(
                    "Bytes: {} · records: {} · buffers full: {}",
                    metric_with_reason(&wal.bytes_per_second, " B/s"),
                    metric_with_reason(&wal.records_per_second, "/s"),
                    metric_with_reason(&wal.buffers_full_per_second, "/s")
                )));
            }
        }
    }
    heading(
        &mut lines,
        "I/O BY BACKEND / OBJECT / CONTEXT · CUMULATIVE CLUSTER COUNTERS",
    );
    match &snapshot.health.io {
        Observation::Unavailable(reason) => unavailable(&mut lines, "I/O", reason),
        Observation::Available(rows) => {
            if rows.is_empty() {
                lines.push(Line::raw("No I/O statistics returned."));
            }
            for row in rows {
                lines.push(
                    Line::raw(format!(
                        "{} / {} / {} · reset {}",
                        clean(&row.backend_type),
                        clean(&row.object),
                        clean(&row.context),
                        timestamp(row.stats_reset)
                    ))
                    .fg(ACCENT),
                );
                lines.push(Line::raw(format!(
                    "  Reads {} / {} · writes {} / {} · hits {} · evictions {} · fsyncs {}",
                    optional_integer(row.reads),
                    optional_float(row.read_time_ms, " ms"),
                    optional_integer(row.writes),
                    optional_float(row.write_time_ms, " ms"),
                    optional_integer(row.hits),
                    optional_integer(row.evictions),
                    optional_integer(row.fsyncs)
                )));
            }
        }
    }
    lines.push(
        Line::raw(
            "— means unavailable or inapplicable. Timing requires the relevant I/O timing setting.",
        )
        .fg(MUTED),
    );
    heading(&mut lines, "VACUUM PROGRESS · CURRENT DATABASE");
    match &snapshot.health.vacuum {
        Observation::Unavailable(reason) => unavailable(&mut lines, "Vacuum progress", reason),
        Observation::Available(rows) => {
            if rows.is_empty() {
                lines.push(Line::raw("No active VACUUM observed."));
            }
            for row in rows {
                let relation = snapshot
                    .health
                    .tables
                    .available()
                    .and_then(|tables| {
                        tables
                            .tables
                            .iter()
                            .find(|table| table.oid == row.table_oid)
                    })
                    .map_or_else(
                        || format!("table OID {}", row.table_oid),
                        |table| format!("{}.{}", clean(&table.schema), clean(&table.name)),
                    );
                lines.push(
                    Line::raw(format!(
                        "PID {} · {relation} · {}",
                        row.pid,
                        clean(&row.phase)
                    ))
                    .fg(ACCENT),
                );
                lines.push(Line::raw(format!("  Heap blocks scanned {} / {} · vacuumed {} (phase counters, not total completion)", row.heap_blocks_scanned, row.heap_blocks_total, row.heap_blocks_vacuumed)));
            }
        }
    }
    scroll(
        frame,
        area,
        app,
        " I/O, WAL and maintenance · j/k: scroll ",
        lines,
    );
}

pub(super) fn statement_rates(frame: &mut Frame, area: Rect, app: &App) {
    let Some(analysis) = &app.analysis else {
        render_empty(
            frame,
            area,
            " Statement intervals ",
            "No observation available yet.",
        );
        return;
    };
    if let Observation::Unavailable(reason) = &analysis.rates.statements {
        render_empty(
            frame,
            area,
            " Statement intervals unavailable ",
            &format!(
                "{}\n\nTwo compatible observations and unchanged statistics epochs are required.\nPress v for cumulative statistics. Offline interval reports are available through History A/B comparison.",
                clean(reason)
            ),
        );
        return;
    }
    let rates = app.filtered_statement_rates();
    if rates.is_empty() {
        render_empty(
            frame,
            area,
            " Statement intervals ",
            "No statement identities match this interval and filter. Press v for cumulative statistics.",
        );
        return;
    }
    let details_height = if area.height >= 13 { 8 } else { 0 };
    let [table_area, details] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(details_height)]).areas(area);
    let wide = area.width >= 100;
    let rows = rates.iter().map(|r| {
        let mut cells = vec![
            r.identity.queryid.to_string(),
            metric(&r.calls_per_second),
            metric(&r.exec_ms_per_second),
            metric(&r.mean_exec_ms),
            metric(&r.temp_blocks_per_second),
        ];
        if wide {
            cells.push(metric(&r.shared_reads_per_second));
        }
        Row::new(cells)
    });
    let mut widths = vec![
        Constraint::Min(18),
        Constraint::Length(8),
        Constraint::Length(10),
        Constraint::Length(11),
        Constraint::Length(13),
    ];
    let mut headers = vec![
        "Query ID",
        "Calls/s",
        "Exec ms/s",
        "Mean exec ms",
        "Temp blocks/s",
    ];
    if wide {
        widths.push(Constraint::Length(12));
        headers.push("Reads/s");
    }
    let table = Table::new(rows, widths)
        .header(table_header(headers))
        .block(panel(format!(
            " Statements · interval {:.2}s · sort: {} (s) · v: cumulative ",
            analysis.rates.elapsed_seconds.unwrap_or_default(),
            app.sort.title()
        )));
    render_table(frame, table_area, table, app.selected());
    if let Some(rate) = rates.get(app.selected()) {
        let mut lines = vec![Line::raw(format!(
            "User {} · database {} · query {} · top-level {}",
            rate.identity.userid, rate.identity.dbid, rate.identity.queryid, rate.identity.toplevel
        ))];
        for (label, observation, unit) in [
            ("Calls", &rate.calls_per_second, "/s"),
            ("Execution", &rate.exec_ms_per_second, " ms/s"),
            ("Mean completed execution", &rate.mean_exec_ms, " ms"),
            ("Shared reads", &rate.shared_reads_per_second, " blocks/s"),
            (
                "Temporary writes",
                &rate.temp_blocks_per_second,
                " blocks/s",
            ),
        ] {
            lines.push(Line::raw(format!(
                "{label}: {}",
                metric_with_reason(observation, unit)
            )));
        }
        frame.render_widget(
            Paragraph::new(lines)
                .block(panel(" Interval details · — retains its reason below "))
                .wrap(Wrap { trim: false }),
            details,
        );
    }
}

pub(super) fn incidents(frame: &mut Frame, area: Rect, app: &App) {
    let incidents = app.filtered_incidents();
    if incidents.is_empty() && app.incident.is_none() {
        render_empty(
            frame,
            area,
            " Local incidents ",
            "No incidents match this view.\n\nPress i to create a case, c to capture evidence, and n to add a note.\nOpen incidents group related observations across database restarts.\nReports can be exported with: pgtrail incident show ID --output incident.md",
        );
        return;
    }
    let [table_area, details] =
        Layout::vertical([Constraint::Percentage(42), Constraint::Percentage(58)]).areas(area);
    let rows = incidents.iter().map(|i| {
        Row::new(vec![
            if app.active_incident == Some(i.id) {
                "●".into()
            } else {
                String::new()
            },
            i.id.to_string(),
            clean(&i.title),
            if i.closed_at.is_some() {
                "closed".into()
            } else {
                "open".into()
            },
            i.capture_count.to_string(),
            i.note_count.to_string(),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(1),
            Constraint::Length(6),
            Constraint::Min(18),
            Constraint::Length(7),
            Constraint::Length(8),
            Constraint::Length(6),
        ],
    )
    .header(table_header(vec![
        "", "ID", "Incident", "State", "Captures", "Notes",
    ]))
    .block(panel(
        " Incidents · ● active capture destination · Enter: open ",
    ));
    render_table(frame, table_area, table, app.selected());
    if let Some(incident) = &app.incident {
        let mut lines = vec![Line::raw(format!(
            "Opened {} · {}",
            timestamp(Some(incident.summary.created_at)),
            incident.summary.closed_at.map_or_else(
                || "open".into(),
                |at| format!("closed {}", timestamp(Some(at)))
            )
        ))];
        lines.push(Line::raw("Recent notes").fg(ACCENT));
        if incident.notes.is_empty() {
            lines.push(Line::raw(
                "No notes yet. Press n to add context to the active incident.",
            ));
        }
        for note in incident.notes.iter().rev().take(3) {
            lines.push(Line::raw(format!(
                "{}  {}",
                note.created_at.format("%m-%d %H:%M UTC"),
                clean(&note.text)
            )));
        }
        lines.push(Line::raw("Recent captures · inspect in History (5)").fg(ACCENT));
        if incident.captures.is_empty() {
            lines.push(Line::raw(
                "No captures attached. Press c for fresh evidence, or I on a History row.",
            ));
        }
        for capture in incident.captures.iter().rev().take(4) {
            lines.push(Line::raw(format!(
                "#{}  {}  {}  {}",
                capture.id,
                capture.captured_at.format("%H:%M:%S UTC"),
                clean(&capture.label),
                if capture.complete {
                    "complete"
                } else {
                    "INCOMPLETE"
                }
            )));
        }
        lines.push(
            Line::raw(format!(
                "Full timeline and report: pgtrail incident show {}",
                incident.summary.id
            ))
            .fg(MUTED),
        );
        frame.render_widget(
            Paragraph::new(lines)
                .block(panel(format!(
                    " Opened #{} · {} ",
                    incident.summary.id,
                    clean(&incident.summary.title)
                )))
                .wrap(Wrap { trim: false }),
            details,
        );
    } else {
        render_empty(
            frame,
            details,
            " Incident details ",
            "Select an incident and press Enter to read its notes and captures.\nAn open incident becomes the destination for new captures.",
        );
    }
}

pub(super) fn prompt(frame: &mut Frame, area: Rect, app: &App) {
    let Some(prompt) = &app.prompt else {
        return;
    };
    let width = area.width.saturating_sub(2).min(100);
    let height = area.height.saturating_sub(2).min(9);
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    let title = match prompt.kind {
        PromptKind::Incident => " New incident ".into(),
        PromptKind::Note(id) => format!(" Note for incident #{id} "),
        PromptKind::Label(id) => format!(" Label for capture #{id} "),
    };
    let max_visible =
        usize::from(width.saturating_sub(4)) * usize::from(height.saturating_sub(4)).max(1);
    let count = prompt.value.chars().count();
    let value: String = prompt
        .value
        .chars()
        .skip(count.saturating_sub(max_visible))
        .collect();
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(vec![
            Line::raw(format!(
                "{}{}▏",
                if count > max_visible { "…" } else { "" },
                clean(&value)
            )),
            Line::raw(""),
            Line::raw("Enter: save · Esc: cancel · Ctrl+U: clear · Backspace: erase").fg(MUTED),
        ])
        .block(
            Block::bordered()
                .title(title)
                .border_style(Style::new().fg(ACCENT)),
        )
        .wrap(Wrap { trim: false }),
        popup,
    );
}

fn observed<'a>(frame: &mut Frame, area: Rect, app: &'a App, title: &str) -> Option<&'a Snapshot> {
    if app.snapshot().is_none() {
        render_empty(
            frame,
            area,
            title,
            "No observation available yet. Connect live or inspect a saved capture from History (5).",
        );
    }
    app.snapshot()
}

fn heading(lines: &mut Vec<Line<'static>>, title: &str) {
    if !lines.is_empty() {
        lines.push(Line::raw(""));
    }
    lines.push(Line::raw(title.to_owned()).fg(ACCENT).bold());
}

fn unavailable(lines: &mut Vec<Line<'static>>, label: &str, reason: &str) {
    lines.push(Line::raw(format!("{label} unavailable: {}", clean(reason))).fg(WARNING));
}

fn scroll(frame: &mut Frame, area: Rect, app: &App, title: &str, lines: Vec<Line<'static>>) {
    // Step through logical lines so all metrics stay reachable when text wraps.
    let offset = app.selected().min(lines.len().saturating_sub(1));
    frame.render_widget(
        Paragraph::new(lines.into_iter().skip(offset).collect::<Vec<_>>())
            .block(panel(title.to_owned()))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn severity(severity: Severity) -> (&'static str, Color) {
    match severity {
        Severity::Critical => ("CRITICAL", Color::Red),
        Severity::Warning => ("WARNING", WARNING),
        Severity::Info => ("INFO", ACCENT),
    }
}

fn on(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}
fn timestamp(value: Option<DateTime<Utc>>) -> String {
    value.map_or_else(
        || "unavailable / never recorded".into(),
        |at| at.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
    )
}
fn maintenance_time(value: Option<DateTime<Utc>>) -> String {
    value.map_or_else(|| "—".into(), |at| at.format("%Y-%m-%d %H:%M").to_string())
}
fn optional_integer(value: Option<i64>) -> String {
    value.map_or_else(|| "—".into(), |v| v.to_string())
}
fn optional_float(value: Option<f64>, unit: &str) -> String {
    value
        .filter(|v| v.is_finite())
        .map_or_else(|| "—".into(), |v| format!("{v:.2}{unit}"))
}
fn optional_bytes(value: Option<i64>) -> String {
    value.map_or_else(|| "—".into(), |v| bytes(v as f64))
}
fn metric(value: &Observation<f64>) -> String {
    optional_float(value.available().copied(), "")
}
fn metric_with_reason(value: &Observation<f64>, unit: &str) -> String {
    match value {
        Observation::Available(value) => optional_float(Some(*value), unit),
        Observation::Unavailable(reason) => format!("unavailable ({})", clean(reason)),
    }
}

fn bytes(value: f64) -> String {
    if !value.is_finite() || value < 0.0 {
        return "unavailable".into();
    }
    let units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = value;
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", units[unit])
}

/// A missing sample is a gap, never a zero. Graphs are ordered, not time-resampled.
fn sparkline(values: &[Option<f64>], width: usize) -> String {
    if values.is_empty() {
        return "waiting for samples".into();
    }
    let values = &values[values.len().saturating_sub(width)..];
    let max = values
        .iter()
        .filter_map(|v| *v)
        .filter(|v| v.is_finite() && *v >= 0.0)
        .fold(0.0, f64::max);
    let glyphs = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    values
        .iter()
        .map(|value| match value {
            Some(value) if value.is_finite() && *value >= 0.0 => {
                glyphs[if *value > 0.0 && max > 0.0 {
                    1 + (value / max * 6.0).round() as usize
                } else {
                    0
                }]
            }
            _ => '×',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trends_preserve_unknown_zero_order_and_window() {
        assert_eq!(sparkline(&[None, Some(0.0), Some(8.0)], 3), "×▁█");
        assert_eq!(sparkline(&[Some(8.0), None, Some(0.0)], 2), "×▁");
        assert_eq!(sparkline(&[Some(f64::NAN), Some(-1.0)], 2), "××");
        assert_eq!(sparkline(&[Some(1.0)], 0), "");
    }

    #[test]
    fn missing_rates_keep_their_reason_and_zero_is_present() {
        assert_eq!(
            metric_with_reason(&Observation::Unavailable("counter reset".into()), "/s"),
            "unavailable (counter reset)"
        );
        assert_eq!(
            metric_with_reason(&Observation::Available(0.0), "/s"),
            "0.00/s"
        );
        assert_eq!(bytes(1024.0), "1.0 KiB");
    }
}
