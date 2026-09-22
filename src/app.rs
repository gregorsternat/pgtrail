use crate::event::Message;

#[derive(Debug, Default)]
pub(crate) struct App {
    should_quit: bool,
}

impl App {
    pub(crate) fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub(crate) fn update(&mut self, message: Message) {
        match message {
            Message::Quit => self.should_quit = true,
            Message::Redraw => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redraw_keeps_running_and_quit_is_final() {
        let mut app = App::default();
        app.update(Message::Redraw);
        assert!(!app.should_quit());
        app.update(Message::Quit);
        app.update(Message::Redraw);
        assert!(app.should_quit());
    }
}
