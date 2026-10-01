//! Shared geometry for painting, pointer targeting and display-row scrolling.
use ratatui::layout::{Constraint, Layout, Rect};

use crate::app::{App, ReportKind, Tab};

#[derive(Debug, Clone, Copy)]
pub(crate) struct Screen {
    pub(crate) header: Rect,
    pub(crate) sidebar: Rect,
    pub(crate) title: Rect,
    pub(crate) toolbar: Rect,
    pub(crate) content: Rect,
    pub(crate) inspector: Rect,
    pub(crate) status: Rect,
    pub(crate) footer: Rect,
    pub(crate) split: bool,
}

impl Screen {
    pub(crate) fn new(area: Rect, app: &App) -> Self {
        let [header, middle, status, footer] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(area);
        let [sidebar, main] = Layout::horizontal([
            Constraint::Length(if area.width >= 100 { 22 } else { 0 }),
            Constraint::Min(0),
        ])
        .areas(middle);
        let [title, toolbar, body] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .areas(main);
        let split =
            area.width >= 140 && app.report.is_some() && app.report_kind == ReportKind::Selection;
        let (content, inspector) = if split {
            let [content, inspector] =
                Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
                    .areas(body);
            (content, inspector)
        } else if app.report.is_some() {
            (Rect::default(), body)
        } else {
            (body, Rect::default())
        };
        Self {
            header,
            sidebar,
            title,
            toolbar,
            content,
            inspector,
            status,
            footer,
            split,
        }
    }

    pub(crate) fn for_app(app: &App) -> Self {
        Self::new(
            Rect::new(0, 0, app.viewport_width, app.viewport_height),
            app,
        )
    }

    pub(crate) fn menu(self) -> Rect {
        if self.sidebar.width > 0 {
            self.sidebar
        } else {
            Rect::new(
                0,
                self.header.bottom(),
                22.min(self.title.width),
                self.status.y.saturating_sub(self.header.bottom()),
            )
        }
    }

    pub(crate) fn table(self, app: &App) -> Rect {
        let summary = match app.tab {
            Tab::Overview | Tab::History => 2,
            _ => 0,
        };
        Rect::new(
            self.content.x,
            self.content.y + summary.min(self.content.height),
            self.content.width,
            self.content.height.saturating_sub(summary),
        )
    }
}

pub(crate) fn popup(width: u16, height: u16, max_width: u16, max_height: u16) -> Rect {
    let w = width.saturating_sub(2).min(max_width);
    let h = height.saturating_sub(2).min(max_height);
    Rect::new((width - w) / 2, (height - h) / 2, w, h)
}

pub(crate) fn table_window(app: &App, area: Rect) -> (usize, usize) {
    let visible = usize::from(area.height.saturating_sub(3)).max(1);
    let selected = app.row_selection();
    let mut offset = app.table_offsets[app.tab.index()];
    if selected < offset {
        offset = selected;
    }
    if selected >= offset.saturating_add(visible) {
        offset = selected + 1 - visible;
    }
    offset = offset.min(app.row_count().saturating_sub(visible));
    (offset, visible)
}
