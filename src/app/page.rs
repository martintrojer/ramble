//! Loading documents and laying them out at the current width.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;

use super::motion::row_cells;
use super::{App, Cursor, Mode, Page};
use crate::doc::{self, Document};
use crate::render::{self, RenderedPage, SrcMap};

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
        self.set_page(Some(path.to_path_buf()), doc);
        if lossy {
            self.set_status("not valid UTF-8");
        }
        true
    }

    pub(super) fn open_stdin(&mut self, text: Arc<String>) {
        self.set_page(None, doc::parse(text.as_ref().clone()));
        self.stdin = Some(text);
    }

    fn set_page(&mut self, path: Option<PathBuf>, mut doc: Document) {
        self.add_code_path_links(path.as_deref(), &mut doc);
        // Before the layout, so the first render uses the right width.
        self.sidebar_auto_reading();
        let rendered = self.render_page(&doc, false);
        self.page = Some(Page {
            path,
            doc,
            rendered,
            raw: false,
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
        self.review_page_changed();
    }

    /// Lay out `doc` at the current width, as raw source or rendered.
    pub(super) fn render_page(&self, doc: &Document, raw: bool) -> RenderedPage {
        let width = self.render_width();
        if raw {
            render::render_raw(doc, width, &self.theme)
        } else {
            render::render(doc, width, &self.theme)
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

    /// Where the cursor is, as a source byte: `(byte, fallback)` where
    /// `fallback` means the cursor was on a blank row and `byte` comes from
    /// the nearest row with text.
    pub(super) fn cursor_anchor(&self) -> Option<(usize, bool)> {
        let p = self.page.as_ref()?;
        let map = &p.rendered.srcmap;
        if let Some(b) = map.source_at(self.cursor.row, self.cursor.col) {
            return Some((b, false));
        }
        let n = p.rendered.lines.len();
        (0..self.cursor.row)
            .rev()
            .chain(self.cursor.row + 1..n)
            .find_map(|r| map.source_at(r, 0))
            .map(|b| (b, true))
    }

    /// Put the cursor back on an anchor from [`App::cursor_anchor`].
    pub(super) fn restore_anchor(&mut self, anchor: Option<(usize, bool)>) {
        if let (Some((byte, fallback)), Some(p)) = (anchor, &self.page) {
            let map = &p.rendered.srcmap;
            if let Some(row) = map.row_for(byte) {
                let col = if fallback { 0 } else { col_for(map, byte) };
                self.cursor.row = row;
                self.want_col = col;
            }
        }
        self.set_col(self.want_col);
    }

    /// Re-render for a new terminal size, keeping the cursor on the same
    /// source byte.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let anchor = self.cursor_anchor();
        self.size = (cols, rows);
        if let Some(p) = &self.page {
            let rendered = self.render_page(&p.doc, p.raw);
            if let Some(p) = &mut self.page {
                p.rendered = rendered;
            }
        }
        self.rebuild_rows();
        self.refresh_search();
        self.restore_anchor(anchor);
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
