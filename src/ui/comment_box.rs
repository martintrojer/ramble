//! The comment box: a bordered multi-line editor over the content pane,
//! next to the rows being commented on (placement in
//! [`crate::app::App::comment_box`]). The title names the kind and the
//! lines; the bottom border lists the keys; the terminal cursor sits on
//! the text cursor (so an IME pops up there).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::app::App;

/// Draw the comment box; returns its rect.
pub(super) fn draw(frame: &mut Frame, app: &App, pane: Rect) -> Option<Rect> {
    let b = app.comment_box(pane)?;
    let p = app.theme().palette;
    let base = Style::new().fg(p.text).bg(p.bg);
    frame.render_widget(Clear, b.rect);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(base.fg(p.purple))
        .title(Line::styled(
            b.title,
            base.fg(p.purple).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Line::styled(b.hint, base.fg(p.muted)))
        .style(base);
    let lines: Vec<Line> = b.lines.into_iter().map(Line::raw).collect();
    frame.render_widget(Paragraph::new(lines).style(base).block(block), b.rect);
    frame.set_cursor_position(b.cursor);
    Some(b.rect)
}
