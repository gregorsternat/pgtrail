//! Read-only PostgreSQL investigation and local snapshot comparison.

mod app;
mod cli;
mod collector;
mod compare;
mod demo;
mod event;
mod model;
mod report;
mod store;
mod terminal;
mod ui;

use std::{io::Write, path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::event::EventStream;
use tokio::{task::JoinSet, time::Instant};

use cli::{Cli, Command, Format};
use collector::Collector;
use model::Snapshot;
use store::{SnapshotSummary, Store};

/// Parse command-line arguments and run a command or the interactive application.
///
/// # Errors
/// Returns a contextual error for invalid configuration, collection, storage, or terminal I/O.
pub async fn run() -> Result<()> {
    let cli = Cli::parse();
    if let Some(command) = &cli.command {
        run_command(&cli, command).await
    } else {
        run_interactive(&cli).await
    }
}

fn collector(cli: &Cli) -> Result<Option<Arc<Collector>>> {
    cli.database_url()
        .map(|dsn| {
            Collector::new(
                &dsn,
                cli.include_query_text,
                Duration::from_secs(cli.timeout),
            )
            .map(Arc::new)
            .map_err(anyhow::Error::from)
        })
        .transpose()
}

async fn collect_once(cli: &Cli) -> Result<Snapshot> {
    if cli.demo {
        // Headless demo captures also advance counters between invocations.
        let sequence =
            u64::try_from(chrono::Utc::now().timestamp()).unwrap_or_default() % 1_000_000;
        return Ok(demo::snapshot(sequence, cli.include_query_text));
    }
    collector(cli)?
        .context("no PostgreSQL connection configured; set PGTRAIL_DATABASE_URL or use --demo")?
        .collect()
        .await
        .map_err(anyhow::Error::from)
}

async fn run_command(cli: &Cli, command: &Command) -> Result<()> {
    match command {
        Command::Check { format, output } => {
            let snapshot = collect_once(cli).await?;
            emit(
                &snapshot,
                report::snapshot_markdown(&snapshot),
                *format,
                output.as_deref(),
            )
        }
        Command::Capture { label } => {
            anyhow::ensure!(
                label.chars().count() <= 200,
                "capture label must be at most 200 characters"
            );
            let snapshot = collect_once(cli).await?;
            let store = Store::open(&cli.store_path()?).await?;
            let id = store.save(&snapshot, label).await?;
            write_output(
                &format!(
                    "Saved capture #{id} ({}).\n",
                    if snapshot.is_complete() {
                        "complete"
                    } else {
                        "partial; inspect with show"
                    }
                ),
                None,
            )
        }
        Command::Snapshots { json } => {
            let store = Store::open(&cli.store_path()?).await?;
            let history = store.list().await?;
            if *json {
                write_output(&serde_json::to_string_pretty(&history)?, None)
            } else {
                let mut output = String::from("ID\tCaptured (UTC)\tCoverage\tSource\tLabel\n");
                for entry in history {
                    output.push_str(&format!(
                        "{}\t{}\t{}\t{}\t{}\n",
                        entry.id,
                        entry.captured_at.to_rfc3339(),
                        if entry.complete {
                            "complete"
                        } else {
                            "partial"
                        },
                        plain(&entry.source),
                        plain(&entry.label)
                    ));
                }
                write_output(&output, None)
            }
        }
        Command::Show { id, format, output } => {
            let store = Store::open(&cli.store_path()?).await?;
            let snapshot = store.load(*id).await?;
            emit(
                &snapshot,
                report::snapshot_markdown(&snapshot),
                *format,
                output.as_deref(),
            )
        }
        Command::Compare {
            before,
            after,
            format,
            output,
        } => {
            let store = Store::open(&cli.store_path()?).await?;
            let before = store.load(*before).await?;
            let after = store.load(*after).await?;
            let comparison = compare::compare(&before, &after);
            emit(
                &comparison,
                report::comparison_markdown(&comparison),
                *format,
                output.as_deref(),
            )
        }
        Command::Delete { id } => {
            let store = Store::open(&cli.store_path()?).await?;
            store.delete(*id).await?;
            write_output(&format!("Deleted capture #{id}.\n"), None)
        }
    }
}

fn plain(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

fn emit(
    value: &impl serde::Serialize,
    markdown: String,
    format: Format,
    path: Option<&Path>,
) -> Result<()> {
    let output = match format {
        Format::Markdown => markdown,
        Format::Json => serde_json::to_string_pretty(value)?,
    };
    write_output(&output, path)
}

fn write_output(output: &str, path: Option<&Path>) -> Result<()> {
    if let Some(path) = path {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(path)
            .context("could not create export; choose a writable, new file path")?;
        file.write_all(output.as_bytes())
            .context("could not write export")?;
        file.sync_all().context("could not flush export")?;
    } else {
        let mut stdout = std::io::stdout().lock();
        if let Err(error) = writeln!(stdout, "{output}")
            && error.kind() != std::io::ErrorKind::BrokenPipe
        {
            return Err(error).context("could not write output");
        }
    }
    Ok(())
}

enum Work {
    Collected(Result<Snapshot>),
    History(Result<Vec<SnapshotSummary>>),
    Saved(Result<i64>),
    Loaded(u64, Result<Snapshot>),
    Compared(u64, Result<String>),
}

async fn run_interactive(cli: &Cli) -> Result<()> {
    let mut terminal = terminal::Session::start()?;
    let collector = collector(cli)?;
    let path = cli.store_path()?;
    let mut events = EventStream::new();
    let mut app = app::App::new(cli.demo);
    if collector.is_none() && !cli.demo {
        app.set_loading(false);
    }
    let mut work = JoinSet::new();
    let mut refreshing = false;
    let mut saving = false;
    let mut capture_requested = false;
    let mut capture_in_flight = false;
    let mut view_generation = 0_u64;
    let mut last_live: Option<Snapshot> = None;
    let mut last_live_error: Option<String> = None;
    let mut demo_sequence = 0_u64;
    let refresh_period = Duration::from_secs(cli.refresh);
    let mut last_refresh = Instant::now() - refresh_period;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let history_path = path.clone();
    work.spawn(async move {
        Work::History(async { Store::open(&history_path).await?.list().await }.await)
    });

    while !app.should_quit() {
        terminal
            .terminal
            .draw(|frame| ui::render(frame, &app))
            .context("could not draw the terminal")?;
        let mut refresh_requested = false;
        tokio::select! {
            message = event::next(&mut events) => {
                let message = message?;
                if matches!(message, event::Message::Key(_)) {
                    view_generation = view_generation.wrapping_add(1);
                }
                if let Some(action) = app.update(message) {
                    match action {
                        app::Action::Refresh => refresh_requested = true,
                        app::Action::Capture => {
                            if app.is_offline() {
                                app.set_notice("Return to live mode with Esc before capturing.".into());
                            } else if saving || capture_requested || capture_in_flight {
                                app.set_notice("A capture is already in progress.".into());
                            } else if collector.is_none() && !cli.demo {
                                app.set_notice("Set PGTRAIL_DATABASE_URL or restart with --demo to capture.".into());
                            } else {
                                capture_requested = true;
                                refresh_requested = true;
                                app.set_notice("Collecting a fresh manual capture…".into());
                            }
                        }
                        app::Action::Load(id) => {
                            let path = path.clone();
                            work.spawn(async move { Work::Loaded(view_generation, async { Store::open(&path).await?.load(id).await }.await) });
                        }
                        app::Action::Compare(before, after) => {
                            let path = path.clone();
                            work.spawn(async move { Work::Compared(view_generation, async {
                                let store = Store::open(&path).await?;
                                let before = store.load(before).await?;
                                let after = store.load(after).await?;
                                Ok(report::comparison_markdown(&compare::compare(&before, &after)))
                            }.await) });
                        }
                        app::Action::ListHistory => {
                            let path = path.clone();
                            work.spawn(async move { Work::History(async { Store::open(&path).await?.list().await }.await) });
                        }
                        app::Action::ResumeLive => {
                            if let Some(snapshot) = &last_live {
                                app.set_snapshot(snapshot.clone(), false);
                            }
                            if let Some(error) = &last_live_error {
                                app.set_error(error.clone());
                            }
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
                        let save_capture = std::mem::take(&mut capture_in_flight);
                        refresh_requested |= capture_requested;
                        app.set_loading(false);
                        match result {
                            Ok(snapshot) => {
                                last_live_error = None;
                                if save_capture {
                                    saving = true;
                                    let path = path.clone();
                                    let snapshot = snapshot.clone();
                                    work.spawn(async move { Work::Saved(async {
                                        let store = Store::open(&path).await?;
                                        store.save(&snapshot, "Manual TUI capture").await
                                    }.await) });
                                }
                                if !app.is_offline() { app.set_snapshot(snapshot.clone(), false); }
                                last_live = Some(snapshot);
                            }
                            Err(error) => {
                                last_live_error = Some(error.to_string());
                                if save_capture {
                                    app.set_notice("Capture failed; no snapshot was saved.".into());
                                }
                                if app.is_offline() {
                                    app.set_notice(format!("Background live refresh failed: {error}"));
                                } else {
                                    app.set_error(error.to_string());
                                }
                            }
                        }
                    }
                    Some(Ok(Work::History(result))) => match result {
                        Ok(history) => app.set_history(history),
                        Err(error) => app.set_notice(format!("History unavailable: {error}")),
                    },
                    Some(Ok(Work::Saved(result))) => {
                        saving = false;
                        match result {
                            Ok(id) => {
                                app.set_notice(format!("Saved capture #{id}. Open History with 5."));
                                let path = path.clone();
                                work.spawn(async move { Work::History(async { Store::open(&path).await?.list().await }.await) });
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
                    Some(Ok(Work::Loaded(_, _) | Work::Compared(_, _))) => {}
                    Some(Err(_)) => {
                        refreshing = false;
                        saving = false;
                        capture_requested = false;
                        capture_in_flight = false;
                        app.set_loading(false);
                        app.set_error("A background task stopped unexpectedly; press r to retry.".into());
                    }
                    None => {}
                }
            }
        }
        if refresh_requested && !refreshing {
            capture_in_flight = std::mem::take(&mut capture_requested);
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
