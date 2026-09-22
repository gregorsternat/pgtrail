use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::Stylize,
    text::Line,
    widgets::{Block, Paragraph, Wrap},
};

pub(crate) fn render(frame: &mut Frame) {
    let [body, footer] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());
    let content = Paragraph::new(vec![
        Line::from("PostgreSQL diagnostics".bold()),
        Line::from(""),
        Line::from("Data collection is not implemented yet."),
        Line::from("No database connection is open."),
        Line::from(""),
        Line::from("Planned: active sessions, blocking transactions, expensive queries,"),
        Line::from("and local snapshots to compare two situations."),
    ])
    .block(Block::bordered().title(" pgtrail "))
    .wrap(Wrap { trim: false });

    frame.render_widget(content, body);
    frame.render_widget(Paragraph::new(" q / Ctrl+C: quit ").dim(), footer);
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;

    #[test]
    fn welcome_screen_explains_the_current_capabilities() -> Result<(), std::convert::Infallible> {
        let mut terminal = Terminal::new(TestBackend::new(80, 15))?;
        terminal.draw(render)?;
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert!(text.contains("Data collection is not implemented yet."));
        assert!(text.contains("No database connection is open."));
        assert!(text.contains("q / Ctrl+C: quit"));
        Ok(())
    }

    #[test]
    fn rendering_survives_a_tiny_or_empty_terminal() -> Result<(), std::convert::Infallible> {
        for (width, height) in [(1, 1), (10, 3), (0, 0)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height))?;
            terminal.draw(render)?;
        }
        Ok(())
    }
}
