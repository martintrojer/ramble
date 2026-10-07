//! Layout of a [`Document`](crate::doc::Document) for one width into
//! styled lines plus a source map.
//!
//! Interface fixed by the orchestrator before parallel units start; the
//! `render` unit (t03) implements [`render`] and the [`SrcMap`] methods.

use std::ops::Range;

use ratatui::text::Line;

use crate::doc::Document;

/// Colours and styles. `Theme::catppuccin_mocha()` is the default.
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    /// syntect theme name inside two-face's embedded set.
    pub code_theme: String,
}

impl Theme {
    pub fn catppuccin_mocha() -> Self {
        Self {
            name: "catppuccin-mocha".into(),
            code_theme: "catppuccin-mocha".into(),
        }
    }
}

/// A screen span: one drawn row and a column range on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScreenSpan {
    pub row: usize,
    pub col_start: usize,
    /// Exclusive.
    pub col_end: usize,
}

/// One drawn inline segment: the source bytes it came from and where it
/// was drawn. Segments never overlap on screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub src: Range<usize>,
    pub span: ScreenSpan,
    /// Index into `Document::links` when the segment is link text.
    pub link: Option<usize>,
}

/// Two-way map between source byte ranges and screen spans.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SrcMap {
    /// Sorted by (row, col_start).
    pub segments: Vec<Segment>,
}

impl SrcMap {
    /// Screen spans drawn from any byte in `src`. Empty when none of
    /// those bytes is drawn.
    pub fn spans_for(&self, src: Range<usize>) -> Vec<ScreenSpan> {
        let _ = src;
        todo!("t03_render")
    }

    /// The source byte drawn at (row, col), or the nearest drawn byte on
    /// that row, or `None` for an empty row.
    pub fn source_at(&self, row: usize, col: usize) -> Option<usize> {
        let _ = (row, col);
        todo!("t03_render")
    }

    /// The first screen row drawn from a byte at or after `byte`.
    pub fn row_for(&self, byte: usize) -> Option<usize> {
        let _ = byte;
        todo!("t03_render")
    }
}

#[derive(Debug, Clone)]
pub struct RenderedPage {
    pub lines: Vec<Line<'static>>,
    pub srcmap: SrcMap,
    /// For each rendered row, the 1-based source line it starts on (for
    /// `<leader>o` and review markers). Rows with no source (blank
    /// separators) repeat the previous value.
    pub source_lines: Vec<usize>,
}

/// Lay out `doc` at `width` columns.
pub fn render(doc: &Document, width: u16, theme: &Theme) -> RenderedPage {
    let _ = (doc, width, theme);
    todo!("t03_render")
}

/// Write `page` as ANSI-coloured text (truecolor) with a trailing newline
/// per row. Used by print mode.
pub fn to_ansi(page: &RenderedPage) -> String {
    let _ = page;
    todo!("t03_render")
}
