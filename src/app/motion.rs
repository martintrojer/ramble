//! Cursor motions over grapheme cells of the rendered rows.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::App;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Class {
    Empty,
    Blank,
    Word,
    Punct,
}

/// One grapheme on a rendered row.
#[derive(Debug, Clone, Copy)]
pub(super) struct Cell {
    col: usize,
    class: Class,
}

/// A word-motion position: row plus grapheme index (0 on an empty row).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Pos {
    row: usize,
    idx: usize,
}

impl App {
    /// `$`: end of the line `n - 1` rows down.
    pub(super) fn line_end(&mut self, n: usize) {
        self.vertical(n as isize - 1);
        self.want_col = usize::MAX;
        self.clamp_cursor();
    }

    pub(super) fn repeat(&mut self, n: usize, f: fn(&Self, Pos) -> Pos) {
        let mut p = self.pos();
        for _ in 0..n {
            p = f(self, p);
        }
        self.set_pos(p);
    }

    /// Vertical move to `row`, keeping the desired column.
    pub(super) fn move_to_row(&mut self, row: usize) {
        self.cursor.row = row.min(self.last_row());
        self.clamp_cursor();
    }

    /// Jump to `row` at column 0 (gg, G, H, M, L).
    pub(super) fn goto_row(&mut self, row: usize) {
        self.cursor.row = row.min(self.last_row());
        self.set_col(0);
    }

    pub(super) fn vertical(&mut self, delta: isize) {
        let row = self.cursor.row.saturating_add_signed(delta);
        self.move_to_row(row);
    }

    pub(super) fn horizontal(&mut self, delta: isize) {
        let cells = self.cells(self.cursor.row);
        if cells.is_empty() {
            return;
        }
        let idx = self.cell_index(self.cursor.row, self.cursor.col);
        let new = idx
            .saturating_add_signed(delta)
            .min(cells.len().saturating_sub(1));
        self.set_col(cells[new].col);
    }

    pub(super) fn set_col(&mut self, col: usize) {
        self.want_col = col;
        self.clamp_cursor();
        self.want_col = self.cursor.col;
    }

    /// Put the cursor column at the grapheme under `want_col` on its row.
    pub(super) fn clamp_cursor(&mut self) {
        self.cursor.row = self.cursor.row.min(self.last_row());
        self.cursor.col = self.snap_col(self.cursor.row, self.want_col);
    }

    /// Column of the grapheme under `col` on `row` (0 on an empty row).
    pub(super) fn snap_col(&self, row: usize, col: usize) -> usize {
        let cells = self.cells(row);
        cells
            .iter()
            .rposition(|c| c.col <= col)
            .map_or(0, |i| cells[i].col)
    }

    pub(super) fn cells(&self, row: usize) -> &[Cell] {
        self.rows.get(row).map_or(&[], Vec::as_slice)
    }

    pub(super) fn cell_index(&self, row: usize, col: usize) -> usize {
        self.cells(row)
            .iter()
            .rposition(|c| c.col <= col)
            .unwrap_or(0)
    }

    /// First and last column of the vim `iw` word at (`row`, `col`): the
    /// run of cells on the row with the same class as the one under `col`.
    /// `None` on an empty row.
    pub(super) fn word_at(&self, row: usize, col: usize) -> Option<(usize, usize)> {
        let cells = self.cells(row);
        if cells.is_empty() {
            return None;
        }
        let i = self.cell_index(row, col);
        let class = cells[i].class;
        let first = cells[..i]
            .iter()
            .rposition(|c| c.class != class)
            .map_or(0, |j| j + 1);
        let last = cells[i..]
            .iter()
            .position(|c| c.class != class)
            .map_or(cells.len() - 1, |j| i + j - 1);
        Some((cells[first].col, cells[last].col))
    }

    pub(super) fn pos(&self) -> Pos {
        Pos {
            row: self.cursor.row,
            idx: self.cell_index(self.cursor.row, self.cursor.col),
        }
    }

    pub(super) fn set_pos(&mut self, p: Pos) {
        self.cursor.row = p.row.min(self.last_row());
        let col = self.cells(self.cursor.row).get(p.idx).map_or(0, |c| c.col);
        self.set_col(col);
    }

