//! The content view: the rendered page with cursorline and cursor.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::App;
use crate::render::palette;

/// Background of the cursorline (Catppuccin Mocha surface0).
const CURSORLINE_BG: Color = Color::Rgb(0x31, 0x32, 0x44);

pub(super) fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let Some(page) = app.page() else {
        if let Some(msg) = app.placeholder() {
            let lines = msg.len() as u16 / area.width.max(1) + 1;
            let y = area.y + area.height.saturating_sub(lines) / 2;
            let rect = Rect::new(area.x, y, area.width, lines.min(area.height));
            frame.render_widget(
                Paragraph::new(msg)
                    .style(Style::new().fg(palette::OVERLAY))
                    .alignment(Alignment::Center)
                    .wrap(Wrap { trim: true }),
                rect,
            );
        }
        return;
    };
    let cursor = app.cursor();
    let lines: Vec<Line> = page
        .rendered
        .lines
        .iter()
        .skip(app.scroll())
        .take(area.height as usize)
        .cloned()
        .collect();
    frame.render_widget(Paragraph::new(lines), area);

    if cursor.row >= app.scroll() && cursor.row < app.scroll() + area.height as usize {
        let y = area.y + (cursor.row - app.scroll()) as u16;
        let buf = frame.buffer_mut();
        let line_rect = Rect::new(area.x, y, area.width, 1);
        buf.set_style(line_rect, Style::new().bg(CURSORLINE_BG));
        if (cursor.col as u16) < area.width {
            let x = area.x + cursor.col as u16;
            buf.set_style(
                Rect::new(x, y, 1, 1),
                Style::new().add_modifier(Modifier::REVERSED),
            );
        }
    }
}
