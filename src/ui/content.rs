//! The content view: the rendered page with cursorline and cursor.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};

use crate::app::App;

pub(super) fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let p = app.theme().palette;
    // A search hit, and the hit under the cursor: bg on yellow / orange.
    let hit = Style::new().fg(p.bg).bg(p.yellow);
    let current_hit = Style::new().fg(p.bg).bg(p.orange);
    // A link a diagnostic covers (e.g. a dead link).
    let broken_link = Style::new()
        .fg(p.muted)
        .add_modifier(Modifier::DIM)
        .remove_modifier(Modifier::UNDERLINED);
    let Some(page) = app.page() else {
        if let Some(msg) = app.placeholder() {
            let lines = msg.len() as u16 / area.width.max(1) + 1;
            let y = area.y + area.height.saturating_sub(lines) / 2;
            let rect = Rect::new(area.x, y, area.width, lines.min(area.height));
            frame.render_widget(
                Paragraph::new(msg)
                    .style(Style::new().fg(p.muted))
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

    let top = app.scroll();
    let bottom = top + area.height as usize;
    let on_screen = |row: usize| row >= top && row < bottom;
    let buf = frame.buffer_mut();
    // Links a diagnostic covers: muted.
    let broken = app.broken_links();
    for seg in &page.rendered.srcmap.segments {
        let span = seg.span;
        if !seg.link.is_some_and(|i| broken.contains(&i))
            || !on_screen(span.row)
            || span.col_start as u16 >= area.width
        {
            continue;
        }
        let w = (span.col_end.min(area.width as usize) - span.col_start) as u16;
        buf.set_style(
            Rect::new(
                area.x + span.col_start as u16,
                area.y + (span.row - top) as u16,
                w,
                1,
            ),
            broken_link,
        );
    }
    // Cursorline first, so search hits on the cursor row keep their colours.
    if on_screen(cursor.row) {
        let y = area.y + (cursor.row - top) as u16;
        buf.set_style(
            Rect::new(area.x, y, area.width, 1),
            Style::new().bg(p.cursor_bg),
        );
    }
    for s in app.search_highlights() {
        if !on_screen(s.row) || s.col_start as u16 >= area.width {
            continue;
        }
        let w = (s.col_end.min(area.width as usize) - s.col_start) as u16;
        let rect = Rect::new(
            area.x + s.col_start as u16,
            area.y + (s.row - top) as u16,
            w,
            1,
        );
        let current = s.row == cursor.row && (s.col_start..s.col_end).contains(&cursor.col);
        buf.set_style(rect, if current { current_hit } else { hit });
    }
    // A comment being typed keeps its rows highlighted.
    if let Some((lo, hi)) = app.comment_rows() {
        for row in lo.max(top)..=hi.min(bottom.saturating_sub(1)) {
            let y = area.y + (row - top) as u16;
            buf.set_style(
                Rect::new(area.x, y, area.width, 1),
                Style::new().fg(p.text).bg(p.selection_bg),
            );
        }
    }
    // The selection wins over the cursorline and search hits; the cursor
    // stays reversed on top.
    for s in app.selection_spans() {
        if !on_screen(s.row) || s.col_start as u16 >= area.width {
            continue;
        }
        let w = (s.col_end.min(area.width as usize) - s.col_start) as u16;
        let rect = Rect::new(
            area.x + s.col_start as u16,
            area.y + (s.row - top) as u16,
            w,
            1,
        );
        buf.set_style(rect, Style::new().fg(p.text).bg(p.selection_bg));
    }
    if on_screen(cursor.row) && (cursor.col as u16) < area.width {
        let (x, y) = (
            area.x + cursor.col as u16,
            area.y + (cursor.row - top) as u16,
        );
        // Inside a selection the cursor keeps the cursorline background,
        // so it reads as the cursor rather than a selected cell.
        let mut style = Style::new().add_modifier(Modifier::REVERSED);
        if app.visual_kind().is_some() {
            style = style.bg(p.cursor_bg);
        }
        buf.set_style(Rect::new(x, y, 1, 1), style);
    }

    if let Some(banner) = app.banner() {
        let rect = Rect::new(area.x, area.y, area.width, 1.min(area.height));
        frame.render_widget(
            Paragraph::new(format!(" {banner}"))
                .style(Style::new().fg(p.red).bg(p.bg).add_modifier(Modifier::BOLD)),
            rect,
        );
    }
}
