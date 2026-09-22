use anyhow::{Context, Result};
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Message {
    Quit,
    Redraw,
    Key(KeyEvent),
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
        Event::Key(key) if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                Some(Message::Quit)
            } else {
                Some(Message::Key(key))
            }
        }
        Event::Resize(_, _) => Some(Message::Redraw),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_c_always_quits_but_plain_characters_reach_the_editor() {
        assert_eq!(
            translate(Event::Key(KeyEvent::new(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL
            ))),
            Some(Message::Quit)
        );
        for character in ['q', 'c', '/', 'é'] {
            let key = KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE);
            assert_eq!(translate(Event::Key(key)), Some(Message::Key(key)));
        }
    }

    #[test]
    fn releases_are_ignored_and_resize_requests_a_redraw() {
        assert_eq!(
            translate(Event::Key(KeyEvent::new_with_kind(
                KeyCode::Char('q'),
                KeyModifiers::NONE,
                KeyEventKind::Release,
            ))),
            None
        );
        assert_eq!(translate(Event::Resize(40, 10)), Some(Message::Redraw));
    }
}
