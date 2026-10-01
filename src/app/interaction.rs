//! Keyboard, pointer and command palette share the same action dispatch.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Position;

use super::{Action, App, Tab};
use crate::ui::{self, layout::Screen};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Focus {
    Sidebar,
    #[default]
    Content,
    Inspector,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ReportKind {
    #[default]
    Comparison,
    Selection,
    Coverage,
    Note,
    Status,
}

#[derive(Debug, Default)]
pub(crate) enum Overlay {
    #[default]
    None,
    Menu,
    Commands {
        query: String,
        selected: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommandId {
    View(Tab),
    Menu,
    Commands,
    Inspect,
    Back,
    ReturnLive,
    Filter,
    Refresh,
    Pause,
    Capture,
    Coverage,
    Evidence,
    Blocker,
    Waiter,
    Mode,
    Sort,
    Before,
    After,
    Compare,
    Label,
    Attach,
    NewIncident,
    Note,
    ExportMarkdown,
    ExportJson,
    ToggleIncident,
    ClearIncident,
    Timeline,
    Help,
    Quit,
    Theme,
    Status,
    Close,
    PreviousSection,
    NextSection,
}

#[derive(Debug, Clone)]
pub(crate) struct Command {
    pub(crate) id: CommandId,
    pub(crate) label: String,
    pub(crate) key: &'static str,
    pub(crate) disabled: Option<&'static str>,
}

impl Command {
    fn new(
        id: CommandId,
        label: impl Into<String>,
        key: &'static str,
        disabled: Option<&'static str>,
    ) -> Self {
        Self {
            id,
            label: label.into(),
            key,
            disabled,
        }
    }
}

pub(crate) const NAVIGATION: [(Tab, &str); 10] = [
    (Tab::Overview, "Investigate"),
    (Tab::Activity, ""),
    (Tab::Blocking, ""),
    (Tab::Statements, ""),
    (Tab::Database, "Monitor"),
    (Tab::Relations, ""),
    (Tab::Replication, ""),
    (Tab::Io, ""),
    (Tab::History, "Workspace"),
    (Tab::Incidents, ""),
];

impl App {
    pub(crate) fn row_selection(&self) -> usize {
        if self.tab == Tab::Incidents && self.incident_timeline {
            self.incident_cursor
        } else {
            self.selected()
        }
    }

    pub(crate) fn row_count(&self) -> usize {
        if (self.tab == Tab::History && self.history_error.is_some())
            || (self.tab == Tab::Incidents
                && !self.incident_timeline
                && self.incidents_error.is_some())
        {
            return 0;
        }
        match self.tab {
            Tab::Overview => self.filtered_findings().len(),
            Tab::Activity => self.filtered_sessions().len(),
            Tab::Blocking => self.blocking_edges().len(),
            Tab::Statements if self.statement_interval => self.filtered_statement_rates().len(),
            Tab::Statements => self.filtered_statements().len(),
            Tab::History => self.filtered_history().len(),
            Tab::Relations if self.show_indexes => self.filtered_indexes().len(),
            Tab::Relations => self.filtered_tables().len(),
            Tab::Incidents if self.incident_timeline => self
                .incident
                .as_ref()
                .map_or(0, |i| crate::incidents::timeline(i).len()),
            Tab::Incidents => self.filtered_incidents().len(),
            _ => 0,
        }
    }

    pub(crate) fn commands(&self) -> Vec<Command> {
        use CommandId::*;
        let missing = (self.row_count() == 0).then_some("Select an available row first");
        let report = self.report.is_some().then_some("Close the inspector first");
        let live = (self.is_offline() || !self.connection_configured)
            .then_some("Requires a live connection or demo observation");
        let incident = self
            .active_incident
            .is_none()
            .then_some("Create or activate an open incident first");
        let mut commands = Vec::new();
        if self.report.is_some() {
            commands.push(Command::new(Close, "Close", "Esc", None));
        }
        if self.report.is_some() || matches!(self.tab, Tab::Database | Tab::Replication | Tab::Io) {
            commands.extend([
                Command::new(PreviousSection, "Previous section", "[", None),
                Command::new(NextSection, "Next section", "]", None),
            ]);
        }
        if !matches!(self.tab, Tab::Database | Tab::Replication | Tab::Io) {
            commands.push(Command::new(
                Inspect,
                if self.tab == Tab::History {
                    "Open capture"
                } else {
                    "Inspect"
                },
                "Enter",
                if self.focus == Focus::Content && self.report_kind == ReportKind::Selection {
                    missing
                } else {
                    report.or(missing)
                },
            ));
        }
        if self.tab == Tab::History {
            commands.extend([
                Command::new(Before, "Mark before", "a", report.or(missing)),
                Command::new(After, "Mark after", "b", report.or(missing)),
                Command::new(
                    Compare,
                    "Compare",
                    "d",
                    report.or((self.compare_a.is_none() || self.compare_b.is_none())
                        .then_some("Mark a before and an after capture first")),
                ),
                Command::new(Label, "Edit capture label", "l", report.or(missing)),
                Command::new(
                    Attach,
                    "Attach to incident",
                    "I",
                    report.or(missing).or(incident),
                ),
            ]);
        }
        if self.tab == Tab::Overview {
            commands.push(Command::new(Evidence, "Follow evidence", "e", missing));
        }
        if matches!(self.tab, Tab::Activity | Tab::Blocking) {
            commands.extend([
                Command::new(Blocker, "Follow blocker", "b", missing),
                Command::new(Waiter, "Follow waiter", "w", missing),
            ]);
        }
        if matches!(self.tab, Tab::Statements | Tab::Relations) {
            let mode = match (self.tab, self.statement_interval, self.show_indexes) {
                (Tab::Statements, true, _) => "Mode: interval",
                (Tab::Statements, false, _) => "Mode: cumulative",
                (_, _, true) => "Mode: indexes",
                _ => "Mode: tables",
            };
            commands.push(Command::new(Mode, mode, "v", report));
            let sort = if self.tab == Tab::Statements {
                self.sort.title()
            } else {
                ["size", "maintenance / validity", "scans"][self.relation_sort.min(2)]
            };
            commands.push(Command::new(Sort, format!("Sort: {sort}"), "s", report));
        }
        if self.tab == Tab::Incidents {
            let selected = if self.incident_timeline {
                self.incident.as_ref().map(|i| &i.summary)
            } else if self.incidents_error.is_some() {
                None
            } else {
                self.filtered_incidents().get(self.selected()).copied()
            };
            let no_incident = selected.is_none().then_some("Select an incident first");
            commands.extend([
                Command::new(
                    Timeline,
                    if self.incident_timeline {
                        "Incident list"
                    } else {
                        "Chronology"
                    },
                    "t",
                    self.incident.is_none().then_some("Open an incident first"),
                ),
                Command::new(ExportMarkdown, "Export Markdown", "e", no_incident),
                Command::new(ExportJson, "Export JSON", "E", no_incident),
                Command::new(
                    ToggleIncident,
                    if selected.is_some_and(|i| i.closed_at.is_some()) {
                        "Reopen incident"
                    } else {
                        "Close incident"
                    },
                    "o",
                    no_incident,
                ),
                Command::new(ClearIncident, "Clear active incident", "x", incident),
            ]);
        }
        if self.filter_available() {
            commands.push(Command::new(Filter, "Filter", "/", report));
        }
        commands.extend([
            Command::new(Capture, "Save capture", "c", live),
            Command::new(
                Refresh,
                if matches!(self.tab, Tab::History | Tab::Incidents) {
                    "Reload list"
                } else {
                    "Refresh"
                },
                "r",
                if matches!(self.tab, Tab::History | Tab::Incidents) {
                    None
                } else {
                    live
                },
            ),
            Command::new(
                Pause,
                if self.paused {
                    "Resume collection"
                } else {
                    "Pause collection"
                },
                "p",
                live,
            ),
            Command::new(
                Coverage,
                "Coverage and limits",
                "h",
                self.analysis
                    .is_none()
                    .then_some("No observation is available"),
            ),
            Command::new(NewIncident, "Create incident", "i", None),
            Command::new(Note, "Add incident note", "n", incident),
            Command::new(
                Back,
                "Back to origin",
                "Backspace",
                (!self.has_return_path() && self.report.is_none())
                    .then_some("No previous evidence"),
            ),
            Command::new(
                ReturnLive,
                "Return to live",
                "",
                (!self.is_offline()).then_some("Already in live mode"),
            ),
            Command::new(
                Status,
                "Read full status",
                "",
                (self.error.is_none() && self.notice.is_none()).then_some("No status message"),
            ),
            Command::new(Theme, "Switch dark / terminal theme", "", None),
            Command::new(Menu, "Menu", "F2", None),
            Command::new(Commands, "Commands", "Ctrl+K", None),
            Command::new(Help, "Help", "?", None),
            Command::new(Quit, "Quit", "q", None),
        ]);
        for (tab, _) in NAVIGATION {
            commands.push(Command::new(
                View(tab),
                format!("Go to {}", tab.title()),
                tab.shortcut(),
                None,
            ));
        }
        commands
    }

    pub(crate) fn palette_commands(&self) -> Vec<Command> {
        let query = match &self.overlay {
            Overlay::Commands { query, .. } => query.to_lowercase(),
            _ => String::new(),
        };
        self.commands()
            .into_iter()
            .filter(|c| {
                c.id != CommandId::Commands
                    && query
                        .split_whitespace()
                        .all(|word| c.label.to_lowercase().contains(word))
            })
            .collect()
    }

    pub(crate) fn execute(&mut self, command: CommandId) -> Option<Action> {
        use CommandId::*;
        if let Some(reason) = self
            .commands()
            .iter()
            .find(|c| c.id == command)
            .and_then(|c| c.disabled)
        {
            self.set_notice(reason.into());
            return None;
        }
        let key = match command {
            View(tab) => {
                self.overlay = Overlay::None;
                self.focus = Focus::Content;
                return self.switch_tab(tab.index());
            }
            Menu => {
                self.toggle_menu();
                return None;
            }
            Commands => {
                self.overlay = Overlay::Commands {
                    query: String::new(),
                    selected: 0,
                };
                return None;
            }
            Theme => {
                self.theme = if self.theme == ui::Theme::Dark {
                    ui::Theme::Terminal
                } else {
                    ui::Theme::Dark
                };
                return None;
            }
            ReturnLive => {
                self.navigation.clear();
                self.report = None;
                self.offline = false;
                self.capture = None;
                self.snapshot = None;
                self.analysis = None;
                self.focus = Focus::Content;
                self.overlay = Overlay::None;
                return Some(Action::ResumeLive);
            }
            Status => {
                let message = [self.error.as_deref(), self.notice.as_deref()]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join("\n\n");
                self.open_report(ReportKind::Status, "Status details", message);
                return None;
            }
            Inspect if self.report.is_some() && self.report_kind == ReportKind::Selection => {
                self.focus = Focus::Inspector;
                return None;
            }
            Inspect => KeyCode::Enter,
            Close => KeyCode::Esc,
            PreviousSection => KeyCode::Char('['),
            NextSection => KeyCode::Char(']'),
            Back if self.report.is_some() && !self.has_return_path() => KeyCode::Esc,
            Back => KeyCode::Backspace,
            Filter => KeyCode::Char('/'),
            Refresh => KeyCode::Char('r'),
            Pause => KeyCode::Char('p'),
            Capture => KeyCode::Char('c'),
            Coverage => KeyCode::Char('h'),
            Evidence => KeyCode::Char('e'),
            Blocker => KeyCode::Char('b'),
            Waiter => KeyCode::Char('w'),
            Mode => KeyCode::Char('v'),
            Sort => KeyCode::Char('s'),
            Before => KeyCode::Char('a'),
            After => KeyCode::Char('b'),
            Compare => KeyCode::Char('d'),
            Label => KeyCode::Char('l'),
            Attach => KeyCode::Char('I'),
            NewIncident => KeyCode::Char('i'),
            Note => KeyCode::Char('n'),
            ExportMarkdown => KeyCode::Char('e'),
            ExportJson => KeyCode::Char('E'),
            ToggleIncident => KeyCode::Char('o'),
            ClearIncident => KeyCode::Char('x'),
            Timeline => KeyCode::Char('t'),
            Help => KeyCode::Char('?'),
            Quit => KeyCode::Char('q'),
        };
        self.domain_key(KeyEvent::new(key, KeyModifiers::NONE))
    }

    fn toggle_menu(&mut self) {
        self.menu_cursor = NAVIGATION
            .iter()
            .position(|(t, _)| *t == self.tab)
            .unwrap_or(0);
        if self.viewport_width < 100 {
            self.overlay = if matches!(self.overlay, Overlay::Menu) {
                Overlay::None
            } else {
                Overlay::Menu
            };
        } else {
            self.focus = if self.focus == Focus::Sidebar {
                self.content_focus()
            } else {
                Focus::Sidebar
            };
        }
    }

    fn content_focus(&self) -> Focus {
        if self.report.is_some() {
            Focus::Inspector
        } else {
            Focus::Content
        }
    }

    fn cycle_focus(&mut self, backwards: bool) {
        let screen = Screen::for_app(self);
        let mut zones = Vec::new();
        if screen.sidebar.width > 0 {
            zones.push(Focus::Sidebar);
        }
        if screen.content.width > 0 {
            zones.push(Focus::Content);
        }
        if screen.inspector.width > 0 {
            zones.push(Focus::Inspector);
        }
        if zones.is_empty() {
            return;
        }
        let index = zones.iter().position(|f| *f == self.focus).unwrap_or(0);
        self.focus = zones[(index + if backwards { zones.len() - 1 } else { 1 }) % zones.len()];
        if self.focus == Focus::Sidebar {
            self.menu_cursor = NAVIGATION
                .iter()
                .position(|(t, _)| *t == self.tab)
                .unwrap_or(0);
        }
    }

    pub(super) fn key(&mut self, key: KeyEvent) -> Option<Action> {
        if self.prompt.is_some() || self.editing_filter {
            return self.domain_key(key);
        }
        if self.help {
            if key.code == KeyCode::Char('a') {
                self.help_all = !self.help_all;
                self.help_scroll = 0;
                return None;
            }
            return self.domain_key(key);
        }
        if matches!(self.overlay, Overlay::Commands { .. }) {
            return self.palette_key(key);
        }
        if key.code == KeyCode::Char('k') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return self.execute(CommandId::Commands);
        }
        if key.code == KeyCode::F(2) {
            return self.execute(CommandId::Menu);
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
        {
            return None;
        }
        if matches!(self.overlay, Overlay::Menu) || self.focus == Focus::Sidebar {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.menu_cursor = self.menu_cursor.saturating_sub(1)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.menu_cursor = (self.menu_cursor + 1).min(9)
                }
                KeyCode::Home | KeyCode::Char('g') => self.menu_cursor = 0,
                KeyCode::End | KeyCode::Char('G') => self.menu_cursor = 9,
                KeyCode::Enter => {
                    return self.execute(CommandId::View(NAVIGATION[self.menu_cursor].0));
                }
                KeyCode::Esc => {
                    self.overlay = Overlay::None;
                    self.focus = self.content_focus();
                }
                KeyCode::Tab | KeyCode::BackTab => {
                    self.overlay = Overlay::None;
                    self.cycle_focus(key.code == KeyCode::BackTab);
                }
                KeyCode::Char('0'..='9' | 'q' | '?') => {}
                _ => return None,
            }
            if !matches!(key.code, KeyCode::Char('0'..='9' | 'q' | '?')) {
                return None;
            }
        }
        if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
            self.cycle_focus(key.code == KeyCode::BackTab);
            return None;
        }
        let name = match key.code {
            KeyCode::Enter => "Enter".to_owned(),
            KeyCode::Backspace => "Backspace".to_owned(),
            KeyCode::Char(c) => c.to_string(),
            _ => String::new(),
        };
        if !name.is_empty()
            && let Some(command) = self.commands().iter().find(|c| c.key == name)
        {
            return self.execute(command.id);
        }
        // Horizontal arrows are reserved for local controls, not global navigation.
        if matches!(key.code, KeyCode::Left | KeyCode::Right) {
            return None;
        }
        let action = self.domain_key(key);
        if self.focus == Focus::Content
            && self.report.is_some()
            && self.report_kind == ReportKind::Selection
        {
            self.refresh_inspector();
        }
        action
    }

    fn palette_key(&mut self, key: KeyEvent) -> Option<Action> {
        let commands = self.palette_commands();
        let page = usize::from(ui::palette_area(self).height.saturating_sub(4)).max(1);
        let Overlay::Commands { query, selected } = &mut self.overlay else {
            return None;
        };
        match key.code {
            KeyCode::Esc => self.overlay = Overlay::None,
            KeyCode::Down => *selected = (*selected + 1).min(commands.len().saturating_sub(1)),
            KeyCode::Up => *selected = selected.saturating_sub(1),
            KeyCode::PageDown => {
                *selected = (*selected + page).min(commands.len().saturating_sub(1))
            }
            KeyCode::PageUp => *selected = selected.saturating_sub(page),
            KeyCode::Home => *selected = 0,
            KeyCode::End => *selected = commands.len().saturating_sub(1),
            KeyCode::Enter => {
                if let Some(command) = commands.get(*selected) {
                    if command.disabled.is_some() {
                        self.set_notice(command.disabled.unwrap_or_default().into());
                    } else {
                        let id = command.id;
                        self.overlay = Overlay::None;
                        return self.execute(id);
                    }
                }
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                query.clear();
                *selected = 0;
            }
            KeyCode::Backspace => {
                query.pop();
                *selected = 0;
            }
            KeyCode::Char(c)
                if !c.is_control()
                    && !key.modifiers.intersects(
                        KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                    ) =>
            {
                query.push(c);
                *selected = 0;
            }
            _ => {}
        }
        None
    }

    pub(crate) fn open_report(&mut self, kind: ReportKind, title: &'static str, report: String) {
        self.report = Some(report);
        self.report_kind = kind;
        self.report_title = title;
        self.report_scroll = 0;
        self.focus = Focus::Inspector;
        self.loading = false;
    }

    fn refresh_inspector(&mut self) {
        let before = self.report.take();
        self.domain_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        if self.report.is_none() {
            self.report = before;
        }
        self.focus = Focus::Content;
    }

    pub(super) fn mouse(&mut self, event: MouseEvent) -> Option<Action> {
        if !self.mouse_enabled || self.viewport_width < 24 || self.viewport_height < 8 {
            return None;
        }
        let point = Position::new(event.column, event.row);
        let click = event.kind == MouseEventKind::Down(MouseButton::Left);
        let movement = match event.kind {
            MouseEventKind::ScrollDown => 3,
            MouseEventKind::ScrollUp => -3,
            _ => 0,
        };
        if self.prompt.is_some() || self.editing_filter {
            if click
                && let Some(control) = ui::editor_controls(self)
                    .iter()
                    .find(|c| c.area.contains(point))
            {
                return self.domain_key(KeyEvent::new(control.key, KeyModifiers::NONE));
            }
            return None;
        }
        if self.help {
            let area = ui::layout::popup(self.viewport_width, self.viewport_height, 90, 31);
            if area.contains(point) {
                if click
                    && let Some(control) = ui::overlay_controls(self, area)
                        .iter()
                        .find(|c| c.area.contains(point))
                {
                    return self.key(KeyEvent::new(control.key, KeyModifiers::NONE));
                }
                if movement != 0 {
                    self.help_scroll = self.help_scroll.saturating_add_signed(movement);
                    self.clamp_scroll();
                }
            } else if click {
                self.help = false;
            }
            return None;
        }
        if let Overlay::Commands { selected, .. } = self.overlay {
            let area = ui::palette_area(self);
            if click
                && ui::overlay_controls(self, area)
                    .iter()
                    .any(|c| c.area.contains(point))
            {
                self.overlay = Overlay::None;
                return None;
            }
            let visible = usize::from(area.height.saturating_sub(4)).max(1);
            let offset = selected.saturating_sub(visible - 1);
            if !area.contains(point) {
                if click {
                    self.overlay = Overlay::None;
                }
                return None;
            }
            if movement != 0 {
                let count = self.palette_commands().len();
                if let Overlay::Commands { selected, .. } = &mut self.overlay {
                    *selected = selected
                        .saturating_add_signed(movement)
                        .min(count.saturating_sub(1));
                }
            } else if click && point.y >= area.y + 2 && point.y < area.bottom().saturating_sub(2) {
                let index = offset + usize::from(point.y - area.y - 2);
                if let Some(command) = self.palette_commands().get(index) {
                    if command.disabled.is_none() {
                        let id = command.id;
                        self.overlay = Overlay::None;
                        return self.execute(id);
                    }
                    if let Overlay::Commands { selected, .. } = &mut self.overlay {
                        *selected = index;
                    }
                }
            }
            return None;
        }
        let screen = Screen::for_app(self);
        let menu = if matches!(self.overlay, Overlay::Menu) {
            screen.menu()
        } else {
            screen.sidebar
        };
        if menu.contains(point) {
            if movement != 0 {
                self.menu_cursor = self.menu_cursor.saturating_add_signed(movement).min(9);
                self.focus = Focus::Sidebar;
            }
            if click
                && let Some((_, tab, _)) = ui::menu_items(self, menu)
                    .into_iter()
                    .find(|(rect, _, _)| rect.contains(point))
            {
                return self.execute(CommandId::View(tab));
            }
            return None;
        }
        if matches!(self.overlay, Overlay::Menu) {
            if click {
                self.overlay = Overlay::None;
            }
            return None;
        }
        if click {
            if screen.status.contains(point) && (self.error.is_some() || self.notice.is_some()) {
                return self.execute(CommandId::Status);
            }
            for control in ui::controls(self, &screen) {
                if control.area.contains(point) {
                    return self.execute(control.command.id);
                }
            }
        }
        if screen.inspector.contains(point) {
            self.focus = Focus::Inspector;
            if movement != 0 {
                self.move_selection(movement);
            }
        } else if screen.content.contains(point) {
            self.focus = Focus::Content;
            if movement != 0 {
                self.move_selection(movement);
            }
            if click && self.row_count() > 0 {
                let table = screen.table(self);
                let (offset, _) = ui::layout::table_window(self, table);
                if point.y >= table.y + 2 && point.y < table.bottom().saturating_sub(1) {
                    let row = offset + usize::from(point.y - table.y - 2);
                    if row < self.row_count() {
                        if self.tab == Tab::Incidents && self.incident_timeline {
                            self.incident_cursor = row;
                        } else {
                            self.selected[self.tab.index()] = row;
                        }
                    }
                }
            }
            if (click || movement != 0)
                && self.report.is_some()
                && self.report_kind == ReportKind::Selection
            {
                self.refresh_inspector();
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Message;

    fn app(width: u16) -> App {
        let mut app = App::new(true);
        app.viewport_width = width;
        app.viewport_height = 36;
        app.set_snapshot(crate::demo::snapshot(0, true), false);
        app
    }
    fn key(app: &mut App, code: KeyCode) -> Option<Action> {
        app.update(Message::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }
    fn click(app: &mut App, x: u16, y: u16) -> Option<Action> {
        app.update(Message::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        }))
    }

    #[test]
    fn focus_cycle_inspection_and_compact_navigation_keep_the_view_explicit() {
        let mut app = app(160);
        key(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Focus::Sidebar);
        key(&mut app, KeyCode::Down);
        assert_eq!(
            app.tab,
            Tab::Overview,
            "moving the menu cursor must not change the view"
        );
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.tab, Tab::Activity);
        assert_eq!(app.focus, Focus::Content);
        key(&mut app, KeyCode::Enter);
        let observed = app.snapshot().unwrap().completed_at;
        assert_eq!(app.focus, Focus::Inspector);
        assert!(app.is_offline());
        key(&mut app, KeyCode::Tab);
        key(&mut app, KeyCode::Tab);
        assert_eq!(app.focus, Focus::Content);
        let before = app.report.clone();
        key(&mut app, KeyCode::Down);
        assert_ne!(
            app.report, before,
            "the inspector follows the selected frozen row"
        );
        assert_eq!(app.snapshot().unwrap().completed_at, observed);
        app.viewport_width = 80;
        app.clamp_scroll();
        assert_eq!(app.focus, Focus::Inspector);
        key(&mut app, KeyCode::Esc);
        key(&mut app, KeyCode::F(2));
        assert!(matches!(app.overlay, Overlay::Menu));
        key(&mut app, KeyCode::End);
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.tab, Tab::Incidents);
        assert!(matches!(app.overlay, Overlay::None));
        key(&mut app, KeyCode::F(2));
        app.viewport_width = 100;
        app.clamp_scroll();
        assert!(
            matches!(app.overlay, Overlay::None),
            "the permanent sidebar must stop intercepting content clicks as an overlay"
        );
        assert_eq!(app.focus, Focus::Sidebar);
    }

    #[test]
    fn command_search_consumes_text_and_explains_unavailable_actions() {
        let mut app = app(80);
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Char('k'),
            KeyModifiers::CONTROL,
        )));
        for character in "go to ioq".chars() {
            key(&mut app, KeyCode::Char(character));
        }
        assert!(!app.should_quit());
        assert_eq!(app.tab, Tab::Overview);
        assert!(app.palette_commands().is_empty());
        app.overlay = Overlay::Commands {
            query: "go to activity".into(),
            selected: 0,
        };
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.tab, Tab::Activity);
        key(&mut app, KeyCode::Enter);
        app.overlay = Overlay::Commands {
            query: "save capture".into(),
            selected: 0,
        };
        assert!(app.palette_commands()[0].disabled.is_some());
        assert_eq!(key(&mut app, KeyCode::Enter), None);
        assert!(matches!(app.overlay, Overlay::Commands { .. }));
        assert!(app.report.is_some());
    }

    #[test]
    fn pointer_selects_the_displayed_row_after_scroll_and_resize() {
        let mut app = app(160);
        app.tab = Tab::History;
        app.history = (1..=80)
            .map(|id| crate::store::SnapshotSummary {
                id,
                label: format!("Capture {id}"),
                captured_at: app.now,
                source: "synthetic".into(),
                complete: true,
            })
            .collect();
        key(&mut app, KeyCode::End);
        app.clamp_scroll();
        for width in [160, 80, 120] {
            app.viewport_width = width;
            app.viewport_height = 24;
            app.clamp_scroll();
            let screen = Screen::for_app(&app);
            let table = screen.table(&app);
            let offset = ui::layout::table_window(&app, table).0;
            click(&mut app, table.x + 3, table.y + 3);
            assert_eq!(app.selected(), offset + 1);
            assert_eq!(
                app.filtered_history()[app.selected()].id,
                (offset + 2) as i64
            );
            app.clamp_scroll();
            assert_eq!(
                app.table_offsets[Tab::History.index()],
                offset,
                "clicking a visible row must not jump the viewport"
            );
        }
        let selected = app.selected();
        app.mouse_enabled = false;
        let table = Screen::for_app(&app).table(&app);
        click(&mut app, table.x + 3, table.y + 2);
        assert_eq!(app.selected(), selected);
    }

    #[test]
    fn palette_and_menu_intercept_pointer_input_and_share_actions() {
        let mut app = app(120);
        let expected = key(&mut app, KeyCode::Char('c'));
        let screen = Screen::for_app(&app);
        let button = ui::controls(&app, &screen)
            .into_iter()
            .find(|c| c.command.id == CommandId::Capture)
            .unwrap();
        assert_eq!(click(&mut app, button.area.x, button.area.y), expected);
        app.execute(CommandId::Commands);
        let area = ui::palette_area(&app);
        app.overlay = Overlay::Commands {
            query: "go to relations".into(),
            selected: 0,
        };
        click(&mut app, area.x + 2, area.y + 2);
        assert_eq!(app.tab, Tab::Relations);
        app.viewport_width = 80;
        app.execute(CommandId::Menu);
        let tab_before = app.tab;
        click(&mut app, 70, 10);
        assert_eq!(app.tab, tab_before);
        assert!(matches!(app.overlay, Overlay::None));
    }

    #[test]
    fn navigation_from_an_inspector_restores_offsets_and_selection() {
        let mut app = app(160);
        app.filter = "blocking".into();
        key(&mut app, KeyCode::Enter);
        let report = app.report.clone();
        key(&mut app, KeyCode::PageDown);
        let scroll = app.report_scroll;
        key(&mut app, KeyCode::Char('e'));
        assert!(app.has_return_path());
        assert_eq!(app.focus, Focus::Content);
        key(&mut app, KeyCode::Backspace);
        assert_eq!(app.tab, Tab::Overview);
        assert_eq!(app.report, report);
        assert_eq!(app.report_scroll, scroll);
        assert_eq!(app.focus, Focus::Inspector);
    }

    #[test]
    fn report_only_views_scroll_display_rows_and_jump_to_real_sections() {
        let mut app = app(80);
        for tab in [Tab::Database, Tab::Replication, Tab::Io] {
            app.viewport_height = 12;
            app.tab = tab;
            app.selected[tab.index()] = 0;
            key(&mut app, KeyCode::Char(']'));
            assert!(app.selected() > 0);
            key(&mut app, KeyCode::End);
            let end = app.selected();
            key(&mut app, KeyCode::Up);
            assert_eq!(app.selected(), end.saturating_sub(1));
            app.viewport_height = 60;
            app.clamp_scroll();
            let area = Screen::for_app(&app).content;
            assert!(
                app.selected()
                    <= ui::metric_rows(&app, area.width - 2)
                        .len()
                        .saturating_sub(usize::from(area.height - 2))
            );
            app.viewport_height = 24;
        }
    }

    #[test]
    fn editor_buttons_submit_and_cancel_without_leaking_global_shortcuts() {
        let mut app = app(80);
        key(&mut app, KeyCode::Char('i'));
        key(&mut app, KeyCode::Char('q'));
        key(&mut app, KeyCode::Char('é'));
        click(&mut app, 1, 0);
        assert_eq!(app.prompt.as_ref().unwrap().value, "qé");
        assert!(!app.should_quit());
        let save = ui::editor_controls(&app)
            .into_iter()
            .find(|c| c.key == KeyCode::Enter)
            .unwrap();
        assert_eq!(
            click(&mut app, save.area.x, save.area.y),
            Some(Action::CreateIncident("qé".into()))
        );
        assert!(app.prompt.is_none());

        app.filter = "blocking".into();
        key(&mut app, KeyCode::Char('/'));
        key(&mut app, KeyCode::Char('q'));
        let cancel = ui::editor_controls(&app)
            .into_iter()
            .find(|c| c.key == KeyCode::Esc)
            .unwrap();
        click(&mut app, cancel.area.x, cancel.area.y);
        assert!(!app.editing_filter);
        assert_eq!(app.filter, "blocking");
        key(&mut app, KeyCode::Char('i'));
        key(&mut app, KeyCode::Enter);
        assert!(
            app.prompt.is_some(),
            "empty submission should keep the editor open"
        );
    }
}
