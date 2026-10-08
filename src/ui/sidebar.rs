//! The sidebar: file tree and/or outline, with a border on the right, and
//! the filter prompt over the status row.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::clip::{clip_end, clip_middle};
use crate::app::sidebar::OUTLINE_TITLE;
use crate::app::sidebar_width::{GUTTER_COLS, ICON_COLS as ICON};
use crate::app::{App, Focus, ListArea};
use crate::render::palette;

/// Background of the selected row (Catppuccin Mocha surface0).
const SELECTED_BG: Color = Color::Rgb(0x31, 0x32, 0x44);
/// Background of the filter prompt (Catppuccin Mocha mantle).
const PROMPT_BG: Color = Color::Rgb(0x18, 0x18, 0x25);
/// Columns a name or heading keeps at least before indenting stops.
const MIN_NAME: usize = 8;
/// The current-row marker (D8), drawn in the left gutter.
const CURRENT: &str = "▎";

/// Where a row of nesting `depth` starts in a pane `width` wide whose rows
/// put `lead` columns (the folder arrow) before the name: `(indent_cols,
/// arrow_col)`. `indent_cols` is the indent after the marker gutter;
/// `arrow_col` is where the arrow (or the text, with no lead) starts,
/// from the pane's left edge. Indenting stops at the depth that would
/// leave the name fewer than `MIN_NAME` columns, so deep rows share the
/// deepest indent that fits.
pub(crate) fn row_prefix(depth: usize, width: usize, lead: usize) -> (usize, usize) {
    let room = width.saturating_sub(GUTTER_COLS + lead + MIN_NAME);
    let indent = (2 * depth).min(room - room % 2);
    (indent, GUTTER_COLS + indent)
}

/// Column, from the files pane's left edge, of the 2-column `▸ `/`▾ `
/// cell of a folder row at `depth` in a pane `width` wide (after the
/// marker gutter and the capped indent; see [`row_prefix`]).
pub fn arrow_col(depth: usize, width: u16) -> u16 {
    row_prefix(depth, width as usize, ICON).1 as u16
}

/// The marker gutter of a row: `▎` in peach on the current row.
fn gutter(current: bool) -> Span<'static> {
    if current {
        Span::styled(CURRENT, Style::new().fg(palette::PEACH))
    } else {
        Span::raw(" ".repeat(GUTTER_COLS))
    }
}

/// Draw the sidebar; returns where the files and outline lists went.
pub(super) fn draw(
    frame: &mut Frame,
    app: &App,
    area: Rect,
) -> (Option<ListArea>, Option<ListArea>) {
    let block = Block::new()
        .borders(Borders::RIGHT)
        .border_style(Style::new().fg(palette::OVERLAY));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let files = app.tree().is_some();
    match app.sidebar_panes() {
        [Focus::Files, Focus::Outline] if files => {
            let pct = (app.sidebar_split_ratio() * 100.0).round() as u16;
            let [top, bottom] =
                Layout::vertical([Constraint::Percentage(pct), Constraint::Fill(1)]).areas(inner);
            (
                draw_files(frame, app, top),
                draw_outline(frame, app, bottom),
            )
        }
        [Focus::Files] if files => (draw_files(frame, app, inner), None),
        _ => (None, draw_outline(frame, app, inner)),
    }
}

fn title(text: String, focused: bool) -> Line<'static> {
    let mut style = Style::new().fg(palette::BLUE).add_modifier(Modifier::BOLD);
    if !focused {
        style = style.fg(palette::OVERLAY);
    }
    Line::from(Span::styled(text, style))
}

/// The first item to draw: `prev` (the last frame's) moved only as far as
/// needed to show `selected` in `body` rows, so a click does not shift the
/// list under the pointer. No selection draws from the top.
fn list_skip(prev: usize, selected: Option<usize>, body: usize, len: usize) -> usize {
    let Some(s) = selected else { return 0 };
    let prev = prev.min(len.saturating_sub(body));
    if s < prev {
        s
    } else if s >= prev + body {
        (s + 1).saturating_sub(body)
    } else {
        prev
    }
}

