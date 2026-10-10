//! Loading documents and laying them out at the current width.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;

use super::motion::row_cells;
use super::{App, Cursor, Mode, Page};
use crate::doc::{self, Document};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::render::{self, RenderedPage, Segment, SrcMap};

/// A layout-independent position: a source byte and a row offset. Offset
/// 0: the position is on that byte. Otherwise it is on a row with no
/// source, that many rows below (positive) or above (negative) the row
/// drawing the byte.
pub(super) type Anchor = (usize, isize);

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

    /// Where `c` is, layout-independently: the source byte drawn at `c`
    /// (offset 0). On a row with no source (a blank separator) the last
    /// byte of the nearest row above with source and the number of rows
    /// down from it, or, above the first such row, the first byte of the
    /// next one and the (negative) rows up from it.
    pub(super) fn anchor_at(&self, c: Cursor) -> Option<Anchor> {
        let p = self.page.as_ref()?;
        let map = &p.rendered.srcmap;
        if let Some(b) = source_at(map, &p.doc.source, c.row, c.col) {
            return Some((b, 0));
        }
        let rows = |r: usize| self.row_segments(r);
        if let Some(r) = (0..c.row).rev().find(|&r| !rows(r).is_empty()) {
            let b = rows(r).iter().map(|s| s.src.end - 1).max()?;
            return Some((b, (c.row - r) as isize));
        }
        let n = p.rendered.lines.len();
        let r = (c.row + 1..n).find(|&r| !rows(r).is_empty())?;
        let b = rows(r).iter().map(|s| s.src.start).min()?;
        Some((b, c.row as isize - r as isize))
    }

    /// Row and (unsnapped) column of an anchor on the current layout.
    fn anchor_row_col(&self, (byte, off): Anchor) -> Option<(usize, usize)> {
        let p = self.page.as_ref()?;
        let map = &p.rendered.srcmap;
        let row = map.row_for(byte)?;
        if off == 0 {
            return Some((row, col_for(map, &p.doc.source, byte)));
        }
        let last = p.rendered.lines.len().saturating_sub(1);
        Some((row.saturating_add_signed(off).min(last), 0))
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

    /// Put the cursor back on an anchor from [`App::cursor_anchor`]. From
    /// a blank row it lands at the start of the row holding the anchor
    /// byte (the row above), not on the blank row.
    pub(super) fn restore_anchor(&mut self, anchor: Option<Anchor>) {
        let at = anchor.and_then(|(b, off)| self.anchor_row_col((b, 0)).map(|rc| (rc, off)));
        if let Some(((row, col), off)) = at {
            self.cursor.row = row;
            self.want_col = if off == 0 { col } else { 0 };
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

/// The segments drawn on `row`.
pub(super) fn row_segments(map: &SrcMap, row: usize) -> &[Segment] {
    let lo = map.segments.partition_point(|s| s.span.row < row);
    let hi = map.segments.partition_point(|s| s.span.row <= row);
    &map.segments[lo..hi]
}

/// Columns a source grapheme takes when drawn as text (see
/// `render::push_grapheme`): a tab four, other control characters one.
fn drawn_width(g: &str) -> usize {
    if g == "\t" {
        4
    } else if g.chars().any(char::is_control) {
        1
    } else {
        g.width()
    }
}

/// Start byte and column of each source grapheme of `s`, when they add
/// up to its drawn width (the text is drawn as is); `None` when the drawn
/// text differs from the source (escapes, entities, math).
fn grapheme_cols(s: &Segment, source: &str) -> Option<Vec<(usize, usize)>> {
    let text = source.get(s.src.clone())?;
    let mut col = s.span.col_start;
    let mut out = Vec::new();
    for (i, g) in text.grapheme_indices(true) {
        let w = drawn_width(g);
        if w > 0 {
            out.push((s.src.start + i, col));
        }
        col += w;
    }
    (col == s.span.col_end).then_some(out)
}

/// The source byte drawn at (`row`, `col`): exactly the grapheme there
/// when the segment's graphemes add up to its width, else
/// [`SrcMap::source_at`]'s scaled estimate.
fn source_at(map: &SrcMap, source: &str, row: usize, col: usize) -> Option<usize> {
    let exact = row_segments(map, row)
        .iter()
        .find(|s| (s.span.col_start..s.span.col_end).contains(&col))
        .and_then(|s| grapheme_cols(s, source))
        .and_then(|gs| gs.into_iter().rev().find(|&(_, c)| c <= col))
        .map(|(b, _)| b);
    exact.or_else(|| map.source_at(row, col))
}

/// Screen column of `byte` on the first segment drawn at or after it:
/// the column of the grapheme holding it, exactly when the segment's
/// graphemes add up to its width, else scaled.
fn col_for(map: &SrcMap, source: &str, byte: usize) -> usize {
    let Some(s) = map.segments.iter().find(|s| s.src.end > byte) else {
        return 0;
    };
    if byte <= s.src.start {
        return s.span.col_start;
    }
    if let Some(gs) = grapheme_cols(s, source) {
        return gs
            .iter()
            .rev()
            .find(|&&(b, _)| b <= byte)
            .map_or(s.span.col_start, |&(_, c)| c);
    }
    let cols = s.span.col_end - s.span.col_start;
    let len = (s.src.end - s.src.start).max(1);
    s.span.col_start + (byte - s.src.start) * cols / len
}
