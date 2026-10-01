use super::layout::Screen;
use super::*;
use crate::app::{Command, CommandId, Focus, NAVIGATION, Overlay, ReportKind};

pub(crate) struct Control {
    pub(crate) area: Rect,
    pub(crate) command: Command,
    text: String,
}

pub(crate) struct KeyControl {
    pub(crate) area: Rect,
    pub(crate) key: crossterm::event::KeyCode,
    label: &'static str,
}

pub(crate) fn prompt_area(app: &App) -> Rect {
    super::layout::popup(app.viewport_width, app.viewport_height, 100, 9)
}

pub(crate) fn editor_controls(app: &App) -> Vec<KeyControl> {
    use crate::app::PromptKind;
    use crossterm::event::KeyCode;
    let (area, label) = if let Some(prompt) = &app.prompt {
        let popup = prompt_area(app);
        (
            Rect::new(
                popup.x + 2,
                popup.bottom().saturating_sub(2),
                popup.width.saturating_sub(4),
                1,
            ),
            if matches!(prompt.kind, PromptKind::ExportIncident(_, _)) {
                "Enter Export"
            } else {
                "Enter Save"
            },
        )
    } else if app.editing_filter {
        let toolbar = Screen::for_app(app).toolbar;
        (
            Rect::new(
                toolbar.right().saturating_sub(27).max(toolbar.x),
                toolbar.y,
                toolbar.width.min(27),
                1,
            ),
            "Enter Apply",
        )
    } else {
        return Vec::new();
    };
    let mut x = area.x;
    [(KeyCode::Enter, label), (KeyCode::Esc, "Esc Cancel")]
        .into_iter()
        .filter_map(|(key, label)| {
            let width = label.len() as u16 + 2;
            if x + width > area.right() {
                return None;
            }
            let control = KeyControl {
                area: Rect::new(x, area.y, width, 1),
                key,
                label,
            };
            x += width + 1;
            Some(control)
        })
        .collect()
}

pub(super) fn render_editor_controls(frame: &mut Frame, app: &App) {
    render_key_controls(frame, editor_controls(app));
}

fn render_key_controls(frame: &mut Frame, controls: Vec<KeyControl>) {
    for control in controls {
        frame.render_widget(
            Paragraph::new(format!(" {} ", control.label))
                .fg(ACCENT)
                .bg(SELECTED),
            control.area,
        );
    }
}

pub(crate) fn overlay_controls(app: &App, area: Rect) -> Vec<KeyControl> {
    use crossterm::event::KeyCode;
    let mut controls = Vec::new();
    let mut right = area.right().saturating_sub(1);
    for (key, label) in [
        (KeyCode::Esc, "Esc Close"),
        (
            KeyCode::Char('a'),
            if app.help_all {
                "a Context"
            } else {
                "a All shortcuts"
            },
        ),
    ] {
        if key != KeyCode::Esc && !app.help {
            continue;
        }
        let width = label.len() as u16 + 2;
        if right.saturating_sub(area.x) < width + 16 {
            continue;
        }
        right -= width;
        controls.push(KeyControl {
            area: Rect::new(right, area.y, width, 1),
            key,
            label,
        });
        right = right.saturating_sub(1);
    }
    controls
}

// Keep the insertion point visible, using terminal cells rather than byte lengths.
fn input_tail(value: &str, width: usize) -> String {
    let value = format!("{}▏", clean(value));
    let mut start = 0;
    while Span::raw(&value[start..]).width() > width {
        let Some(c) = value[start..].chars().next() else {
            break;
        };
        start += c.len_utf8();
    }
    value[start..].into()
}

