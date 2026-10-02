mod details;
mod investigation;
pub(crate) mod layout;
#[cfg(test)]
mod review;
mod shell;
mod theme;
pub(crate) use shell::{
    controls, editor_controls, help_rows, menu_items, overlay_controls, palette_area,
};
pub(crate) use theme::Theme;
use theme::{ACCENT, BORDER, MUTED, SELECTED, SURFACE, WARNING};

pub(crate) use details::detail_report;
pub(crate) use investigation::metric_rows;

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph, Row, Table, TableState, Wrap},
};

use crate::{
    app::{App, Tab},
    model::{Observation, Session},
};

pub(crate) fn render(frame: &mut Frame, app: &App) {
    shell::render(frame, app);
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
    let table_area = area;
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
            data_row(cells, &[0, 3, 4]).style(if session.blockers.is_empty() {
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
        .header(table_header(headers, &[0, 3, 4]))
        .block(content_panel(
            app,
            format!(
                " Activity · {} visible · current elapsed time ",
                sessions.len()
            ),
        ));
    render_table(frame, table_area, table, app);
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
    let table_area = area;
    let rows: Vec<_> = edges
        .iter()
        .map(|(waiter, blocker_pid)| {
            let blocker = all_sessions.and_then(|sessions| unique_session(sessions, *blocker_pid));
            data_row(
                vec![
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
                ],
                &[0, 1, 2, 5],
            )
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
    .header(table_header(
        vec![
            "Waiter",
            "Blocker",
            "Waiter tx",
            "Wait event",
            "Blocker state",
            "Blocker tx",
        ],
        &[0, 1, 2, 5],
    ))
    .block(content_panel(
        app,
        format!(" Blocking · {} server-reported relationships ", edges.len()),
    ));
    render_table(frame, table_area, table, app);
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
    let table_area = area;
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
            data_row(cells, &[0, 1, 2, 3, 4, 5])
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
        Constraint::Length(12),
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
        .header(table_header(headers, &[0, 1, 2, 3, 4, 5]))
        .block(content_panel(app, title));
    render_table(frame, table_area, table, app);
}

fn render_history(frame: &mut Frame, area: Rect, app: &App) {
    if let Some(error) = &app.history_error {
        render_empty(
            frame,
            area,
            " Captures unavailable ",
            &format!(
                "{}\n\nPress r to reload the local capture list.",
                clean(error)
            ),
        );
        return;
    }
    let [summary, area] = Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).areas(area);
    let capture_name = |id: Option<i64>| {
        id.map_or_else(
            || "not selected".into(),
            |id| {
                app.history.iter().find(|c| c.id == id).map_or_else(
                    || format!("#{id}"),
                    |c| format!("#{id} {}", clean(&c.label)),
                )
            },
        )
    };
    let half = usize::from(summary.width.saturating_sub(24)) / 2;
    frame.render_widget(
        Paragraph::new(vec![
            Line::raw(format!(
                " Before: {}  →  After: {}",
                fit_text(&capture_name(app.compare_a), half),
                fit_text(&capture_name(app.compare_b), half)
            ))
            .fg(ACCENT),
            Line::raw(
                " Mark two observations, then Compare (d). Source and counter resets are checked.",
            )
            .fg(MUTED),
        ]),
        summary,
    );
    let captures = app.filtered_history();
    if captures.is_empty() {
        render_empty(
            frame,
            area,
            " Captures ",
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
            data_row(
                vec![
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
                ],
                &[1],
            )
        })
        .collect();
    let title = " Saved captures ";
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
    .header(table_header(
        vec!["", "ID", "Captured UTC", "Label", "Quality", "Source"],
        &[1],
    ))
    .block(content_panel(app, title));
    render_table(frame, area, table, app);
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
    frame.render_widget(
        Paragraph::new(lines).block(
            panel(title)
                .title_bottom(" Esc: close · [ / ]: sections ")
                .border_style(
                    Style::new().fg(if app.focus == crate::app::Focus::Inspector {
                        ACCENT
                    } else {
                        BORDER
                    }),
                ),
        ),
        area,
    );
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
    let line = if line.starts_with("# ") || line.starts_with("## ") || line.starts_with("### ") {
        line.trim_start_matches('#').trim_start().to_owned()
    } else {
        line
    };
    Line::styled(line, style)
}

/// Prewrap by terminal cell width so scrolling counts displayed rows, including
/// very long SQL values, rather than skipping whole unwrapped source lines.
pub(crate) fn report_lines(report: &str, width: u16) -> Vec<String> {
    let width = usize::from(width.max(2));
    let mut rows = Vec::new();
    if report.is_empty() {
        return vec![String::new()];
    }
    for line in report.lines() {
        let mut row = String::new();
        for character in clean(line).chars() {
            row.push(character);
            if Span::raw(row.as_str()).width() > width {
                row.pop();
                // Prefer word boundaries without losing characters from captured SQL.
                let boundary = row
                    .char_indices()
                    .filter(|(_, c)| c.is_whitespace())
                    .map(|(i, c)| i + c.len_utf8())
                    .next_back();
                if let Some(boundary) = boundary {
                    let tail = row.split_off(boundary);
                    rows.push(std::mem::replace(&mut row, tail));
                } else {
                    rows.push(std::mem::take(&mut row));
                }
                row.push(character);
            }
        }
        rows.push(row);
    }
    rows
}

fn fit_text(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
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

fn wrap_styled_line(line: Line<'static>, width: u16) -> Vec<Line<'static>> {
    let spans = line
        .spans
        .into_iter()
        .map(|span| (clean(&span.content), span.style))
        .collect::<Vec<_>>();
    let text = spans
        .iter()
        .map(|(text, _)| text.as_str())
        .collect::<String>();
    let mut span_index = 0;
    let mut span_byte = 0;
    report_lines(&text, width)
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            let mut remaining = row.len();
            let mut output = Vec::new();
            while remaining > 0 && span_index < spans.len() {
                let (text, style) = &spans[span_index];
                let count = remaining.min(text.len() - span_byte);
                if count > 0 {
                    output.push(Span::styled(
                        text[span_byte..span_byte + count].to_owned(),
                        *style,
                    ));
                }
                span_byte += count;
                remaining -= count;
                if span_byte == text.len() {
                    span_index += 1;
                    span_byte = 0;
                }
            }
            Line::from(output).style(if index == 0 {
                line.style
            } else {
                line.style.remove_modifier(ratatui::style::Modifier::BOLD)
            })
        })
        .collect()
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
Tab / Shift+Tab          Move between visible zones
F2 / Ctrl+K              Open navigation / search commands
1–9,0                    Switch among ten investigation views
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

# CAPTURES AND INCIDENTS
5 Captures: Enter         Inspect selected capture offline
a / b / d / l            Mark earlier / later / compare / edit label
i                        Create and activate a local incident
0 Incidents: Enter       Open the full chronology; activate if open
a in chronology         Read capture metadata and full annotation
n                        Add a note to the active incident
I in Captures             Attach selected capture to active incident
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

fn render_table(frame: &mut Frame, area: Rect, table: Table<'_>, app: &App) {
    let (offset, visible) = layout::table_window(app, area);
    let mut state = TableState::new()
        .with_selected(Some(app.row_selection()))
        .with_offset(offset);
    let count = app.row_count();
    let position = format!(
        " {}–{} / {} · {} ",
        if count == 0 { 0 } else { offset + 1 },
        (offset + visible).min(count),
        count,
        if app.focus == crate::app::Focus::Content {
            "Content"
        } else {
            "Tab: focus"
        }
    );
    frame.render_stateful_widget(
        table
            .row_highlight_style(Style::new().fg(Color::White).bg(SELECTED).bold())
            .highlight_symbol("› "),
        area,
        &mut state,
    );
    if area.height > 1 && area.width > 4 {
        frame.render_widget(
            Paragraph::new(fit_text(&position, area.width.saturating_sub(4) as usize))
                .fg(if app.focus == crate::app::Focus::Content {
                    ACCENT
                } else {
                    MUTED
                })
                .bg(SURFACE),
            Rect::new(
                area.x + 2,
                area.bottom() - 1,
                (Span::raw(&position).width() as u16).min(area.width - 4),
                1,
            ),
        );
    }
}

fn table_header(labels: Vec<&str>, numeric: &[usize]) -> Row<'static> {
    data_row(labels.into_iter().map(str::to_owned).collect(), numeric)
        .style(Style::new().fg(ACCENT).bold())
}
fn panel<'a>(title: impl Into<Line<'a>>) -> Block<'a> {
    Block::bordered()
        .title(title.into().fg(MUTED))
        .border_style(Style::new().fg(BORDER))
        .bg(SURFACE)
}

fn content_panel<'a>(app: &App, title: impl Into<Line<'a>>) -> Block<'a> {
    panel(title).border_style(Style::new().fg(if app.focus == crate::app::Focus::Content {
        ACCENT
    } else {
        BORDER
    }))
}

