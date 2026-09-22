use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::{
    event::Message,
    model::{Session, Snapshot, Statement},
    store::SnapshotSummary,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    Refresh,
    Capture,
    ListHistory,
    Load(i64),
    Compare(i64, i64),
    ResumeLive,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Tab {
    #[default]
    Overview,
    Activity,
    Blocking,
    Statements,
    History,
}

impl Tab {
    pub(crate) const ALL: [Self; 5] = [
        Self::Overview,
        Self::Activity,
        Self::Blocking,
        Self::Statements,
        Self::History,
    ];

    pub(crate) fn index(self) -> usize {
        Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0)
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Activity => "Activity",
            Self::Blocking => "Blocking",
            Self::Statements => "Statements",
            Self::History => "History",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum StatementSort {
    #[default]
    Total,
    Mean,
    Calls,
}

impl StatementSort {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Total => "total execution time",
            Self::Mean => "mean execution time",
            Self::Calls => "calls",
        }
    }
}

#[derive(Debug)]
pub(crate) struct App {
    should_quit: bool,
    snapshot: Option<Snapshot>,
    paused: bool,
    offline: bool,
    selected: [usize; 5],
    filter_before_edit: String,
    pub(crate) demo: bool,
    pub(crate) tab: Tab,
    pub(crate) filter: String,
    pub(crate) editing_filter: bool,
    pub(crate) help: bool,
    pub(crate) loading: bool,
    pub(crate) now: DateTime<Utc>,
    pub(crate) error: Option<String>,
    pub(crate) notice: Option<String>,
    pub(crate) sort: StatementSort,
    pub(crate) history: Vec<SnapshotSummary>,
    pub(crate) compare_a: Option<i64>,
    pub(crate) compare_b: Option<i64>,
    pub(crate) report: Option<String>,
    pub(crate) report_scroll: usize,
}

impl App {
    pub(crate) fn new(demo: bool) -> Self {
        Self {
            should_quit: false,
            snapshot: None,
            paused: false,
            offline: false,
            selected: [0; 5],
            filter_before_edit: String::new(),
            demo,
            tab: Tab::Overview,
            filter: String::new(),
            editing_filter: false,
            help: false,
            loading: !demo,
            now: Utc::now(),
            error: None,
            notice: None,
            sort: StatementSort::Total,
            history: Vec::new(),
            compare_a: None,
            compare_b: None,
            report: None,
            report_scroll: 0,
        }
    }

    pub(crate) fn should_quit(&self) -> bool {
        self.should_quit
    }
    pub(crate) fn paused(&self) -> bool {
        self.paused
    }
    pub(crate) fn is_offline(&self) -> bool {
        self.offline || self.report.is_some()
    }
    pub(crate) fn snapshot(&self) -> Option<&Snapshot> {
        self.snapshot.as_ref()
    }
    pub(crate) fn selected(&self) -> usize {
        self.selected[self.tab.index()]
    }

    pub(crate) fn set_snapshot(&mut self, snapshot: Snapshot, offline: bool) {
        self.snapshot = Some(snapshot);
        self.offline = offline;
        if offline {
            self.tab = Tab::Overview;
            self.filter.clear();
            self.selected = [0; 5];
            self.report = None;
        }
        self.loading = false;
        self.error = None;
        self.now = Utc::now();
        self.clamp_selection();
    }

    pub(crate) fn set_history(&mut self, history: Vec<SnapshotSummary>) {
        let selected_id = self
            .filtered_history()
            .get(self.selected[Tab::History.index()])
            .map(|row| row.id);
        self.history = history;
        if let Some(id) = selected_id
            && let Some(index) = self.filtered_history().iter().position(|row| row.id == id)
        {
            self.selected[Tab::History.index()] = index;
        }
        self.clamp_selection();
    }

    pub(crate) fn set_report(&mut self, report: String) {
        self.report = Some(report);
        self.report_scroll = 0;
        self.loading = false;
    }

    pub(crate) fn set_error(&mut self, error: String) {
        self.error = Some(error);
        self.loading = false;
        self.now = Utc::now();
    }

