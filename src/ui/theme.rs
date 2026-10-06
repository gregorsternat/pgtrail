//! Widgets use a small semantic palette, resolved once at the frame boundary.
use ratatui::{Frame, style::Color};

pub(super) const ACCENT: Color = Color::Cyan;
pub(super) const MUTED: Color = Color::DarkGray;
pub(super) const WARNING: Color = Color::Yellow;
pub(super) const BORDER: Color = Color::Indexed(8);
pub(super) const SURFACE: Color = Color::Black;
pub(super) const SELECTED: Color = Color::Blue;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Theme {
    #[default]
    Dark,
    Terminal,
}

impl Theme {
    pub(super) fn apply(self, frame: &mut Frame) {
        for cell in &mut frame.buffer_mut().content {
            cell.fg = if self == Self::Terminal && cell.bg == SELECTED {
                Color::White
            } else {
                self.color(cell.fg, false)
            };
            cell.bg = self.color(cell.bg, true);
        }
    }

    fn color(self, color: Color, background: bool) -> Color {
        if self == Self::Terminal {
            return match color {
                MUTED | BORDER => Color::Reset,
                SURFACE if background => Color::Reset,
                other => other,
            };
        }
        match color {
            Color::Reset if background => Color::Rgb(15, 23, 42),
            Color::Reset | Color::White => Color::Rgb(226, 232, 240),
            SURFACE if background => Color::Rgb(23, 32, 51),
            SELECTED if background => Color::Rgb(30, 58, 76),
            ACCENT => Color::Rgb(103, 232, 249),
            MUTED => Color::Rgb(148, 163, 184),
            BORDER => Color::Rgb(51, 65, 85),
            WARNING => Color::Rgb(251, 191, 36),
            Color::Red => Color::Rgb(248, 113, 113),
            Color::Green => Color::Rgb(134, 239, 172),
            Color::Magenta => Color::Rgb(216, 180, 254),
            other => other,
        }
    }
}