/// Draw `rows` under `head`, scrolled from `prev` so `selected` is visible;
/// returns where the items went (for mouse hit-testing).
fn draw_list(
    frame: &mut Frame,
    area: Rect,
    head: Line,
    rows: Vec<Line>,
    (selected, prev): (Option<usize>, usize),
) -> ListArea {
    if area.height == 0 {
        return ListArea::default();
    }
    let body = (area.height - 1) as usize;
    let skip = list_skip(prev, selected, body, rows.len());
    let shown = rows.len().saturating_sub(skip).min(body) as u16;
    let list = ListArea {
        pane: area,
        items: Rect::new(area.x, area.y + 1, area.width, shown),
        skip,
    };
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
    list
}

fn draw_files(frame: &mut Frame, app: &App, area: Rect) -> Option<ListArea> {
    let tree = app.tree()?;
    let items = tree.visible_items();
    let width = area.width as usize;
    let open = app.sidebar_current_file();
    let rows = items
        .iter()
        .map(|i| {
            let (indent, arrow_col) = row_prefix(i.depth, width, ICON);
            let (icon, style) = match (i.is_dir, i.expanded, i.markdown) {
                (true, true, _) => ("▾ ", Style::new().fg(palette::BLUE)),
                (true, false, _) => ("▸ ", Style::new().fg(palette::BLUE)),
                (false, _, true) => ("  ", Style::new().fg(palette::TEXT)),
                (false, _, false) => ("  ", Style::new().fg(palette::OVERLAY)),
            };
            let lead = arrow_col + ICON;
            let name_w = Span::raw(i.name.as_str()).width();
            let mark = review_mark(app, &i.path, width.saturating_sub(lead), name_w);
            let room = width.saturating_sub(lead + mark.width());
            let text = format!("{}{icon}{}", " ".repeat(indent), clip_middle(&i.name, room));
            let current = open.as_deref() == Some(i.path.as_path());
            Line::from(vec![gutter(current), Span::styled(text, style), mark])
        })
        .collect();
    let focused = app.focus() == Focus::Files;
    Some(draw_list(
        frame,
        area,
        title(clip_middle(&app.sidebar_files_title(), width), focused),
        rows,
        (
            tree.selected_index(&items),
            app.layout().files.map_or(0, |l| l.skip),
        ),
    ))
}

/// The review marker of a tree row with `room` columns after the indent
/// and icon, for a name `name_w` wide: `●`, plus the comment count when
/// the name fits beside it or keeps `MIN_NAME` columns; dim on folders.
/// Never clipped: the name gives way.
fn review_mark(app: &App, path: &std::path::Path, room: usize, name_w: usize) -> Span<'static> {
    match app.file_marker(path) {
        Some((c, Some(n))) => {
            let full = format!(" {c} {n}");
            let full_w = Span::raw(full.as_str()).width();
            let fits = name_w + full_w <= room || room >= full_w + MIN_NAME;
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

fn draw_outline(frame: &mut Frame, app: &App, area: Rect) -> Option<ListArea> {
    let width = area.width as usize;
    let rows = app
        .outline()
        .into_iter()
        .map(|o| {
            let (indent, text_col) = row_prefix(o.level.saturating_sub(1).into(), width, 0);
            let room = width.saturating_sub(text_col);
            Line::from(vec![
                gutter(o.current),
                Span::styled(
                    format!("{}{}", " ".repeat(indent), clip_end(&o.text, room)),
                    Style::new().fg(palette::TEXT),
                ),
            ])
        })
        .collect();
    let focused = app.focus() == Focus::Outline;
    Some(draw_list(
        frame,
        area,
        title(clip_end(OUTLINE_TITLE, width), focused),
        rows,
        (
            app.outline_selected(),
            app.layout().outline.map_or(0, |l| l.skip),
        ),
    ))
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

#[cfg(test)]
mod tests {
    use super::list_skip;

    #[test]
    fn list_skip_keeps_the_offset_while_the_selection_is_visible() {
        // 30 items, 6 rows.
        assert_eq!(list_skip(0, Some(29), 6, 30), 24, "jump to the end");
        assert_eq!(list_skip(24, Some(26), 6, 30), 24, "visible: stays");
        assert_eq!(list_skip(24, Some(20), 6, 30), 20, "above: scrolls up");
        assert_eq!(list_skip(10, Some(16), 6, 30), 11, "below: scrolls down");
        assert_eq!(list_skip(28, Some(27), 6, 30), 24, "clamped to the end");
        assert_eq!(list_skip(9, None, 6, 30), 0);
    }
}
