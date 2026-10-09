//! The status line: title, status message (else the comment on the
//! cursor line), the `g?` hint, LSP, position,
//! history depth.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::clip::clip_end;
use crate::app::App;
use crate::render::palette;

/// The key that opens the help overlay, shown dimmed left of the right
/// group while it fits.
pub const HELP_HINT: &str = "g? help";

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
    // A sidebar filter or `:` prompt owns the row and is drawn over it
    // later; its Paragraph only covers its own text, so draw nothing here.
    if app.filter_prompt().is_some() || app.cmdline_prompt().is_some() {
        return;
    }
    let review = match app.review_count() {
        0 => String::new(),
        n => format!("review {n}  "),
    } + if app.raw() { "RAW  " } else { "" }
        + app.visual_label();
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
    let right_w = Span::raw(right.as_str()).width();
    if !app.status().is_empty() {
        spans.push(Span::styled(
            format!("  {}", app.status()),
            base.fg(palette::YELLOW),
        ));
    } else if let Some(preview) = app.review_preview() {
        // The comment on the cursor line, cut to the room left.
        let title_w: usize = spans.iter().map(Span::width).sum();
        let room = (area.width as usize).saturating_sub(title_w + right_w + 2);
        spans.push(Span::styled(
            format!("  {}", clip_end(&preview, room)),
            base.fg(palette::YELLOW),
        ));
    }
    let left_w: usize = spans.iter().map(Span::width).sum();
    let hint_w = Span::raw(HELP_HINT).width();
    let hint = left_w + right_w + hint_w + 2 <= area.width as usize;
    let used = left_w + right_w + if hint { hint_w } else { 0 };
    let pad = (area.width as usize).saturating_sub(used);
    spans.push(Span::styled(" ".repeat(pad), base));
    if hint {
        spans.push(Span::styled(HELP_HINT, base.fg(palette::OVERLAY)));
    }
    spans.push(Span::styled(right, base));
    frame.render_widget(Paragraph::new(Line::from(spans)).style(base), area);
}
