//! Investigation views consume state only; collection and persistence live elsewhere.
use chrono::{DateTime, Utc};

use super::*;
use crate::{app::PromptKind, diagnostics::Severity, model::Snapshot};

pub(super) fn overview(frame: &mut Frame, area: Rect, app: &App) {
    let Some(snapshot) = app.snapshot() else {
        render_empty(
            frame,
            area,
            " Welcome to pgtrail ",
            "No observation available yet.

Connect with --profile NAME or PGTRAIL_DATABASE_URL.
Explore synthetic data with --demo.

Open Captures (5) or Incidents (0) to investigate saved evidence.",
        );
        return;
    };
    let Some(analysis) = &app.analysis else {
        return;
    };
    let [summary, list] = Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).areas(area);
    let activity = snapshot.activity.available().map_or_else(
        || "Activity unavailable".into(),
        |sessions| {
            format!(
                "{} visible sessions  ·  {} blocked",
                sessions.len(),
                sessions.iter().filter(|s| !s.blockers.is_empty()).count()
            )
        },
    );
    let coverage = crate::diagnostics::collection_summary(snapshot)
        .replace("; complete collection (not a health verdict)", "")
        .replace("; incomplete collection", " · PARTIAL");
    let ready = [
        analysis.rates.database.available().is_some(),
        analysis.rates.wal.available().is_some(),
        analysis.rates.statements.available().is_some(),
    ]
    .into_iter()
    .filter(|ready| *ready)
    .count();
    frame.render_widget(
        Paragraph::new(vec![
            Line::raw(format!(
                " {activity}  ·  {} findings",
                analysis.findings.len()
            ))
            .bold(),
            Line::raw(format!(
                " {coverage} · Intervals {ready}/3 ready · h: coverage"
            ))
            .fg(MUTED),
        ]),
        summary,
    );
    let findings = app.filtered_findings();
    if findings.is_empty() {
        render_empty(
            frame,
            list,
            " Findings ",
            if app.filter.is_empty() {
                "No configured findings triggered in the available evidence.
This does not prove database health; review coverage with h."
            } else {
                "No findings match this filter. Press Esc to clear it."
            },
        );
        return;
    }
    let rows = findings.iter().map(|finding| {
        Row::new(vec![
            ratatui::widgets::Cell::from(severity(finding.severity).0)
                .style(Style::new().fg(severity(finding.severity).1)),
            ratatui::widgets::Cell::from(clean(&finding.title)),
        ])
    });
    let table = Table::new(rows, [Constraint::Length(9), Constraint::Min(10)])
        .header(table_header(vec!["Severity", "Investigation finding"], &[]))
        .block(content_panel(app, " Findings · ordered by severity "));
    render_table(frame, list, table, app);
}

pub(super) fn database(frame: &mut Frame, area: Rect, app: &App) {
    scroll(frame, area, app, " Database ");
}

fn database_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let Some(snapshot) = app.snapshot() else {
        return vec![Line::raw(
            "No observation available yet. Open Captures (5) or connect with --profile NAME.",
        )];
    };
    let mut lines = Vec::new();
    heading(&mut lines, "CURRENT DATABASE AND CLUSTER CAPACITY");
    match &snapshot.health.database {
        Observation::Unavailable(reason) => unavailable(&mut lines, "Database", reason),
        Observation::Available(db) => {
            field_grid(
                &mut lines,
                width,
                vec![
                    ("Database size", bytes(db.size_bytes as f64)),
                    ("Database connections", db.num_backends.to_string()),
                    (
                        "Cluster clients",
                        format!("{} / {} max", db.cluster_backends, db.max_connections),
                    ),
                    ("Reserved connections", db.reserved_connections.to_string()),
                    ("Autovacuum", on(db.autovacuum).into()),
                    ("Track counts", on(db.track_counts).into()),
                    ("I/O timing", on(db.track_io_timing).into()),
                    ("Database XID age", db.frozen_xid_age.to_string()),
                ],
            );
            lines
                .push(Line::raw(format!("Counter reset: {}", timestamp(db.stats_reset))).fg(MUTED));
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
                let fields = [
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
                ]
                .into_iter()
                .map(|(label, metric, unit)| (label, metric_with_reason(metric, unit)))
                .collect();
                field_grid(&mut lines, width, fields);
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
        let width = usize::from(width.saturating_sub(24));
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
        field_grid(
            &mut lines,
            width,
            vec![
                ("Commits", db.xact_commit.to_string()),
                ("Rollbacks", db.xact_rollback.to_string()),
                ("Deadlocks", db.deadlocks.to_string()),
                ("Recovery conflicts", db.conflicts.to_string()),
                ("Temporary files", db.temp_files.to_string()),
                ("Temporary writes", bytes(db.temp_bytes as f64)),
                ("Shared reads", db.blks_read.to_string()),
                ("Shared hits", db.blks_hit.to_string()),
                ("Tuples returned", db.tup_returned.to_string()),
                ("Tuples fetched", db.tup_fetched.to_string()),
                ("Tuples inserted", db.tup_inserted.to_string()),
                ("Tuples updated", db.tup_updated.to_string()),
                ("Tuples deleted", db.tup_deleted.to_string()),
            ],
        );
    }
    lines.push(
        Line::raw(
            "Shared-buffer hits do not measure the OS cache. Statistics can lag current activity.",
        )
        .fg(MUTED),
    );
    lines
}

