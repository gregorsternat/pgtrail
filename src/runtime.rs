use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use crossterm::event::EventStream;
use tokio::{task::JoinSet, time::Instant};

use crate::{
    app::{self, Action},
    cli::Cli,
    collector, compare, demo, event,
    model::Snapshot,
    report,
    store::{Incident, IncidentSummary, SnapshotSummary, Store},
    terminal, ui,
};

enum Work {
    Collected(Result<Snapshot>),
    History(u64, Result<Vec<SnapshotSummary>>),
    Saved(Result<(i64, Option<i64>)>),
    Loaded(u64, Result<Snapshot>),
    Compared(u64, Result<String>),
    Incidents(u64, Result<Vec<IncidentSummary>>),
    SelectedIncident(u64, Result<Incident>),
    UpdatedIncident(u64, i64, Result<Incident>),
    Changed(u64, Result<Change>),
}

struct Change {
    notice: String,
    created_incident: Option<Incident>,
}

#[derive(Default)]
struct Refreshes {
    history: u64,
    incidents: u64,
    detail: u64,
}

fn refresh_history(work: &mut JoinSet<Work>, path: &Path, requests: &mut Refreshes) {
    requests.history = requests.history.wrapping_add(1);
    let request = requests.history;
    let path = path.to_owned();
    work.spawn(async move {
        Work::History(
            request,
            async { Store::open(&path).await?.list().await }.await,
        )
    });
}

fn refresh_incidents(
    work: &mut JoinSet<Work>,
    path: &Path,
    displayed: Option<i64>,
    requests: &mut Refreshes,
) {
    requests.incidents = requests.incidents.wrapping_add(1);
    requests.detail = requests.detail.wrapping_add(1);
    let request = requests.incidents;
    let detail_request = requests.detail;
    let path = path.to_owned();
    let detail_path = path.clone();
    work.spawn(async move {
        Work::Incidents(
            request,
            async { Store::open(&path).await?.incidents().await }.await,
        )
    });
    if let Some(id) = displayed {
        work.spawn(async move {
            Work::UpdatedIncident(
                detail_request,
                id,
                async { Store::open(&detail_path).await?.incident(id).await }.await,
            )
        });
    }
}

async fn change(path: PathBuf, action: Action) -> Result<Change> {
    let store = Store::open(&path).await?;
    let mut created_incident = None;
    let notice = match action {
        Action::CreateIncident(title) => {
            let id = store.create_incident(&title).await?;
            // Creating the incident succeeded even if the follow-up read fails.
            created_incident = store.incident(id).await.ok();
            format!("Created incident #{id}. Select it with 0 then Enter to attach captures.")
        }
        Action::NoteIncident(id, text) => {
            store.add_note(id, &text).await?;
            format!("Added note to incident #{id}.")
        }
        Action::CloseIncident(id, closed) => {
            store.set_incident_closed(id, closed).await?;
            format!(
                "{} incident #{id}.",
                if closed { "Closed" } else { "Reopened" }
            )
        }
        Action::AttachCapture(id, capture) => {
            store.attach_capture(id, capture).await?;
            format!("Attached capture #{capture} to incident #{id}.")
        }
        Action::LabelCapture(id, label) => {
            store.annotate_capture(id, Some(&label), None).await?;
            format!("Updated capture #{id} label.")
        }
        _ => anyhow::bail!("unsupported local change"),
    };
    Ok(Change {
        notice,
        created_incident,
    })
}

