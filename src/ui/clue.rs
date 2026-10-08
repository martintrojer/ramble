//! The key clue: a bordered box at the bottom right listing the keys that
//! can follow the pending sequence. Rows wrap into columns; what still
//! doesn't fit ends in `… N more`.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::app::{App, ClueRow};
use crate::render::palette;

/// Background of the box (Catppuccin Mocha base).
const BG: Color = Color::Rgb(0x1e, 0x1e, 0x2e);
/// Columns between a key and its description, and between columns.
const GAP: usize = 2;

/// One drawn column: its rows and the width of its key column.
struct Col<'a> {
    rows: Vec<(&'a str, String, bool)>,
    key_w: usize,
}

impl Col<'_> {
    fn width(&self) -> usize {
        self.rows
            .iter()
            .map(|(_, d, _)| self.key_w + GAP + d.width())
            .max()
            .unwrap_or(0)
    }
}

fn column<'a>(rows: &'a [ClueRow], more: Option<usize>) -> Col<'a> {
    let mut out: Vec<(&str, String, bool)> = rows
        .iter()
        .map(|r| (r.key.as_str(), r.desc.clone(), r.group))
        .collect();
    if let Some(n) = more {
        out.push(("…", format!("{n} more"), true));
    }
    let key_w = out.iter().map(|(k, ..)| k.width()).max().unwrap_or(0);
    Col { rows: out, key_w }
}

/// Split `rows` into columns of `per` rows that fit in `width` inner
/// columns; the last visible slot says how many rows were left out.
fn layout(rows: &[ClueRow], per: usize, width: usize) -> Vec<Col<'_>> {
    let chunks: Vec<&[ClueRow]> = rows.chunks(per).collect();
    let mut cols: Vec<Col> = Vec::new();
    let mut used = 0;
    for chunk in &chunks {
        let col = column(chunk, None);
        let add = col.width() + if cols.is_empty() { 0 } else { GAP };
        if used + add <= width || cols.is_empty() {
            used += add;
            cols.push(col);
            continue;
        }
        // Out of room: the last shown column gives its last row to `… N more`.
        let last = cols.len() - 1;
        let start = last * per;
        let hidden = rows.len() - (start + per - 1);
        cols[last] = column(&rows[start..start + per - 1], Some(hidden));
        return cols;
    }
    cols
}

/// Draw the key clue box; returns its rect.
pub(super) fn draw(frame: &mut Frame, app: &App, area: Rect) -> Option<Rect> {
    if !app.clue_visible() || area.width < 6 || area.height < 3 {
        return None;
    }
    let rows = app.clue_rows();
    let max_h = (area.height / 2).max(3);
    let per = (max_h as usize - 2).max(1);
    let max_inner = area.width.saturating_sub(4 + 2) as usize;
    let cols = layout(&rows, per, max_inner);
    let title = format!(" {} ", app.clue_title());
    let content_w = cols.iter().map(Col::width).sum::<usize>() + GAP * (cols.len() - 1);
    let inner_w = content_w.max(title.width()).min(max_inner);
    let inner_h = cols.iter().map(|c| c.rows.len()).max().unwrap_or(0);
    let w = inner_w as u16 + 2;
    let h = inner_h as u16 + 2;
    let rect = Rect::new(
        area.right().saturating_sub(w + 1),
        area.bottom().saturating_sub(h),
        w,
        h,
    )
    .intersection(area);
    frame.render_widget(Clear, rect);
    let base = Style::new().fg(palette::TEXT).bg(BG);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(palette::OVERLAY))
        .title(title)
        .style(base);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let mut x = inner.x;
    for col in &cols {
        let cw = col.width() as u16;
        let lines: Vec<Line> = col
            .rows
            .iter()
            .map(|(k, d, group)| {
                let pad = " ".repeat(col.key_w - k.width() + GAP);
                let ds = if *group {
                    base.fg(palette::MAUVE)
                } else {
                    base
                };
                Line::from(vec![
                    Span::styled(format!("{k}{pad}"), base.fg(palette::BLUE)),
                    Span::styled(d.clone(), ds),
                ])
            })
            .collect();
        let r = Rect::new(x, inner.y, cw, inner.height).intersection(inner);
        frame.render_widget(Paragraph::new(lines).style(base), r);
        x = x.saturating_add(cw + GAP as u16);
    }
    Some(rect)
}
