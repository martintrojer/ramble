//! Front matter fold (`za`, `Enter` on the marker): the per-page
//! `fm_expanded` flag and the re-layout. Reloads of the same page keep it.

use super::App;

impl App {
    /// Whether the current page's front matter is expanded.
    pub fn front_matter_expanded(&self) -> bool {
        self.page.as_ref().is_some_and(|p| p.fm_expanded)
    }

    /// The cursor is on the front-matter marker row (the first row of a
    /// rendered page with front matter).
    pub(super) fn on_front_matter_marker(&self) -> bool {
        self.cursor.row == 0
            && self
                .page
                .as_ref()
                .is_some_and(|p| !p.raw && p.doc.front_matter.is_some())
    }

    /// `za`: fold or expand the front matter, keeping the cursor on the
    /// same source byte. Nothing without front matter or in raw view.
    pub(super) fn toggle_front_matter(&mut self) {
        let Some(p) = &self.page else { return };
        if p.raw || p.doc.front_matter.is_none() {
            return;
        }
        let on = !p.fm_expanded;
        self.set_front_matter_expanded(on);
    }

    /// Lay the page out with the front matter folded or expanded.
    pub(super) fn set_front_matter_expanded(&mut self, on: bool) {
        let Some(p) = &mut self.page else { return };
        if p.fm_expanded == on {
            return;
        }
        p.fm_expanded = on;
        // `resize` keeps the cursor on its source byte; the marker maps to
        // the opening fence, so a cursor on it stays there.
        let (cols, rows) = self.size;
        self.resize(cols, rows);
        self.keep_visible();
    }
}