pub(super) fn relations(frame: &mut Frame, area: Rect, app: &App) {
    let Some(snapshot) = observed(frame, area, app, " Relations ") else {
        return;
    };
    let stats = match &snapshot.health.tables {
        Observation::Unavailable(reason) => {
            render_empty(
                frame,
                area,
                " Relations unavailable ",
                &format!(
                    "{}\n\nPress r to retry collection or h to inspect coverage.",
                    clean(reason)
                ),
            );
            return;
        }
        Observation::Available(stats) => stats,
    };
    let limited = if stats.truncated {
        " · LIMITED coverage"
    } else {
        ""
    };
    if app.show_indexes {
        let indexes = app.filtered_indexes();
        if indexes.is_empty() {
            render_empty(
                frame,
                area,
                " Indexes ",
                if app.filter.is_empty() {
                    "No indexes were returned in this observation. Press v to inspect tables or r to refresh."
                } else {
                    "No indexes match this filter. Press Esc to clear it or v to inspect tables."
                },
            );
            return;
        }
        let rows = indexes.iter().map(|i| {
            data_row(
                vec![
                    format!("{}.{}", clean(&i.schema), clean(&i.name)),
                    bytes(i.size_bytes as f64),
                    i.scans.to_string(),
                    if i.valid { "valid" } else { "INVALID" }.into(),
                    if i.primary {
                        "primary"
                    } else if i.unique {
                        "unique"
                    } else {
                        "ordinary"
                    }
                    .into(),
                ],
                &[1, 2],
            )
            .style(Style::new().fg(if i.valid { Color::Reset } else { WARNING }))
        });
        let table = Table::new(
            rows,
            [
                Constraint::Min(18),
                Constraint::Length(10),
                Constraint::Length(10),
                Constraint::Length(9),
                Constraint::Length(10),
            ],
        )
        .header(table_header(
            vec!["Index", "Size", "Scans", "Validity", "Constraint"],
            &[1, 2],
        ))
        .block(content_panel(
            app,
            format!(" Indexes · cumulative scans{limited} "),
        ));
        render_table(frame, area, table, app);
    } else {
        let tables = app.filtered_tables();
        if tables.is_empty() {
            render_empty(
                frame,
                area,
                " Tables ",
                if app.filter.is_empty() {
                    "No tables were returned in this observation. Press v to inspect indexes or r to refresh."
                } else {
                    "No tables match this filter. Press Esc to clear it."
                },
            );
            return;
        }
        let wide = area.width >= 95;
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
            data_row(cells, &[1, 2, 3, 4, 5])
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
            widths.push(Constraint::Length(10));
            headers.push("Idx scans");
        }
        let table = Table::new(rows, widths)
            .header(table_header(headers, &[1, 2, 3, 4, 5]))
            .block(content_panel(
                app,
                format!(" Tables · tuple counts are estimates{limited} "),
            ));
        render_table(frame, area, table, app);
    }
}

pub(super) fn replication(frame: &mut Frame, area: Rect, app: &App) {
    scroll(frame, area, app, " Replication ");
}

fn replication_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let Some(snapshot) = app.snapshot() else {
        return vec![Line::raw(
            "No observation available yet. Open Captures (5) or connect with --profile NAME.",
        )];
    };
    let stats = match &snapshot.health.replication {
        Observation::Unavailable(reason) => {
            return vec![
                Line::raw(format!("Replication unavailable: {}", clean(reason))).fg(WARNING),
            ];
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
        field_grid(
            &mut lines,
            width,
            vec![
                (
                    "Sent-to-replay backlog",
                    optional_bytes(sender.sent_replay_lag_bytes),
                ),
                (
                    "Reported replay lag",
                    optional_float(sender.replay_lag_ms, " ms"),
                ),
            ],
        );
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
        field_grid(
            &mut lines,
            width,
            vec![
                ("Retained WAL", optional_bytes(slot.retained_bytes)),
                ("WAL status", value(slot.wal_status.as_deref())),
                ("Safe WAL headroom", optional_bytes(slot.safe_wal_size)),
            ],
        );
        lines.push(Line::raw("  NULL headroom can mean unlimited retention or an unusable slot; inspect WAL status.").fg(MUTED));
    }
    lines.push(Line::raw("Retention is a potential disk-pressure signal. pgtrail never drops slots or changes replication.").fg(MUTED));
    lines
}

