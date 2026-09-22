use super::{Action, App, Tab};
use crate::{
    compare::{SessionIdentity, StatementIdentity},
    model::{Session, Snapshot, Statement},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Identity {
    Session(i32, Option<chrono::DateTime<chrono::Utc>>),
    Blocking(
        i32,
        Option<chrono::DateTime<chrono::Utc>>,
        i32,
        Option<chrono::DateTime<chrono::Utc>>,
    ),
    Statement(StatementIdentity),
    Relation(i64),
    Finding(String),
}

impl Identity {
    fn reliable(&self) -> bool {
        !matches!(
            self,
            Self::Session(_, None) | Self::Blocking(_, None, _, _) | Self::Blocking(_, _, _, None)
        )
    }
    fn description(&self) -> String {
        match self {
            Self::Session(pid, _) => format!("PID {pid}"),
            Self::Blocking(waiter, _, blocker, _) => format!("PID {waiter} waiting for {blocker}"),
            Self::Statement(id) => format!(
                "statement {} / user {} / database {} / top-level {}",
                id.queryid, id.userid, id.dbid, id.toplevel
            ),
            Self::Relation(oid) => format!("relation OID {oid}"),
            Self::Finding(id) => format!("finding {id}"),
        }
    }
}

#[derive(Debug)]
pub(super) struct Location {
    tab: Tab,
    selected: [usize; 10],
    filters: [String; 10],
    filter: String,
    report: Option<String>,
    report_title: &'static str,
    report_scroll: usize,
    snapshot: Option<Snapshot>,
    analysis: Option<crate::diagnostics::Analysis>,
    offline: bool,
    capture: Option<crate::store::SnapshotSummary>,
    show_indexes: bool,
    statement_interval: bool,
    incident_timeline: bool,
    incident_cursor: usize,
    incident: Option<crate::store::Incident>,
}

fn statement_identity(statement: &Statement) -> StatementIdentity {
    StatementIdentity {
        userid: statement.userid,
        dbid: statement.dbid,
        queryid: statement.queryid,
        toplevel: statement.toplevel,
    }
}

impl App {
    pub(super) fn max_report_scroll(&self) -> usize {
        let header = if self.capture.is_some() { 3 } else { 2 };
        let message = if self.error.is_some() || self.notice.is_some() {
            2
        } else {
            0
        };
        let visible = usize::from(
            self.viewport_height
                .saturating_sub(header + 2 + message + 1 + 1 + 2),
        )
        .max(1);
        self.report.as_ref().map_or(0, |report| {
            crate::ui::report_lines(report, self.viewport_width.saturating_sub(2))
                .len()
                .saturating_sub(visible)
        })
    }

    pub(crate) fn clamp_scroll(&mut self) {
        self.report_scroll = self.report_scroll.min(self.max_report_scroll());
        let (width, height) = crate::ui::help_dimensions(self.viewport_width, self.viewport_height);
        self.help_scroll = self.help_scroll.min(
            crate::ui::help_lines(width.saturating_sub(2))
                .len()
                .saturating_sub(usize::from(height.saturating_sub(2))),
        );
    }

    pub(crate) fn has_return_path(&self) -> bool {
        !self.navigation.is_empty()
    }

    pub(crate) fn filter_available(&self) -> bool {
        !matches!(self.tab, Tab::Database | Tab::Replication | Tab::Io)
            && !(self.tab == Tab::Incidents && self.incident_timeline)
    }

    pub(crate) fn statement_for_identity(&self, id: &StatementIdentity) -> Option<&Statement> {
        self.snapshot()?
            .statements
            .available()?
            .entries
            .iter()
            .find(|s| statement_identity(s) == *id)
    }

    fn identities(&self, tab: Tab) -> Vec<Identity> {
        match tab {
            Tab::Overview => self
                .filtered_findings()
                .iter()
                .map(|f| Identity::Finding(f.id.clone()))
                .collect(),
            Tab::Activity => self
                .filtered_sessions()
                .iter()
                .map(|s| Identity::Session(s.pid, s.backend_start))
                .collect(),
            Tab::Blocking => self
                .blocking_edges()
                .iter()
                .map(|(waiter, pid)| {
                    let start = self.session(*pid).and_then(|s| s.backend_start);
                    Identity::Blocking(waiter.pid, waiter.backend_start, *pid, start)
                })
                .collect(),
            Tab::Statements if self.statement_interval => self
                .filtered_statement_rates()
                .iter()
                .map(|s| Identity::Statement(s.identity.clone()))
                .collect(),
            Tab::Statements => self
                .filtered_statements()
                .iter()
                .map(|s| Identity::Statement(statement_identity(s)))
                .collect(),
            Tab::Relations if self.show_indexes => self
                .filtered_indexes()
                .iter()
                .map(|s| Identity::Relation(s.oid))
                .collect(),
            Tab::Relations => self
                .filtered_tables()
                .iter()
                .map(|s| Identity::Relation(s.oid))
                .collect(),
            _ => Vec::new(),
        }
    }

    pub(super) fn selection_identities(&self) -> Vec<(Tab, Identity, bool)> {
        std::iter::once(self.tab)
            .chain(Tab::ALL.into_iter().filter(|tab| *tab != self.tab))
            .filter_map(|tab| {
                let identities = self.identities(tab);
                identities
                    .get(self.selected[tab.index()])
                    .cloned()
                    .map(|id| {
                        let unique = identities
                            .iter()
                            .filter(|candidate| **candidate == id)
                            .count()
                            == 1;
                        (tab, id, unique)
                    })
            })
            .collect()
    }

    pub(super) fn restore_identities(
        &mut self,
        identities: Vec<(Tab, Identity, bool)>,
        same_source: bool,
    ) {
        let mut lost = Vec::new();
        for (tab, identity, was_unique) in identities {
            let candidates = self.identities(tab);
            let matches = candidates
                .iter()
                .enumerate()
                .filter(|(_, id)| **id == identity)
                .map(|(index, _)| index)
                .collect::<Vec<_>>();
            let ambiguous = !was_unique || matches.len() > 1;
            let index = if same_source && identity.reliable() && !ambiguous {
                matches.first().copied()
            } else {
                None
            };
            if let Some(index) = index {
                self.selected[tab.index()] = index;
            } else {
                self.selected[tab.index()] = 0;
                let reason = if !same_source {
                    "source changed"
                } else if ambiguous {
                    "identity is ambiguous"
                } else if !identity.reliable() {
                    "backend identity unavailable"
                } else {
                    "no longer visible"
                };
                lost.push(format!(
                    "{}: {} ({reason})",
                    tab.title(),
                    identity.description()
                ));
            }
        }
        if !lost.is_empty() {
            self.notice = Some(format!(
                "Selection changed: {}. Selected the first visible row.",
                lost.join("; ")
            ));
        }
    }

    fn session(&self, pid: i32) -> Option<&Session> {
        let mut matches = self
            .snapshot()?
            .activity
            .available()?
            .iter()
            .filter(|s| s.pid == pid);
        let session = matches.next()?;
        matches.next().is_none().then_some(session)
    }

    pub(super) fn remember_location(&mut self) {
        self.navigation.push(Location {
            tab: self.tab,
            selected: self.selected,
            filters: self.filters.clone(),
            filter: self.filter.clone(),
            report: self.report.clone(),
            report_title: self.report_title,
            report_scroll: self.report_scroll,
            snapshot: self.snapshot.clone(),
            analysis: self.analysis.clone(),
            offline: self.offline,
            capture: self.capture.clone(),
            show_indexes: self.show_indexes,
            statement_interval: self.statement_interval,
            incident_timeline: self.incident_timeline,
            incident_cursor: self.incident_cursor,
            incident: self.incident.clone(),
        });
    }

    pub(super) fn return_to_origin(&mut self) -> Option<Action> {
        let origin = self.navigation.pop()?;
        self.tab = origin.tab;
        self.selected = origin.selected;
        self.filters = origin.filters;
        self.filter = origin.filter;
        self.report = origin.report;
        self.report_title = origin.report_title;
        self.report_scroll = origin.report_scroll;
        self.snapshot = origin.snapshot;
        self.analysis = origin.analysis;
        self.offline = origin.offline;
        self.capture = origin.capture;
        self.show_indexes = origin.show_indexes;
        self.statement_interval = origin.statement_interval;
        self.incident_timeline = origin.incident_timeline;
        self.incident_cursor = origin.incident_cursor;
        if self.tab == Tab::Incidents {
            self.incident = origin.incident;
        }
        self.notice = Some("Returned to the originating evidence and selection.".into());
        if self.tab == Tab::Incidents {
            Some(Action::ListIncidents)
        } else {
            (!self.is_offline()).then_some(Action::ResumeLive)
        }
    }

    pub(crate) fn discard_pending_capture_return(&mut self, request: u64) {
        if self.pending_capture_request != Some(request) {
            return;
        }
        self.pending_capture_request = None;
        if self.tab == Tab::Incidents
            && self.incident_timeline
            && self
                .navigation
                .last()
                .is_some_and(|origin| origin.tab == Tab::Incidents)
        {
            self.navigation.pop();
        }
    }

    fn evidence_view(&mut self, tab: Tab) {
        self.remember_location();
        self.filters[self.tab.index()] = self.filter.clone();
        self.tab = tab;
        self.filter.clear();
        self.report = None;
        self.report_scroll = 0;
        self.notice =
            Some("Evidence from the same observation. Backspace: return to origin.".into());
    }

    fn open_session(&mut self, pid: i32) {
        let identity = self.session(pid).and_then(|s| {
            s.backend_start
                .map(|backend_start| SessionIdentity { pid, backend_start })
        });
        let Some(identity) = identity else {
            self.notice = Some(format!(
                "PID {pid} has no reliable visible backend identity in this observation."
            ));
            return;
        };
        self.evidence_view(Tab::Activity);
        self.selected[Tab::Activity.index()] = self
            .filtered_sessions()
            .iter()
            .position(|s| s.pid == identity.pid && s.backend_start == Some(identity.backend_start))
            .unwrap_or(0);
    }

    pub(super) fn follow_evidence(&mut self) {
        if self.tab == Tab::Blocking {
            self.follow_session(true);
            return;
        }
        if self.tab != Tab::Overview {
            return;
        }
        let Some(finding) = self
            .filtered_findings()
            .get(self.selected())
            .map(|f| (*f).clone())
        else {
            return;
        };
        if let Some(pid) = finding.related_pids.first() {
            self.open_session(*pid);
            return;
        }
        let id = finding.id.as_str();
        let relation = [
            "dead-tuples:",
            "analyze-drift:",
            "xid-age:table:",
            "invalid-index:",
        ]
        .iter()
        .find_map(|prefix| id.strip_prefix(prefix).and_then(|v| v.parse::<i64>().ok()));
        if let Some(oid) = relation {
            let indexes = id.starts_with("invalid-index:");
            let exists = self
                .snapshot()
                .and_then(|s| s.health.tables.available())
                .is_some_and(|r| {
                    if indexes {
                        r.indexes.iter().any(|i| i.oid == oid)
                    } else {
                        r.tables.iter().any(|t| t.oid == oid)
                    }
                });
            if exists {
                self.evidence_view(Tab::Relations);
                self.show_indexes = indexes;
                self.selected[Tab::Relations.index()] = self
                    .identities(Tab::Relations)
                    .iter()
                    .position(|id| *id == Identity::Relation(oid))
                    .unwrap_or(0);
                return;
            }
        }
        let statement = ["statement-share:", "statement-latency:"]
            .iter()
            .find_map(|prefix| id.strip_prefix(prefix));
        if let Some(statement) = statement {
            let parts: Vec<_> = statement.split(':').collect();
            if let [user, database, query, top] = parts.as_slice()
                && let (Ok(userid), Ok(dbid), Ok(queryid), Ok(toplevel)) =
                    (user.parse(), database.parse(), query.parse(), top.parse())
            {
                let identity = StatementIdentity {
                    userid,
                    dbid,
                    queryid,
                    toplevel,
                };
                if self.statement_for_identity(&identity).is_some() {
                    self.evidence_view(Tab::Statements);
                    self.statement_interval = false;
                    self.selected[Tab::Statements.index()] = self
                        .identities(Tab::Statements)
                        .iter()
                        .position(|id| *id == Identity::Statement(identity.clone()))
                        .unwrap_or(0);
                    return;
                }
            }
        }
        self.notice = Some("This finding has no reliable object link in the observed data. Read its evidence and next steps with Enter.".into());
    }

    pub(super) fn follow_session(&mut self, blocker: bool) {
        if self.tab == Tab::Blocking {
            if let Some((waiter, pid)) = self.blocking_edges().get(self.selected()) {
                self.open_session(if blocker { *pid } else { waiter.pid });
            }
            return;
        }
        let Some(session) = self.filtered_sessions().get(self.selected()).copied() else {
            return;
        };
        let pid = session.pid;
        // The blocking view exposes every relationship, including multiple waiters/blockers.
        let has_edges = self
            .snapshot()
            .and_then(|s| s.activity.available())
            .is_some_and(|sessions| {
                sessions.iter().any(|s| {
                    if blocker {
                        s.pid == pid && !s.blockers.is_empty()
                    } else {
                        s.blockers.contains(&pid)
                    }
                })
            });
        if has_edges {
            self.evidence_view(Tab::Blocking);
            self.selected[Tab::Blocking.index()] = self
                .blocking_edges()
                .iter()
                .position(|(s, b)| if blocker { s.pid == pid } else { *b == pid })
                .unwrap_or(0);
        } else {
            self.notice = Some(format!(
                "No {} relationship observed for PID {pid}.",
                if blocker { "blocker" } else { "waiter" }
            ));
        }
    }

    pub(super) fn open_coverage(&mut self) {
        if let Some(analysis) = &self.analysis {
            self.set_report(format!(
                "# Coverage and limits\n\n{}",
                analysis.coverage.join("\n\n")
            ));
            self.report_title = "Coverage and limits";
        }
    }

    pub(super) fn report_section(&mut self, next: bool) {
        let Some(report) = &self.report else {
            return;
        };
        let lines = crate::ui::report_lines(report, self.viewport_width.saturating_sub(2));
        let headings: Vec<_> = lines
            .iter()
            .enumerate()
            .filter_map(|(i, line)| line.starts_with('#').then_some(i))
            .collect();
        self.report_scroll = if next {
            headings
                .into_iter()
                .find(|i| *i > self.report_scroll)
                .unwrap_or(self.report_scroll)
        } else {
            headings
                .into_iter()
                .rev()
                .find(|i| *i < self.report_scroll)
                .unwrap_or(0)
        };
        self.clamp_scroll();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{event::Message, model::Observation};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(app: &mut App, code: KeyCode) -> Option<Action> {
        app.update(Message::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }
    fn sample(sequence: u64) -> Snapshot {
        let mut s = crate::demo::snapshot(sequence, true);
        s.started_at =
            chrono::DateTime::from_timestamp(1_790_000_000 + sequence as i64 * 5, 0).unwrap();
        s.completed_at = s.started_at;
        s
    }
    fn filter(app: &mut App, text: &str) {
        key(app, KeyCode::Char('/'));
        for c in text.chars() {
            key(app, KeyCode::Char(c));
        }
        key(app, KeyCode::Enter);
    }

    #[test]
    fn refresh_tracks_backend_start_and_reports_disappearance_and_pid_reuse() {
        let first = sample(0);
        let mut app = App::new(true);
        app.set_snapshot(first.clone(), false);
        key(&mut app, KeyCode::Char('2'));
        key(&mut app, KeyCode::Down);
        let chosen = app.filtered_sessions()[app.selected()].clone();
        let mut next = first.clone();
        if let Observation::Available(rows) = &mut next.activity {
            rows.iter_mut()
                .find(|s| s.pid == chosen.pid)
                .unwrap()
                .query_age_ms = Some(i64::MAX);
            rows.reverse();
        }
        app.set_snapshot(next.clone(), false);
        assert_eq!(app.filtered_sessions()[app.selected()].pid, chosen.pid);
        assert_eq!(app.selected(), 0);
        if let Observation::Available(rows) = &mut next.activity {
            rows.iter_mut()
                .find(|s| s.pid == chosen.pid)
                .unwrap()
                .backend_start = Some(next.completed_at);
        }
        app.set_snapshot(next.clone(), false);
        assert!(
            app.notice
                .as_ref()
                .unwrap()
                .contains(&format!("PID {} (no longer visible)", chosen.pid))
        );
        if let Observation::Available(rows) = &mut next.activity {
            rows.retain(|s| s.pid != chosen.pid);
        }
        app.set_snapshot(next, false);
        assert!(app.notice.as_ref().unwrap().contains("no longer visible"));
        assert_ne!(app.filtered_sessions()[app.selected()].pid, chosen.pid);
    }

    #[test]
    fn refresh_preserves_offscreen_statement_relation_and_finding_identities() {
        let mut app = App::new(true);
        let mut first = sample(0);
        if let Observation::Available(stats) = &mut first.statements {
            let mut duplicate_query = stats.entries[0].clone();
            duplicate_query.userid += 1;
            duplicate_query.dbid += 1;
            duplicate_query.toplevel = !duplicate_query.toplevel;
            duplicate_query.calls = 1;
            duplicate_query.total_exec_ms = 1.0;
            stats.entries.push(duplicate_query);
        }
        app.set_snapshot(first.clone(), false);
        key(&mut app, KeyCode::Char('4'));
        key(&mut app, KeyCode::End);
        let statement = statement_identity(app.filtered_statements()[app.selected()]);
        key(&mut app, KeyCode::Char('7'));
        key(&mut app, KeyCode::Down);
        let relation = app.filtered_tables()[app.selected()].oid;
        key(&mut app, KeyCode::Char('1'));
        let finding = app.filtered_findings()[app.selected()].id.clone();
        let mut next = first;
        if let Observation::Available(stats) = &mut next.statements {
            stats
                .entries
                .iter_mut()
                .find(|s| statement_identity(s) == statement)
                .unwrap()
                .total_exec_ms = 1e12;
        }
        if let Observation::Available(stats) = &mut next.health.tables {
            stats
                .tables
                .iter_mut()
                .find(|s| s.oid == relation)
                .unwrap()
                .total_bytes = i64::MAX;
        }
        app.set_snapshot(next, false);
        assert_eq!(app.filtered_findings()[app.selected()].id, finding);
        key(&mut app, KeyCode::Char('4'));
        assert_eq!(
            statement_identity(app.filtered_statements()[app.selected()]),
            statement
        );
        key(&mut app, KeyCode::Char('7'));
        assert_eq!(app.filtered_tables()[app.selected()].oid, relation);
    }

    #[test]
    fn blocking_reordering_keeps_both_backend_identities_and_rejects_reuse() {
        let mut app = App::new(true);
        let mut next = sample(0);
        app.set_snapshot(next.clone(), false);
        key(&mut app, KeyCode::Char('3'));
        let (waiter, blocker) = app.blocking_edges()[app.selected()];
        let waiter_pid = waiter.pid;
        if let Observation::Available(rows) = &mut next.activity {
            rows.reverse();
        }
        app.set_snapshot(next.clone(), false);
        assert_eq!(app.blocking_edges()[app.selected()].0.pid, waiter_pid);
        assert_eq!(app.blocking_edges()[app.selected()].1, blocker);
        if let Observation::Available(rows) = &mut next.activity {
            rows.iter_mut()
                .find(|s| s.pid == blocker)
                .unwrap()
                .backend_start = Some(next.completed_at);
        }
        app.set_snapshot(next, false);
        assert!(app.notice.as_ref().unwrap().contains(&format!(
            "Blocking: PID {waiter_pid} waiting for {blocker} (no longer visible)"
        )));
    }

    #[test]
    fn ambiguous_full_identities_do_not_silently_attach_selection_to_first_match() {
        let mut app = App::new(true);
        let mut next = sample(0);
        app.set_snapshot(next.clone(), false);
        key(&mut app, KeyCode::Char('4'));
        if let Observation::Available(stats) = &mut next.statements {
            stats.entries.push(stats.entries[0].clone());
        }
        app.set_snapshot(next, false);
        assert!(
            app.notice
                .as_ref()
                .unwrap()
                .contains("identity is ambiguous")
        );
        assert_eq!(app.selected(), 0);
    }

    #[test]
    fn unknown_backend_start_never_asserts_stable_identity() {
        let mut s = sample(0);
        if let Observation::Available(rows) = &mut s.activity {
            for row in rows {
                row.backend_start = None;
            }
        }
        let mut app = App::new(true);
        app.set_snapshot(s.clone(), false);
        key(&mut app, KeyCode::Char('2'));
        app.set_snapshot(s, false);
        assert!(
            app.notice
                .as_ref()
                .unwrap()
                .contains("backend identity unavailable")
        );
    }

    #[test]
    fn filters_are_scoped_and_sql_matches_in_both_statement_modes() {
        let mut app = App::new(true);
        app.set_snapshot(sample(0), false);
        app.set_snapshot(sample(1), false);
        filter(&mut app, "checkout");
        key(&mut app, KeyCode::Char('4'));
        assert!(app.filter.is_empty());
        let sql = app.filtered_statements()[0].query.as_ref().unwrap().clone();
        filter(&mut app, &sql);
        let ids: Vec<_> = app
            .filtered_statements()
            .iter()
            .map(|s| statement_identity(s))
            .collect();
        assert!(!ids.is_empty());
        key(&mut app, KeyCode::Char('v'));
        assert_eq!(
            app.filtered_statement_rates()
                .iter()
                .map(|s| s.identity.clone())
                .collect::<Vec<_>>(),
            ids
        );
        key(&mut app, KeyCode::Char('6'));
        assert!(app.filter.is_empty());
        key(&mut app, KeyCode::Char('/'));
        assert!(!app.editing_filter);
        key(&mut app, KeyCode::Char('1'));
        assert_eq!(app.filter, "checkout");
        key(&mut app, KeyCode::Char('4'));
        assert_eq!(app.filter, sql);
    }

    #[test]
    fn root_blocker_waiter_and_return_preserve_observed_evidence() {
        let mut app = App::new(true);
        app.set_snapshot(sample(0), false);
        let index = app
            .filtered_findings()
            .iter()
            .position(|f| f.id.starts_with("blocking-root:"))
            .unwrap();
        app.selected[Tab::Overview.index()] = index;
        let finding = app.filtered_findings()[index].clone();
        key(&mut app, KeyCode::Enter);
        key(&mut app, KeyCode::PageDown);
        let scroll = app.report_scroll;
        key(&mut app, KeyCode::Char('e'));
        assert_eq!(app.tab, Tab::Activity);
        let root = app.filtered_sessions()[app.selected()].pid;
        assert_eq!(root, finding.related_pids[0]);
        assert!(
            app.is_offline(),
            "contextual navigation freezes one observed sample"
        );
        key(&mut app, KeyCode::Char('w'));
        assert_eq!(app.tab, Tab::Blocking);
        let (waiter, blocker) = app.blocking_edges()[app.selected()];
        assert_eq!(blocker, root);
        let waiter = waiter.pid;
        key(&mut app, KeyCode::Char('w'));
        assert_eq!(app.filtered_sessions()[app.selected()].pid, waiter);
        for _ in 0..3 {
            key(&mut app, KeyCode::Backspace);
        }
        assert_eq!(app.tab, Tab::Overview);
        assert_eq!(app.filtered_findings()[app.selected()].id, finding.id);
        assert_eq!(app.report_title, "Finding details");
        assert_eq!(app.report_scroll, scroll);
    }

    #[test]
    fn related_relation_and_complete_statement_identity_open_without_typing_ids() {
        let mut app = App::new(true);
        app.set_snapshot(sample(0), false);
        let mut next = sample(1);
        if let Observation::Available(stats) = &mut next.statements {
            stats.entries[0].total_exec_ms += 10_000.0;
        }
        app.set_snapshot(next, false);
        let relation = app
            .filtered_findings()
            .iter()
            .position(|f| f.id.starts_with("dead-tuples:"))
            .unwrap();
        app.selected[Tab::Overview.index()] = relation;
        let oid = app.filtered_findings()[relation]
            .id
            .strip_prefix("dead-tuples:")
            .unwrap()
            .parse::<i64>()
            .unwrap();
        key(&mut app, KeyCode::Char('e'));
        assert_eq!(app.tab, Tab::Relations);
        assert_eq!(app.filtered_tables()[app.selected()].oid, oid);
        key(&mut app, KeyCode::Backspace);
        let statement = app
            .filtered_findings()
            .iter()
            .position(|f| f.id.starts_with("statement-share:"))
            .unwrap();
        app.selected[Tab::Overview.index()] = statement;
        let id = app.filtered_findings()[statement].id.clone();
        key(&mut app, KeyCode::Char('e'));
        assert_eq!(app.tab, Tab::Statements);
        let s = app.filtered_statements()[app.selected()];
        assert_eq!(
            id,
            format!(
                "statement-share:{}:{}:{}:{}",
                s.userid, s.dbid, s.queryid, s.toplevel
            )
        );
    }

    #[test]
    fn long_sql_help_and_sections_use_display_rows_at_both_terminal_sizes() {
        for (width, height) in [(80, 24), (120, 36)] {
            let mut app = App::new(true);
            app.viewport_width = width;
            app.viewport_height = height;
            let mut s = sample(0);
            if let Observation::Available(rows) = &mut s.activity {
                for row in rows {
                    row.query = Some(format!(
                        "{}SQL_END_SENTINEL",
                        "long expression ".repeat(600)
                    ));
                }
            }
            app.set_snapshot(s, false);
            key(&mut app, KeyCode::Char('2'));
            key(&mut app, KeyCode::Enter);
            key(&mut app, KeyCode::End);
            let lines = crate::ui::report_lines(app.report.as_ref().unwrap(), width - 2);
            assert!(
                lines[app.report_scroll..]
                    .join(" ")
                    .contains("SQL_END_SENTINEL")
            );
            let end = app.report_scroll;
            key(&mut app, KeyCode::Up);
            assert_eq!(app.report_scroll, end - 1, "Up moves immediately after End");
            app.viewport_width = 120;
            app.viewport_height = 36;
            app.clamp_scroll();
            assert!(app.report_scroll <= app.max_report_scroll());
            key(&mut app, KeyCode::Home);
            key(&mut app, KeyCode::Char(']'));
            assert!(app.report_scroll > 0);
            key(&mut app, KeyCode::Char('?'));
            key(&mut app, KeyCode::End);
            assert!(app.help_scroll > 0);
            key(&mut app, KeyCode::Home);
            assert_eq!(app.help_scroll, 0);
        }
    }
}
