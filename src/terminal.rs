use std::io::{self, IsTerminal};

use anyhow::{Context, Result, ensure};
use ratatui::DefaultTerminal;

pub(crate) struct Session {
    pub(crate) terminal: DefaultTerminal,
}

impl Session {
    pub(crate) fn start() -> Result<Self> {
        ensure!(
            io::stdin().is_terminal() && io::stdout().is_terminal(),
            "pgtrail requires an interactive terminal on stdin and stdout; use --help for usage"
        );

        // try_init installs Ratatui's restoration hook for panics. Restore after
        // partial initialization too, before returning the original I/O error.
        match ratatui::try_init() {
            Ok(terminal) => Ok(Self { terminal }),
            Err(error) => {
                ratatui::restore();
                Err(error).context("could not initialize the terminal")
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        ratatui::restore();
    }
}
