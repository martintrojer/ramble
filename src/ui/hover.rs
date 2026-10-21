//! The hover popup: the server's markdown shown as plain text, `Esc` closes.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::app::App;

/// Draw the hover popup; returns its rect.
pub(super) fn draw(frame: &mut Frame, app: &App, area: Rect) -> Option<Rect> {
    let text = app.hover_popup()?;
    let p = app.theme().palette;
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
            .style(Style::new().fg(p.text).bg(p.bg))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::new().fg(p.muted))
                    .title(" hover "),
            ),
        rect,
    );
    Some(rect)
}
