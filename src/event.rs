use anyhow::{Context, Result};
use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Message {
    Quit,
    Redraw,
}

pub(crate) async fn next(events: &mut EventStream) -> Result<Message> {
    loop {
        tokio::select! {
            signal = tokio::signal::ctrl_c() => {
                signal.context("could not listen for Ctrl+C")?;
                return Ok(Message::Quit);
            }
            event = events.next() => {
                let event = event.context("terminal input stream closed")?
                    .context("could not read a terminal event")?;
                if let Some(message) = translate(event) {
                    return Ok(message);
                }
            }
        }
    }
}

fn translate(event: Event) -> Option<Message> {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
            KeyCode::Char('q') if key.modifiers.is_empty() => Some(Message::Quit),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Message::Quit)
            }
            _ => None,
        },
        Event::Resize(_, _) => Some(Message::Redraw),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyEvent;

    use super::*;

    #[test]
    fn quit_keys_are_translated() {
        for key in [
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        ] {
            assert_eq!(translate(Event::Key(key)), Some(Message::Quit));
        }
    }

    #[test]
    fn unrelated_keys_and_key_releases_are_ignored() {
        for key in [
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
            KeyEvent::new_with_kind(
                KeyCode::Char('q'),
                KeyModifiers::NONE,
                KeyEventKind::Release,
            ),
        ] {
            assert_eq!(translate(Event::Key(key)), None);
        }
    }

    #[test]
    fn resize_requests_a_redraw() {
        assert_eq!(translate(Event::Resize(40, 10)), Some(Message::Redraw));
    }
}
