mod details;
mod investigation;

pub(crate) use details::detail_report;

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
// Use the terminal's configured foreground for readable secondary text on both
// light and dark themes. Status and selection also have textual markers.
const MUTED: Color = Color::Reset;
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
        Constraint::Length(if app.capture.is_some() { 3 } else { 2 }),
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
    let [first_tabs, second_tabs] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(tabs);
    for (offset, row) in [(0, first_tabs), (5, second_tabs)] {
        let titles: Vec<Line<'_>> = Tab::ALL
            .iter()
            .enumerate()
            .skip(offset)
            .take(5)
            .map(|(index, tab)| Line::from(format!("{} {}", (index + 1) % 10, tab.title())))
            .collect();
        let selected = (offset..offset + 5)
            .contains(&app.tab.index())
            .then_some(app.tab.index().saturating_sub(offset));
        frame.render_widget(
            Tabs::new(titles)
                .select(selected)
                .highlight_style(Style::new().fg(ACCENT).bold())
                .divider("│"),
            row,
        );
    }
    if let Some(report) = &app.report {
        render_report(frame, body, app, report);
    } else {
        match app.tab {
            Tab::Overview => investigation::overview(frame, body, app),
            Tab::Activity => render_activity(frame, body, app),
            Tab::Blocking => render_blocking(frame, body, app),
            Tab::Statements => render_statements(frame, body, app),
            Tab::History => render_history(frame, body, app),
            Tab::Database => investigation::database(frame, body, app),
            Tab::Relations => investigation::relations(frame, body, app),
            Tab::Replication => investigation::replication(frame, body, app),
            Tab::Io => investigation::io(frame, body, app),
            Tab::Incidents => investigation::incidents(frame, body, app),
        }
    }
    if let Some(error) = &app.error {
        frame.render_widget(
            Paragraph::new(format!(" Error: {}", clean(error)))
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
    let filter_text = if app.report.is_some() && app.has_return_path() {
        " [ / ]: previous / next section  Home/End: first/last  Backspace: return".into()
    } else if app.report.is_some() {
        " [ / ]: previous / next section  Home/End: first/last".into()
    } else if !app.filter_available() {
        " j/k: scroll  Tab / 1–9,0: view  h: coverage".into()
    } else if app.editing_filter {
        format!(
            " {} /{}▏  Enter: apply  Esc: cancel  Ctrl+U: clear",
            app.tab.title(),
            app.filter
        )
    } else if !app.filter.is_empty() {
        format!(
            " {} filter: {}  /: edit  Esc: clear",
            app.tab.title(),
            app.filter
        )
    } else {
        format!(
            " /: filter {}  j/k: select  Tab / 1–9,0: view",
            app.tab.title()
        )
    };
    frame.render_widget(
        Paragraph::new(filter_text).fg(if app.editing_filter { ACCENT } else { MUTED }),
        filter,
    );
    let footer_text = if app.report.is_some() && app.tab == Tab::Overview {
        " ↑/↓ PgUp/PgDn: scroll  e: evidence  Esc: close  ?: help  q: quit"
    } else if app.report.is_some() && matches!(app.tab, Tab::Activity | Tab::Blocking) {
        " ↑/↓ PgUp/PgDn: scroll  b/w: blocker/waiter  Esc: close  ?: help"
    } else if app.report.is_some() {
        " ↑/↓ PgUp/PgDn: scroll  Esc: close  ?: help  q: quit"
    } else if app.tab == Tab::History {
        " Enter: inspect  a/b: mark  d: compare  l: label  I: attach  ?: help"
    } else if app.tab == Tab::Incidents {
        " Enter: inspect  e/E: export md/json  t: list/timeline  n: note  ?: help"
    } else if app.tab == Tab::Relations {
        " Enter: full details  v: tables/indexes  s: sort  c: capture  ?: help"
    } else if app.tab == Tab::Statements {
        " Enter: full details  v: cumulative/interval  s: sort  c: capture  ?: help"
    } else if matches!(app.tab, Tab::Activity | Tab::Blocking) && app.has_return_path() {
        " Enter: full details  b/w: blocker/waiter  Backspace: return  ?: help"
    } else if matches!(app.tab, Tab::Activity | Tab::Blocking) {
        " Enter: full details  b/w: blocker/waiter  c: capture  ?: help  q: quit"
    } else if app.tab == Tab::Overview {
        " Enter: finding  e: evidence  h: coverage  c: capture  ?: help  q: quit"
    } else if app.is_offline() {
        " OFFLINE  Esc: return to live  5: history  ?: help  q: quit"
    } else {
        " r: refresh  p: pause  c: capture  5: history  ?: help  q: quit"
    };
    frame.render_widget(Paragraph::new(footer_text).fg(ACCENT), footer);
    if app.help {
        render_help(frame, area, app.help_scroll);
    }
    if app.prompt.is_some() {
        investigation::prompt(frame, area, app);
    }
}

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let comparison = app.report.is_some() && app.report_title == "Offline comparison";
    let report_status = app.report_title.to_uppercase();
    let (status, color) = if app.report.is_some() && !comparison {
        (report_status.as_str(), ACCENT)
    } else if comparison {
        ("OFFLINE COMPARISON", WARNING)
    } else if app.capture.is_none() && app.has_return_path() {
        ("FROZEN EVIDENCE", WARNING)
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
        Span::styled("● ", Style::new().fg(color)),
        Span::styled(status, Style::new().bold()),
    ];
    if let Some(id) = app.active_incident {
        headline.push(Span::styled(
            format!("  incident #{id}"),
            Style::new().fg(ACCENT),
        ));
    }
    if app.loading {
        headline.push(Span::styled("  collecting…", Style::new().fg(MUTED)));
    }
    if app.paused() && app.demo && !app.is_offline() {
        headline.push(Span::styled("  PAUSED", Style::new().fg(WARNING)));
    }
    let source = if comparison {
        " Comparing saved captures offline · live refresh is suspended".into()
    } else {
        match app.snapshot() {
            Some(snapshot) => {
                let age = (app.now - snapshot.completed_at).num_seconds().max(0);
                let context_width = usize::from(area.width.saturating_sub(36));
                let endpoint = fit_text(&clean(&snapshot.source.endpoint), context_width / 2);
                let database = fit_text(&clean(&snapshot.source.database), context_width / 2);
                format!(
                    " Observed {age}s ago · {endpoint} / {database} · PG {}",
                    clean(&snapshot.source.server_version)
                )
            }
            None => {
                " Read-only PostgreSQL investigation · local snapshots · no server mutations".into()
            }
        }
    };
    let mut lines = vec![Line::from(headline), Line::from(source)];
    if let Some(capture) = &app.capture {
        lines.push(Line::raw(format!(
            " Capture #{} · {} · {} UTC",
            capture.id,
            clean(&capture.label),
            capture.captured_at.format("%Y-%m-%d %H:%M:%S")
        )));
    }
    frame.render_widget(Paragraph::new(lines), area);
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
        Constraint::Length(if wide { 13 } else { 10 }),
        Constraint::Length(if wide { 20 } else { 14 }),
        Constraint::Length(10),
        Constraint::Length(9),
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
        render_session_details(
            frame,
            details,
            session,
            " Selected session · Enter: full details ",
        );
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
            let blocker = all_sessions.and_then(|sessions| unique_session(sessions, *blocker_pid));
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
            all_sessions.and_then(|sessions| unique_session(sessions, *blocker_pid))
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
    if app.statement_interval {
        investigation::statement_rates(frame, area, app);
        return;
    }
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
    let wide = area.width >= 100;
    let rows: Vec<_> = statements
        .iter()
        .map(|statement| {
            let mut cells = vec![
                statement.queryid.to_string(),
                statement.calls.to_string(),
                format!("{:.2}", statement.total_exec_ms),
                format!("{:.2}", statement.mean_exec_ms),
                statement.temp_blks_written.to_string(),
            ];
            if wide {
                cells.push(statement.rows.to_string());
            }
            Row::new(cells)
        })
        .collect();
    let title = format!(
        " Statements · cumulative · sort: {} ↓ (s) · v: interval{} ",
        app.sort.title(),
        if stats.truncated { " · LIMITED" } else { "" }
    );
    let mut widths = vec![
        Constraint::Min(18),
        Constraint::Length(8),
        Constraint::Length(13),
        Constraint::Length(11),
        Constraint::Length(11),
    ];
    let mut headers = vec![
        "Query ID",
        "Calls",
        "Total exec ms",
        "Mean exec ms",
        "Temp blocks",
    ];
    if wide {
        widths.push(Constraint::Length(12));
        headers.push("Rows");
    }
    let table = Table::new(rows, widths)
        .header(table_header(headers))
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
                .block(panel(" Statement preview · Enter: full details "))
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
    let rows = report_lines(report, area.width.saturating_sub(2));
    let visible = usize::from(area.height.saturating_sub(2));
    let offset = app.report_scroll.min(rows.len().saturating_sub(visible));
    let end = (offset + visible).min(rows.len());
    let title = format!(
        " {} · rows {}–{}/{} ",
        app.report_title,
        offset + usize::from(!rows.is_empty()),
        end,
        rows.len()
    );
    let lines: Vec<_> = rows
        .into_iter()
        .skip(offset)
        .take(visible)
        .map(report_line)
        .collect();
    frame.render_widget(Paragraph::new(lines).block(panel(title)), area);
}

