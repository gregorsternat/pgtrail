use crate::{
    cli::{Cli, Command, Format, IncidentCommand, ProfileCommand},
    collect_once, compare, report,
    store::Store,
};
use anyhow::{Context, Result};
use std::{io::Write, path::Path};

pub(crate) async fn run(cli: &Cli, command: &Command) -> Result<()> {
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
            let id = if let Some(incident) = cli.incident {
                store
                    .save_for_incident(&snapshot, label, Some(incident))
                    .await?
            } else {
                store.save(&snapshot, label).await?
            };
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
            let metadata = store.capture_metadata(*id).await?;
            let mut payload = serde_json::to_value(&snapshot)?;
            payload["capture"] =
                serde_json::json!({"id": id, "label": metadata.label, "note": metadata.note});
            let markdown = format!(
                "# Capture #{id}: {}\n\n{}\n\n{}",
                markdown_text(&metadata.label),
                markdown_text(&metadata.note),
                report::snapshot_markdown(&snapshot)
            );
            emit(&payload, markdown, *format, output.as_deref())
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
        Command::Diagnose {
            sample_seconds,
            format,
            output,
            fail_on,
        } => {
            let before = collect_once(cli).await?;
            tokio::time::sleep(std::time::Duration::from_secs(*sample_seconds)).await;
            let after = collect_once(cli).await?;
            let analysis = crate::diagnostics::analyze(&after, Some(&before));
            let payload = serde_json::json!({"source": after.source, "started_at": before.started_at, "completed_at": after.completed_at, "analysis": analysis});
            let markdown = format!(
                "# PostgreSQL investigation\n\n- Source: {} / {}\n- PostgreSQL: {}\n- First sample: {}\n- Second sample: {}\n\n{}",
                markdown_text(&after.source.endpoint),
                markdown_text(&after.source.database),
                markdown_text(&after.source.server_version),
                before.completed_at.to_rfc3339(),
                after.completed_at.to_rfc3339(),
                report::analysis_markdown(&analysis)
            );
            emit(&payload, markdown, *format, output.as_deref())?;
            if let Some(level) = fail_on {
                let matched = analysis
                    .findings
                    .iter()
                    .any(|finding| match finding.severity {
                        crate::diagnostics::Severity::Critical => true,
                        crate::diagnostics::Severity::Warning => level == "warning",
                        crate::diagnostics::Severity::Info => false,
                    });
                anyhow::ensure!(
                    !matched,
                    "diagnostic findings reached the requested failure threshold"
                );
            }
            Ok(())
        }
        Command::Record {
            count,
            interval,
            label,
        } => {
            tokio::select! {
                signal = tokio::signal::ctrl_c() => {
                    signal?;
                    write_output("Recording interrupted. Saved captures remain available with snapshots.", None)
                }
                result = record(cli, *count, *interval, label) => result,
            }
        }
        Command::Annotate { id, label, note } => {
            Store::open(&cli.store_path()?)
                .await?
                .annotate_capture(*id, label.as_deref(), note.as_deref())
                .await?;
            write_output(&format!("Updated capture #{id}."), None)
        }
        Command::Incident { command } => incident_command(cli, command).await,
        Command::Profile { command } => profile_command(cli, command).await,
        Command::Delete { id } => {
            let store = Store::open(&cli.store_path()?).await?;
            store.delete(*id).await?;
            write_output(&format!("Deleted capture #{id}.\n"), None)
        }
    }
}

