//! The status line: title, status message, LSP, position, history depth.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::App;
use crate::render::palette;

/// Background of the status line (Catppuccin Mocha mantle).
const STATUS_BG: Color = Color::Rgb(0x18, 0x18, 0x25);

pub(super) fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let base = Style::new().fg(palette::TEXT).bg(STATUS_BG);
    if let Some(prompt) = app.search_prompt() {
        let x = area.x + Span::raw(prompt.as_str()).width() as u16;
        frame.render_widget(Paragraph::new(prompt).style(base), area);
        frame.set_cursor_position((x.min(area.right().saturating_sub(1)), area.y));
        return;
    }
    let review = match app.review_count() {
        0 => String::new(),
        n => format!("review {n}  "),
    };
    let right = format!(
        " {review}{}  {}  ← {} ",
        app.lsp_label(),
        app.position_label(),
        app.history_depth()
    );
    let mut spans = vec![Span::styled(
        format!(" {}", app.title()),
        base.fg(palette::BLUE),
    )];
    if !app.status().is_empty() {
        spans.push(Span::styled(
            format!("  {}", app.status()),
            base.fg(palette::YELLOW),
        ));
    }
    let left_w: usize = spans.iter().map(Span::width).sum();
    let right_w = Span::raw(right.as_str()).width();
    let pad = (area.width as usize).saturating_sub(left_w + right_w);
    spans.push(Span::styled(" ".repeat(pad), base));
    spans.push(Span::styled(right, base));
    frame.render_widget(Paragraph::new(Line::from(spans)).style(base), area);
}
