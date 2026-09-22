use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::VecDeque;

use crate::{
    event::Message,
    model::{Session, Snapshot, Statement},
    store::SnapshotSummary,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Action {
    Refresh,
    Capture,
    ListHistory,
    Load(i64),
    Compare(i64, i64),
    ResumeLive,
    ListIncidents,
    SelectIncident(i64),
    CreateIncident(String),
    NoteIncident(i64, String),
    CloseIncident(i64, bool),
    AttachCapture(i64, i64),
    LabelCapture(i64, String),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Tab {
    #[default]
    Overview,
    Activity,
    Blocking,
    Statements,
    History,
    Database,
    Relations,
    Replication,
    Io,
    Incidents,
}

impl Tab {
    pub(crate) const ALL: [Self; 10] = [
        Self::Overview,
        Self::Activity,
        Self::Blocking,
        Self::Statements,
        Self::History,
        Self::Database,
        Self::Relations,
        Self::Replication,
        Self::Io,
        Self::Incidents,
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
            Self::Database => "Database",
            Self::Relations => "Relations",
            Self::Replication => "Replication",
            Self::Io => "I/O",
            Self::Incidents => "Incidents",
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

#[derive(Debug, Clone, Copy)]
pub(crate) enum PromptKind {
    Incident,
    Note(i64),
    Label(i64),
}

#[derive(Debug)]
pub(crate) struct Prompt {
    pub(crate) kind: PromptKind,
    pub(crate) value: String,
}

#[derive(Debug, Clone)]
pub(crate) struct TrendPoint {
    pub(crate) at: DateTime<Utc>,
    pub(crate) transactions: Option<f64>,
    pub(crate) wal_bytes: Option<f64>,
    pub(crate) active: Option<f64>,
    pub(crate) blocked: Option<f64>,
}

#[derive(Debug)]
pub(crate) struct App {
    should_quit: bool,
    snapshot: Option<Snapshot>,
    paused: bool,
    offline: bool,
    selected: [usize; 10],
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
    pub(crate) report_title: &'static str,
    pub(crate) report_scroll: usize,
    pub(crate) analysis: Option<crate::diagnostics::Analysis>,
    pub(crate) trend: VecDeque<TrendPoint>,
    live_current: Option<Snapshot>,
    live_previous: Option<Snapshot>,
    pub(crate) statement_interval: bool,
    pub(crate) show_indexes: bool,
    pub(crate) relation_sort: usize,
    pub(crate) incidents: Vec<crate::store::IncidentSummary>,
    pub(crate) incident: Option<crate::store::Incident>,
    pub(crate) active_incident: Option<i64>,
    pub(crate) prompt: Option<Prompt>,
}

impl App {
    pub(crate) fn new(demo: bool) -> Self {
        Self {
            should_quit: false,
            snapshot: None,
            paused: false,
            offline: false,
            selected: [0; 10],
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
            report_title: "Offline comparison",
            report_scroll: 0,
            analysis: None,
            trend: VecDeque::new(),
            live_current: None,
            live_previous: None,
            statement_interval: false,
            show_indexes: false,
            relation_sort: 0,
            incidents: Vec::new(),
            incident: None,
            active_incident: None,
            prompt: None,
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
        if !offline {
            let is_new = self.live_current.as_ref().is_none_or(|old| {
                old.completed_at != snapshot.completed_at || old.source != snapshot.source
            });
            if is_new {
                if self.live_current.as_ref().is_some_and(|old| {
                    !crate::compare::sources_compatible(&old.source, &snapshot.source)
                }) {
                    self.trend.clear();
                }
                self.live_previous = self.live_current.take();
                self.live_current = Some(snapshot.clone());
            }
            let analysis = crate::diagnostics::analyze(&snapshot, self.live_previous.as_ref());
            if is_new {
                self.trend.push_back(TrendPoint {
                    at: snapshot.completed_at,
                    transactions: analysis
                        .rates
                        .database
                        .available()
                        .and_then(|v| v.transactions_per_second.available().copied()),
                    wal_bytes: analysis
                        .rates
                        .wal
                        .available()
                        .and_then(|v| v.bytes_per_second.available().copied()),
                    active: snapshot.activity.available().map(|v| {
                        v.iter()
                            .filter(|s| s.state.as_deref() == Some("active"))
                            .count() as f64
                    }),
                    blocked: snapshot
                        .activity
                        .available()
                        .map(|v| v.iter().filter(|s| !s.blockers.is_empty()).count() as f64),
                });
                while self.trend.len() > 120 {
                    self.trend.pop_front();
                }
            }
            self.analysis = Some(analysis);
        } else {
            self.analysis = Some(crate::diagnostics::analyze(&snapshot, None));
        }
        self.snapshot = Some(snapshot);
        self.offline = offline;
        if offline {
            self.tab = Tab::Overview;
            self.filter.clear();
            self.selected = [0; 10];
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
        self.report_title = "Offline comparison";
        self.report = Some(report);
        self.report_scroll = 0;
        self.loading = false;
    }

    pub(crate) fn set_error(&mut self, error: String) {
        self.error = Some(error);
        self.loading = false;
        self.now = Utc::now();
    }

    pub(crate) fn collection_failed(&mut self, error: String) {
        self.set_error(error);
        self.trend.push_back(TrendPoint {
            at: self.now,
            transactions: None,
            wal_bytes: None,
            active: None,
            blocked: None,
        });
        while self.trend.len() > 120 {
            self.trend.pop_front();
        }
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
        if self.prompt.is_some() {
            return self.edit_prompt(key);
        }
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
            KeyCode::Tab | KeyCode::Right => return self.switch_tab((self.tab.index() + 1) % 10),
            KeyCode::BackTab | KeyCode::Left => {
                return self.switch_tab((self.tab.index() + 9) % 10);
            }
            KeyCode::Char('1'..='9') => {
                if let KeyCode::Char(number) = key.code {
                    return self.switch_tab(number as usize - '1' as usize);
                }
            }
            KeyCode::Char('0') => return self.switch_tab(9),
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
            KeyCode::Char('i') => {
                self.prompt = Some(Prompt {
                    kind: PromptKind::Incident,
                    value: String::new(),
                });
            }
            KeyCode::Char('n') if self.active_incident.is_some() => {
                self.prompt = self.active_incident.map(|id| Prompt {
                    kind: PromptKind::Note(id),
                    value: String::new(),
                });
            }
            KeyCode::Char('x') if self.tab == Tab::Incidents => {
                self.active_incident = None;
                self.notice = Some("New captures will not be attached to an incident.".into());
            }
            KeyCode::Char('o') if self.tab == Tab::Incidents => {
                if let Some(incident) = self.filtered_incidents().get(self.selected()) {
                    return Some(Action::CloseIncident(
                        incident.id,
                        incident.closed_at.is_none(),
                    ));
                }
            }
            KeyCode::Enter if self.tab == Tab::Overview && self.report.is_none() => {
                let coverage = self
                    .analysis
                    .as_ref()
                    .map(|a| a.coverage.join("\n\n"))
                    .unwrap_or_else(|| "No analysis is available yet.".into());
                let source = self
                    .snapshot()
                    .map(|s| {
                        format!(
                            "Source: {} / {}\nObserved: {}",
                            s.source.endpoint,
                            s.source.database,
                            s.completed_at.to_rfc3339()
                        )
                    })
                    .unwrap_or_else(|| "No observation is available.".into());
                if let Some(finding) = self.filtered_findings().get(self.selected()) {
                    let finding = (*finding).clone();
                    self.set_report(format!("# {}\n\n{source}\n\nSeverity: {:?}\n\n## Evidence\n\n{}\n\n## Interpretation\n\n{}\n\n## Next steps\n\n{}\n\n## Coverage and limits\n\n{coverage}", finding.title, finding.severity, finding.evidence.join("\n\n"), finding.interpretation, finding.next_steps.join("\n\n")));
                    self.report_title = "Finding details";
                } else if self.analysis.is_some() {
                    self.set_report(format!("# Coverage and limits\n\n{source}\n\nNo finding matches this view. Absence of findings does not establish database health.\n\n{coverage}"));
                    self.report_title = "Coverage and limits";
                }
            }
            KeyCode::Enter if self.tab == Tab::Incidents => {
                if let Some(incident) = self.filtered_incidents().get(self.selected()) {
                    return Some(Action::SelectIncident(incident.id));
                }
            }
            KeyCode::Char('I') if self.tab == Tab::History => {
                if let (Some(incident), Some(capture)) = (
                    self.active_incident,
                    self.filtered_history().get(self.selected()),
                ) {
                    return Some(Action::AttachCapture(incident, capture.id));
                }
                self.notice = Some(
                    "Activate an open incident with 0 then Enter, or create one with i.".into(),
                );
            }
            KeyCode::Char('l') if self.tab == Tab::History => {
                if let Some(capture) = self.filtered_history().get(self.selected()) {
                    self.prompt = Some(Prompt {
                        kind: PromptKind::Label(capture.id),
                        value: capture.label.clone(),
                    });
                }
            }
            KeyCode::Char('v') if self.tab == Tab::Statements => {
                self.statement_interval = !self.statement_interval;
                self.selected[self.tab.index()] = 0;
            }
            KeyCode::Char('v') if self.tab == Tab::Relations => {
                self.show_indexes = !self.show_indexes;
                self.selected[self.tab.index()] = 0;
            }
            KeyCode::Char('s') if self.tab == Tab::Relations => {
                self.relation_sort = (self.relation_sort + 1) % 3;
                self.selected[self.tab.index()] = 0;
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
            KeyCode::Char('r') if self.tab == Tab::Incidents => return Some(Action::ListIncidents),
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
                self.selected = [0; 10];
            }
            KeyCode::Esc if self.offline => {
                self.offline = false;
                self.snapshot = None;
                self.analysis = None;
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
            match self.tab {
                Tab::History => Some(Action::ListHistory),
                Tab::Incidents => Some(Action::ListIncidents),
                _ => None,
            }
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
        self.selected = [0; 10];
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
            Tab::Overview => self.filtered_findings().len(),
            Tab::Activity => self.filtered_sessions().len(),
            Tab::Blocking => self.blocking_edges().len(),
            Tab::Statements if self.statement_interval => self.filtered_statement_rates().len(),
            Tab::Statements => self.filtered_statements().len(),
            Tab::History => self.filtered_history().len(),
            Tab::Relations if self.show_indexes => self.filtered_indexes().len(),
            Tab::Relations => self.filtered_tables().len(),
            Tab::Incidents => self.filtered_incidents().len(),
            Tab::Database => 40,
            Tab::Replication => self
                .snapshot()
                .and_then(|s| s.health.replication.available())
                .map_or(20, |r| (r.senders.len() + r.slots.len()) * 3 + 20),
            Tab::Io => self.snapshot().map_or(20, |s| {
                s.health.io.available().map_or(0, |rows| rows.len() * 2)
                    + s.health.vacuum.available().map_or(0, |rows| rows.len() * 2)
                    + 20
            }),
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
    fn edit_prompt(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Esc => self.prompt = None,
            KeyCode::Enter => {
                let prompt = self.prompt.take()?;
                if prompt.value.trim().is_empty() {
                    self.notice = Some("Text cannot be empty.".into());
                    return None;
                }
                return Some(match prompt.kind {
                    PromptKind::Incident => Action::CreateIncident(prompt.value),
                    PromptKind::Note(id) => Action::NoteIncident(id, prompt.value),
                    PromptKind::Label(id) => Action::LabelCapture(id, prompt.value),
                });
            }
            KeyCode::Backspace => {
                if let Some(prompt) = &mut self.prompt {
                    prompt.value.pop();
                }
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(prompt) = &mut self.prompt {
                    prompt.value.clear();
                }
            }
            KeyCode::Char(value)
                if !value.is_control()
                    && !key.modifiers.intersects(
                        KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER,
                    ) =>
            {
                if let Some(prompt) = &mut self.prompt {
                    let limit = if matches!(prompt.kind, PromptKind::Note(_)) {
                        4000
                    } else {
                        200
                    };
                    if prompt.value.chars().count() < limit {
                        prompt.value.push(value);
                    }
                }
            }
            _ => {}
        }
        None
    }

    pub(crate) fn set_incidents(&mut self, incidents: Vec<crate::store::IncidentSummary>) {
        let selected_id = self
            .filtered_incidents()
            .get(self.selected[Tab::Incidents.index()])
            .map(|v| v.id);
        self.incidents = incidents;
        if let Some(id) = selected_id
            && let Some(index) = self.filtered_incidents().iter().position(|v| v.id == id)
        {
            self.selected[Tab::Incidents.index()] = index;
        }
        if self.active_incident.is_some_and(|id| {
            self.incidents
                .iter()
                .any(|v| v.id == id && v.closed_at.is_some())
        }) {
            self.active_incident = None;
        }
        self.clamp_selection();
    }
    pub(crate) fn set_incident(&mut self, incident: crate::store::Incident) {
        self.active_incident = incident
            .summary
            .closed_at
            .is_none()
            .then_some(incident.summary.id);
        self.incident = Some(incident);
    }
    pub(crate) fn filtered_incidents(&self) -> Vec<&crate::store::IncidentSummary> {
        self.incidents
            .iter()
            .filter(|v| self.matches(&format!("{} {}", v.id, v.title)))
            .collect()
    }
    pub(crate) fn filtered_findings(&self) -> Vec<&crate::diagnostics::Finding> {
        self.analysis
            .iter()
            .flat_map(|a| &a.findings)
            .filter(|f| self.matches(&format!("{} {} {}", f.id, f.title, f.evidence.join(" "))))
            .collect()
    }
    pub(crate) fn filtered_tables(&self) -> Vec<&crate::model::TableStats> {
        let mut rows: Vec<_> = self
            .snapshot
            .iter()
            .filter_map(|s| s.health.tables.available())
            .flat_map(|r| &r.tables)
            .filter(|t| self.matches(&format!("{} {} {}", t.oid, t.schema, t.name)))
            .collect();
        rows.sort_by(|a, b| {
            match self.relation_sort {
                1 => b.dead_tuples.cmp(&a.dead_tuples),
                2 => b.seq_scan.cmp(&a.seq_scan),
                _ => b.total_bytes.cmp(&a.total_bytes),
            }
            .then(a.oid.cmp(&b.oid))
        });
        rows
    }
    pub(crate) fn filtered_indexes(&self) -> Vec<&crate::model::IndexStats> {
        let mut rows: Vec<_> = self
            .snapshot
            .iter()
            .filter_map(|s| s.health.tables.available())
            .flat_map(|r| &r.indexes)
            .filter(|t| self.matches(&format!("{} {} {} {}", t.oid, t.schema, t.table, t.name)))
            .collect();
        rows.sort_by(|a, b| {
            match self.relation_sort {
                1 => a.valid.cmp(&b.valid),
                2 => b.scans.cmp(&a.scans),
                _ => b.size_bytes.cmp(&a.size_bytes),
            }
            .then(a.oid.cmp(&b.oid))
        });
        rows
    }
    pub(crate) fn filtered_statement_rates(&self) -> Vec<&crate::metrics::StatementRate> {
        let mut rows: Vec<_> = self
            .analysis
            .iter()
            .filter_map(|a| a.rates.statements.available())
            .flatten()
            .filter(|r| {
                self.matches(&format!(
                    "{} {} {}",
                    r.identity.queryid, r.identity.userid, r.identity.dbid
                ))
            })
            .collect();
        let metric = |r: &crate::metrics::StatementRate| match self.sort {
            StatementSort::Total => r.exec_ms_per_second.available().copied(),
            StatementSort::Mean => r.mean_exec_ms.available().copied(),
            StatementSort::Calls => r.calls_per_second.available().copied(),
        };
        rows.sort_by(|a, b| {
            match (metric(a), metric(b)) {
                (Some(a), Some(b)) => b.total_cmp(&a),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
            .then(a.identity.queryid.cmp(&b.identity.queryid))
        });
        rows
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

#[cfg(test)]
mod investigation_tests {
    use super::*;
    use crate::model::Observation;

    fn key(app: &mut App, code: KeyCode) -> Option<Action> {
        app.update(Message::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }

    fn sample(sequence: u64) -> Snapshot {
        let mut snapshot = crate::demo::snapshot(sequence, false);
        let time =
            DateTime::from_timestamp(1_790_000_000 + sequence as i64 * 5, 0).unwrap_or_default();
        snapshot.started_at = time;
        snapshot.completed_at = time;
        snapshot
    }

    #[test]
    fn live_rates_and_bounded_trends_keep_offline_evidence_separate() {
        let mut app = App::new(true);
        app.set_snapshot(sample(0), false);
        assert_eq!(app.trend.len(), 1);
        assert!(app.trend[0].transactions.is_none());
        app.set_snapshot(sample(1), false);
        let rate = app.trend[1]
            .transactions
            .expect("second observation has an interval");
        assert!((rate - 30.6).abs() < 0.001);
        app.set_snapshot(sample(1), false);
        assert_eq!(
            app.trend.len(),
            2,
            "restoring an unchanged observation adds no point"
        );
        app.set_snapshot(sample(10), true);
        assert!(
            app.analysis
                .as_ref()
                .expect("offline diagnostics")
                .rates
                .elapsed_seconds
                .is_none()
        );
        assert_eq!(app.trend.len(), 2);
        assert_eq!(key(&mut app, KeyCode::Esc), Some(Action::ResumeLive));
        assert!(app.snapshot().is_none());
        assert!(
            app.analysis.is_none(),
            "offline findings cannot masquerade as live evidence"
        );
        app.set_snapshot(sample(1), false);
        assert_eq!(app.trend.len(), 2);
        app.collection_failed("unreachable".into());
        assert!(
            app.trend
                .back()
                .is_some_and(|point| point.transactions.is_none() && point.active.is_none())
        );
        for sequence in 2..130 {
            app.set_snapshot(sample(sequence), false);
        }
        assert_eq!(app.trend.len(), 120);
        let mut other = sample(130);
        other.source.database = "another_database".into();
        app.set_snapshot(other, false);
        assert_eq!(
            app.trend.len(),
            1,
            "trends never combine incompatible targets"
        );
        assert!(app.trend[0].transactions.is_none());
    }

    #[test]
    fn unavailable_counters_do_not_become_zero_rates_or_hide_observed_activity() {
        let mut app = App::new(true);
        app.set_snapshot(sample(0), false);
        let mut next = sample(1);
        next.health.database = Observation::Unavailable("permission denied".into());
        app.set_snapshot(next, false);
        let point = app.trend.back().expect("new point");
        assert!(point.transactions.is_none());
        assert!(point.active.is_some());
        assert!(
            app.analysis
                .as_ref()
                .expect("analysis")
                .coverage
                .iter()
                .any(|v| v.contains("permission denied"))
        );
    }

    #[test]
    fn incident_and_label_editors_accept_text_without_triggering_global_shortcuts() {
        let mut app = App::new(true);
        key(&mut app, KeyCode::Char('i'));
        for character in "Query queue".chars() {
            key(&mut app, KeyCode::Char(character));
        }
        assert!(!app.should_quit());
        assert_eq!(
            key(&mut app, KeyCode::Enter),
            Some(Action::CreateIncident("Query queue".into()))
        );
        app.active_incident = Some(7);
        key(&mut app, KeyCode::Char('n'));
        for character in "qé evidence".chars() {
            key(&mut app, KeyCode::Char(character));
        }
        assert_eq!(
            key(&mut app, KeyCode::Enter),
            Some(Action::NoteIncident(7, "qé evidence".into()))
        );
        key(&mut app, KeyCode::Char('i'));
        key(&mut app, KeyCode::Esc);
        assert!(app.prompt.is_none());
        assert!(!app.is_offline());
        key(&mut app, KeyCode::Char('0'));
        key(&mut app, KeyCode::Char('x'));
        assert!(app.active_incident.is_none());
        key(&mut app, KeyCode::Char('i'));
        app.update(Message::Quit);
        assert!(app.should_quit(), "Ctrl+C still exits an editor");
    }

    #[test]
    fn finding_drilldown_and_new_views_have_working_navigation() {
        let mut app = App::new(true);
        app.set_snapshot(sample(0), false);
        assert!(!app.filtered_findings().is_empty());
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.report_title, "Finding details");
        assert!(
            app.report
                .as_ref()
                .is_some_and(|v| v.contains("## Evidence") && v.contains("## Next steps"))
        );
        key(&mut app, KeyCode::PageDown);
        assert!(app.report_scroll > 0);
        key(&mut app, KeyCode::Esc);
        assert!(app.report.is_none());
        key(&mut app, KeyCode::Char('7'));
        assert_eq!(app.tab, Tab::Relations);
        let largest = app.filtered_tables()[0].oid;
        key(&mut app, KeyCode::Char('s'));
        assert_ne!(
            app.filtered_tables()[0].oid,
            largest,
            "maintenance sorting differs from size ranking"
        );
        key(&mut app, KeyCode::Char('v'));
        assert!(app.show_indexes);
        assert!(!app.filtered_indexes().is_empty());
        key(&mut app, KeyCode::Char('0'));
        assert_eq!(app.tab, Tab::Incidents);
        key(&mut app, KeyCode::Tab);
        assert_eq!(app.tab, Tab::Overview);
        key(&mut app, KeyCode::BackTab);
        assert_eq!(app.tab, Tab::Incidents);
    }
}
