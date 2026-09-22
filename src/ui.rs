use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph, Row, Table, TableState, Tabs, Wrap},
};

use crate::{
    app::{App, Tab},
    model::{Observation, Session},
};

const ACCENT: Color = Color::Cyan;
const MUTED: Color = Color::DarkGray;
const WARNING: Color = Color::Yellow;

pub(crate) fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    if area.width < 24 || area.height < 8 {
        frame.render_widget(
            Paragraph::new("pgtrail\nExpand terminal\nq: quit  ?: help").wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    let [header, tabs, body, message, filter, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(if app.error.is_some() || app.notice.is_some() {
            2
        } else {
            0
        }),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);

    render_header(frame, header, app);
    let titles: Vec<Line<'_>> = Tab::ALL
        .iter()
        .enumerate()
        .map(|(index, tab)| Line::from(format!("{} {}", index + 1, tab.title())))
        .collect();
    frame.render_widget(
        Tabs::new(titles)
            .select(app.tab.index())
            .highlight_style(Style::new().fg(ACCENT).bold())
            .divider("│"),
        tabs,
    );
    if let Some(report) = &app.report {
        render_report(frame, body, app, report);
    } else {
        match app.tab {
            Tab::Overview => render_overview(frame, body, app),
            Tab::Activity => render_activity(frame, body, app),
            Tab::Blocking => render_blocking(frame, body, app),
            Tab::Statements => render_statements(frame, body, app),
            Tab::History => render_history(frame, body, app),
        }
    }
    if let Some(error) = &app.error {
        frame.render_widget(
            Paragraph::new(format!(" Collection failed: {}", clean(error)))
                .fg(Color::Red)
                .wrap(Wrap { trim: true }),
            message,
        );
    } else if let Some(notice) = &app.notice {
        frame.render_widget(
            Paragraph::new(format!(" {}", clean(notice)))
                .fg(WARNING)
                .wrap(Wrap { trim: true }),
            message,
        );
    }
    let filter_text = if app.editing_filter {
        format!(
            " /{}▏  Enter: apply  Esc: cancel  Ctrl+U: clear",
            app.filter
        )
    } else if !app.filter.is_empty() {
        format!(" Filter: {}  /: edit  Esc: clear", app.filter)
    } else {
        " /: filter  j/k: select  Tab / 1–5: view".into()
    };
    frame.render_widget(
        Paragraph::new(filter_text).fg(if app.editing_filter { ACCENT } else { MUTED }),
        filter,
    );
    let footer_text = if app.report.is_some() {
        " ↑/↓ PgUp/PgDn: scroll  Esc: close comparison  ?: help  q: quit"
    } else if app.tab == Tab::History {
        " Enter: inspect  a/b: mark  d: compare  c: capture  ?: help  q: quit"
    } else if app.is_offline() {
        " OFFLINE  Esc: return to live  5: history  ?: help  q: quit"
    } else {
        " r: refresh  p: pause  c: capture  5: history  ?: help  q: quit"
    };
    frame.render_widget(Paragraph::new(footer_text).fg(ACCENT), footer);
    if app.help {
        render_help(frame, area);
    }
}

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let (status, color) = if app.report.is_some() {
        ("OFFLINE COMPARISON", WARNING)
    } else if app.is_offline() {
        ("OFFLINE CAPTURE", WARNING)
    } else if app.demo {
        ("DEMO · SYNTHETIC DATA", Color::Magenta)
    } else if app.error.is_some() && app.snapshot().is_some() {
        ("STALE · COLLECTION FAILED", Color::Red)
    } else if app.error.is_some() {
        ("DISCONNECTED", Color::Red)
    } else if app.snapshot().is_none() && app.loading {
        ("CONNECTING", WARNING)
    } else if app.snapshot().is_none() {
        ("NO LIVE CONNECTION", WARNING)
    } else if app.paused() {
        ("PAUSED", WARNING)
    } else {
        ("LIVE · CONNECTED", Color::Green)
    };
    let mut headline = vec![
        Span::styled(" pgtrail ", Style::new().bold().fg(ACCENT)),
        Span::raw(" │ "),
        Span::styled(status, Style::new().fg(color).bold()),
    ];
    if app.loading {
        headline.push(Span::styled("  collecting…", Style::new().fg(MUTED)));
    }
    if app.paused() && app.demo && !app.is_offline() {
        headline.push(Span::styled("  PAUSED", Style::new().fg(WARNING)));
    }
    let source = if app.report.is_some() {
        " Comparing saved captures offline · live refresh is suspended".into()
    } else {
        match app.snapshot() {
            Some(snapshot) => {
                let age = (app.now - snapshot.completed_at).num_seconds().max(0);
                format!(
                    " {} / {} · PostgreSQL {} · observed {} UTC · {}s ago",
                    clean(&snapshot.source.endpoint),
                    clean(&snapshot.source.database),
                    clean(&snapshot.source.server_version),
                    snapshot.completed_at.format("%H:%M:%S"),
                    age
                )
            }
            None => {
                " Read-only PostgreSQL investigation · local snapshots · no server mutations".into()
            }
        }
    };
    frame.render_widget(
        Paragraph::new(vec![Line::from(headline), Line::from(source).fg(MUTED)]),
        area,
    );
}