    pub(super) fn class(&self, p: Pos) -> Class {
        self.cells(p.row)
            .get(p.idx)
            .map_or(Class::Empty, |c| c.class)
    }

    pub(super) fn next(&self, p: Pos) -> Option<Pos> {
        if p.idx + 1 < self.cells(p.row).len() {
            Some(Pos {
                row: p.row,
                idx: p.idx + 1,
            })
        } else if p.row + 1 < self.total_rows() {
            Some(Pos {
                row: p.row + 1,
                idx: 0,
            })
        } else {
            None
        }
    }

    pub(super) fn prev(&self, p: Pos) -> Option<Pos> {
        if p.idx > 0 {
            Some(Pos {
                row: p.row,
                idx: p.idx - 1,
            })
        } else if p.row > 0 {
            let row = p.row - 1;
            Some(Pos {
                row,
                idx: self.cells(row).len().saturating_sub(1),
            })
        } else {
            None
        }
    }

    /// `w`: start of the next word; an empty row counts as a word.
    pub(super) fn word_forward(&self, p: Pos) -> Pos {
        let start = self.class(p);
        let mut q = p;
        if matches!(start, Class::Word | Class::Punct) {
            loop {
                let Some(n) = self.next(q) else { return q };
                let crossed = n.row != q.row;
                q = n;
                if crossed || self.class(q) != start {
                    break;
                }
            }
        } else {
            match self.next(q) {
                Some(n) => q = n,
                None => return q,
            }
        }
        while self.class(q) == Class::Blank {
            match self.next(q) {
                Some(n) => q = n,
                None => return q,
            }
        }
        q
    }

    /// `e`: end of the current or next word.
    pub(super) fn word_end(&self, p: Pos) -> Pos {
        let Some(mut q) = self.next(p) else { return p };
        while matches!(self.class(q), Class::Blank | Class::Empty) {
            match self.next(q) {
                Some(n) => q = n,
                None => return q,
            }
        }
        let c = self.class(q);
        while let Some(n) = self.next(q) {
            if n.row != q.row || self.class(n) != c {
                break;
            }
            q = n;
        }
        q
    }

    /// `b`: start of the current or previous word.
    pub(super) fn word_backward(&self, p: Pos) -> Pos {
        let Some(mut q) = self.prev(p) else { return p };
        while self.class(q) == Class::Blank {
            match self.prev(q) {
                Some(n) => q = n,
                None => return q,
            }
        }
        let c = self.class(q);
        if c == Class::Empty {
            return q;
        }
        while let Some(n) = self.prev(q) {
            if n.row != q.row || self.class(n) != c {
                break;
            }
            q = n;
        }
        q
    }

    pub(super) fn blank_row(&self, row: usize) -> bool {
        self.cells(row).iter().all(|c| c.class == Class::Blank)
    }

    /// `}`: the next blank row after the current paragraph (or the last row).
    pub(super) fn paragraph_forward(&self, p: Pos) -> Pos {
        let n = self.total_rows();
        let mut r = p.row;
        while r < n && self.blank_row(r) {
            r += 1;
        }
        while r < n && !self.blank_row(r) {
            r += 1;
        }
        Pos {
            row: r.min(self.last_row()),
            idx: 0,
        }
    }

    /// `{`: the previous blank row before the current paragraph (or row 0).
    pub(super) fn paragraph_backward(&self, p: Pos) -> Pos {
        let mut r = p.row;
        while r > 0 && self.blank_row(r) {
            r -= 1;
        }
        while r > 0 && !self.blank_row(r) {
            r -= 1;
        }
        Pos { row: r, idx: 0 }
    }
}

/// Grapheme cells of one rendered line.
pub(super) fn row_cells(line: &ratatui::text::Line<'_>) -> Vec<Cell> {
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    let mut col = 0;
    let mut cells = Vec::new();
    for g in text.graphemes(true) {
        let w = g.width();
        if w == 0 {
            continue;
        }
        let ch = g.chars().next().unwrap_or(' ');
        let class = if ch.is_whitespace() {
            Class::Blank
        } else if ch.is_alphanumeric() || ch == '_' {
            Class::Word
        } else {
            Class::Punct
        };
        cells.push(Cell { col, class });
        col += w;
    }
    cells
}