fn report_line(line: String) -> Line<'static> {
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
    Line::styled(line, style)
}

/// Prewrap by terminal cell width so scrolling counts displayed rows, including
/// very long SQL values, rather than skipping whole unwrapped source lines.
pub(crate) fn report_lines(report: &str, width: u16) -> Vec<String> {
    let width = usize::from(width.max(2));
    let mut rows = Vec::new();
    for line in report.lines() {
        let mut row = String::new();
        for character in clean(line).chars() {
            row.push(character);
            if Span::raw(row.as_str()).width() > width {
                row.pop();
                rows.push(std::mem::take(&mut row));
                row.push(character);
            }
        }
        rows.push(row);
    }
    rows
}

fn fit_text(text: &str, width: usize) -> String {
    if Span::raw(text).width() <= width {
        return text.into();
    }
    let mut shortened = String::new();
    for character in text.chars() {
        shortened.push(character);
        if Span::raw(shortened.as_str()).width() >= width {
            shortened.pop();
            break;
        }
    }
    shortened.push('…');
    shortened
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

pub(crate) fn help_dimensions(width: u16, height: u16) -> (u16, u16) {
    (
        width.saturating_sub(2).min(90),
        height.saturating_sub(2).min(31),
    )
}

pub(crate) fn help_lines(width: u16) -> Vec<String> {
    report_lines(
        "# NAVIGATION
Tab / Shift+Tab / 1–9,0   Switch among ten investigation views
↑/↓ or j/k · PgUp/PgDn    Select rows / scroll; Home/End: first/last
/                        Filter this view; Enter: apply, Esc: cancel
Esc                      Clear filter / close report / return live
Backspace                Return to the originating finding or incident
r / p / c                Refresh / pause / collect and save

# INVESTIGATION
Enter                    Full finding, session, blocking, statement or relation details
e in Overview            Follow the selected finding to related evidence
b / w                    Follow a blocker / waiter from a session or relationship
h                        Read provenance, collection coverage and interval readiness
[ / ] in a report        Previous / next section; full evidence stays accessible
4 Statements: v / s      Cumulative ↔ interval / change ranking
7 Relations: v / s        Tables ↔ indexes / change ranking
6 Database · 8 Replica · 9 I/O: j/k scroll detailed metrics

# HISTORY AND INCIDENTS
5 History: Enter         Inspect selected capture offline
a / b / d / l            Mark earlier / later / compare / edit label
i                        Create and activate a local incident
0 Incidents: Enter       Open the full chronology; activate if open
n                        Add a note to the active incident
I in History             Attach selected capture to active incident
o / x in Incidents       Close or reopen / stop attaching new captures
t / Esc in timeline      Return to incident list; t reopens the timeline
Enter in timeline        Open a capture or the full selected note
e / E in Incidents       Export Markdown / JSON to a new private destination

# SAFETY AND EXIT
Read-only. Unavailable metrics remain unavailable. SQL text requires opt-in.
? / Esc                  Close help
q / Ctrl+C               Quit and restore the terminal",
        width,
    )
}

fn render_help(frame: &mut Frame, area: Rect, scroll: usize) {
    let (width, height) = help_dimensions(area.width, area.height);
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    let rows = help_lines(width.saturating_sub(2));
    let visible = usize::from(height.saturating_sub(2));
    let offset = scroll.min(rows.len().saturating_sub(visible));
    let title = format!(
        " Keyboard help · {}/{} · ↑/↓ PgUp/PgDn Home/End ",
        offset + 1,
        rows.len()
    );
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(
            rows.into_iter()
                .skip(offset)
                .take(visible)
                .map(report_line)
                .collect::<Vec<_>>(),
        )
        .block(
            Block::bordered()
                .title(title)
                .border_style(Style::new().fg(ACCENT)),
        ),
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
            .row_highlight_style(Style::new().reversed().bold())
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
        "Not visible / exited / ambiguous"
    }
}

