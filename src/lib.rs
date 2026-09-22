//! Read-only PostgreSQL investigation and local snapshot comparison.

mod app;
mod cli;
mod collector;
mod commands;
mod compare;
mod demo;
mod diagnostics;
mod event;
mod incidents;
mod metrics;
mod model;
mod profiles;
mod report;
mod runtime;
mod store;
mod terminal;
mod ui;

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use clap::Parser;

use cli::Cli;
use collector::Collector;
use model::Snapshot;

/// Parse command-line arguments and run a command or the interactive application.
///
/// # Errors
/// Returns a contextual error for invalid configuration, collection, storage, or terminal I/O.
pub async fn run() -> Result<()> {
    let cli = Cli::parse();
    if let Some(command) = &cli.command {
        commands::run(&cli, command).await
    } else {
        runtime::run(&cli).await
    }
}

async fn collector(cli: &Cli) -> Result<Option<Arc<Collector>>> {
    let dsn = if let Some(name) = &cli.profile {
        Some(
            profiles::load(&cli.profiles_path()?)
                .await?
                .get(name)?
                .connection_url()?,
        )
    } else {
        cli.database_url()
    };
    dsn.map(|dsn| {
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
    collector(cli)
        .await?
        .context("no PostgreSQL connection configured; set PGTRAIL_DATABASE_URL or use --demo")?
        .collect()
        .await
        .map_err(anyhow::Error::from)
}
