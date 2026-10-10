//! Marks within a page (`m{a-z}`, `'{a-z}`) and path/link yank (`yf`, `yF`, `yu`).

use std::collections::HashMap;
use std::path::PathBuf;

use super::App;

/// Marks per page, keyed by path (`None` for the stdin page). Each mark is
/// a source byte, so it survives resize and reload.
pub(super) type Marks = HashMap<Option<PathBuf>, HashMap<char, usize>>;

impl App {
    fn page_key(&self) -> Option<Option<PathBuf>> {
        self.page.as_ref().map(|p| p.path.clone())
    }

    /// Source byte under the cursor, or the first drawn byte at or above
    /// it (blank rows have none).
    pub(super) fn cursor_byte(&self) -> Option<usize> {
        let map = &self.page.as_ref()?.rendered.srcmap;
        map.source_at(self.cursor.row, self.cursor.col)
            .or_else(|| (0..self.cursor.row).rev().find_map(|r| map.source_at(r, 0)))
    }

    /// `m{a-z}`.
    pub(super) fn set_mark(&mut self, c: char) {
        let (Some(key), Some(byte)) = (self.page_key(), self.cursor_byte()) else {
            return;
        };
        self.marks.entry(key).or_default().insert(c, byte);
    }

    /// `'{a-z}`: the start of the marked row. False (cursor unchanged)
    /// when the mark is not set or no longer on the page.
    pub(super) fn goto_mark(&mut self, c: char) -> bool {
        let Some(key) = self.page_key() else {
            return false;
        };
        let Some(&byte) = self.marks.get(&key).and_then(|m| m.get(&c)) else {
            self.set_status(format!("Mark not set: {c}"));
            return false;
        };
        let Some(row) = self
            .page
            .as_ref()
            .and_then(|p| p.rendered.srcmap.row_for(byte))
        else {
            return false;
        };
        self.goto_row(row);
        true
    }

    /// `yu` has something to copy: a link under the cursor or a file path.
    pub(crate) fn can_yank(&self) -> bool {
        self.link_under_cursor().is_some() || self.page.as_ref().is_some_and(|p| p.path.is_some())
    }

    /// `yu`: copy the link destination (as written) under the cursor, else
    /// the file path.
    pub(super) fn yank(&mut self) {
        if !self.can_yank() {
            self.set_status("Nothing to yank (stdin has no path)");
            return;
        }
        let link = self
            .link_under_cursor()
            .and_then(|i| self.page.as_ref()?.doc.links.get(i))
            .map(|l| l.dest.clone());
        match link {
            Some(dest) => self.copy_status(&dest),
            None => self.yank_path(true),
        }
    }

    /// `yf` (absolute) / `yF` (relative to the tree root): copy the file path.
    pub(super) fn yank_path(&mut self, absolute: bool) {
        let Some(p) = self.page.as_ref().and_then(|p| p.path.as_deref()) else {
            self.set_status("No file (stdin)");
            return;
        };
        let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
        let text = if absolute {
            abs.display().to_string()
        } else {
            let root =
                std::path::absolute(&self.tree_root).unwrap_or_else(|_| self.tree_root.clone());
            abs.strip_prefix(&root)
                .unwrap_or(&abs)
                .display()
                .to_string()
        };
        self.copy_status(&text);
    }

    fn copy_status(&mut self, text: &str) {
        match self.clipboard.copy(text) {
            Ok(()) => self.set_status(format!("Copied {text}")),
            Err(e) => self.set_status(format!("copy failed: {e:#}")),
        }
    }
}