fn render_overview(frame: &mut Frame, area: Rect, app: &App) {
    let Some(snapshot) = app.snapshot() else {
        let text = if app.error.is_some() {
            "No observation is available.\n\nPress r to retry the connection.\nOpen History with 5 to inspect saved captures while disconnected.\n\nNo unavailable metric is treated as zero."
        } else if app.loading {
            "Collecting the first observation…\n\nKeyboard input remains available while PostgreSQL responds.\nPress 5 to browse local history or ? for keyboard help."
        } else {
            "No live connection configured.\n\nSet PGTRAIL_DATABASE_URL to monitor PostgreSQL.\nStart pgtrail --demo to explore with synthetic data.\n\nPress 5 to inspect saved captures offline, or ? for keyboard help."
        };
        render_empty(frame, area, " Overview ", text);
        return;
    };
    let mut lines = vec![Line::from(" OBSERVATION").fg(ACCENT).bold(), Line::raw("")];
    match &snapshot.activity {
        Observation::Available(sessions) => {
            let active = sessions
                .iter()
                .filter(|s| s.state.as_deref() == Some("active"))
                .count();
            let blocked = sessions.iter().filter(|s| !s.blockers.is_empty()).count();
            let idle_tx = sessions
                .iter()
                .filter(|s| {
                    s.state
                        .as_deref()
                        .is_some_and(|state| state.starts_with("idle in transaction"))
                })
                .count();
            let longest = sessions.iter().filter_map(|s| s.query_age_ms).max();
            lines.push(Line::from(format!(" {} sessions  ·  {active} active  ·  {blocked} blocked  ·  {idle_tx} idle in transaction", sessions.len())));
            lines.push(Line::from(format!(
                " Longest current query: {}  ·  Activity sorted by current query age",
                duration(longest)
            )));
        }
        Observation::Unavailable(reason) => {
            lines.push(Line::from(format!(" Activity unavailable: {}", clean(reason))).fg(WARNING))
        }
    }
    lines.push(Line::raw(""));
    match &snapshot.statements {
        Observation::Available(stats) => {
            lines.push(Line::from(format!(
                " {} aggregate statements{}",
                stats.entries.len(),
                if stats.truncated {
                    " · LIMITED COLLECTION (incomplete)"
                } else {
                    ""
                }
            )));
            lines.push(Line::from(format!(
                " Counters since: {}",
                stats
                    .reset_at
                    .map(|time| format!("{} UTC", time.format("%Y-%m-%d %H:%M:%S")))
                    .unwrap_or_else(
                        || "unknown reset time; comparison validity may be limited".into()
                    )
            )));
        }
        Observation::Unavailable(reason) => lines.push(
            Line::from(format!(" Statement metrics unavailable: {}", clean(reason))).fg(WARNING),
        ),
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(" INVESTIGATE").fg(ACCENT).bold());
    lines.push(Line::raw(
        " 2 Activity     Current queries, waits, identity and transaction age",
    ));
    lines.push(Line::raw(
        " 3 Blocking     Waiting PID → actual blocker PID and blocker details",
    ));
    lines.push(Line::raw(
        " 4 Statements   Cumulative execution time, mean time and calls",
    ));
    lines.push(Line::raw(
        " 5 History      Saved captures; inspect or compare A → B offline",
    ));
    lines.push(Line::raw(""));
    lines.push(
        Line::from(format!(
            " Capture quality: {} · collection {} ms",
            if snapshot.is_complete() {
                "complete"
            } else {
                "INCOMPLETE"
            },
            (snapshot.completed_at - snapshot.started_at)
                .num_milliseconds()
                .max(0)
        ))
        .fg(if snapshot.is_complete() {
            Color::Green
        } else {
            WARNING
        }),
    );
    for warning in &snapshot.warnings {
        lines.push(Line::from(format!(" Warning: {}", clean(warning))).fg(WARNING));
    }
    lines
        .push(Line::raw(" SQL text is hidden unless collection was explicitly enabled.").fg(MUTED));
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(" Overview "))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_activity(frame: &mut Frame, area: Rect, app: &App) {
    if let Some(reason) = activity_unavailable(app) {
        render_empty(frame, area, " Activity ", &reason);
        return;
    }
    let sessions = app.filtered_sessions();
    if sessions.is_empty() {
        render_empty(
            frame,
            area,
            " Activity ",
            if app.filter.is_empty() {
                "No visible sessions in this observation."
            } else {
                "No sessions match this filter. Press Esc to clear it."
            },
        );
        return;
    }
    let detail_height = if area.height >= 12 { 7 } else { 0 };
    let [table_area, details] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(detail_height)]).areas(area);
    let wide = area.width >= 95;
    let rows: Vec<_> = sessions
        .iter()
        .map(|session| {
            let mut cells = vec![
                session.pid.to_string(),
                value(session.user.as_deref()),
                value(session.state.as_deref()),
                duration(session.query_age_ms),
                duration(session.transaction_age_ms),
                wait(session),
            ];
            if wide {
                cells.push(clean(&session.application));
            }
            Row::new(cells).style(if session.blockers.is_empty() {
                Style::new()
            } else {
                Style::new().fg(WARNING)
            })
        })
        .collect();
    let mut headers = vec!["PID", "User", "State", "Query age", "Tx age", "Wait"];
    let mut widths = vec![
        Constraint::Length(7),
        Constraint::Length(13),
        Constraint::Length(20),
        Constraint::Length(11),
        Constraint::Length(11),
        Constraint::Min(8),
    ];
    if wide {
        headers.push("Application");
        widths.push(Constraint::Percentage(22));
    }
    let table = Table::new(rows, widths)
        .header(table_header(headers))
        .block(panel(format!(
            " Activity · {} visible · current elapsed time ",
            sessions.len()
        )));
    render_table(frame, table_area, table, app.selected());
    if detail_height > 0
        && let Some(session) = sessions.get(app.selected())
    {
        render_session_details(frame, details, session, " Selected session ");
    }
}

