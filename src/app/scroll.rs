//! Scrolling and screen-relative motions (C-d/u/f/b/e/y, z*, H/M/L).

use super::App;

impl App {
    /// `C-d` (down) / `C-u` (up).
    pub(super) fn half_page(&mut self, down: bool) {
        let half = (self.viewport_height() / 2).max(1);
        let max = self.max_scroll();
        let row = if down {
            (self.cursor.row + half).min(self.last_row())
        } else {
            self.cursor.row.saturating_sub(half)
        };
        self.scroll = if down {
            (self.scroll + half).min(max)
        } else {
            self.scroll.saturating_sub(half)
        };
        self.move_to_row(row);
    }

    /// `C-f` / `PageDown`.
    pub(super) fn page_down(&mut self) {
        let new = (self.scroll + self.viewport_height()).min(self.max_scroll());
        if new == self.scroll {
            self.move_to_row(self.last_row());
        } else {
            self.scroll = new;
            self.move_to_row(self.cursor.row.max(new));
        }
    }

    /// `C-b` / `PageUp`.
    pub(super) fn page_up(&mut self) {
        let vh = self.viewport_height();
        let new = self.scroll.saturating_sub(vh);
        if new == self.scroll {
            self.move_to_row(0);
        } else {
            self.scroll = new;
            self.move_to_row(self.cursor.row.min(new + vh - 1));
        }
    }

    /// `C-e`.
    pub(super) fn line_down(&mut self) {
        self.scroll = (self.scroll + 1).min(self.max_scroll());
        if self.cursor.row < self.scroll {
            self.move_to_row(self.scroll);
        }
    }

    /// `C-y`.
    pub(super) fn line_up(&mut self) {
        let vh = self.viewport_height();
        self.scroll = self.scroll.saturating_sub(1);
        if self.cursor.row >= self.scroll + vh {
            self.move_to_row(self.scroll + vh - 1);
        }
    }

    /// `zz`.
    pub(super) fn center_cursor(&mut self) {
        let half = (self.viewport_height() - 1) / 2;
        self.scroll = self.cursor.row.saturating_sub(half).min(self.max_scroll());
    }

    /// `zt`.
    pub(super) fn cursor_to_top(&mut self) {
        self.scroll = self.cursor.row.min(self.max_scroll());
    }

    /// `zb`.
    pub(super) fn cursor_to_bottom(&mut self) {
        self.scroll = (self.cursor.row + 1).saturating_sub(self.viewport_height());
    }

    /// `H`: row `n` from the top of the view.
    pub(super) fn screen_top(&mut self, n: usize) {
        let bottom = self.visible_bottom();
        self.goto_row((self.scroll + n - 1).min(bottom));
    }

    /// `L`: row `n` from the bottom of the view.
    pub(super) fn screen_bottom(&mut self, n: usize) {
        let bottom = self.visible_bottom();
        self.goto_row(bottom.saturating_sub(n - 1).max(self.scroll));
    }

    /// `M`: middle of the view.
    pub(super) fn screen_middle(&mut self) {
        let mid = self.scroll + (self.visible_bottom() - self.scroll) / 2;
        self.goto_row(mid);
    }

    fn visible_bottom(&self) -> usize {
        (self.scroll + self.viewport_height() - 1).min(self.last_row())
    }

    /// Scroll just enough to show the cursor row.
    pub(super) fn keep_visible(&mut self) {
        let vh = self.viewport_height();
        if self.cursor.row < self.scroll {
            self.scroll = self.cursor.row;
        } else if self.cursor.row >= self.scroll + vh {
            self.scroll = self.cursor.row + 1 - vh;
        }
        self.comment_fit();
    }
}
