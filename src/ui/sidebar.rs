//! The sidebar: file tree and/or outline, with a border on the right, and
//! the filter prompt over the status row.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::app::{App, Focus};
use crate::render::palette;

/// Background of the selected row (Catppuccin Mocha surface0).
const SELECTED_BG: Color = Color::Rgb(0x31, 0x32, 0x44);
/// Background of the filter prompt (Catppuccin Mocha mantle).
const PROMPT_BG: Color = Color::Rgb(0x18, 0x18, 0x25);

pub(super) fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::new()
        .borders(Borders::RIGHT)
        .border_style(Style::new().fg(palette::OVERLAY));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let files = app.tree().is_some();
    match app.sidebar_mode() {
        crate::config::SidebarMode::Split if files => {
            let pct = (app.sidebar_split_ratio() * 100.0).round() as u16;
            let [top, bottom] =
                Layout::vertical([Constraint::Percentage(pct), Constraint::Fill(1)]).areas(inner);
            draw_files(frame, app, top);
            draw_outline(frame, app, bottom);
        }
        crate::config::SidebarMode::Files if files => draw_files(frame, app, inner),
        _ => draw_outline(frame, app, inner),
    }
}

fn title(text: String, focused: bool, filter: Option<&str>) -> Line<'static> {
    let mut style = Style::new().fg(palette::BLUE).add_modifier(Modifier::BOLD);
    if !focused {
        style = style.fg(palette::OVERLAY);
    }
    let text = match filter {
        Some(f) => format!("{text} /{f}"),
        None => text,
    };
    Line::from(Span::styled(text, style))
}

/// Draw `rows` under `head`, scrolled so `selected` is visible.
fn draw_list(frame: &mut Frame, area: Rect, head: Line, rows: Vec<Line>, selected: Option<usize>) {
    if area.height == 0 {
        return;
    }
    let body = (area.height - 1) as usize;
    let skip = selected.map_or(0, |s| (s + 1).saturating_sub(body));
    let mut lines = vec![head];
    lines.extend(rows.into_iter().skip(skip).take(body));
    frame.render_widget(Paragraph::new(lines), area);
    if let Some(s) = selected
        && body > 0
    {
        let y = area.y + 1 + (s - skip) as u16;
        frame.buffer_mut().set_style(
            Rect::new(area.x, y, area.width, 1),
            Style::new().bg(SELECTED_BG),
        );
    }
}

fn draw_files(frame: &mut Frame, app: &App, area: Rect) {
    let Some(tree) = app.tree() else { return };
    let items = tree.visible_items();
    let rows = items
        .iter()
        .map(|i| {
            let indent = "  ".repeat(i.depth);
            let (icon, style) = match (i.is_dir, i.expanded, i.markdown) {
                (true, true, _) => ("▾ ", Style::new().fg(palette::BLUE)),
                (true, false, _) => ("▸ ", Style::new().fg(palette::BLUE)),
                (false, _, true) => ("  ", Style::new().fg(palette::TEXT)),
                (false, _, false) => ("  ", Style::new().fg(palette::OVERLAY)),
            };
            let text = format!("{indent}{icon}{}", i.name);
            let mark = review_mark(app, &i.path, Span::raw(text.as_str()).width(), area.width);
            Line::from(vec![Span::styled(text, style), mark])
        })
        .collect();
    let focused = app.focus() == Focus::Files;
    draw_list(
        frame,
        area,
        title(app.sidebar_title(), focused, tree.filter()),
        rows,
        tree.selected_index(&items),
    );
}

/// The review marker after a tree row of `used` columns: `●`, plus the
/// comment count when it fits in `width`; dim on folders.
fn review_mark(app: &App, path: &std::path::Path, used: usize, width: u16) -> Span<'static> {
    match app.file_marker(path) {
        Some((c, Some(n))) => {
            let full = format!(" {c} {n}");
            let fits = used + Span::raw(full.as_str()).width() <= width as usize;
            let text = if fits { full } else { format!(" {c}") };
            Span::styled(text, Style::new().fg(palette::PEACH))
        }
        Some((c, None)) => Span::styled(
            format!(" {c}"),
            Style::new().fg(palette::PEACH).add_modifier(Modifier::DIM),
        ),
        None => Span::raw(""),
    }
}

fn draw_outline(frame: &mut Frame, app: &App, area: Rect) {
    let rows = app
        .outline()
        .into_iter()
        .map(|o| {
            let indent = "  ".repeat(o.level.saturating_sub(1) as usize);
            let mark = if o.current { " ◂" } else { "" };
            Line::from(vec![
                Span::styled(
                    format!("{indent}{}", o.text),
                    Style::new().fg(palette::TEXT),
                ),
                Span::styled(mark, Style::new().fg(palette::PEACH)),
            ])
        })
        .collect();
    let focused = app.focus() == Focus::Outline;
    draw_list(
        frame,
        area,
        title("Outline".into(), focused, None),
        rows,
        app.outline_selected(),
    );
}

/// The filter prompt over the status row while one is being typed.
pub(super) fn draw_prompt(frame: &mut Frame, app: &App, area: Rect) {
    let Some(prompt) = app.filter_prompt() else {
        return;
    };
    let x = area.x + Span::raw(prompt.as_str()).width() as u16;
    frame.render_widget(
        Paragraph::new(prompt).style(Style::new().fg(palette::TEXT).bg(PROMPT_BG)),
        area,
    );
    frame.set_cursor_position((x.min(area.right().saturating_sub(1)), area.y));
}
