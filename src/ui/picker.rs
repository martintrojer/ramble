//! The picker overlay (centered list with a query line, and a key
//! footer for the review picker) and the `:`
//! command prompt on the status row.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};

use crate::app::{App, ListArea};
use crate::render::palette;

/// Background of the overlay (Catppuccin Mocha base).
const BG: Color = Color::Rgb(0x1e, 0x1e, 0x2e);
/// Background of the selected row (Catppuccin Mocha surface0).
const SEL_BG: Color = Color::Rgb(0x31, 0x32, 0x44);

/// Draw the open picker; returns its box and item list.
pub(super) fn draw(frame: &mut Frame, app: &App, area: Rect) -> Option<ListArea> {
    let p = app.picker()?;
    let width = area.width.saturating_sub(4).clamp(1, 80);
    let height = area.height.saturating_sub(2).clamp(1, 20);
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, rect);
    let base = Style::new().fg(palette::TEXT).bg(BG);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(palette::OVERLAY))
        .title(format!(" {} ", p.title))
        .style(base);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let footer_h = u16::from(p.footer.is_some());
    let [query, list, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(footer_h),
    ])
    .areas(inner);
    if let Some(f) = p.footer {
        frame.render_widget(Paragraph::new(f).style(base.fg(palette::OVERLAY)), footer);
    }
    let prefix = if p.prompting { "query: " } else { "> " };
    let q = format!("{prefix}{}", p.input);
    let x = query.x + Span::raw(q.as_str()).width() as u16;
    frame.render_widget(Paragraph::new(q).style(base.fg(palette::BLUE)), query);
    frame.set_cursor_position((x.min(query.right().saturating_sub(1)), query.y));
    let note = if p.loading {
        Some("Loading…")
    } else if p.prompting {
        Some("Enter sends the query to zk")
    } else if p.items.is_empty() {
        Some("No matches")
    } else {
        None
    };
    let mut hit = ListArea {
        pane: rect,
        items: Rect::default(),
        skip: 0,
    };
    if let Some(note) = note {
        frame.render_widget(Paragraph::new(note).style(base.fg(palette::OVERLAY)), list);
        return Some(hit);
    }
    let count = p.items.len();
    let rows: Vec<ListItem> = p
        .items
        .iter()
        .map(|it| {
            ListItem::new(Line::from(vec![
                Span::raw(it.label.clone()),
                Span::styled(format!("  {}", it.detail), base.fg(palette::OVERLAY)),
            ]))
        })
        .collect();
    // Start from the last frame's offset, so a click does not shift the
    // list under the pointer; ratatui scrolls only to show the selection.
    let prev = app.layout().picker.map_or(0, |l| l.skip);
    let mut state = ListState::default()
        .with_selected(Some(p.selected))
        .with_offset(prev);
    frame.render_stateful_widget(
        List::new(rows)
            .style(base)
            .highlight_style(Style::new().bg(SEL_BG).add_modifier(Modifier::BOLD)),
        list,
        &mut state,
    );
    hit.skip = state.offset();
    let shown = count.saturating_sub(hit.skip).min(list.height as usize) as u16;
    hit.items = Rect::new(list.x, list.y, list.width, shown);
    Some(hit)
}

/// The `:` prompt (or the `comment: ` prompt, scrolled to its cursor)
/// over the status line.
pub(super) fn draw_cmdline(frame: &mut Frame, app: &App, area: Rect) {
    let (prompt, x) = if let Some((p, col)) = app.comment_prompt_view(area.width as usize) {
        (p, area.x.saturating_add(col as u16))
    } else if let Some(p) = app.cmdline_prompt() {
        let x = area.x + Span::raw(p.as_str()).width() as u16;
        (p, x)
    } else {
        return;
    };
    frame.render_widget(
        Paragraph::new(prompt).style(Style::new().fg(palette::TEXT).bg(BG)),
        area,
    );
    frame.set_cursor_position((x.min(area.right().saturating_sub(1)), area.y));
}
