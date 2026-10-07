//! ratatui widgets only, no logic: content view, status line.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::App;
use crate::render::palette;

/// Background of the cursorline (Catppuccin Mocha surface0).
const CURSORLINE_BG: Color = Color::Rgb(0x31, 0x32, 0x44);
/// Background of the status line (Catppuccin Mocha mantle).
const STATUS_BG: Color = Color::Rgb(0x18, 0x18, 0x25);

/// Draw one full screen: content view plus status line.
pub fn draw(frame: &mut Frame, app: &App) {
    let [content, status] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());
    draw_content(frame, app, content);
    draw_status(frame, app, status);
}

fn draw_content(frame: &mut Frame, app: &App, area: Rect) {
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

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    let base = Style::new().fg(palette::TEXT).bg(STATUS_BG);
    let right = format!(
        " {}  {}  ← {} ",
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
