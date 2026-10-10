//! Loading documents and laying them out at the current width.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;

use super::motion::row_cells;
use super::{App, Cursor, Mode, Page};
use crate::doc::{self, Document};
use crate::render::{self, RenderedPage, SrcMap};

/// A layout-independent position: a source byte and whether it was taken
/// from a nearby row because the position was on a blank one.
pub(super) type Anchor = (usize, bool);

impl App {
    /// Load and render `path`, cursor to the top. An I/O error is returned;
    /// a binary file leaves the current page unchanged and sets the status.
    pub fn open_file(&mut self, path: &Path) -> anyhow::Result<()> {
        let bytes = std::fs::read(path).with_context(|| format!("{}", path.display()))?;
        self.open_bytes(path, &bytes);
        Ok(())
    }

    /// Show `bytes` as the page for `path`. False (page unchanged) when the
    /// bytes look binary.
    pub(super) fn open_bytes(&mut self, path: &Path, bytes: &[u8]) -> bool {
        let Some(doc) = doc::from_bytes(bytes) else {
            self.set_status("looks binary");
            return false;
        };
        let lossy = doc.lossy;
        self.loaded_hash = Some(super::watch::content_hash(bytes));
        self.set_page(Some(path.to_path_buf()), doc);
        if lossy {
            self.set_status("not valid UTF-8");
        }
        true
    }

    pub(super) fn open_stdin(&mut self, text: Arc<String>) {
        self.loaded_hash = None;
        self.set_page(None, doc::parse(text.as_ref().clone()));
        self.stdin = Some(text);
    }

    fn set_page(&mut self, path: Option<PathBuf>, mut doc: Document) {
        self.add_code_path_links(path.as_deref(), &mut doc);
        // Before the layout, so the first render uses the right width.
        let first = self.sidebar_page_loading();
        self.sidebar_auto_reading();
        let rendered = self.render_page(&doc, false, false);
        self.page = Some(Page {
            path,
            doc,
            rendered,
            raw: false,
            fm_expanded: false,
        });
        self.placeholder = None;
        self.rebuild_rows();
        self.cursor = Cursor::default();
        self.want_col = 0;
        self.scroll = 0;
        self.status.clear();
        self.stdin = None;
        self.banner = None;
        self.visual = Default::default();
        // The selection anchor and a pending `y` belong to the old page.
        if matches!(self.mode, Mode::Visual(_) | Mode::OpPending) {
            self.mode = Mode::Normal;
        }
        self.refresh_search();
        self.retarget_watch();
        self.lsp_page_changed();
        self.sync_tree();
        self.sidebar_page_loaded(first);
        self.review_page_changed();
    }

    /// Lay out `doc` at the current width, as raw source or rendered
    /// with its front matter folded or expanded.
    pub(super) fn render_page(&self, doc: &Document, raw: bool, fm_expanded: bool) -> RenderedPage {
        let width = self.render_width();
        if raw {
            render::render_raw(doc, width, &self.theme)
        } else {
            render::render_page(doc, width, &self.theme, fm_expanded)
        }
    }

    fn render_width(&self) -> u16 {
        let cols = self
            .size
            .0
            .saturating_sub(self.sidebar_cols())
            .saturating_sub(self.review_gutter());
        cols.min(self.config.render.max_width).max(1)
    }

    pub(super) fn rebuild_rows(&mut self) {
        self.rows = match &self.page {
            None => Vec::new(),
            Some(p) => p.rendered.lines.iter().map(row_cells).collect(),
        };
    }

    /// Where the cursor is, as a source byte (see [`App::anchor_at`]).
    pub(super) fn cursor_anchor(&self) -> Option<Anchor> {
        self.anchor_at(self.cursor)
    }

    /// Where `c` is, as a source byte: `(byte, fallback)` where `fallback`
    /// means `c` was on a blank row and `byte` comes from the nearest row
    /// with text.
    pub(super) fn anchor_at(&self, c: Cursor) -> Option<Anchor> {
        let p = self.page.as_ref()?;
        let map = &p.rendered.srcmap;
        if let Some(b) = map.source_at(c.row, c.col) {
            return Some((b, false));
        }
        let n = p.rendered.lines.len();
        (0..c.row)
            .rev()
            .chain(c.row + 1..n)
            .find_map(|r| map.source_at(r, 0))
            .map(|b| (b, true))
    }

    /// Row and (unsnapped) column of an anchor on the current layout.
    fn anchor_row_col(&self, (byte, fallback): Anchor) -> Option<(usize, usize)> {
        let map = &self.page.as_ref()?.rendered.srcmap;
        let row = map.row_for(byte)?;
        Some((row, if fallback { 0 } else { col_for(map, byte) }))
    }

    /// The position of an anchor from [`App::anchor_at`] on the current
    /// layout, on a grapheme boundary.
    pub(super) fn anchor_cursor(&self, anchor: Anchor) -> Option<Cursor> {
        let (row, col) = self.anchor_row_col(anchor)?;
        Some(Cursor {
            row,
            col: self.snap_col(row, col),
        })
    }

    /// Where `old`, which was at `anchor` before a re-layout, is now:
    /// `old` itself when it still maps to `anchor` (the layout did not move
    /// it; exact on blank rows too), else the anchor's new position.
    pub(super) fn reanchor(&self, old: Cursor, anchor: Option<Anchor>) -> Option<Cursor> {
        let a = anchor?;
        if self.anchor_at(old) == Some(a) {
            return Some(old);
        }
        self.anchor_cursor(a)
    }

    /// Put the cursor back on an anchor from [`App::cursor_anchor`].
    pub(super) fn restore_anchor(&mut self, anchor: Option<Anchor>) {
        if let Some((row, col)) = anchor.and_then(|a| self.anchor_row_col(a)) {
            self.cursor.row = row;
            self.want_col = col;
        }
        self.set_col(self.want_col);
    }

    /// Re-render for a new terminal size, keeping the cursor and the visual
    /// selections (active and `gv`) on the same source bytes.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let anchor = self.cursor_anchor();
        let visual = self.visual_anchors();
        self.size = (cols, rows);
        if let Some(p) = &self.page {
            let rendered = self.render_page(&p.doc, p.raw, p.fm_expanded);
            if let Some(p) = &mut self.page {
                p.rendered = rendered;
            }
        }
        self.rebuild_rows();
        self.refresh_search();
        self.restore_anchor(anchor);
        self.restore_visual_anchors(visual);
        self.keep_visible();
    }
}

/// Screen column of `byte` on the first segment drawn at or after it.
fn col_for(map: &SrcMap, byte: usize) -> usize {
    let Some(s) = map.segments.iter().find(|s| s.src.end > byte) else {
        return 0;
    };
    if byte <= s.src.start {
        return s.span.col_start;
    }
    let cols = s.span.col_end - s.span.col_start;
    let len = (s.src.end - s.src.start).max(1);
    s.span.col_start + (byte - s.src.start) * cols / len
}
