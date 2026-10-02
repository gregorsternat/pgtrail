//! Opt-in visual evidence from the actual renderer, using synthetic observations.
use super::*;
use crate::{
    app::{CommandId, Focus},
    event::Message,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};

#[test]
#[ignore = "writes synthetic review buffers under PGTRAIL_REVIEW_DIR"]
fn export_review_frames() -> anyhow::Result<()> {
    let Ok(directory) = std::env::var("PGTRAIL_REVIEW_DIR") else {
        return Ok(());
    };
    let directory = std::path::PathBuf::from(directory);
    std::fs::create_dir_all(&directory)?;
    for (width, height) in [(80, 24), (120, 36), (160, 48)] {
        let mut app = App::new(true);
        app.viewport_width = width;
        app.viewport_height = height;
        let mut before = crate::demo::snapshot(0, true);
        let mut after = crate::demo::snapshot(1, true);
        before.started_at = chrono::Utc::now() - chrono::Duration::seconds(6);
        before.completed_at = before.started_at;
        after.started_at = before.completed_at + chrono::Duration::seconds(5);
        after.completed_at = after.started_at;
        let comparison =
            crate::report::comparison_markdown(&crate::compare::compare(&before, &after));
        app.set_snapshot(before, false);
        app.set_snapshot(after, false);
        app.now = app.snapshot().unwrap().completed_at + chrono::Duration::seconds(2);
        app.history = (1..=2)
            .map(|id| crate::store::SnapshotSummary {
                id,
                label: if id == 1 {
                    "Before pool adjustment"
                } else {
                    "After pool adjustment"
                }
                .into(),
                captured_at: app.now - chrono::Duration::seconds((3 - id) * 5),
                source: "synthetic demo / demo_shop".into(),
                complete: true,
            })
            .collect();
        app.compare_a = Some(1);
        app.compare_b = Some(2);
        let summary = crate::store::IncidentSummary {
            id: 1,
            title: "Checkout latency".into(),
            created_at: app.now - chrono::Duration::minutes(12),
            closed_at: None,
            capture_count: 2,
            note_count: 1,
        };
        app.incidents = vec![summary.clone()];
        app.active_incident = Some(1);
        app.incident = Some(crate::store::Incident {
            summary,
            notes: vec![crate::store::IncidentNote {
                created_at: app.now - chrono::Duration::minutes(10),
                text: "Investigating the oldest blocking transaction".into(),
            }],
            captures: app.history.clone(),
            capture_notes: [(1, "Captured before adjusting the pool.\nThe oldest transaction still blocks checkout writes.".into())].into_iter().collect(),
        });
        for (name, tab) in [
            ("overview", Tab::Overview),
            ("activity", Tab::Activity),
            ("blocking", Tab::Blocking),
            ("statements", Tab::Statements),
            ("captures", Tab::History),
            ("database", Tab::Database),
            ("relations", Tab::Relations),
            ("replication", Tab::Replication),
            ("io", Tab::Io),
            ("incidents", Tab::Incidents),
        ] {
            app.tab = tab;
            write_frame(&directory, name, &mut app)?;
        }
        app.incident_timeline = true;
        write_frame(&directory, "chronology", &mut app)?;
        app.incident_cursor = 1;
        app.execute(CommandId::CaptureDetails);
        write_frame(&directory, "capture-details", &mut app)?;
        app.report = None;
        app.tab = Tab::History;
        app.set_report(comparison);
        app.comparison = Some(crate::app::ComparisonContext {
            before: app.history[0].clone(),
            after: app.history[1].clone(),
            synthetic: true,
        });
        write_frame(&directory, "comparison", &mut app)?;
        app.report = None;
        app.tab = Tab::Overview;
        app.execute(CommandId::Inspect);
        write_frame(&directory, "inspector", &mut app)?;
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        app.execute(CommandId::Commands);
        write_frame(&directory, "commands", &mut app)?;
        app.update(Message::Key(KeyEvent::new(
            KeyCode::Esc,
            KeyModifiers::NONE,
        )));
        app.theme = Theme::Terminal;
        app.focus = Focus::Content;
        write_frame(&directory, "terminal-theme", &mut app)?;
        app.theme = Theme::Dark;
        app.execute(CommandId::NewIncident);
        app.prompt.as_mut().unwrap().value = "Checkout latency".into();
        write_frame(&directory, "editor", &mut app)?;
    }
    Ok(())
}

fn write_frame(directory: &std::path::Path, name: &str, app: &mut App) -> anyhow::Result<()> {
    app.clamp_scroll();
    let mut terminal = Terminal::new(TestBackend::new(app.viewport_width, app.viewport_height))?;
    terminal.draw(|frame| render(frame, app))?;
    let cells = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| {
            serde_json::json!({
                "text": cell.symbol(), "fg": color(cell.fg), "bg": color(cell.bg),
                "bold": cell.modifier.contains(ratatui::style::Modifier::BOLD),
            })
        })
        .collect::<Vec<_>>();
    let path = directory.join(format!(
        "{name}-{}x{}.json",
        app.viewport_width, app.viewport_height
    ));
    std::fs::write(
        path,
        serde_json::to_vec(
            &serde_json::json!({"width": app.viewport_width, "height": app.viewport_height, "cells": cells}),
        )?,
    )?;
    Ok(())
}

fn color(color: Color) -> String {
    match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Reset => "reset".into(),
        Color::Cyan => "#008b9a".into(),
        Color::Blue => "#174b85".into(),
        Color::Red => "#c12d38".into(),
        Color::Yellow => "#896700".into(),
        Color::Green => "#22813d".into(),
        Color::Magenta => "#a43b99".into(),
        Color::White => "#ffffff".into(),
        _ => "#687082".into(),
    }
}
