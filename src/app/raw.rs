//! Raw view (`gR`, `:Raw`): the current page's markdown source instead of
//! the rendered layout. Per page; history entries and live reload keep it.

use super::App;

impl App {
    /// Whether the current page shows its raw source.
    pub fn raw(&self) -> bool {
        self.page.as_ref().is_some_and(|p| p.raw)
    }

    /// `gR` / `:Raw`: toggle the raw view of the current page.
    pub(super) fn toggle_raw(&mut self) {
        if self.page.is_some() {
            self.set_raw(!self.raw());
        }
    }

    /// Show the current page raw (or rendered), keeping the cursor on the
    /// same source byte (and the visual selections), as resize does.
    pub(super) fn set_raw(&mut self, on: bool) {
        if self.page.is_none() || self.raw() == on {
            return;
        }
        let anchor = self.cursor_anchor();
        let visual = self.visual_anchors();
        self.relayout_raw(on);
        self.restore_anchor(anchor);
        self.restore_visual_anchors(visual);
        self.keep_visible();
    }

    /// Re-lay out the current page raw (or rendered) without moving the
    /// cursor. Reload and history restore call this before placing it.
    pub(super) fn relayout_raw(&mut self, on: bool) {
        let Some(p) = &self.page else { return };
        if p.raw == on {
            return;
        }
        let rendered = self.render_page(&p.doc, on, p.fm_expanded);
        if let Some(p) = &mut self.page {
            p.raw = on;
            p.rendered = rendered;
        }
        self.rebuild_rows();
        self.refresh_search();
    }
}
