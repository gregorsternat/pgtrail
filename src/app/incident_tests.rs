use super::*;
use crate::{
    incidents::{TimelineId, timeline},
    store::{Incident, IncidentNote, IncidentSummary},
};
use chrono::Duration;

fn key(app: &mut App, code: KeyCode) -> Option<Action> {
    app.update(Message::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}

fn incident() -> Incident {
    let at = crate::demo::snapshot(0, false).completed_at;
    Incident {
        summary: IncidentSummary {
            id: 9,
            title: "Checkout queue".into(),
            created_at: at,
            closed_at: None,
            capture_count: 7,
            note_count: 8,
        },
        notes: (0..8)
            .map(|index| IncidentNote {
                created_at: at + Duration::seconds(index * 2),
                text: format!("Note {index}: {} NOTE_END_{index}", "evidence ".repeat(400)),
            })
            .collect(),
        captures: (1..=7)
            .map(|id| SnapshotSummary {
                id,
                label: format!("Captured observation {id}"),
                captured_at: at + Duration::seconds(id * 2 - 1),
                source: "Synthetic local fixture".into(),
                complete: true,
            })
            .collect(),
        capture_notes: Default::default(),
    }
}

fn app() -> App {
    let mut app = App::new(true);
    app.set_snapshot(crate::demo::snapshot(0, false), false);
    let incident = incident();
    app.set_incidents(vec![incident.summary.clone()]);
    app.set_incident(incident);
    app.tab = Tab::Incidents;
    app
}

fn selected_id(app: &App) -> TimelineId {
    timeline(app.incident.as_ref().unwrap())[app.incident_cursor]
        .id
        .clone()
}

#[test]
fn full_chronology_navigation_and_refresh_keep_selected_event() {
    let mut app = app();
    assert_eq!(timeline(app.incident.as_ref().unwrap()).len(), 15);
    key(&mut app, KeyCode::End);
    assert_eq!(app.incident_cursor, 14);
    key(&mut app, KeyCode::PageUp);
    assert_eq!(app.incident_cursor, 4);
    key(&mut app, KeyCode::Home);
    assert_eq!(app.incident_cursor, 0);
    key(&mut app, KeyCode::Down);
    assert_eq!(selected_id(&app), TimelineId::Capture(1));
    let mut updated = app.incident.clone().unwrap();
    updated.notes.insert(
        0,
        IncidentNote {
            created_at: updated.summary.created_at - Duration::seconds(1),
            text: "Earlier event attached while reviewing".into(),
        },
    );
    updated.captures.reverse();
    app.update_incident(updated);
    assert_eq!(app.incident_cursor, 2);
    assert_eq!(selected_id(&app), TimelineId::Capture(1));
    assert!(!app.filter_available());
    key(&mut app, KeyCode::Char('/'));
    assert!(!app.editing_filter);
}

#[test]
fn old_long_note_is_readable_to_end_at_both_terminal_sizes() {
    use ratatui::{Terminal, backend::TestBackend};
    for (width, height) in [(120, 36), (80, 24)] {
        let mut app = app();
        app.viewport_width = width;
        app.viewport_height = height;
        key(&mut app, KeyCode::Enter);
        assert_eq!(app.report_title, "Incident note");
        assert_eq!(
            app.report.as_ref().unwrap().matches("NOTE_END_0").count(),
            1
        );
        key(&mut app, KeyCode::End);
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, &app))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let screen: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert!(
            screen.contains("NOTE_END_0"),
            "note tail missing at {width}x{height}"
        );
        key(&mut app, KeyCode::Esc);
        assert!(app.report.is_none());
        assert!(app.incident_timeline);
        assert_eq!(app.incident_cursor, 0);
    }
}

#[test]
fn attached_capture_opens_and_returns_to_exact_timeline_selection() {
    let mut app = app();
    app.filters[Tab::Overview.index()] = "old overview search".into();
    key(&mut app, KeyCode::Down);
    assert_eq!(key(&mut app, KeyCode::Enter), Some(Action::Load(1)));
    assert!(app.has_return_path());
    app.set_snapshot(crate::demo::snapshot(1, false), true);
    assert_eq!(app.tab, Tab::Overview);
    assert!(app.is_offline());
    key(&mut app, KeyCode::Esc);
    assert_eq!(app.tab, Tab::Incidents);
    assert!(app.incident_timeline);
    assert_eq!(selected_id(&app), TimelineId::Capture(1));
    assert!(!app.has_return_path());
    key(&mut app, KeyCode::Esc);
    assert!(!app.incident_timeline);
}