pub(super) fn io(frame: &mut Frame, area: Rect, app: &App) {
    scroll(frame, area, app, " I/O, WAL and maintenance ");
}

fn io_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let Some(snapshot) = app.snapshot() else {
        return vec![Line::raw(
            "No observation available yet. Open Captures (5) or connect with --profile NAME.",
        )];
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
                field_grid(
                    &mut lines,
                    width,
                    vec![
                        ("Reads", optional_integer(row.reads)),
                        ("Read time", optional_float(row.read_time_ms, " ms")),
                        ("Writes", optional_integer(row.writes)),
                        ("Write time", optional_float(row.write_time_ms, " ms")),
                        ("Hits", optional_integer(row.hits)),
                        ("Evictions", optional_integer(row.evictions)),
                        ("Fsyncs", optional_integer(row.fsyncs)),
                    ],
                );
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
    lines
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
                "{}\n\nTwo compatible observations and unchanged statistics epochs are required.\nPress v for cumulative statistics. Offline interval reports are available through Captures before/after comparison.",
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
            if app.filter.is_empty() {
                "No shared statement entries were available in this interval. Press v for cumulative statistics or r for another observation."
            } else {
                "No statements match this filter in this interval. Press Esc to clear it or v for cumulative statistics."
            },
        );
        return;
    }
    let table_area = area;
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
        data_row(cells, &[0, 1, 2, 3, 4, 5])
    });
    let mut widths = vec![
        Constraint::Min(18),
        Constraint::Length(8),
        Constraint::Length(10),
        Constraint::Length(12),
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
        .header(table_header(headers, &[0, 1, 2, 3, 4, 5]))
        .block(content_panel(
            app,
            format!(
                " Statements · interval {:.2}s · sort: {} (s) · v: cumulative ",
                analysis.rates.elapsed_seconds.unwrap_or_default(),
                app.sort.title()
            ),
        ));
    render_table(frame, table_area, table, app);
}

pub(super) fn incidents(frame: &mut Frame, area: Rect, app: &App) {
    if app.incident_timeline
        && let Some(incident) = &app.incident
    {
        incident_timeline(frame, area, app, incident);
        return;
    }
    if let Some(error) = &app.incidents_error {
        render_empty(
            frame,
            area,
            " Incidents unavailable ",
            &format!(
                "{}\n\nPress r to reload the local incident list.",
                clean(error)
            ),
        );
        return;
    }
    let incidents = app.filtered_incidents();
    if incidents.is_empty() {
        let message = if app.filter.is_empty() {
            "No incidents yet. Press i to create a case, c to capture evidence, and n to add a note.".into()
        } else {
            format!(
                "No incidents match filter {:?}. Press / to change or clear it.",
                app.filter
            )
        };
        render_empty(frame, area, " Local incidents ", &message);
        return;
    }
    let table_area = area;
    let rows = incidents.iter().map(|incident| {
        data_row(
            vec![
                if app.active_incident == Some(incident.id) {
                    "*".into()
                } else {
                    String::new()
                },
                incident.id.to_string(),
                clean(&incident.title),
                if incident.closed_at.is_some() {
                    "closed".into()
                } else {
                    "open".into()
                },
                incident.capture_count.to_string(),
                incident.note_count.to_string(),
            ],
            &[1, 4, 5],
        )
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(1),
            Constraint::Length(5),
            Constraint::Min(10),
            Constraint::Length(6),
            Constraint::Length(8),
            Constraint::Length(5),
        ],
    )
    .header(table_header(
        vec!["", "ID", "Incident", "State", "Captures", "Notes"],
        &[1, 4, 5],
    ))
    .block(content_panel(
        app,
        " Incidents · * active destination · Enter: timeline ",
    ));
    render_table(frame, table_area, table, app);
}

fn incident_timeline(frame: &mut Frame, area: Rect, app: &App, incident: &crate::store::Incident) {
    let entries = crate::incidents::timeline(incident);
    let table_area = area;
    if entries.is_empty() {
        render_empty(
            frame,
            table_area,
            " Full chronology ",
            "No observations yet. Press n to add a note or c to capture evidence.",
        );
        return;
    }
    let table = Table::new(
        entries.iter().map(|entry| {
            Row::new(vec![
                entry.at.format("%m-%d %H:%M:%S").to_string(),
                clean(&entry.title),
            ])
        }),
        [Constraint::Length(15), Constraint::Min(10)],
    )
    .header(table_header(vec!["Time (UTC)", "Evidence / note"], &[]))
    .block(content_panel(
        app,
        format!(
            " Full chronology · #{}: {} · {} ",
            incident.summary.id,
            clean(&incident.summary.title),
            if incident.summary.closed_at.is_some() {
                "closed"
            } else {
                "open"
            }
        ),
    ));
    render_table(frame, table_area, table, app);
}