fn render_blocking(frame: &mut Frame, area: Rect, app: &App) {
    if let Some(reason) = activity_unavailable(app) {
        render_empty(frame, area, " Blocking ", &reason);
        return;
    }
    let edges = app.blocking_edges();
    if edges.is_empty() {
        render_empty(
            frame,
            area,
            " Blocking ",
            if app.filter.is_empty() {
                "No blocking relationships observed.\n\nRelationships come from pg_blocking_pids at collection time.\nA session that vanishes between reads remains an unresolved identity."
            } else {
                "No blocking relationships match this filter. Press Esc to clear it."
            },
        );
        return;
    }
    let all_sessions = app.snapshot().and_then(|s| s.activity.available());
    let detail_height = if area.height >= 12 { 7 } else { 0 };
    let [table_area, details] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(detail_height)]).areas(area);
    let rows: Vec<_> = edges
        .iter()
        .map(|(waiter, blocker_pid)| {
            let blocker =
                all_sessions.and_then(|sessions| sessions.iter().find(|s| s.pid == *blocker_pid));
            Row::new(vec![
                waiter.pid.to_string(),
                format!("→ {blocker_pid}"),
                duration(waiter.transaction_age_ms),
                wait(waiter),
                blocker
                    .map(|s| value(s.state.as_deref()))
                    .unwrap_or_else(|| unresolved_blocker(*blocker_pid).into()),
                blocker
                    .map(|s| duration(s.transaction_age_ms))
                    .unwrap_or_else(|| "—".into()),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(12),
            Constraint::Min(10),
            Constraint::Percentage(28),
            Constraint::Length(12),
        ],
    )
    .header(table_header(vec![
        "Waiter",
        "Blocker",
        "Waiter tx",
        "Wait event",
        "Blocker state",
        "Blocker tx",
    ]))
    .block(panel(format!(
        " Blocking · {} server-reported relationships ",
        edges.len()
    )));
    render_table(frame, table_area, table, app.selected());
    if detail_height > 0
        && let Some((waiter, blocker_pid)) = edges.get(app.selected())
    {
        if let Some(blocker) =
            all_sessions.and_then(|sessions| sessions.iter().find(|s| s.pid == *blocker_pid))
        {
            render_session_details(
                frame,
                details,
                blocker,
                &format!(" Blocker {blocker_pid} · waiting session {} ", waiter.pid),
            );
        } else {
            render_empty(
                frame,
                details,
                " Unresolved blocker ",
                &format!(
                    "Waiter {} is blocked by PID {blocker_pid}.\n{}; no identity or transaction age is inferred.",
                    waiter.pid,
                    unresolved_blocker(*blocker_pid)
                ),
            );
        }
    }
}

fn render_statements(frame: &mut Frame, area: Rect, app: &App) {
    let Some(snapshot) = app.snapshot() else {
        render_empty(frame, area, " Statements ", "No observation available yet.");
        return;
    };
    let stats = match &snapshot.statements {
        Observation::Available(stats) => stats,
        Observation::Unavailable(reason) => {
            render_empty(
                frame,
                area,
                " Statements unavailable ",
                &format!(
                    "{}\n\npg_stat_statements and readable statistics are required.\npgtrail never creates extensions or resets server statistics.",
                    clean(reason)
                ),
            );
            return;
        }
    };
    let statements = app.filtered_statements();
    if statements.is_empty() {
        render_empty(
            frame,
            area,
            " Statements ",
            if app.filter.is_empty() {
                "No statement statistics returned. Counters may have been recently reset."
            } else {
                "No statements match this filter. Press Esc to clear it."
            },
        );
        return;
    }
    let detail_height = if area.height >= 11 { 6 } else { 0 };
    let [table_area, details] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(detail_height)]).areas(area);
    let rows: Vec<_> = statements
        .iter()
        .map(|statement| {
            Row::new(vec![
                statement.queryid.to_string(),
                statement.calls.to_string(),
                format!("{:.2}", statement.total_exec_ms),
                format!("{:.2}", statement.mean_exec_ms),
                statement.rows.to_string(),
                statement.temp_blks_written.to_string(),
            ])
        })
        .collect();
    let title = format!(
        " Statements · sort: {} ↓ (s){} ",
        app.sort.title(),
        if stats.truncated { " · LIMITED" } else { "" }
    );
    let table = Table::new(
        rows,
        [
            Constraint::Min(18),
            Constraint::Length(10),
            Constraint::Length(15),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(11),
        ],
    )
    .header(table_header(vec![
        "Query ID",
        "Calls",
        "Total exec ms",
        "Mean exec ms",
        "Rows",
        "Temp blocks",
    ]))
    .block(panel(title));
    render_table(frame, table_area, table, app.selected());
    if detail_height > 0
        && let Some(statement) = statements.get(app.selected())
    {
        let total_blocks = statement
            .shared_blks_hit
            .saturating_add(statement.shared_blks_read);
        let hit_ratio = if total_blocks > 0 {
            format!(
                "{:.1}%",
                statement.shared_blks_hit as f64 * 100.0 / total_blocks as f64
            )
        } else {
            "unavailable (no block accesses)".into()
        };
        let lines = vec![
            Line::raw(format!("User ID {} · database ID {} · top-level {} · shared-buffer hit {}", statement.userid, statement.dbid, statement.toplevel, hit_ratio)),
            Line::raw(format!("Shared blocks: {} hits / {} reads. These are cumulative counters.", statement.shared_blks_hit, statement.shared_blks_read)).fg(MUTED),
            Line::raw("Current query age belongs to Activity; this ranking aggregates completed executions.").fg(MUTED),
            Line::raw(format!("SQL: {}", statement.query.as_deref().map(clean).unwrap_or_else(|| "not captured (opt in with --include-query-text)".into()))),
        ];
        frame.render_widget(
            Paragraph::new(lines)
                .block(panel(" Statement details "))
                .wrap(Wrap { trim: true }),
            details,
        );
    }
}