pub(super) fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    frame.render_widget(Block::new(), area);
    if area.width < 24 || area.height < 8 {
        frame.render_widget(
            Paragraph::new("pgtrail\nExpand terminal\nCtrl+C: quit").wrap(Wrap { trim: false }),
            area,
        );
        app.theme.apply(frame);
        return;
    }
    let screen = Screen::new(area, app);
    header(frame, screen.header, app);
    if screen.sidebar.width > 0 {
        menu(frame, screen.sidebar, app);
    }
    let title = format!(" {}{}", app.tab.title(), app.breadcrumb());
    let title_area = Rect::new(
        screen.title.x + 10.min(screen.title.width),
        screen.title.y,
        screen.title.width.saturating_sub(10),
        screen.title.height,
    );
    frame.render_widget(
        Paragraph::new(fit_text(&title, title_area.width as usize)).bold(),
        title_area,
    );
    if screen.content.width > 0 {
        match app.tab {
            Tab::Overview => investigation::overview(frame, screen.content, app),
            Tab::Activity => render_activity(frame, screen.content, app),
            Tab::Blocking => render_blocking(frame, screen.content, app),
            Tab::Statements => render_statements(frame, screen.content, app),
            Tab::History => render_history(frame, screen.content, app),
            Tab::Database => investigation::database(frame, screen.content, app),
            Tab::Relations => investigation::relations(frame, screen.content, app),
            Tab::Replication => investigation::replication(frame, screen.content, app),
            Tab::Io => investigation::io(frame, screen.content, app),
            Tab::Incidents => investigation::incidents(frame, screen.content, app),
        }
    }
    if let Some(report) = &app.report {
        render_report(frame, screen.inspector, app, report);
    }
    for control in controls(app, &screen) {
        let disabled = control.command.disabled.is_some();
        let key_style = Style::new()
            .fg(if disabled { MUTED } else { ACCENT })
            .bg(SELECTED)
            .bold();
        let spans = if control.command.key.is_empty() {
            vec![Span::raw(control.text)]
        } else {
            vec![
                Span::styled(format!(" {} ", control.command.key), key_style),
                Span::raw(format!("{} ", control.command.label)),
            ]
        };
        frame.render_widget(
            Paragraph::new(Line::from(spans))
                .fg(if disabled { MUTED } else { Color::Reset })
                .bg(SURFACE),
            control.area,
        );
    }
    if app.editing_filter {
        let width = screen.toolbar.width.saturating_sub(30) as usize;
        frame.render_widget(
            Paragraph::new(format!(" / {}", input_tail(&app.filter, width)))
                .fg(ACCENT)
                .bg(SURFACE),
            screen.toolbar,
        );
        render_editor_controls(frame, app);
    } else if !app.filter.is_empty() {
        let text = format!(" Filter: {} ", clean(&app.filter));
        let width = (Span::raw(&text).width() as u16).min(screen.title.width.saturating_sub(28));
        frame.render_widget(
            Paragraph::new(text).fg(ACCENT),
            Rect::new(
                screen.title.right().saturating_sub(width),
                screen.title.y,
                width,
                1,
            ),
        );
    }
    let (message, color) = if let Some(error) = &app.error {
        (
            format!(" Error: {} · Ctrl+K → Read full status", clean(error)),
            Color::Red,
        )
    } else if let Some(notice) = &app.notice {
        (format!(" {}", clean(notice)), WARNING)
    } else {
        (
            format!(
                " {} · Tab: focus · ↑/↓: {} · read-only",
                match app.focus {
                    Focus::Sidebar => "Navigation",
                    Focus::Content => "Content",
                    Focus::Inspector => "Inspector",
                },
                if app.row_count() > 0 && app.focus != Focus::Inspector {
                    "select"
                } else {
                    "scroll"
                }
            ),
            MUTED,
        )
    };
    frame.render_widget(
        Paragraph::new(fit_text(&message, screen.status.width as usize))
            .fg(color)
            .bg(SURFACE),
        screen.status,
    );
    if matches!(app.overlay, Overlay::Menu) {
        menu(frame, screen.menu(), app);
    }
    if matches!(app.overlay, Overlay::Commands { .. }) {
        palette(frame, app);
    }
    if app.help {
        help(frame, app);
    }
    if app.prompt.is_some() {
        investigation::prompt(frame, app);
    }
    app.theme.apply(frame);
}