pub(super) fn prompt(frame: &mut Frame, app: &App) {
    let Some(prompt) = &app.prompt else {
        return;
    };
    let popup = super::shell::prompt_area(app);
    let title = match prompt.kind {
        PromptKind::Incident => " New incident ".into(),
        PromptKind::Note(id) => format!(" Note for incident #{id} "),
        PromptKind::Label(id) => format!(" Label for capture #{id} "),
        PromptKind::ExportIncident(id, json) => format!(
            " Export incident #{id} as {} · new file path ",
            if json { "JSON" } else { "Markdown" }
        ),
    };
    frame.render_widget(Clear, popup);
    frame.render_widget(panel(title).border_style(Style::new().fg(ACCENT)), popup);
    let text_area = Rect::new(
        popup.x + 2,
        popup.y + 1,
        popup.width.saturating_sub(4),
        popup.height.saturating_sub(4),
    );
    let rows = report_lines(&format!("{}▏", clean(&prompt.value)), text_area.width);
    let offset = rows.len().saturating_sub(text_area.height as usize);
    frame.render_widget(
        Paragraph::new(
            rows.into_iter()
                .skip(offset)
                .map(Line::raw)
                .collect::<Vec<_>>(),
        ),
        text_area,
    );
    frame.render_widget(
        Paragraph::new("Ctrl+U: clear · Backspace: erase").fg(MUTED),
        Rect::new(
            text_area.x,
            popup.bottom().saturating_sub(3),
            text_area.width,
            1,
        ),
    );
    super::shell::render_editor_controls(frame, app);
}

fn observed<'a>(frame: &mut Frame, area: Rect, app: &'a App, title: &str) -> Option<&'a Snapshot> {
    if app.snapshot().is_none() {
        render_empty(
            frame,
            area,
            title,
            "No observation available yet. Connect live or inspect a saved capture from Captures (5).",
        );
    }
    app.snapshot()
}

fn field_grid(lines: &mut Vec<Line<'static>>, width: u16, fields: Vec<(&str, String)>) {
    let column_width = usize::from(width.saturating_sub(4)) / 2;
    let mut pending: Vec<Span<'static>> = Vec::new();
    for (label, value) in fields {
        let text = format!("{label}: {value}");
        let fits = width >= 72 && Span::raw(&text).width() + 2 <= column_width;
        if !fits {
            if !pending.is_empty() {
                lines.push(Line::from(std::mem::take(&mut pending)));
            }
            lines.push(Line::from(vec![
                Span::styled(format!("{label}: "), Style::new().fg(MUTED)),
                Span::raw(value),
            ]));
            continue;
        }
        let padding =
            column_width.saturating_sub(Span::raw(label).width() + Span::raw(&value).width() + 1);
        pending.push(Span::styled(
            format!("{label}:{}", " ".repeat(padding)),
            Style::new().fg(MUTED),
        ));
        pending.push(Span::raw(value));
        if pending.len() == 2 {
            pending.push(Span::raw("    "));
        } else {
            lines.push(Line::from(std::mem::take(&mut pending)));
        }
    }
    if !pending.is_empty() {
        lines.push(Line::from(pending));
    }
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

pub(crate) fn metric_rows(app: &App, width: u16) -> Vec<Line<'static>> {
    let lines = match app.tab {
        Tab::Database => database_lines(app, width),
        Tab::Replication => replication_lines(app, width),
        Tab::Io => io_lines(app, width),
        _ => return Vec::new(),
    };
    lines
        .into_iter()
        .flat_map(|line| super::wrap_styled_line(line, width))
        .collect()
}

fn scroll(frame: &mut Frame, area: Rect, app: &App, title: &str) {
    let rows = metric_rows(app, area.width.saturating_sub(2));
    let visible = usize::from(area.height.saturating_sub(2));
    let offset = app.selected().min(rows.len().saturating_sub(visible));
    let position = format!(
        " {}–{} / {} · [ / ] sections ",
        offset + usize::from(!rows.is_empty()),
        (offset + visible).min(rows.len()),
        rows.len()
    );
    frame.render_widget(
        Paragraph::new(
            rows.into_iter()
                .skip(offset)
                .take(visible)
                .collect::<Vec<_>>(),
        )
        .block(
            content_panel(app, title.to_owned())
                .title_bottom(position)
                .border_style(Style::new().fg(if app.focus == crate::app::Focus::Content {
                    ACCENT
                } else {
                    BORDER
                })),
        ),
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