fn render_history(frame: &mut Frame, area: Rect, app: &App) {
    let captures = app.filtered_history();
    if captures.is_empty() {
        render_empty(
            frame,
            area,
            " Local history ",
            if app.filter.is_empty() {
                "No saved captures.\n\nPress c after an observation to save it locally in SQLite.\nCaptures remain available after restart and while disconnected.\n\nMark an earlier capture with a, a later capture with b, then d to compare."
            } else {
                "No captures match this filter. Press Esc to clear it."
            },
        );
        return;
    }
    let rows: Vec<_> = captures
        .iter()
        .map(|capture| {
            let mark = match (
                app.compare_a == Some(capture.id),
                app.compare_b == Some(capture.id),
            ) {
                (true, true) => "AB",
                (true, false) => "A",
                (false, true) => "B",
                _ => "",
            };
            Row::new(vec![
                mark.into(),
                capture.id.to_string(),
                capture.captured_at.format("%Y-%m-%d %H:%M:%S").to_string(),
                clean(&capture.label),
                if capture.complete {
                    "complete".into()
                } else {
                    "INCOMPLETE".into()
                },
                clean(&capture.source),
            ])
        })
        .collect();
    let title = format!(
        " Local history · {} captures · A: {} → B: {} ",
        captures.len(),
        app.compare_a
            .map(|id| id.to_string())
            .unwrap_or_else(|| "—".into()),
        app.compare_b
            .map(|id| id.to_string())
            .unwrap_or_else(|| "—".into())
    );
    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Length(6),
            Constraint::Length(19),
            Constraint::Percentage(25),
            Constraint::Length(10),
            Constraint::Min(10),
        ],
    )
    .header(table_header(vec![
        "",
        "ID",
        "Captured UTC",
        "Label",
        "Quality",
        "Source",
    ]))
    .block(panel(title));
    render_table(frame, area, table, app.selected());
}