fn header(frame: &mut Frame, area: Rect, app: &App) {
    let comparing = app.report.is_some() && app.report_kind == ReportKind::Comparison;
    let synthetic = if comparing {
        app.comparison.as_ref().is_some_and(|c| c.synthetic)
    } else {
        app.snapshot().map_or(app.demo, |s| s.is_synthetic())
    };
    let (mode, color) = if comparing {
        ("OFFLINE COMPARISON", WARNING)
    } else if app.capture.is_some()
        || (app.is_offline() && app.report.is_none() && !app.has_return_path())
    {
        ("OFFLINE CAPTURE", WARNING)
    } else if app.report.is_some() || app.has_return_path() {
        ("FROZEN EVIDENCE", WARNING)
    } else if app.error.is_some() && app.snapshot().is_some() {
        ("STALE · COLLECTION FAILED", Color::Red)
    } else if app.error.is_some() {
        ("DISCONNECTED", Color::Red)
    } else if app.demo {
        ("DEMO · SYNTHETIC DATA", Color::Magenta)
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
        Span::styled(" pgtrail ", Style::new().fg(ACCENT).bold()),
        Span::raw("  "),
        Span::styled(mode, Style::new().fg(color).bold()),
    ];
    if let Some(snapshot) = app.snapshot().filter(|_| !comparing) {
        let age = (app.now - snapshot.completed_at).num_seconds().max(0);
        headline.push(Span::raw(format!("  · Observed {age}s ago")));
    }
    if synthetic && mode != "DEMO · SYNTHETIC DATA" {
        headline.push(Span::styled(
            " · SYNTHETIC DATA",
            Style::new().fg(Color::Magenta),
        ));
    }
    if app.loading {
        headline.push(Span::styled(" · collecting…", Style::new().fg(MUTED)));
    }
    if app.demo && app.paused() && !app.is_offline() {
        headline.push(Span::styled(" · PAUSED", Style::new().fg(WARNING)));
    }
    if app.capture.is_some()
        && !comparing
        && let Some(snapshot) = app.snapshot()
    {
        let used = Line::from(headline.clone()).width() + 3;
        headline.push(Span::styled(
            format!(
                " · {}",
                fit_text(
                    &clean(&snapshot.source.endpoint),
                    usize::from(area.width).saturating_sub(used)
                )
            ),
            Style::new().fg(MUTED),
        ));
    }
    let source = if comparing {
        app.comparison.as_ref().map_or_else(
            || " Saved capture comparison · [ / ] to navigate sections".into(),
            |context| {
                format!(
                    " Before #{} → After #{} · {}",
                    context.before.id,
                    context.after.id,
                    clean(&context.after.source)
                )
            },
        )
    } else if let Some(capture) = &app.capture {
        let database = app.snapshot().map_or("", |s| s.source.database.as_str());
        format!(
            " Capture #{} · {} · {} · {} UTC",
            capture.id,
            clean(&capture.label),
            clean(database),
            capture.captured_at.format("%Y-%m-%d %H:%M:%S")
        )
    } else if let Some(snapshot) = app.snapshot() {
        let available = usize::from(area.width.saturating_sub(32));
        format!(
            " {} / {} · PG {}{}",
            fit_text(&clean(&snapshot.source.endpoint), available / 2),
            fit_text(&clean(&snapshot.source.database), available / 2),
            clean(&snapshot.source.server_version),
            app.active_incident
                .map_or_else(String::new, |id| format!(" · incident #{id}"))
        )
    } else {
        " Read-only PostgreSQL investigation · 5 Captures · 0 Incidents".into()
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(headline),
            Line::raw(fit_text(&source, area.width as usize)).fg(MUTED),
        ])
        .bg(SURFACE),
        area,
    );
}