#[test]
fn export_destination_editor_preserves_quit_characters_and_selected_target() {
    let mut app = app();
    for (shortcut, json, destination) in [
        ('e', false, "/tmp/queue incident.md"),
        ('E', true, "/tmp/queue incident.json"),
    ] {
        key(&mut app, KeyCode::Char(shortcut));
        for character in destination.chars() {
            key(&mut app, KeyCode::Char(character));
        }
        assert!(!app.should_quit());
        assert_eq!(
            key(&mut app, KeyCode::Enter),
            Some(Action::ExportIncident(9, json, destination.into()))
        );
        assert!(app.prompt.is_none());
    }
    key(&mut app, KeyCode::Char('e'));
    key(&mut app, KeyCode::Esc);
    assert!(app.prompt.is_none());
    key(&mut app, KeyCode::Char('E'));
    assert_eq!(key(&mut app, KeyCode::Enter), None);
    assert!(app.notice.as_ref().unwrap().contains("empty"));
    assert!(!app.should_quit());
}

#[test]
fn export_from_list_uses_selected_incident_not_loaded_timeline() {
    let mut app = app();
    let mut second = incident().summary;
    second.id = 11;
    second.title = "Another investigation".into();
    app.set_incidents(vec![second, incident().summary]);
    key(&mut app, KeyCode::Esc);
    key(&mut app, KeyCode::Home);
    key(&mut app, KeyCode::Char('e'));
    assert!(matches!(
        app.prompt.as_ref().map(|p| p.kind),
        Some(PromptKind::ExportIncident(11, false))
    ));
}

#[test]
fn returning_from_capture_restores_original_incident_without_changing_new_active_target() {
    use ratatui::{Terminal, backend::TestBackend};
    let mut app = app();
    key(&mut app, KeyCode::Down);
    assert_eq!(key(&mut app, KeyCode::Enter), Some(Action::Load(1)));
    app.set_snapshot(crate::demo::snapshot(1, false), true);
    key(&mut app, KeyCode::Char('i'));
    for character in "Other incident".chars() {
        key(&mut app, KeyCode::Char(character));
    }
    assert_eq!(
        key(&mut app, KeyCode::Enter),
        Some(Action::CreateIncident("Other incident".into()))
    );
    let mut created = incident();
    created.summary.id = 12;
    created.summary.title = "Other incident".into();
    created.notes.clear();
    created.captures.clear();
    created.capture_notes.clear();
    created.summary.capture_count = 0;
    created.summary.note_count = 0;
    app.set_incident(created);
    assert_eq!(app.active_incident, Some(12));
    assert_eq!(
        key(&mut app, KeyCode::Backspace),
        Some(Action::ListIncidents)
    );
    assert_eq!(app.tab, Tab::Incidents);
    assert_eq!(app.incident.as_ref().unwrap().summary.id, 9);
    assert_eq!(selected_id(&app), TimelineId::Capture(1));
    assert_eq!(app.active_incident, Some(12));
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| crate::ui::render(frame, &app))
        .unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("#9: Checkout queue"));
    assert!(!screen.contains("#12: Other incident"));
}

#[test]
fn failed_capture_load_removes_pending_return_path_and_resumes_live_eligibility() {
    let mut app = app();
    key(&mut app, KeyCode::Down);
    assert_eq!(key(&mut app, KeyCode::Enter), Some(Action::Load(1)));
    app.pending_capture_request = Some(41);
    assert!(app.has_return_path());
    assert!(app.is_offline());
    app.update(Message::Redraw);
    assert_eq!(app.pending_capture_request, Some(41));
    assert!(app.has_return_path());
    app.discard_pending_capture_return(41);
    assert_eq!(app.pending_capture_request, None);
    assert!(!app.has_return_path());
    assert!(!app.is_offline());
    assert!(app.incident_timeline);
    assert_eq!(selected_id(&app), TimelineId::Capture(1));
    assert_eq!(key(&mut app, KeyCode::Backspace), None);
}

#[test]
fn canceled_capture_receipt_never_removes_a_newer_capture_return_path() {
    let mut app = app();
    key(&mut app, KeyCode::Down);
    assert_eq!(key(&mut app, KeyCode::Enter), Some(Action::Load(1)));
    app.pending_capture_request = Some(41);
    // The next key cancels the pending load before changing selection; the
    // older async receipt may arrive after another load has already started.
    key(&mut app, KeyCode::Down);
    assert_eq!(app.pending_capture_request, None);
    assert!(!app.has_return_path());
    key(&mut app, KeyCode::Down);
    assert_eq!(key(&mut app, KeyCode::Enter), Some(Action::Load(2)));
    app.pending_capture_request = Some(42);
    app.discard_pending_capture_return(41);
    assert_eq!(app.pending_capture_request, Some(42));
    assert!(app.has_return_path());
    assert_eq!(selected_id(&app), TimelineId::Capture(2));
    app.discard_pending_capture_return(42);
    assert_eq!(app.pending_capture_request, None);
    assert!(!app.has_return_path());
    assert!(!app.is_offline());
}
