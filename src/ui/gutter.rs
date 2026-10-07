//! The one-column review gutter left of the content: a marker on every
//! visible row whose source line has a review comment.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::app::{App, REVIEW_MARKER};
use crate::render::palette;

/// Split `area` into the gutter (empty when the app reserves none) and the
/// content.
pub(super) fn split(app: &App, area: Rect) -> (Rect, Rect) {
    let w = app.review_gutter().min(area.width);
    (
        Rect::new(area.x, area.y, w, area.height),
        Rect::new(area.x + w, area.y, area.width - w, area.height),
    )
}

pub(super) fn draw(frame: &mut Frame, app: &App, area: Rect) {
    if area.width == 0 || app.page().is_none() {
        return;
    }
    let marked = app.review_marked_rows();
    let buf = frame.buffer_mut();
    for y in 0..area.height {
        if marked.binary_search(&(app.scroll() + y as usize)).is_ok() {
            buf[(area.x, area.y + y)]
                .set_char(REVIEW_MARKER)
                .set_style(Style::new().fg(palette::PEACH));
        }
    }
}