    pub(crate) fn set_notice(&mut self, notice: String) {
        self.notice = Some(notice);
    }
    pub(crate) fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }

    pub(crate) fn update(&mut self, message: Message) -> Option<Action> {
        self.now = Utc::now();
        match message {
            Message::Quit => {
                self.should_quit = true;
                None
            }
            Message::Redraw => None,
            Message::Key(key) => self.key(key),
        }
    }

    fn key(&mut self, key: KeyEvent) -> Option<Action> {
        if self.editing_filter {
            self.edit_filter(key);
            return None;
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
        {
            return None;
        }
        if self.help {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Enter => self.help = false,
                KeyCode::Char('q') => self.should_quit = true,
                _ => {}
            }
            return None;
        }
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.help = true,
            KeyCode::Tab | KeyCode::Right => return self.switch_tab((self.tab.index() + 1) % 5),
            KeyCode::BackTab | KeyCode::Left => return self.switch_tab((self.tab.index() + 4) % 5),
            KeyCode::Char('1'..='5') => {
                if let KeyCode::Char(number) = key.code {
                    return self.switch_tab(number as usize - '1' as usize);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::PageDown => self.move_selection(10),
            KeyCode::PageUp => self.move_selection(-10),
            KeyCode::Home | KeyCode::Char('g') => self.move_selection(isize::MIN),
            KeyCode::End | KeyCode::Char('G') => self.move_selection(isize::MAX),
            KeyCode::Char('/') => {
                self.editing_filter = true;
                self.filter_before_edit = self.filter.clone();
            }
            KeyCode::Char('s') if self.tab == Tab::Statements => {
                self.sort = match self.sort {
                    StatementSort::Total => StatementSort::Mean,
                    StatementSort::Mean => StatementSort::Calls,
                    StatementSort::Calls => StatementSort::Total,
                };
                self.selected[self.tab.index()] = 0;
            }
            KeyCode::Char('p') if !self.is_offline() => {
                self.paused = !self.paused;
                self.notice = Some(
                    if self.paused {
                        "Automatic refresh paused. Press r for one refresh."
                    } else {
                        "Automatic refresh resumed."
                    }
                    .into(),
                );
            }
            KeyCode::Char('r') if self.tab == Tab::History => return Some(Action::ListHistory),
            KeyCode::Char('r') if !self.is_offline() => return Some(Action::Refresh),
            KeyCode::Char('c') => return Some(Action::Capture),
            KeyCode::Enter if self.tab == Tab::History && self.report.is_none() => {
                if let Some(summary) = self.filtered_history().get(self.selected()) {
                    return Some(Action::Load(summary.id));
                }
            }
            KeyCode::Char('a' | 'b') if self.tab == Tab::History => {
                if let Some(id) = self
                    .filtered_history()
                    .get(self.selected())
                    .map(|row| row.id)
                {
                    if key.code == KeyCode::Char('a') {
                        self.compare_a = Some(id);
                    } else {
                        self.compare_b = Some(id);
                    }
                }
            }
            KeyCode::Char('d') if self.tab == Tab::History => {
                if let (Some(a), Some(b)) = (self.compare_a, self.compare_b) {
                    return Some(Action::Compare(a, b));
                }
                self.notice = Some(
                    "Select an earlier capture with a and a later capture with b, then press d."
                        .into(),
                );
            }
            KeyCode::Esc if self.report.is_some() => {
                self.report = None;
                self.report_scroll = 0;
                if !self.offline {
                    return Some(Action::ResumeLive);
                }
            }
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                self.selected = [0; 5];
            }
            KeyCode::Esc if self.offline => {
                self.offline = false;
                self.snapshot = None;
                self.notice = Some("Returned to live monitoring.".into());
                return Some(Action::ResumeLive);
            }
            KeyCode::Esc => {
                self.notice = None;
            }
            _ => {}
        }
        None
    }

    fn switch_tab(&mut self, index: usize) -> Option<Action> {
        let resume_live = self.report.is_some() && !self.offline;
        self.tab = Tab::ALL[index];
        self.report = None;
        self.clamp_selection();
        if resume_live {
            Some(Action::ResumeLive)
        } else {
            (self.tab == Tab::History).then_some(Action::ListHistory)
        }
    }

    fn edit_filter(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.editing_filter = false,
            KeyCode::Esc => {
                self.filter.clone_from(&self.filter_before_edit);
                self.editing_filter = false;
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.filter.clear()
            }
            KeyCode::Backspace => {
                self.filter.pop();
            }
            KeyCode::Char(character)
                if !character.is_control()
                    && !key.modifiers.intersects(
                        KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                    ) =>
            {
                self.filter.push(character);
            }
            _ => {}
        }
        self.selected = [0; 5];
    }

    fn move_selection(&mut self, movement: isize) {
        if let Some(report) = &self.report {
            self.report_scroll = self
                .report_scroll
                .saturating_add_signed(movement)
                .min(report.lines().count().saturating_sub(1));
        } else {
            self.selected[self.tab.index()] = self.selected().saturating_add_signed(movement);
            self.clamp_selection();
        }
    }

    fn clamp_selection(&mut self) {
        let count = match self.tab {
            Tab::Overview => 1,
            Tab::Activity => self.filtered_sessions().len(),
            Tab::Blocking => self.blocking_edges().len(),
            Tab::Statements => self.filtered_statements().len(),
            Tab::History => self.filtered_history().len(),
        };
        self.selected[self.tab.index()] = self.selected().min(count.saturating_sub(1));
    }

    pub(crate) fn filtered_sessions(&self) -> Vec<&Session> {
        let mut sessions: Vec<_> = self
            .snapshot
            .as_ref()
            .and_then(|s| s.activity.available())
            .into_iter()
            .flatten()
            .filter(|session| {
                self.matches(&format!(
                    "{} {} {} {} {} {} {}",
                    session.pid,
                    session.user.as_deref().unwrap_or_default(),
                    session.application,
                    session.state.as_deref().unwrap_or_default(),
                    session.wait_event.as_deref().unwrap_or_default(),
                    session.database.as_deref().unwrap_or_default(),
                    session.query.as_deref().unwrap_or_default()
                ))
            })
            .collect();
        sessions.sort_by(|a, b| {
            b.query_age_ms
                .cmp(&a.query_age_ms)
                .then_with(|| a.pid.cmp(&b.pid))
        });
        sessions
    }

    pub(crate) fn blocking_edges(&self) -> Vec<(&Session, i32)> {
        self.snapshot
            .as_ref()
            .and_then(|s| s.activity.available())
            .into_iter()
            .flatten()
            .flat_map(|session| {
                session
                    .blockers
                    .iter()
                    .map(move |blocker| (session, *blocker))
            })
            .filter(|(session, blocker)| {
                self.matches(&format!(
                    "{} {} {} {}",
                    session.pid,
                    blocker,
                    session.application,
                    session.user.as_deref().unwrap_or_default()
                ))
            })
            .collect()
    }

    pub(crate) fn filtered_statements(&self) -> Vec<&Statement> {
        let mut statements: Vec<_> = self
            .snapshot
            .as_ref()
            .and_then(|s| s.statements.available())
            .into_iter()
            .flat_map(|s| &s.entries)
            .filter(|s| {
                self.matches(&format!(
                    "{} {} {} {}",
                    s.queryid,
                    s.userid,
                    s.dbid,
                    s.query.as_deref().unwrap_or_default()
                ))
            })
            .collect();
        statements.sort_by(|a, b| {
            match self.sort {
                StatementSort::Total => b.total_exec_ms.total_cmp(&a.total_exec_ms),
                StatementSort::Mean => b.mean_exec_ms.total_cmp(&a.mean_exec_ms),
                StatementSort::Calls => b.calls.cmp(&a.calls),
            }
            .then_with(|| a.queryid.cmp(&b.queryid))
            .then_with(|| a.userid.cmp(&b.userid))
            .then_with(|| a.toplevel.cmp(&b.toplevel))
        });
        statements
    }

    pub(crate) fn filtered_history(&self) -> Vec<&SnapshotSummary> {
        self.history
            .iter()
            .filter(|s| self.matches(&format!("{} {} {}", s.id, s.label, s.source)))
            .collect()
    }

    fn matches(&self, value: &str) -> bool {
        value.to_lowercase().contains(&self.filter.to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(app: &mut App, code: KeyCode) -> Option<Action> {
        app.update(Message::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }

    fn history() -> Vec<SnapshotSummary> {
        [10, 20]
            .into_iter()
            .map(|id| SnapshotSummary {
                id,
                label: format!("Capture {id}"),
                captured_at: Utc::now(),
                source: "localhost/test".into(),
                complete: true,
            })
            .collect()
    }

    #[test]
    fn filter_accepts_quit_character_unicode_and_restores_on_cancel() {
        let mut app = App::new(true);
        key(&mut app, KeyCode::Char('/'));
        for character in ['q', 'é'] {
            key(&mut app, KeyCode::Char(character));
        }
        assert_eq!(app.filter, "qé");
        assert!(!app.should_quit());
        key(&mut app, KeyCode::Backspace);
        assert_eq!(app.filter, "q");
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::Char('/'));
        key(&mut app, KeyCode::Char('x'));
        key(&mut app, KeyCode::Esc);
        assert_eq!(app.filter, "q");
        app.update(Message::Quit);
        assert!(app.should_quit());
    }

    #[test]
    fn history_selects_loads_and_compares_real_ids() {
        let mut app = App::new(false);
        app.set_history(history());
        assert_eq!(key(&mut app, KeyCode::Char('5')), Some(Action::ListHistory));
        key(&mut app, KeyCode::Char('a'));
        key(&mut app, KeyCode::Down);
        key(&mut app, KeyCode::Char('b'));
        assert_eq!(
            key(&mut app, KeyCode::Char('d')),
            Some(Action::Compare(10, 20))
        );
        assert_eq!(key(&mut app, KeyCode::Enter), Some(Action::Load(20)));
        key(&mut app, KeyCode::End);
        assert_eq!(app.selected(), 1);
        app.set_history(Vec::new());
        assert_eq!(app.selected(), 0);
        assert_eq!(key(&mut app, KeyCode::Enter), None);
    }

    #[test]
    fn pause_allows_explicit_refresh_and_capture_requests_a_fresh_observation() {
        let mut app = App::new(false);
        key(&mut app, KeyCode::Char('p'));
        assert!(app.paused());
        assert_eq!(key(&mut app, KeyCode::Char('r')), Some(Action::Refresh));
        assert_eq!(key(&mut app, KeyCode::Char('c')), Some(Action::Capture));
    }

    #[test]
    fn comparison_scroll_and_help_do_not_change_history_selection() {
        let mut app = App::new(true);
        app.set_history(history());
        key(&mut app, KeyCode::Char('5'));
        app.set_report("first\nsecond\nthird".into());
        key(&mut app, KeyCode::End);
        assert_eq!(app.report_scroll, 2);
        assert_eq!(app.selected(), 0);
        key(&mut app, KeyCode::Char('?'));
        key(&mut app, KeyCode::Down);
        assert_eq!(app.report_scroll, 2);
        key(&mut app, KeyCode::Esc);
        assert!(!app.help);
        assert!(app.report.is_some());
        key(&mut app, KeyCode::Esc);
        assert!(app.report.is_none());
        assert!(!app.is_offline());
    }

    #[test]
    fn refreshing_history_keeps_the_selected_capture_id() {
        let mut app = App::new(true);
        app.set_history(history());
        key(&mut app, KeyCode::Char('5'));
        key(&mut app, KeyCode::Down);
        let mut updated = history();
        updated.reverse();
        app.set_history(updated);
        assert_eq!(key(&mut app, KeyCode::Enter), Some(Action::Load(20)));
    }

    #[test]
    fn dismissing_a_notice_does_not_hide_collection_failure() {
        let mut app = App::new(false);
        app.set_error("Connection unavailable".into());
        app.set_notice("Refresh requested".into());
        key(&mut app, KeyCode::Esc);
        assert!(app.notice.is_none());
        assert_eq!(app.error.as_deref(), Some("Connection unavailable"));
    }

    #[test]
    fn closing_a_comparison_preserves_live_collection_failure() {
        let mut app = App::new(false);
        app.set_error("Connection unavailable".into());
        app.set_report("Comparison is available offline".into());
        assert!(app.is_offline());
        assert_eq!(key(&mut app, KeyCode::Esc), Some(Action::ResumeLive));
        assert!(!app.is_offline());
        assert_eq!(app.error.as_deref(), Some("Connection unavailable"));
    }
    #[test]
    fn statement_ranking_switches_between_total_mean_and_calls_and_filters() {
        let mut snapshot = crate::demo::snapshot(0, false);
        if let crate::model::Observation::Available(stats) = &mut snapshot.statements {
            for (statement, (total, calls)) in
                stats
                    .entries
                    .iter_mut()
                    .zip([(1000.0, 100), (200.0, 1), (300.0, 300)])
            {
                statement.total_exec_ms = total;
                statement.calls = calls;
                statement.mean_exec_ms = total / calls as f64;
            }
        }
        let mut app = App::new(true);
        app.set_snapshot(snapshot, false);
        key(&mut app, KeyCode::Char('4'));
        assert_eq!(
            app.filtered_statements()
                .iter()
                .map(|s| s.queryid)
                .collect::<Vec<_>>(),
            [701, 703, 702]
        );
        key(&mut app, KeyCode::Char('s'));
        assert_eq!(
            app.filtered_statements()
                .iter()
                .map(|s| s.queryid)
                .collect::<Vec<_>>(),
            [702, 701, 703]
        );
        key(&mut app, KeyCode::Char('s'));
        assert_eq!(
            app.filtered_statements()
                .iter()
                .map(|s| s.queryid)
                .collect::<Vec<_>>(),
            [703, 701, 702]
        );
        key(&mut app, KeyCode::Char('/'));
        for digit in "703".chars() {
            key(&mut app, KeyCode::Char(digit));
        }
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.filtered_statements().len(), 1);
        assert_eq!(app.filtered_statements()[0].queryid, 703);
    }
}