pub(crate) fn menu_items(app: &App, area: Rect) -> Vec<(Rect, Tab, &'static str)> {
    let mut y = area.y + 1;
    let mut items = Vec::new();
    let full = area.height >= 17;
    for (tab, group) in NAVIGATION {
        if full && !group.is_empty() {
            y += 1;
        }
        items.push((
            Rect::new(area.x + 1, y, area.width.saturating_sub(2), 1),
            tab,
            if full { group } else { "" },
        ));
        y += 1;
        if full && matches!(tab, Tab::Statements | Tab::Io) {
            y += 1;
        }
    }
    let cursor = if app.focus == Focus::Sidebar || matches!(app.overlay, Overlay::Menu) {
        app.menu_cursor
    } else {
        NAVIGATION
            .iter()
            .position(|(t, _)| *t == app.tab)
            .unwrap_or(0)
    };
    let overflow = items.get(cursor).map_or(0, |(rect, _, _)| {
        rect.y.saturating_sub(area.bottom().saturating_sub(2))
    });
    for (rect, _, _) in &mut items {
        rect.y = rect.y.saturating_sub(overflow);
    }
    items
        .into_iter()
        .filter(|(rect, _, _)| rect.y > area.y && rect.y < area.bottom().saturating_sub(1))
        .collect()
}

fn menu(frame: &mut Frame, area: Rect, app: &App) {
    frame.render_widget(Clear, area);
    let focused = app.focus == Focus::Sidebar || matches!(app.overlay, Overlay::Menu);
    frame.render_widget(
        panel(" Navigate ")
            .border_style(Style::new().fg(if focused { ACCENT } else { BORDER }))
            .bg(SURFACE),
        area,
    );
    for (rect, tab, group) in menu_items(app, area) {
        if !group.is_empty() && rect.y > area.y + 1 {
            frame.render_widget(
                Paragraph::new(group).fg(MUTED),
                Rect::new(rect.x, rect.y - 1, rect.width, 1),
            );
        }
        let cursor = focused && NAVIGATION[app.menu_cursor].0 == tab;
        let active = tab == app.tab;
        let text = format!(
            "{} {} {}",
            if cursor {
                "›"
            } else if active {
                "•"
            } else {
                " "
            },
            tab.shortcut(),
            tab.title()
        );
        let style = if active || cursor {
            Style::new().bg(SELECTED).fg(ACCENT).bold()
        } else {
            Style::new()
        };
        frame.render_widget(Paragraph::new(text).style(style), rect);
    }
}

pub(crate) fn controls(app: &App, screen: &Screen) -> Vec<Control> {
    use CommandId::*;
    let commands = app.commands();
    let mut controls = Vec::new();
    let mut add = |area: Rect, ids: Vec<CommandId>| {
        let mut x = area.x;
        for id in ids {
            let Some(command) = commands.iter().find(|c| c.id == id) else {
                continue;
            };
            let text = format!(
                " {}{}{} ",
                command.key,
                if command.key.is_empty() { "" } else { " " },
                command.label
            );
            let width = Span::raw(&text).width() as u16;
            if x + width > area.right() {
                continue;
            }
            controls.push(Control {
                area: Rect::new(x, area.y, width, 1),
                command: command.clone(),
                text,
            });
            x += width + 1;
        }
    };
    add(
        Rect::new(
            screen.title.x,
            screen.title.y,
            10.min(screen.title.width),
            1,
        ),
        vec![Menu],
    );
    if !app.editing_filter {
        let ids = if app.report.is_some() {
            vec![
                Close,
                PreviousSection,
                NextSection,
                Evidence,
                Blocker,
                Waiter,
            ]
        } else {
            match app.tab {
                Tab::Overview => vec![Inspect, Evidence, Coverage, Filter],
                Tab::Activity | Tab::Blocking => vec![Inspect, Blocker, Waiter, Filter],
                Tab::Statements | Tab::Relations => vec![Inspect, Mode, Sort, Filter],
                Tab::History => vec![Inspect, Before, After, Compare, Filter],
                Tab::Incidents => {
                    vec![Inspect, NewIncident, Note, ExportMarkdown, Timeline, Filter]
                }
                _ => vec![PreviousSection, NextSection, Refresh, Pause, Coverage],
            }
        };
        add(screen.toolbar, ids);
    }
    add(
        screen.footer,
        if app.is_offline() {
            vec![Commands, Back, ReturnLive, Help, Quit]
        } else {
            vec![Commands, Capture, Help, Quit]
        },
    );
    controls
}

pub(crate) fn palette_area(app: &App) -> Rect {
    super::layout::popup(app.viewport_width, app.viewport_height, 84, 24)
}

