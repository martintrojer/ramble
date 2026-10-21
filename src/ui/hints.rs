//! Hint labels drawn over the first visible cell of each link.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::app::App;

pub(super) fn draw(frame: &mut Frame, app: &App, area: Rect) {
    // A hint label: bg on orange, bold.
    let p = app.theme().palette;
    let label_style = Style::new()
        .fg(p.bg)
        .bg(p.orange)
        .add_modifier(Modifier::BOLD);
    let top = app.scroll();
    let buf = frame.buffer_mut();
    for (label, span) in app.hints() {
        let Some(dy) = span.row.checked_sub(top) else {
            continue;
        };
        if dy >= area.height as usize || span.col_start >= area.width as usize {
            continue;
        }
        let x = area.x + span.col_start as u16;
        let w = (area.right() - x) as usize;
        buf.set_stringn(x, area.y + dy as u16, label, w, label_style);
    }
}
