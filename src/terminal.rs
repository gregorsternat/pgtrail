use std::io::{self, IsTerminal};

use anyhow::{Context, Result, ensure};
use crossterm::{
    cursor::Show,
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
};
use ratatui::DefaultTerminal;

pub(crate) struct Session {
    pub(crate) terminal: DefaultTerminal,
    mouse: bool,
}

impl Session {
    pub(crate) fn start(mouse: bool) -> Result<Self> {
        ensure!(
            io::stdin().is_terminal() && io::stdout().is_terminal(),
            "pgtrail requires an interactive terminal on stdin and stdout; use --help for usage"
        );

        // try_init installs Ratatui's restoration hook for panics. Restore after
        // partial initialization too, before returning the original I/O error.
        match ratatui::try_init() {
            Ok(terminal) => {
                if mouse {
                    if let Err(error) = execute!(io::stdout(), EnableMouseCapture) {
                        let _ = execute!(io::stdout(), DisableMouseCapture, Show);
                        ratatui::restore();
                        return Err(error).context("could not enable terminal mouse input");
                    }
                    let previous = std::panic::take_hook();
                    std::panic::set_hook(Box::new(move |info| {
                        let _ = execute!(io::stdout(), DisableMouseCapture, Show);
                        previous(info);
                    }));
                }
                Ok(Self { terminal, mouse })
            }
            Err(error) => {
                ratatui::restore();
                Err(error).context("could not initialize the terminal")
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.mouse {
            let _ = execute!(io::stdout(), DisableMouseCapture);
        }
        let _ = execute!(io::stdout(), Show);
        ratatui::restore();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires PGTRAIL_TERMINAL_TEST=1, a PTY and --nocapture"]
    fn restores_on_error_and_panic() -> Result<()> {
        if std::env::var("PGTRAIL_TERMINAL_TEST").as_deref() != Ok("1") {
            return Ok(());
        }
        let fail = || -> Result<()> {
            let _session = Session::start(true)?;
            anyhow::bail!("injected rendering failure")
        };
        assert!(fail().is_err());
        assert!(!crossterm::terminal::is_raw_mode_enabled()?);
        let panic = std::panic::catch_unwind(|| {
            let _session = Session::start(true).expect("PTY initialization");
            panic!("injected terminal panic");
        });
        assert!(panic.is_err());
        assert!(!crossterm::terminal::is_raw_mode_enabled()?);
        Ok(())
    }
}