fn data_row(values: Vec<String>, numeric: &[usize]) -> Row<'static> {
    Row::new(values.into_iter().enumerate().map(|(index, value)| {
        ratatui::widgets::Cell::from(Line::raw(value).alignment(if numeric.contains(&index) {
            ratatui::layout::Alignment::Right
        } else {
            ratatui::layout::Alignment::Left
        }))
    }))
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

    fn screen(app: &mut App, width: u16, height: u16) -> Result<String, Infallible> {
        app.viewport_width = width;
        app.viewport_height = height;
        app.clamp_scroll();
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
        let text = screen(&mut app, 120, 30)?;
        assert!(text.contains("LIVE · CONNECTED"));
        assert!(text.contains("Collection: 1/8 sections; 7 unavailable"));
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Char('h'),
            KeyModifiers::NONE,
        )));
        assert!(screen(&mut app, 120, 30)?.contains("Extension is not installed"));
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        app.set_error("Database could not be reached".into());
        let text = screen(&mut app, 120, 30)?;
        assert!(text.contains("STALE · COLLECTION FAILED"));
        assert!(text.contains("Database could not be reached"));
        Ok(())
    }

    #[test]
    fn workspace_read_errors_are_distinct_from_empty_lists_and_can_recover()
    -> Result<(), Infallible> {
        use crate::app::CommandId;
        let mut app = App::new(false);
        app.tab = Tab::History;
        app.history_error = Some("Local store could not be opened".into());
        let text = screen(&mut app, 80, 24)?;
        assert!(text.contains("Captures unavailable"));
        assert!(text.contains("Press r to reload"));
        assert!(!text.contains("No saved captures"));
        assert!(
            app.commands()
                .iter()
                .find(|c| c.id == CommandId::Inspect)
                .unwrap()
                .disabled
                .is_some()
        );
        app.set_history(Vec::new());
        assert!(screen(&mut app, 80, 24)?.contains("No saved captures"));
        app.tab = Tab::Incidents;
        app.incidents_error = Some("Local store could not be opened".into());
        let text = screen(&mut app, 80, 24)?;
        assert!(text.contains("Incidents unavailable"));
        assert!(!text.contains("No incidents yet"));
        app.set_incidents(Vec::new());
        assert!(screen(&mut app, 80, 24)?.contains("No incidents yet"));
        Ok(())
    }

    #[test]
    fn demo_offline_and_tiny_terminal_states_render() -> Result<(), Infallible> {
        let mut app = App::new(true);
        app.set_snapshot(snapshot(), false);
        assert!(screen(&mut app, 100, 25)?.contains("DEMO · SYNTHETIC DATA"));
        app.set_snapshot(snapshot(), true);
        assert!(screen(&mut app, 100, 25)?.contains("OFFLINE CAPTURE"));
        for (width, height) in [(0, 0), (1, 1), (10, 3), (25, 8), (50, 12)] {
            for tab in Tab::ALL {
                app.tab = tab;
                screen(&mut app, width, height)?;
            }
            app.help = true;
            screen(&mut app, width, height)?;
            app.help = false;
        }
        Ok(())
    }

    #[test]
    fn responsive_focus_panels_and_overlays_use_the_displayed_geometry() -> Result<(), Infallible> {
        use crate::app::{CommandId, Focus, Overlay};
        for theme in [Theme::Dark, Theme::Terminal] {
            for (width, height) in [
                (24, 8),
                (40, 12),
                (80, 24),
                (99, 24),
                (100, 24),
                (120, 36),
                (139, 36),
                (140, 36),
                (160, 48),
            ] {
                let mut app = App::new(true);
                app.theme = theme;
                app.set_snapshot(crate::demo::snapshot(0, true), false);
                screen(&mut app, width, height)?;
                let geometry = layout::Screen::for_app(&app);
                assert_eq!(geometry.sidebar.width, if width >= 100 { 22 } else { 0 });
                assert_eq!(geometry.content.width, width - geometry.sidebar.width);
                app.execute(CommandId::Inspect);
                screen(&mut app, width, height)?;
                let geometry = layout::Screen::for_app(&app);
                assert_eq!(geometry.split, width >= 140);
                assert_eq!(app.focus, Focus::Inspector);
                assert_eq!(
                    geometry.inspector.width + geometry.content.width + geometry.sidebar.width,
                    width
                );
                app.execute(CommandId::Commands);
                screen(&mut app, width, height)?;
                app.overlay = Overlay::None;
                app.help = true;
                screen(&mut app, width, height)?;
                app.help = false;
                app.execute(CommandId::NewIncident);
                app.prompt.as_mut().unwrap().value = format!("{}LAST_TYPED", "中文é ".repeat(80));
                let text = screen(&mut app, width, height)?;
                assert!(
                    text.contains("LAST_TYPED"),
                    "input end hidden at {width}x{height}"
                );
            }
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
        let text = screen(&mut app, 120, 16)?;
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
            let text = screen(&mut app, 160, 60)?;
            assert!(text.contains(expected), "{tab:?} should contain {expected}");
            assert!(text.contains("0 Incidents"), "Grouped sidebar is present");
        }
        app.tab = Tab::Relations;
        app.show_indexes = true;
        assert!(screen(&mut app, 160, 40)?.contains("Validity"));
        app.tab = Tab::Statements;
        app.statement_interval = true;
        let text = screen(&mut app, 160, 40)?;
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
        let text = screen(&mut app, 160, 60)?;
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
        let text = screen(&mut app, 120, 40)?;
        assert!(text.contains("FROZEN EVIDENCE"));
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
        let text = screen(&mut app, 120, 40)?;
        assert!(text.contains("New incident"));
        assert!(text.contains("q▏"));
        assert!(text.contains("Enter Save"));
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
        for (width, height) in [(80, 24), (120, 36), (160, 48)] {
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
                let text = screen(&mut app, width, height)?;
                assert!(
                    text.contains(expected),
                    "{width}x{height} {tab:?}: missing {expected}"
                );
                assert!(text.contains("F2 Menu"));
                if width >= 100 {
                    assert!(text.contains("0 Incidents"));
                }
            }
            app.tab = Tab::Statements;
            app.statement_interval = true;
            assert!(screen(&mut app, width, height)?.contains("Temp blocks/s"));
            app.statement_interval = false;
            app.tab = Tab::Relations;
            app.show_indexes = true;
            assert!(screen(&mut app, width, height)?.contains("Constraint"));
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
            let text = screen(&mut app, width, height)?;
            assert!(text.contains("Observed 42s ago"));
            assert!(text.contains("checkout"));
            assert!(text.contains("some-extra"));
            assert!(text.contains("SYNTHETIC DATA"));
            assert!(text.contains("Capture #123 · Before pool adjustment"));
            assert!(text.contains("Enter Inspect"));
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
                let text = screen(&mut app, width, height)?;
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
        app.help_all = true;
        for (width, height) in [(80, 24), (120, 36)] {
            app.help_scroll = 0;
            let first = screen(&mut app, width, height)?;
            assert!(first.contains("NAVIGATION"));
            assert!(first.contains("Move between visible zones"));
            app.help_scroll = usize::MAX;
            let last = screen(&mut app, width, height)?;
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
    fn comparison_header_identifies_saved_sources_instead_of_the_live_observation()
    -> Result<(), Infallible> {
        let mut app = App::new(false);
        app.set_snapshot(snapshot(), false);
        app.set_report("# Comparison\nSaved evidence".into());
        let capture = |id| SnapshotSummary {
            id,
            label: format!("Capture {id}"),
            captured_at: app.now,
            source: "demo / saved_shop".into(),
            complete: true,
        };
        app.comparison = Some(crate::app::ComparisonContext {
            before: capture(11),
            after: capture(12),
            synthetic: true,
        });
        let text = screen(&mut app, 80, 24)?;
        assert!(text.contains("Before #11 → After #12 · demo / saved_shop"));
        assert!(text.contains("SYNTHETIC DATA"));
        assert!(!text.contains("localhost:5432"));
        assert!(!text.contains("Observed 0s ago"));
        Ok(())
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
            let text = screen(&mut app, 80, 24)?;
            assert!(!text.contains("/: filter"));
        }
        app.tab = Tab::Activity;
        let mut terminal = Terminal::new(TestBackend::new(80, 24))?;
        terminal.draw(|frame| render(frame, &app))?;
        let buffer = terminal.backend().buffer();
        assert!(buffer.content.iter().any(|cell| cell.symbol() == "›"));
        assert_eq!(buffer[(1, 1)].fg, Color::Rgb(148, 163, 184));
        app.theme = Theme::Terminal;
        terminal.draw(|frame| render(frame, &app))?;
        assert_eq!(terminal.backend().buffer()[(1, 1)].fg, Color::Reset);
        assert!(
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .filter(|cell| cell.bg == Color::Blue)
                .all(|cell| cell.fg == Color::White)
        );
        Ok(())
    }
}
