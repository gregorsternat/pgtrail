//! PostgreSQL diagnostic TUI. Data collection is not implemented yet.

mod app;
mod cli;
mod event;
mod terminal;
mod ui;

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::event::EventStream;

/// Parse command-line arguments and run the interactive application.
///
/// # Errors
/// Returns an error when no interactive terminal is available or terminal I/O fails.
pub async fn run() -> Result<()> {
    cli::Cli::parse();
    let mut terminal = terminal::Session::start()?;
    let mut events = EventStream::new();
    let mut app = app::App::default();

    while !app.should_quit() {
        terminal
            .terminal
            .draw(ui::render)
            .context("could not draw the terminal")?;
        app.update(event::next(&mut events).await?);
    }

    Ok(())
}
