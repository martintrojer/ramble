//! Hint labels drawn over the first visible cell of each link.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::app::App;
use crate::render::palette;

/// A hint label (Catppuccin Mocha base on peach, bold).
const LABEL: Style = Style::new()
    .fg(Color::Rgb(0x1e, 0x1e, 0x2e))
    .bg(palette::PEACH)
    .add_modifier(Modifier::BOLD);

pub(super) fn draw(frame: &mut Frame, app: &App, area: Rect) {
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
        buf.set_stringn(x, area.y + dy as u16, label, w, LABEL);
    }
}