pub(crate) async fn run(cli: &Cli) -> Result<()> {
    let collector = collector(cli).await?;
    let path = cli.store_path()?;
    let mut app = app::App::new(cli.demo);
    if let Some(id) = cli.incident {
        let incident = Store::open(&path).await?.incident(id).await?;
        anyhow::ensure!(
            incident.summary.closed_at.is_none(),
            "incident #{id} is closed; reopen it before using it for new captures"
        );
        app.set_incident(incident);
    }
    let mut terminal = terminal::Session::start()?;
    let mut events = EventStream::new();
    if collector.is_none() && !cli.demo {
        app.set_loading(false);
    }
    let mut work = JoinSet::new();
    let mut requests = Refreshes::default();
    let mut refreshing = false;
    let mut saving = false;
    // Outer Option means a capture is queued; inner Option freezes its incident at keypress.
    let mut capture_requested: Option<Option<i64>> = None;
    let mut capture_in_flight: Option<Option<i64>> = None;
    let mut view_generation = 0_u64;
    let mut last_live: Option<Snapshot> = None;
    let mut last_live_error: Option<String> = None;
    let mut demo_sequence = 0_u64;
    let refresh_period = Duration::from_secs(cli.refresh);
    let mut last_refresh = Instant::now() - refresh_period;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    refresh_history(&mut work, &path, &mut requests);
    refresh_incidents(&mut work, &path, None, &mut requests);

    while !app.should_quit() {
        terminal
            .terminal
            .draw(|frame| ui::render(frame, &app))
            .context("could not draw the terminal")?;
        let mut refresh_requested = false;
        tokio::select! {
            message = event::next(&mut events) => {
                let message = message?;
                if matches!(message, event::Message::Key(_)) { view_generation = view_generation.wrapping_add(1); }
                if let Some(action) = app.update(message) {
                    match action {
                        Action::Refresh => refresh_requested = true,
                        Action::Capture => {
                            if app.is_offline() {
                                app.set_notice("Return to live mode with Esc before capturing.".into());
                            } else if saving || capture_requested.is_some() || capture_in_flight.is_some() {
                                app.set_notice("A capture is already in progress.".into());
                            } else if collector.is_none() && !cli.demo {
                                app.set_notice("Configure a connection profile or restart with --demo to capture.".into());
                            } else {
                                capture_requested = Some(app.active_incident);
                                refresh_requested = true;
                                app.set_notice(match app.active_incident { Some(id) => format!("Collecting a fresh capture for incident #{id}…"), None => "Collecting a fresh manual capture…".into() });
                            }
                        }
                        Action::Load(id) => {
                            let path = path.clone();
                            work.spawn(async move { Work::Loaded(view_generation, async { Store::open(&path).await?.load(id).await }.await) });
                        }
                        Action::Compare(before, after) => {
                            let path = path.clone();
                            work.spawn(async move { Work::Compared(view_generation, async {
                                let store = Store::open(&path).await?;
                                let before = store.load(before).await?;
                                let after = store.load(after).await?;
                                Ok(report::comparison_markdown(&compare::compare(&before, &after)))
                            }.await) });
                        }
                        Action::ListHistory => refresh_history(&mut work, &path, &mut requests),
                        Action::ListIncidents => refresh_incidents(&mut work, &path, app.incident.as_ref().map(|v| v.summary.id), &mut requests),
                        Action::SelectIncident(id) => {
                            let path = path.clone();
                            work.spawn(async move { Work::SelectedIncident(view_generation, async { Store::open(&path).await?.incident(id).await }.await) });
                        }
                        Action::CreateIncident(_) | Action::NoteIncident(_, _) | Action::CloseIncident(_, _) | Action::AttachCapture(_, _) | Action::LabelCapture(_, _) => {
                            let path = path.clone();
                            work.spawn(async move { Work::Changed(view_generation, change(path, action).await) });
                        }
                        Action::ResumeLive => {
                            if let Some(snapshot) = &last_live { app.set_snapshot(snapshot.clone(), false); }
                            if let Some(error) = &last_live_error { app.set_error(error.clone()); }
                            refresh_requested = true;
                        }
                    }
                }
            }
            _ = tick.tick() => {
                app.update(event::Message::Redraw);
                refresh_requested = !app.paused() && !app.is_offline() && last_refresh.elapsed() >= refresh_period;
            }
            result = work.join_next(), if !work.is_empty() => {
                match result {
                    Some(Ok(Work::Collected(result))) => {
                        refreshing = false;
                        let capture_target = capture_in_flight.take();
                        refresh_requested |= capture_requested.is_some();
                        app.set_loading(false);
                        match result {
                            Ok(snapshot) => {
                                last_live_error = None;
                                if let Some(incident) = capture_target {
                                    saving = true;
                                    let path = path.clone();
                                    let snapshot = snapshot.clone();
                                    work.spawn(async move { Work::Saved(async {
                                        let store = Store::open(&path).await?;
                                        store.save_for_incident(&snapshot, "Manual TUI capture", incident).await.map(|id| (id, incident))
                                    }.await) });
                                }
                                if !app.is_offline() { app.set_snapshot(snapshot.clone(), false); }
                                last_live = Some(snapshot);
                            }
                            Err(error) => {
                                last_live_error = Some(error.to_string());
                                if capture_target.is_some() { app.set_notice("Capture failed; no snapshot was saved.".into()); }
                                if app.is_offline() { app.set_notice(format!("Background live refresh failed: {error}")); }
                                else { app.collection_failed(error.to_string()); }
                            }
                        }
                    }
                    Some(Ok(Work::History(request, result))) if request == requests.history => match result {
                        Ok(history) => app.set_history(history),
                        Err(error) => app.set_notice(format!("History unavailable: {error}")),
                    },
                    Some(Ok(Work::Incidents(request, result))) if request == requests.incidents => match result {
                        Ok(incidents) => app.set_incidents(incidents),
                        Err(error) => app.set_notice(format!("Incidents unavailable: {error}")),
                    },
                    Some(Ok(Work::SelectedIncident(generation, result))) if generation == view_generation => match result {
                        Ok(incident) => {
                            let id = incident.summary.id;
                            let closed = incident.summary.closed_at.is_some();
                            app.set_incident(incident);
                            refresh_incidents(&mut work, &path, Some(id), &mut requests);
                            app.set_notice(if closed { format!("Incident #{id} is closed. Press o to reopen it before attaching captures.") } else { format!("Incident #{id} active. New captures attach here; n adds a note, x clears the context.") });
                        }
                        Err(error) => app.set_notice(format!("Could not open incident: {error}")),
                    },
                    Some(Ok(Work::UpdatedIncident(request, id, result))) if request == requests.detail => {
                        if app.incident.as_ref().is_some_and(|v| v.summary.id == id) {
                            match result {
                                Ok(incident) => { app.incident = Some(incident); }
                                Err(error) => app.set_notice(format!("Incident details unavailable: {error}")),
                            }
                        }
                    }
                    Some(Ok(Work::Changed(generation, result))) => match result {
                        Ok(change) => {
                            app.set_notice(change.notice);
                            if generation == view_generation && let Some(incident) = change.created_incident {
                                let id = incident.summary.id;
                                app.set_incident(incident);
                                app.set_notice(format!("Created and activated incident #{id}. New captures attach here; n adds a note."));
                            }
                            refresh_history(&mut work, &path, &mut requests);
                            refresh_incidents(&mut work, &path, app.incident.as_ref().map(|v| v.summary.id), &mut requests);
                        }
                        Err(error) => app.set_notice(format!("Local change failed: {error}")),
                    },
                    Some(Ok(Work::Saved(result))) => {
                        saving = false;
                        match result {
                            Ok((id, incident)) => {
                                app.set_notice(match incident { Some(incident) => format!("Saved capture #{id} in incident #{incident}. Open History with 5."), None => format!("Saved capture #{id}. Open History with 5.") });
                                refresh_history(&mut work, &path, &mut requests);
                                refresh_incidents(&mut work, &path, app.incident.as_ref().map(|v| v.summary.id), &mut requests);
                            }
                            Err(error) => app.set_notice(format!("Could not save capture: {error}")),
                        }
                    }
                    Some(Ok(Work::Loaded(generation, result))) if generation == view_generation => match result {
                        Ok(snapshot) => app.set_snapshot(snapshot, true),
                        Err(error) => app.set_notice(format!("Could not load capture: {error}")),
                    },
                    Some(Ok(Work::Compared(generation, result))) if generation == view_generation => match result {
                        Ok(report) => app.set_report(report),
                        Err(error) => app.set_notice(format!("Could not compare captures: {error}")),
                    },
                    Some(Ok(Work::Loaded(_, _) | Work::Compared(_, _) | Work::SelectedIncident(_, _) | Work::History(_, _) | Work::Incidents(_, _) | Work::UpdatedIncident(_, _, _))) => {}
                    Some(Err(_)) => {
                        refreshing = false;
                        saving = false;
                        capture_requested = None;
                        capture_in_flight = None;
                        app.set_loading(false);
                        app.set_error("A background task stopped unexpectedly; press r to retry.".into());
                    }
                    None => {}
                }
            }
        }
        if refresh_requested && !refreshing {
            capture_in_flight = capture_requested.take();
            last_refresh = Instant::now();
            if cli.demo {
                app.set_loading(true);
                refreshing = true;
                let snapshot = demo::snapshot(demo_sequence, cli.include_query_text);
                demo_sequence = demo_sequence.saturating_add(1);
                work.spawn(async move { Work::Collected(Ok(snapshot)) });
            } else if let Some(collector) = &collector {
                app.set_loading(true);
                refreshing = true;
                let collector = Arc::clone(collector);
                work.spawn(async move {
                    Work::Collected(collector.collect().await.map_err(anyhow::Error::from))
                });
            } else {
                app.set_notice("Set PGTRAIL_DATABASE_URL for live monitoring, or run pgtrail --demo. Saved history works offline.".into());
            }
        }
    }
    work.abort_all();
    // Dropping the terminal guard restores the shell even while network work is pending.
    Ok(())
}