fn render_report(frame: &mut Frame, area: Rect, app: &App, report: &str) {
    let lines: Vec<_> = report
        .lines()
        .skip(app.report_scroll)
        .map(|line| {
            let style = if line.starts_with('#') {
                Style::new().fg(ACCENT).bold()
            } else if line.contains("unavailable")
                || line.contains("reset")
                || line.contains("incompatible")
            {
                Style::new().fg(WARNING)
            } else {
                Style::new()
            };
            Line::styled(clean(line), style)
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(" Offline comparison · A → B · ↑/↓ to scroll "))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_session_details(frame: &mut Frame, area: Rect, session: &Session, title: &str) {
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
    let lines = vec![
        Line::raw(format!(
            "PID {} · user {} · database {} · client {}",
            session.pid,
            value(session.user.as_deref()),
            value(session.database.as_deref()),
            value(session.client.as_deref())
        )),
        Line::raw(format!(
            "Application: {} · backend started: {}",
            clean(&session.application),
            session
                .backend_start
                .map(|time| format!("{} UTC", time.format("%Y-%m-%d %H:%M:%S")))
                .unwrap_or_else(|| "unavailable".into())
        )),
        Line::raw(format!(
            "State: {} · current query: {} · transaction: {} · wait: {}",
            value(session.state.as_deref()),
            duration(session.query_age_ms),
            duration(session.transaction_age_ms),
            wait(session)
        )),
        Line::raw(format!("Blocking PIDs: {blockers}")),
        Line::raw(format!(
            "SQL: {}",
            session
                .query
                .as_deref()
                .map(clean)
                .unwrap_or_else(|| "not captured (opt in with --include-query-text)".into())
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(title.to_owned()))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_help(frame: &mut Frame, area: Rect) {
    let width = area.width.saturating_sub(4).min(76);
    let height = area.height.saturating_sub(2).min(23);
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    let lines = vec![
        Line::raw("NAVIGATION").fg(ACCENT).bold(),
        Line::raw("Tab / Shift+Tab / 1–5     Switch views"),
        Line::raw("↑/↓ or j/k                Select row / scroll comparison"),
        Line::raw("PgUp/PgDn · Home/End      Move faster / first / last"),
        Line::raw("/                        Edit filter; Enter applies, Esc cancels"),
        Line::raw("Esc                      Clear filter / close report / return live"),
        Line::raw(""),
        Line::raw("OBSERVATIONS").fg(ACCENT).bold(),
        Line::raw("r                        Refresh current view"),
        Line::raw("p                        Pause or resume automatic refresh"),
        Line::raw("c                        Collect and save a fresh local capture"),
        Line::raw("s                        Statements: sort total / mean / calls"),
        Line::raw(""),
        Line::raw("HISTORY").fg(ACCENT).bold(),
        Line::raw("Enter                    Inspect the selected capture offline"),
        Line::raw("a / b                    Mark earlier / later capture"),
        Line::raw("d                        Compare A → B offline"),
        Line::raw(""),
        Line::raw("? / Esc                  Close help     q / Ctrl+C: quit"),
        Line::raw("Read-only. No session cancellation or statistic resets.").fg(MUTED),
    ];
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .title(" Keyboard help ")
                    .border_style(Style::new().fg(ACCENT)),
            )
            .wrap(Wrap { trim: false }),
        popup,
    );
}

fn render_table(frame: &mut Frame, area: Rect, table: Table<'_>, selected: usize) {
    // Ephemeral widget state keeps rendering pure while ensuring the selected row
    // remains visible after moving past the bottom of a short viewport.
    let visible_rows = usize::from(area.height.saturating_sub(3)).max(1);
    let mut state = TableState::new()
        .with_selected(Some(selected))
        .with_offset(selected.saturating_sub(visible_rows - 1));
    frame.render_stateful_widget(
        table
            .row_highlight_style(Style::new().bg(Color::DarkGray).fg(Color::White).bold())
            .highlight_symbol("› "),
        area,
        &mut state,
    );
}

fn table_header(labels: Vec<&str>) -> Row<'_> {
    Row::new(labels).style(Style::new().fg(ACCENT).bold())
}
fn panel<'a>(title: impl Into<Line<'a>>) -> Block<'a> {
    Block::bordered()
        .title(title)
        .border_style(Style::new().fg(MUTED))
}