fn markdown_text(value: &str) -> String {
    plain(value)
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\\', "\\\\")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('*', "\\*")
        .replace('_', "\\_")
        .replace('`', "\\`")
        .replace('|', "\\|")
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

async fn incident_command(cli: &Cli, command: &IncidentCommand) -> Result<()> {
    let store = Store::open(&cli.store_path()?).await?;
    match command {
        IncidentCommand::Create { title } => {
            let id = store.create_incident(title).await?;
            write_output(
                &format!("Created incident #{id}. Use --incident {id} with capture or record."),
                None,
            )
        }
        IncidentCommand::List { json } => {
            let incidents = store.incidents().await?;
            if *json {
                return write_output(&serde_json::to_string_pretty(&incidents)?, None);
            }
            let mut text = String::from("ID\tStatus\tCaptures\tNotes\tTitle\n");
            for incident in incidents {
                text.push_str(&format!(
                    "{}\t{}\t{}\t{}\t{}\n",
                    incident.id,
                    if incident.closed_at.is_some() {
                        "closed"
                    } else {
                        "open"
                    },
                    incident.capture_count,
                    incident.note_count,
                    plain(&incident.title)
                ));
            }
            write_output(&text, None)
        }
        IncidentCommand::Show { id, format, output } => {
            let incident = store.incident(*id).await?;
            let mut captures = Vec::new();
            for capture in &incident.captures {
                captures.push((capture.id, store.load(capture.id).await?));
            }
            let markdown = crate::incidents::markdown(&incident, &captures);
            let payload = serde_json::json!({"incident": incident, "captures": captures});
            emit(&payload, markdown, *format, output.as_deref())
        }
        IncidentCommand::Note { id, text } => {
            store.add_note(*id, text).await?;
            write_output(&format!("Added note to incident #{id}."), None)
        }
        IncidentCommand::Attach { id, capture } => {
            store.attach_capture(*id, *capture).await?;
            write_output(
                &format!("Attached capture #{capture} to incident #{id}."),
                None,
            )
        }
        IncidentCommand::Detach { id, capture } => {
            store.detach_capture(*id, *capture).await?;
            write_output(
                &format!("Detached capture #{capture} from incident #{id}."),
                None,
            )
        }
        IncidentCommand::Close { id } => {
            store.set_incident_closed(*id, true).await?;
            write_output(&format!("Closed incident #{id}."), None)
        }
        IncidentCommand::Reopen { id } => {
            store.set_incident_closed(*id, false).await?;
            write_output(&format!("Reopened incident #{id}."), None)
        }
    }
}

async fn profile_command(cli: &Cli, command: &ProfileCommand) -> Result<()> {
    let path = cli.profiles_path()?;
    let mut profiles = crate::profiles::load(&path).await?;
    match command {
        ProfileCommand::List => {
            let mut output =
                String::from("Name\tHost\tPort\tDatabase\tUser\tTLS\tPassword environment\n");
            for (name, profile) in profiles.list() {
                output.push_str(&format!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                    plain(name),
                    plain(&profile.host),
                    profile.port,
                    plain(&profile.database),
                    plain(&profile.user),
                    plain(&profile.sslmode),
                    plain(
                        profile
                            .password_env
                            .as_deref()
                            .unwrap_or("PGPASSWORD / none")
                    )
                ));
            }
            write_output(&output, None)
        }
        ProfileCommand::Add {
            name,
            host,
            port,
            database,
            user,
            sslmode,
            sslrootcert,
            password_env,
        } => {
            profiles.add(
                name,
                crate::profiles::Profile {
                    host: host.clone(),
                    port: *port,
                    database: database.clone(),
                    user: user.clone(),
                    sslmode: sslmode.clone(),
                    sslrootcert: sslrootcert.clone(),
                    password_env: password_env.clone(),
                },
            )?;
            crate::profiles::save(&path, &profiles).await?;
            write_output(
                &format!(
                    "Saved connection profile {}. No password was stored.",
                    plain(name)
                ),
                None,
            )
        }
        ProfileCommand::Remove { name } => {
            profiles.remove(name)?;
            crate::profiles::save(&path, &profiles).await?;
            write_output(
                &format!("Removed connection profile {}.", plain(name)),
                None,
            )
        }
    }
}

async fn record(cli: &Cli, count: u32, interval: u64, label: &str) -> Result<()> {
    anyhow::ensure!(
        label.chars().count() + format!(" {count}/{count}").chars().count() <= 200,
        "recording label plus sample suffix must be at most 200 characters"
    );
    let store = Store::open(&cli.store_path()?).await?;
    if let Some(id) = cli.incident {
        anyhow::ensure!(
            store.incident(id).await?.summary.closed_at.is_none(),
            "incident #{id} is closed; reopen it before recording"
        );
    }
    // The caller keeps one interrupt listener alive across collection, SQLite writes,
    // output, and waits. Completed transactions remain available after interruption.
    for index in 0..count {
        let snapshot = collect_once(cli).await?;
        let id = store
            .save_for_incident(
                &snapshot,
                &format!("{label} {}/{}", index + 1, count),
                cli.incident,
            )
            .await?;
        write_output(
            &format!("Saved capture #{id} ({}/{}).", index + 1, count),
            None,
        )?;
        if index + 1 < count {
            tokio::time::sleep(std::time::Duration::from_secs(interval)).await;
        }
    }
    Ok(())
}
