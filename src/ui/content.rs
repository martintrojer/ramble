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
/// A search hit, and the hit under the cursor (Catppuccin Mocha base on
/// yellow / peach).
const HIT: Style = Style::new().fg(BANNER_BG).bg(palette::YELLOW);
const CURRENT_HIT: Style = Style::new().fg(BANNER_BG).bg(palette::PEACH);
/// The visual selection (Catppuccin Mocha surface1).
const SELECTION_BG: Color = Color::Rgb(0x45, 0x47, 0x5a);
/// A link a diagnostic covers (e.g. a dead link).
const BROKEN_LINK: Style = Style::new()
    .fg(palette::OVERLAY)
    .add_modifier(Modifier::DIM)
    .remove_modifier(Modifier::UNDERLINED);
/// Background of the deleted-file banner (Catppuccin Mocha red on base).
const BANNER_BG: Color = Color::Rgb(0x1e, 0x1e, 0x2e);

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
            BROKEN_LINK,
        );
    }
    // Cursorline first, so search hits on the cursor row keep their colours.
    if on_screen(cursor.row) {
        let y = area.y + (cursor.row - top) as u16;
        buf.set_style(
            Rect::new(area.x, y, area.width, 1),
            Style::new().bg(CURSORLINE_BG),
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
        buf.set_style(rect, if current { CURRENT_HIT } else { HIT });
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
        buf.set_style(rect, Style::new().fg(palette::TEXT).bg(SELECTION_BG));
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
            style = style.bg(CURSORLINE_BG);
        }
        buf.set_style(Rect::new(x, y, 1, 1), style);
    }

    if let Some(banner) = app.banner() {
        let rect = Rect::new(area.x, area.y, area.width, 1.min(area.height));
        frame.render_widget(
            Paragraph::new(format!(" {banner}")).style(
                Style::new()
                    .fg(palette::RED)
                    .bg(BANNER_BG)
                    .add_modifier(Modifier::BOLD),
            ),
            rect,
        );
    }
}
