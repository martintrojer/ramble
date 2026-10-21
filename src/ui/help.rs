//! The `g?` help overlay: a centered box of grouped key bindings with a
//! filter line, scrolled by the app.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::{App, HelpLine, help_rect};

/// Columns given to the keys column (wider keys push the description).
const KEYS_W: usize = 18;

/// Draw the help overlay; returns its rect.
pub(super) fn draw(frame: &mut Frame, app: &App, area: Rect) -> Option<Rect> {
    let view = app.help_view()?;
    if area.width < 3 || area.height < 3 {
        return None;
    }
    let rect = help_rect(area);
    frame.render_widget(Clear, rect);
    let p = app.theme().palette;
    let base = Style::new().fg(p.text).bg(p.bg);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(p.muted))
        .title(" help ")
        .style(base);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let [query, list] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
    let q = if view.filtering || !view.filter.is_empty() {
        format!("/{}", view.filter)
    } else {
        "/ filter   q close".to_string()
    };
    let q_style = if view.filtering || !view.filter.is_empty() {
        base.fg(p.blue)
    } else {
        base.fg(p.muted)
    };
    if view.filtering {
        let x = query.x + Span::raw(q.as_str()).width() as u16;
        frame.set_cursor_position((x.min(query.right().saturating_sub(1)), query.y));
    }
    frame.render_widget(Paragraph::new(q).style(q_style), query);
    if view.lines.is_empty() {
        frame.render_widget(Paragraph::new("No matches").style(base.fg(p.muted)), list);
        return Some(rect);
    }
    let lines: Vec<Line> = view
        .lines
        .iter()
        .skip(view.scroll)
        .take(list.height as usize)
        .map(|l| match l {
            HelpLine::Group(t) => Line::from(Span::styled(
                t.to_string(),
                base.fg(p.purple).add_modifier(Modifier::BOLD),
            )),
            HelpLine::Item { keys, desc, dim } => {
                let pad = KEYS_W
                    .saturating_sub(Span::raw(keys.as_str()).width())
                    .max(1);
                let (k, d) = if *dim {
                    (base.fg(p.muted), base.fg(p.muted))
                } else {
                    (base.fg(p.blue), base)
                };
                Line::from(vec![
                    Span::styled(format!("  {keys}{}", " ".repeat(pad)), k),
                    Span::styled(desc.clone(), d),
                ])
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).style(base), list);
    Some(rect)
}
