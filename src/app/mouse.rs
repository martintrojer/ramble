//! The mouse (docs/specs/2026-10-08-mouse.md): click to focus, select and
//! place the cursor; double and triple clicks; drag to select and copy;
//! the wheel scrolls the pane under the pointer. Hit-testing is in
//! `layout`. Effects reuse the key actions where one exists.

use std::time::{Duration, Instant};

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use super::keys::Action;
use super::layout::Hit;
use super::sidebar::SidebarAction;
use super::{App, Cursor, Focus, HelpAction, Mode, PickerAction, VisualKind};

/// Presses on the same cell this close together count as a double (then
/// triple) click.
pub const MULTI_CLICK: Duration = Duration::from_millis(400);
/// Rows (text, sidebar lists, help) one wheel notch moves.
pub const WHEEL_ROWS: usize = 3;

/// A left press in the text: where, and whether a drag started a
/// selection from it.
#[derive(Debug, Clone, Copy)]
struct Drag {
    anchor: Cursor,
    moved: bool,
}

/// Mouse state owned by [`App`].
#[derive(Debug, Clone, Default)]
pub(super) struct MouseState {
    /// The last left press: cell, time, and click count (1-3).
    last_press: Option<(u16, u16, Instant, u8)>,
    drag: Option<Drag>,
}

impl App {
    /// Handle one mouse event that arrived at `now`.
    pub(super) fn mouse(&mut self, ev: MouseEvent, now: Instant) {
        if !self.config.mouse.enabled {
            return;
        }
        let (col, row) = (ev.column, ev.row);
        match ev.kind {
            MouseEventKind::Down(MouseButton::Left) => self.mouse_press(col, row, now),
            MouseEventKind::Drag(MouseButton::Left) => self.mouse_drag(col, row),
            MouseEventKind::Up(MouseButton::Left) => self.mouse_release(),
            MouseEventKind::ScrollDown => self.mouse_wheel(col, row, true),
            MouseEventKind::ScrollUp => self.mouse_wheel(col, row, false),
            _ => {}
        }
    }

    /// Count this press as a single, double or triple click.
    fn click_count(&mut self, col: u16, row: u16, now: Instant) -> u8 {
        let n = match self.mouse.last_press {
            Some((c, r, t, n))
                if (c, r) == (col, row)
                    && n < 3
                    && now.saturating_duration_since(t) <= MULTI_CLICK =>
            {
                n + 1
            }
            _ => 1,
        };
        self.mouse.last_press = Some((col, row, now, n));
        n
    }

    /// Close the popup a press outside it lands next to. True when one was
    /// closed (the press then does nothing else).
    fn close_popup_outside(&mut self, hit: Hit) -> bool {
        let closed = match self.mode {
            Mode::Picker if !matches!(hit, Hit::Picker | Hit::PickerItem(_)) => {
                self.picker_action(PickerAction::Close);
                true
            }
            Mode::Help if hit != Hit::Help => {
                self.help_action(HelpAction::Close);
                true
            }
            _ if self.hover_popup().is_some() && hit != Hit::Hover => {
                self.hover_close();
                true
            }
            _ => false,
        };
        if closed {
            self.mouse.last_press = None;
        }
        closed
    }

    fn mouse_press(&mut self, col: u16, row: u16, now: Instant) {
        self.mouse.drag = None;
        let hit = self.hit(col, row);
        if self.close_popup_outside(hit) {
            return;
        }
        let typing = matches!(
            self.mode,
            Mode::Search
                | Mode::Filter
                | Mode::Command
                | Mode::Comment
                | Mode::Hint
                | Mode::OpPending
        );
        if typing {
            return;
        }
        let n = self.click_count(col, row, now);
        // Only clicks that act drop a half-typed key sequence and count;
        // status, clue, gutter and border clicks leave them (D3, D7).
        if matches!(hit, Hit::Text { .. } | Hit::Files { .. } | Hit::Outline(_)) {
            self.pending.clear();
            self.count = None;
        }
        match hit {
            Hit::Text { row, col } => self.click_text(row, col, n),
            Hit::Files { index, x } => {
                self.visual_leave();
                self.pane_select(Focus::Files, index);
                let width = self.layout().files.map_or(0, |f| f.pane.width);
                let on_arrow = self.folder_depth(index).is_some_and(|d| {
                    let a = crate::ui::sidebar::arrow_col(d, width);
                    (a..a + 2).contains(&x)
                });
                // The arrow toggles on every press; elsewhere a double
                // click opens (as Enter).
                if on_arrow || n == 2 {
                    self.sidebar_action(SidebarAction::Open);
                }
            }
            Hit::Outline(i) => {
                self.visual_leave();
                self.pane_select(Focus::Outline, i);
                if n == 2 {
                    self.sidebar_action(SidebarAction::Open);
                }
            }
            Hit::PickerItem(i) => {
                self.picker_select(i);
                if n == 2 {
                    self.picker_action(PickerAction::Accept);
                }
            }
            Hit::None
            | Hit::Picker
            | Hit::Help
            | Hit::Hover
            | Hit::Clue
            | Hit::Status
            | Hit::Gutter
            | Hit::Border => {}
        }
        self.keep_visible();
        self.clue_sync();
    }