fn render_empty(frame: &mut Frame, area: Rect, title: &str, message: &str) {
    frame.render_widget(
        Paragraph::new(message)
            .block(panel(title.to_owned()))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn activity_unavailable(app: &App) -> Option<String> {
    match app.snapshot().map(|snapshot| &snapshot.activity) {
        None => Some("No observation available yet.".into()),
        Some(Observation::Unavailable(reason)) => Some(format!(
            "Activity unavailable: {}\n\nNo session count or blocking state is inferred. Refresh with r to retry.",
            clean(reason)
        )),
        Some(Observation::Available(_)) => None,
    }
}

fn unresolved_blocker(pid: i32) -> &'static str {
    if pid == 0 {
        "Prepared transaction (no backend PID)"
    } else {
        "Not visible / already exited"
    }
}

fn value(value: Option<&str>) -> String {
    value.map(clean).unwrap_or_else(|| "—".into())
}

fn duration(milliseconds: Option<i64>) -> String {
    match milliseconds {
        None => "—".into(),
        Some(ms) if ms < 0 => "unavailable".into(),
        Some(ms) if ms < 1_000 => format!("{ms} ms"),
        Some(ms) if ms < 60_000 => format!("{:.1} s", ms as f64 / 1_000.0),
        Some(ms) if ms < 3_600_000 => format!("{}m {:02}s", ms / 60_000, (ms / 1_000) % 60),
        Some(ms) => format!("{}h {:02}m", ms / 3_600_000, (ms / 60_000) % 60),
    }
}

fn wait(session: &Session) -> String {
    match (&session.wait_event_type, &session.wait_event) {
        (Some(kind), Some(event)) => format!("{}:{}", clean(kind), clean(event)),
        (_, Some(event)) => clean(event),
        (Some(kind), None) => clean(kind),
        (None, None) => "—".into(),
    }
}

// Server-provided names, query text and local labels are untrusted terminal text.
fn clean(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        event::Message,
        model::{SNAPSHOT_VERSION, Snapshot, Source},
        store::SnapshotSummary,
    };
    use chrono::Utc;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};
    use std::convert::Infallible;

    fn screen(app: &App, width: u16, height: u16) -> Result<String, Infallible> {
        let mut terminal = Terminal::new(TestBackend::new(width, height))?;
        terminal.draw(|frame| render(frame, app))?;
        Ok(terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect())
    }

    fn snapshot() -> Snapshot {
        let now = Utc::now();
        Snapshot {
            schema_version: SNAPSHOT_VERSION,
            started_at: now,
            completed_at: now,
            source: Source {
                endpoint: "localhost:5432".into(),
                database: "example".into(),
                database_oid: 1,
                server_version: "18".into(),
                server_started_at: now,
                system_identifier: None,
            },
            activity: Observation::Available(vec![]),
            statements: Observation::Unavailable("Extension is not installed".into()),
            warnings: vec![],
        }
    }

    #[test]
    fn unavailable_and_failed_observations_are_explicit() -> Result<(), Infallible> {
        let mut app = App::new(false);
        app.set_snapshot(snapshot(), false);
        let text = screen(&app, 120, 30)?;
        assert!(text.contains("LIVE · CONNECTED"));
        assert!(text.contains("Statement metrics unavailable: Extension is not installed"));
        assert!(text.contains("INCOMPLETE"));
        app.set_error("Database could not be reached".into());
        let text = screen(&app, 120, 30)?;
        assert!(text.contains("STALE · COLLECTION FAILED"));
        assert!(text.contains("Database could not be reached"));
        Ok(())
    }

    #[test]
    fn demo_offline_and_tiny_terminal_states_render() -> Result<(), Infallible> {
        let mut app = App::new(true);
        app.set_snapshot(snapshot(), false);
        assert!(screen(&app, 100, 25)?.contains("DEMO · SYNTHETIC DATA"));
        app.set_snapshot(snapshot(), true);
        assert!(screen(&app, 100, 25)?.contains("OFFLINE CAPTURE"));
        for (width, height) in [(0, 0), (1, 1), (10, 3), (25, 8), (50, 12)] {
            for tab in Tab::ALL {
                app.tab = tab;
                screen(&app, width, height)?;
            }
            app.help = true;
            screen(&app, width, height)?;
            app.help = false;
        }
        Ok(())
    }

    #[test]
    fn selected_history_row_stays_visible_after_scrolling() -> Result<(), Infallible> {
        let mut app = App::new(true);
        app.set_history(
            (1..=40)
                .map(|id| SnapshotSummary {
                    id,
                    label: format!("capture-{id:02}"),
                    captured_at: Utc::now(),
                    source: "localhost/example".into(),
                    complete: true,
                })
                .collect(),
        );
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Char('5'),
            KeyModifiers::NONE,
        )));
        app.update(Message::Key(KeyEvent::new(
            KeyCode::End,
            KeyModifiers::NONE,
        )));
        let text = screen(&app, 120, 16)?;
        assert!(text.contains("capture-40"));
        assert!(!text.contains("capture-01"));
        assert!(text.contains("›"));
        Ok(())
    }

    #[test]
    fn terminal_control_sequences_are_never_rendered_as_controls() {
        assert_eq!(clean("a\u{1b}[31m\r\nb\t"), "a [31m  b ");
        assert_eq!(duration(None), "—");
        assert_eq!(duration(Some(61_000)), "1m 01s");
    }
}