fn unique_session(sessions: &[Session], pid: i32) -> Option<&Session> {
    let mut matches = sessions.iter().filter(|session| session.pid == pid);
    let session = matches.next()?;
    matches.next().is_none().then_some(session)
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
            if character.is_control() || matches!(character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}') {
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
            health: crate::model::Health::default(),
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
        assert!(text.contains("Extension is not installed"));
        assert!(text.contains("Collection: 1/8 sections; 7 unavailable"));
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

    #[test]
    fn investigation_views_render_evidence_and_scope_without_io() -> Result<(), Infallible> {
        let mut app = App::new(true);
        let mut first = crate::demo::snapshot(0, false);
        let mut second = crate::demo::snapshot(1, false);
        first.started_at = Utc::now() - chrono::Duration::seconds(10);
        first.completed_at = first.started_at;
        second.started_at = first.completed_at + chrono::Duration::seconds(5);
        second.completed_at = second.started_at;
        app.set_snapshot(first, false);
        app.set_snapshot(second, false);
        for (tab, expected) in [
            (Tab::Overview, "Investigation finding"),
            (Tab::Database, "Transactions:"),
            (Tab::Relations, "Est. dead"),
            (Tab::Replication, "REPLICATION SLOTS"),
            (Tab::Io, "VACUUM PROGRESS"),
            (Tab::Incidents, "Press i to create"),
        ] {
            app.tab = tab;
            let text = screen(&app, 160, 60)?;
            assert!(text.contains(expected), "{tab:?} should contain {expected}");
            assert!(
                text.contains("0 Incidents"),
                "Second row of view navigation is present"
            );
        }
        app.tab = Tab::Relations;
        app.show_indexes = true;
        assert!(screen(&app, 160, 40)?.contains("Validity"));
        app.tab = Tab::Statements;
        app.statement_interval = true;
        let text = screen(&app, 160, 40)?;
        assert!(text.contains("Calls/s"));
        assert!(text.contains("Temp blocks/s"));
        Ok(())
    }

    #[test]
    fn offline_database_does_not_show_live_trends() -> Result<(), Infallible> {
        let mut app = App::new(true);
        app.set_snapshot(crate::demo::snapshot(0, false), false);
        app.set_snapshot(crate::demo::snapshot(1, false), true);
        app.tab = Tab::Database;
        let text = screen(&app, 160, 60)?;
        assert!(text.contains("Live trends are hidden"));
        assert!(!text.contains("visible samples"));
        Ok(())
    }

    #[test]
    fn finding_report_and_incident_prompt_are_identified_correctly() -> Result<(), Infallible> {
        let mut app = App::new(true);
        app.set_snapshot(crate::demo::snapshot(0, false), false);
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        let text = screen(&app, 120, 40)?;
        assert!(text.contains("FINDING DETAILS"));
        assert!(text.contains("Finding details"));
        assert!(!text.contains("OFFLINE COMPARISON"));
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Char('i'),
            KeyModifiers::NONE,
        )));
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
        )));
        let text = screen(&app, 120, 40)?;
        assert!(text.contains("New incident"));
        assert!(text.contains("q▏"));
        assert!(text.contains("Enter: save"));
        assert!(!app.should_quit());
        Ok(())
    }

    #[test]
    fn compact_and_standard_terminals_keep_primary_columns_and_navigation() -> Result<(), Infallible>
    {
        let mut app = App::new(true);
        let mut first = crate::demo::snapshot(0, false);
        let mut second = crate::demo::snapshot(1, false);
        first.started_at = Utc::now() - chrono::Duration::seconds(10);
        first.completed_at = first.started_at;
        second.started_at = first.completed_at + chrono::Duration::seconds(5);
        second.completed_at = second.started_at;
        app.set_snapshot(first, false);
        app.set_snapshot(second, false);
        for (width, height) in [(80, 24), (140, 36)] {
            for (tab, expected) in [
                (Tab::Overview, "Severity"),
                (Tab::Activity, "Query age"),
                (Tab::Blocking, "Blocker"),
                (Tab::Statements, "Temp blocks"),
                (Tab::History, "No saved captures"),
                (Tab::Database, "Transactions:"),
                (Tab::Relations, "Seq scans"),
                (Tab::Replication, "WAL SENDERS"),
                (Tab::Io, "WAL GENERATION"),
                (Tab::Incidents, "No incidents"),
            ] {
                app.tab = tab;
                let text = screen(&app, width, height)?;
                assert!(
                    text.contains(expected),
                    "{width}x{height} {tab:?}: missing {expected}"
                );
                assert!(text.contains("0 Incidents"));
            }
            app.tab = Tab::Statements;
            app.statement_interval = true;
            assert!(screen(&app, width, height)?.contains("Temp blocks/s"));
            app.statement_interval = false;
            app.tab = Tab::Relations;
            app.show_indexes = true;
            assert!(screen(&app, width, height)?.contains("Constraint"));
            app.show_indexes = false;
        }
        Ok(())
    }

    #[test]
    fn compact_header_keeps_freshness_source_and_capture_identity() -> Result<(), Infallible> {
        let mut app = App::new(true);
        let mut observation = crate::demo::snapshot(0, false);
        observation.source.endpoint = "some-extraordinarily-long-development-endpoint:5432".into();
        observation.source.database = "checkout_database_with_a_long_name".into();
        let now = observation.completed_at + chrono::Duration::seconds(42);
        app.set_snapshot(observation, true);
        app.now = now;
        app.capture = Some(SnapshotSummary {
            id: 123,
            label: "Before pool adjustment".into(),
            captured_at: app.now,
            source: "fixture".into(),
            complete: true,
        });
        app.tab = Tab::Activity;
        for (width, height) in [(80, 24), (120, 36)] {
            let text = screen(&app, width, height)?;
            assert!(text.contains("Observed 42s ago"));
            assert!(text.contains("checkout"));
            assert!(text.contains("some-extraord"));
            assert!(text.contains("Capture #123 · Before pool adjustment"));
            assert!(text.contains("Enter: full details"));
        }
        Ok(())
    }

    #[test]
    fn full_details_reach_long_sql_end_at_both_sizes() -> Result<(), Infallible> {
        for (width, height) in [(80, 24), (120, 36)] {
            let mut app = App::new(true);
            let mut observation = crate::demo::snapshot(0, true);
            let query = format!(
                "SELECT {}\nFROM fixture_table; END_OF_CAPTURED_SQL",
                "long_column, ".repeat(500)
            );
            if let Observation::Available(sessions) = &mut observation.activity {
                for session in sessions {
                    session.query = Some(query.clone());
                }
            }
            if let Observation::Available(statements) = &mut observation.statements {
                for statement in &mut statements.entries {
                    statement.query = Some(query.clone());
                }
            }
            app.set_snapshot(observation, false);
            for tab in [Tab::Activity, Tab::Blocking, Tab::Statements] {
                app.tab = tab;
                let (title, report) =
                    detail_report(&app).expect("demo has selected detail evidence");
                assert!(report.contains(&query));
                app.report = Some(report);
                app.report_title = title;
                app.report_scroll = usize::MAX;
                let text = screen(&app, width, height)?;
                assert!(
                    text.contains("END_OF_CAPTURED_SQL"),
                    "{width}x{height} {tab:?}"
                );
                assert!(text.contains("rows "));
                app.report = None;
            }
        }
        Ok(())
    }

    #[test]
    fn scrollable_help_exposes_final_actions_and_navigation() -> Result<(), Infallible> {
        let mut app = App::new(true);
        app.help = true;
        for (width, height) in [(80, 24), (120, 36)] {
            app.help_scroll = 0;
            let first = screen(&app, width, height)?;
            assert!(first.contains("NAVIGATION"));
            assert!(first.contains("PgUp/PgDn Home/End"));
            app.help_scroll = usize::MAX;
            let last = screen(&app, width, height)?;
            assert!(last.contains("HISTORY AND INCIDENTS"));
            assert!(last.contains("Quit and restore the terminal"));
            assert!(last.contains("SQL text requires opt-in"));
        }
        Ok(())
    }

    #[test]
    fn report_wrap_preserves_sql_and_wide_characters() {
        let report = "SELECT \"中文名称\", extraordinarily_long_column FROM table;";
        for width in [8, 20, 78] {
            let rows = report_lines(report, width);
            assert_eq!(rows.concat(), report);
            assert!(
                rows.iter()
                    .all(|row| Span::raw(row.as_str()).width() <= usize::from(width))
            );
        }
        assert_eq!(report_lines("a\u{1b}[31m\nb", 78), ["a [31m", "b"]);
    }

    #[test]
    fn interval_and_relation_reports_retain_identity_and_missing_values() {
        let mut app = App::new(true);
        let first = crate::demo::snapshot(0, true);
        let mut second = first.clone();
        second.started_at = first.completed_at + chrono::Duration::seconds(5);
        second.completed_at = second.started_at;
        app.set_snapshot(first, false);
        app.set_snapshot(second, false);
        app.tab = Tab::Statements;
        app.statement_interval = true;
        let (_, report) = detail_report(&app).expect("interval detail");
        assert!(report.contains("Top level: true"));
        assert!(report.contains("No completed calls in this interval"));
        assert!(report.contains("SELECT ") || report.contains("UPDATE "));
        assert!(report.contains("Elapsed interval: 5.000 seconds"));
        app.tab = Tab::Relations;
        let (_, table) = detail_report(&app).expect("table detail");
        assert!(table.contains("Relation OID:"));
        assert!(table.contains("Last autoanalyze:"));
        assert!(table.contains("Frozen transaction ID age:"));
        app.show_indexes = true;
        let (_, index) = detail_report(&app).expect("index detail");
        assert!(index.contains("Index OID:"));
        assert!(index.contains("Primary:"));
        assert!(index.contains("Heap tuples fetched:"));
    }

    #[test]
    fn ambiguous_blocker_report_does_not_pick_an_arbitrary_backend() {
        let mut observation = crate::demo::snapshot(0, true);
        if let Observation::Available(sessions) = &mut observation.activity {
            let blocker = sessions
                .iter_mut()
                .find(|session| session.pid == 4101)
                .expect("demo blocker");
            blocker.query = Some("AMBIGUOUS_BLOCKER_SQL".into());
            let duplicate = blocker.clone();
            sessions.push(duplicate);
        }
        let mut app = App::new(true);
        app.set_snapshot(observation, false);
        app.tab = Tab::Blocking;
        let (_, report) = detail_report(&app).expect("blocking detail");
        assert!(report.contains("Identity unavailable or ambiguous"));
        assert!(!report.contains("AMBIGUOUS_BLOCKER_SQL"));
    }

    #[test]
    fn inactive_filter_controls_are_absent_and_selection_has_a_marker() -> Result<(), Infallible> {
        let mut app = App::new(true);
        app.set_snapshot(crate::demo::snapshot(0, false), false);
        for tab in [Tab::Database, Tab::Replication, Tab::Io] {
            app.tab = tab;
            let text = screen(&app, 80, 24)?;
            assert!(!text.contains("/: filter"));
        }
        app.tab = Tab::Activity;
        let mut terminal = Terminal::new(TestBackend::new(80, 24))?;
        terminal.draw(|frame| render(frame, &app))?;
        let buffer = terminal.backend().buffer();
        assert!(buffer.content.iter().any(|cell| cell.symbol() == "›"));
        // Theme-configured foreground keeps freshness readable without assuming
        // a light or dark terminal background.
        assert_eq!(buffer[(1, 1)].fg, Color::Reset);
        Ok(())
    }
}
