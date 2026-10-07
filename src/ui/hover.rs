//! The hover popup: the server's markdown shown as plain text, `Esc` closes.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::app::App;
use crate::render::palette;

/// Background of the popup (Catppuccin Mocha base).
const POPUP_BG: Color = Color::Rgb(0x1e, 0x1e, 0x2e);

pub(super) fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let Some(text) = app.hover_popup() else {
        return;
    };
    let width = area.width.saturating_sub(4).clamp(1, 60);
    let inner = width.saturating_sub(2).max(1) as usize;
    let rows: usize = text
        .lines()
        .map(|l| (unicode_width::UnicodeWidthStr::width(l) / inner) + 1)
        .sum();
    let height = (rows as u16 + 2).min(area.height);
    // Below the cursor row when it fits, else above.
    let row = app.cursor().row.saturating_sub(app.scroll()) as u16;
    let y = if area.y + row + 1 + height <= area.bottom() {
        area.y + row + 1
    } else {
        area.y + row.saturating_sub(height)
    };
    let x = area.x + (app.cursor().col as u16).min(area.width.saturating_sub(width));
    let rect = Rect::new(x, y, width, height).intersection(area);
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(text.to_string())
            .style(Style::new().fg(palette::TEXT).bg(POPUP_BG))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::new().fg(palette::OVERLAY))
                    .title(" hover "),
            ),
        rect,
    );
}
