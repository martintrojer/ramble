//! Where the last frame drew each pane and popup, so mouse events can be
//! mapped back to what is under the pointer (docs/specs/2026-10-08-mouse.md
//! § D2). `ui::draw` records a [`Layout`] at the end of every frame.

use ratatui::layout::{Position, Rect};

use super::App;

/// A list drawn in `pane`: its items start at `items.y`, the first drawn
/// item being number `skip`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ListArea {
    /// The whole pane (title row included).
    pub pane: Rect,
    /// The rows items are drawn in; empty when the list shows no items.
    pub items: Rect,
    /// Index of the item drawn on the first row of `items`.
    pub skip: usize,
}

impl ListArea {
    fn index(&self, p: Position) -> Option<usize> {
        self.items
            .contains(p)
            .then(|| self.skip + (p.y - self.items.y) as usize)
    }
}

/// Rects of the last drawn frame. `Default` (nothing drawn) hits nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Layout {
    /// The whole sidebar, its border included (on the side facing the
    /// content).
    pub sidebar: Option<Rect>,
    pub files: Option<ListArea>,
    pub outline: Option<ListArea>,
    pub gutter: Option<Rect>,
    pub text: Option<Rect>,
    /// The deleted-file banner covers the first text row.
    pub banner: bool,
    pub status: Option<Rect>,
    /// The picker box and its item list.
    pub picker: Option<ListArea>,
    pub help: Option<Rect>,
    pub hover: Option<Rect>,
    pub clue: Option<Rect>,
}

/// What is under a screen cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    None,
    /// A content cell: rendered row (scroll added) and display column.
    Text {
        row: usize,
        col: usize,
    },
    /// A files-pane item; `x` is the column inside the pane.
    Files {
        index: usize,
        x: u16,
    },
    Outline(usize),
    PickerItem(usize),
    /// Inside the picker box, not on an item.
    Picker,
    Help,
    Hover,
    Clue,
    Status,
    Gutter,
    /// The sidebar's border.
    Border,
}

fn inside(r: Option<Rect>, p: Position) -> bool {
    r.is_some_and(|r| r.contains(p))
}

impl App {
    /// Record where the frame just drawn put everything.
    pub fn set_layout(&self, layout: Layout) {
        if let Some(f) = layout.files {
            self.set_sidebar_list_top(super::Focus::Files, f.skip);
        }
        if let Some(o) = layout.outline {
            self.set_sidebar_list_top(super::Focus::Outline, o.skip);
        }
        self.layout.set(layout);
    }

    /// The layout of the last drawn frame.
    pub fn layout(&self) -> Layout {
        self.layout.get()
    }

    /// What is under (`col`, `row`): popups first, top-most first, then
    /// the panes.
    pub fn hit(&self, col: u16, row: u16) -> Hit {
        let l = self.layout.get();
        let p = Position::new(col, row);
        if inside(l.help, p) {
            return Hit::Help;
        }
        if let Some(pk) = l.picker
            && pk.pane.contains(p)
        {
            return pk.index(p).map_or(Hit::Picker, Hit::PickerItem);
        }
        if inside(l.clue, p) {
            return Hit::Clue;
        }
        if inside(l.hover, p) {
            return Hit::Hover;
        }
        if inside(l.status, p) {
            return Hit::Status;
        }
        if let Some(f) = l.files
            && f.pane.contains(p)
        {
            return f.index(p).map_or(Hit::None, |index| Hit::Files {
                index,
                x: col - f.pane.x,
            });
        }
        if let Some(o) = l.outline
            && o.pane.contains(p)
        {
            return o.index(p).map_or(Hit::None, Hit::Outline);
        }
        if inside(l.sidebar, p) {
            return Hit::Border;
        }
        if inside(l.gutter, p) {
            return Hit::Gutter;
        }
        if let Some(t) = l.text
            && t.contains(p)
        {
            if l.banner && row == t.y {
                return Hit::None;
            }
            return Hit::Text {
                row: self.scroll + (row - t.y) as usize,
                col: (col - t.x) as usize,
            };
        }
        Hit::None
    }
}
