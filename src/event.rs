use anyhow::{Context, Result};
use crossterm::event::{
    Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
};
use futures_util::StreamExt;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Message {
    Quit,
    Redraw,
    Key(KeyEvent),
    MouseClick { column: u16, row: u16 },
    ScrollUp { column: u16, row: u16 },
    ScrollDown { column: u16, row: u16 },
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
        Event::Mouse(event) => match event.kind {
            MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                Some(Message::MouseClick {
                    column: event.column,
                    row: event.row,
                })
            }
            MouseEventKind::ScrollUp => Some(Message::ScrollUp {
                column: event.column,
                row: event.row,
            }),
            MouseEventKind::ScrollDown => Some(Message::ScrollDown {
                column: event.column,
                row: event.row,
            }),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::MouseEvent;

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

    #[test]
    fn left_clicks_and_wheels_are_messages_but_other_mouse_input_is_ignored() {
        use crossterm::event::MouseButton;
        let event = |kind| MouseEvent {
            kind,
            column: 4,
            row: 8,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(
            translate(Event::Mouse(event(MouseEventKind::Down(MouseButton::Left)))),
            Some(Message::MouseClick { column: 4, row: 8 })
        );
        assert_eq!(
            translate(Event::Mouse(event(MouseEventKind::ScrollUp))),
            Some(Message::ScrollUp { column: 4, row: 8 })
        );
        assert_eq!(
            translate(Event::Mouse(event(MouseEventKind::ScrollDown))),
            Some(Message::ScrollDown { column: 4, row: 8 })
        );
        for kind in [
            MouseEventKind::Down(MouseButton::Right),
            MouseEventKind::Moved,
            MouseEventKind::Up(MouseButton::Left),
        ] {
            assert_eq!(translate(Event::Mouse(event(kind))), None);
        }
    }
}