    /// A press on rendered `row`, display column `col`, as click number `n`.
    fn click_text(&mut self, row: usize, col: usize, n: u8) {
        self.visual_leave();
        // FocusContent also closes a peek, even with the focus already on
        // the content (D11, P8).
        self.sidebar_action(SidebarAction::FocusContent);
        self.cursor.row = row.min(self.last_row());
        self.set_col(col);
        let at = self.cursor;
        match n {
            1 => {
                self.mouse.drag = Some(Drag {
                    anchor: at,
                    moved: false,
                })
            }
            2 if self.link_under_cursor().is_some() => self.apply(Action::Follow),
            2 => {
                if let Some((c1, c2)) = self.word_at(at.row, at.col) {
                    let (a, b) = (Cursor { col: c1, ..at }, Cursor { col: c2, ..at });
                    self.visual_select(VisualKind::Char, a, b);
                    self.yank_selection(false);
                }
            }
            _ => {
                self.visual_select(VisualKind::Line, at, at);
                self.yank_selection(false);
            }
        }
    }

    fn mouse_drag(&mut self, col: u16, row: u16) {
        let Some(mut d) = self.mouse.drag else { return };
        let Some(t) = self.layout().text else { return };
        if !matches!(self.mode, Mode::Normal | Mode::Visual(VisualKind::Char)) {
            self.mouse.drag = None;
            return;
        }
        // On or past the top row, or past the bottom edge: scroll one row
        // and extend there. The top text row counts as the edge because
        // the text starts at screen row 0, so nothing is reported above it.
        let vh = self.viewport_height();
        let target_row = if row <= t.y {
            self.scroll = self.scroll.saturating_sub(1);
            self.scroll
        } else if row >= t.bottom() {
            self.scroll = (self.scroll + 1).min(self.max_scroll());
            self.scroll + vh - 1
        } else {
            self.scroll + (row - t.y) as usize
        };
        let target = Cursor {
            row: target_row.min(self.last_row()),
            col: col.saturating_sub(t.x) as usize,
        };
        if d.moved {
            self.cursor.row = target.row;
            self.set_col(target.col);
        } else {
            self.cursor.row = target.row;
            self.set_col(target.col);
            if self.cursor == d.anchor {
                return;
            }
            let cursor = self.cursor;
            self.visual_select(VisualKind::Char, d.anchor, cursor);
            d.moved = true;
            self.mouse.drag = Some(d);
        }
    }

    fn mouse_release(&mut self) {
        let Some(d) = self.mouse.drag.take() else {
            return;
        };
        if d.moved && self.mode == Mode::Visual(VisualKind::Char) {
            self.yank_selection(false);
        }
    }

    /// The sidebar pane whose rect contains (`col`, `row`).
    fn sidebar_pane_at(&self, col: u16, row: u16) -> Option<Focus> {
        let l = self.layout();
        let p = ratatui::layout::Position::new(col, row);
        if l.files.is_some_and(|f| f.pane.contains(p)) {
            Some(Focus::Files)
        } else if l.outline.is_some_and(|o| o.pane.contains(p)) {
            Some(Focus::Outline)
        } else {
            None
        }
    }

    fn mouse_wheel(&mut self, col: u16, row: u16, down: bool) {
        let hit = self.hit(col, row);
        match self.mode {
            Mode::Picker => {
                if matches!(hit, Hit::Picker | Hit::PickerItem(_)) {
                    let a = if down {
                        PickerAction::Next
                    } else {
                        PickerAction::Prev
                    };
                    self.picker_action(a);
                }
            }
            Mode::Help => {
                if hit == Hit::Help {
                    let a = if down {
                        HelpAction::Down
                    } else {
                        HelpAction::Up
                    };
                    for _ in 0..WHEEL_ROWS {
                        self.help_action(a);
                    }
                }
            }
            // The wheel scrolls the page behind the comment box.
            Mode::Comment => {
                if matches!(hit, Hit::Text { .. } | Hit::Gutter) {
                    for _ in 0..WHEEL_ROWS {
                        if down {
                            self.line_down();
                        } else {
                            self.line_up();
                        }
                    }
                    self.comment_scrolled();
                    self.keep_visible();
                }
            }
            Mode::Normal | Mode::Visual(_) => {
                if let Some(f) = self.sidebar_pane_at(col, row)
                    && matches!(hit, Hit::Files { .. } | Hit::Outline(_) | Hit::None)
                {
                    let d = WHEEL_ROWS as isize;
                    let l = self.layout();
                    let pane = if f == Focus::Files {
                        l.files
                    } else {
                        l.outline
                    };
                    let body = pane.map_or(0, |p| p.pane.height.saturating_sub(1) as usize);
                    self.pane_wheel(f, if down { d } else { -d }, body);
                } else if matches!(hit, Hit::Text { .. } | Hit::Gutter) {
                    for _ in 0..WHEEL_ROWS {
                        if down {
                            self.line_down();
                        } else {
                            self.line_up();
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
