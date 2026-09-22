use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    version,
    about,
    long_about = "PostgreSQL diagnostics in your terminal.\n\nThis initial skeleton does not connect to PostgreSQL or collect data yet."
)]
pub(crate) struct Cli {}