fn palette(frame: &mut Frame, app: &App) {
    let Overlay::Commands { query, selected } = &app.overlay else {
        return;
    };
    let area = palette_area(app);
    frame.render_widget(Clear, area);
    frame.render_widget(
        panel(" Commands ")
            .border_style(Style::new().fg(ACCENT))
            .bg(SURFACE),
        area,
    );
    let inner = area.inner(ratatui::layout::Margin::new(1, 1));
    frame.render_widget(
        Paragraph::new(format!(
            " / {}",
            input_tail(query, inner.width.saturating_sub(3) as usize)
        ))
        .fg(ACCENT),
        Rect::new(inner.x, inner.y, inner.width, 1),
    );
    let commands = app.palette_commands();
    let visible = usize::from(area.height.saturating_sub(4)).max(1);
    let offset = selected.saturating_sub(visible - 1);
    for (index, command) in commands.iter().enumerate().skip(offset).take(visible) {
        let label = fit_text(&command.label, inner.width.saturating_sub(15) as usize);
        let line = Line::from(vec![
            Span::raw(format!(
                "{} {:11} ",
                if index == *selected { "›" } else { " " },
                command.key
            )),
            Span::raw(label),
        ]);
        frame.render_widget(
            Paragraph::new(line).style(
                Style::new()
                    .fg(if command.disabled.is_some() {
                        MUTED
                    } else {
                        Color::Reset
                    })
                    .bg(if index == *selected {
                        SELECTED
                    } else {
                        SURFACE
                    }),
            ),
            Rect::new(
                inner.x,
                inner.y + 1 + (index - offset) as u16,
                inner.width,
                1,
            ),
        );
    }
    let hint = if commands.is_empty() {
        "No matching commands. Change the search."
    } else {
        commands
            .get(*selected)
            .and_then(|c| c.disabled)
            .unwrap_or("↑/↓ Select · Enter Run · Esc Close")
    };
    frame.render_widget(
        Paragraph::new(fit_text(hint, inner.width as usize)).fg(MUTED),
        Rect::new(inner.x, area.bottom().saturating_sub(2), inner.width, 1),
    );
    render_key_controls(frame, overlay_controls(app, area));
}

pub(crate) fn help_rows(app: &App, width: u16) -> Vec<String> {
    if app.help_all {
        return help_lines(width);
    }
    let mut text = format!(
        "# {}\n\nTab / Shift+Tab   Move between visible zones\nF2               Open navigation\nCtrl+K           Search all actions and views\n1–9,0            Open a view directly\n↑/↓ or j/k       Select or scroll the focused zone\nMouse            Click to select; wheel scrolls the pointed panel\n\n# Actions for this view\n",
        app.tab.title()
    );
    for command in app
        .commands()
        .iter()
        .filter(|c| !matches!(c.id, CommandId::View(_)) && !c.key.is_empty())
    {
        text.push_str(&format!(
            "{:<12} {}{}\n",
            command.key,
            command.label,
            command
                .disabled
                .map_or_else(String::new, |reason| format!(" — {reason}"))
        ));
    }
    text.push_str("\n# More help\na                Show all keyboard shortcuts\nEsc / ? / Enter  Close help\nq / Ctrl+C       Quit and restore the terminal");
    report_lines(&text, width)
}

fn help(frame: &mut Frame, app: &App) {
    let area = super::layout::popup(app.viewport_width, app.viewport_height, 90, 31);
    let rows = help_rows(app, area.width.saturating_sub(2));
    let visible = usize::from(area.height.saturating_sub(2));
    let offset = app.help_scroll.min(rows.len().saturating_sub(visible));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(
            rows.into_iter()
                .skip(offset)
                .take(visible)
                .map(report_line)
                .collect::<Vec<_>>(),
        )
        .block(
            panel(if app.help_all {
                " Keyboard help "
            } else {
                " Context help "
            })
            .border_style(Style::new().fg(ACCENT)),
        )
        .bg(SURFACE),
        area,
    );
    render_key_controls(frame, overlay_controls(app, area));
}
