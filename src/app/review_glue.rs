//! Review markers in the app (spec § Review markers): starts the review
//! thread, keeps its discovery directories on the current file, pauses it
//! around launchers, and maps comment lines to rendered rows for the
//! gutter, the file tree, the status line and `]r` / `[r`.

use std::path::{Path, PathBuf};

use super::{App, AppEvent};
use crate::review::{self, FileMarks, Intervals, Markers, Tuicr};

/// Status when `]r` / `[r` finds nothing at all.
pub const NO_REVIEW: &str = "No review comments";
/// Status when `]r` / `[r` is already at the last / first comment.
pub const NO_MORE_REVIEW: &str = "No more review comments";
/// The marker drawn in the gutter and the file tree.
pub const MARKER: char = '●';

#[derive(Debug, Default)]
pub(super) struct ReviewState {
    handle: Option<review::Handle>,
    markers: Markers,
    /// Columns reserved for the gutter in the current layout (0 or 1).
    pub(super) gutter: u16,
}

impl App {
    /// Start the review thread when `review.enabled` and the command
    /// resolves; markers arrive as [`AppEvent::Review`] through the sender.
    /// Returns whether it started.
    pub fn start_review(&mut self) -> bool {
        self.start_review_with(Intervals::default())
    }

    /// [`App::start_review`] with custom polling intervals (tests).
    pub fn start_review_with(&mut self, every: Intervals) -> bool {
        if !self.config.review.enabled {
            return false;
        }
        let (Some(tx), Some(command)) = (
            self.sender.clone(),
            review::resolve_command(&self.config.review.command),
        ) else {
            return false;
        };
        let handle = review::spawn(Tuicr { command }, every, move |m| {
            let _ = tx.send(AppEvent::Review(m));
        });
        handle.control.set_dirs(self.review_dirs());
        self.review.handle = Some(handle);
        true
    }

    /// Whether the review thread is running.
    pub fn review_running(&self) -> bool {
        self.review.handle.is_some()
    }

    /// The discovery directories for the current page (none for stdin).
    pub fn review_dirs(&self) -> Vec<PathBuf> {
        let path = self.page.as_ref().and_then(|p| p.path.as_deref());
        path.map(review::discovery_dirs).unwrap_or_default()
    }

    /// Pause (true) or resume the review thread; resuming polls at once.
    pub(super) fn review_pause(&self, paused: bool) {
        if let Some(h) = &self.review.handle {
            h.control.set_paused(paused);
        }
    }

    /// Called for every page shown.
    pub(super) fn review_page_changed(&mut self) {
        if let Some(h) = &self.review.handle {
            h.control.set_dirs(self.review_dirs());
        }
        self.review_relayout();
    }

    /// New markers from the review thread.
    pub(super) fn review_event(&mut self, markers: Markers) {
        self.review.markers = markers;
        self.review_relayout();
    }

    /// Reserve the gutter only while the current file has line marks, and
    /// re-render when that changes.
    fn review_relayout(&mut self) {
        let want = u16::from(self.current_marks().is_some_and(|m| !m.lines.is_empty()));
        if want != self.review.gutter {
            self.review.gutter = want;
            let (cols, rows) = self.size;
            self.resize(cols, rows);
        }
    }

    pub fn review_markers(&self) -> &Markers {
        &self.review.markers
    }

    fn current_marks(&self) -> Option<&FileMarks> {
        let path = self.page.as_ref()?.path.as_deref()?;
        self.review.markers.get(path)
    }

    /// Columns of the review gutter left of the content (0 or 1).
    pub fn review_gutter(&self) -> u16 {
        self.review.gutter
    }

    /// Comments on the current file (`review N` in the status line).
    pub fn review_count(&self) -> usize {
        self.current_marks().map_or(0, |m| m.count)
    }

    /// Tree decoration for `path`: the marker and, for a file, its comment
    /// count; a folder holding marked files gets the marker alone.
    pub fn file_marker(&self, path: &Path) -> Option<(char, Option<usize>)> {
        if let Some(m) = self.review.markers.get(path) {
            return Some((MARKER, Some(m.count)));
        }
        (path.is_dir() && self.review.markers.under(path)).then_some((MARKER, None))
    }

    /// Rendered rows showing each line comment of the current file, in
    /// comment order; each list is sorted. A comment's lines map to rows
    /// through the srcmap (so a line in the middle of a reflowed paragraph
    /// finds its row), plus rows that start on one of those lines (blank
    /// code lines draw no segments). A range that reaches no row falls back
    /// to the last drawn row at or before its start line. Lines past the end
    /// of the file count as the last line; rows with no text are never
    /// marked.
    fn review_rows(&self) -> Vec<Vec<usize>> {
        let (Some(m), Some(p)) = (self.current_marks(), &self.page) else {
            return Vec::new();
        };
        let src = p.doc.source.as_str();
        let starts: Vec<usize> = std::iter::once(0)
            .chain(src.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        let last = src.lines().count().max(1);
        // Byte where 1-based `line` starts (the end of the source past it).
        let line_start = |line: usize| starts.get(line - 1).copied().unwrap_or(src.len());
        let lines = &p.rendered.source_lines;
        let drawn = |r: &usize| !self.cells(*r).is_empty();
        m.lines
            .iter()
            .map(|&(s, e)| {
                let (s, e) = (s.clamp(1, last), e.clamp(1, last));
                let (s, e) = (s.min(e), s.max(e));
                let bytes = line_start(s)..line_start(e + 1);
                let mut rows: Vec<usize> = p
                    .rendered
                    .srcmap
                    .spans_for(bytes)
                    .iter()
                    .map(|sp| sp.row)
                    .collect();
                rows.extend(
                    (0..lines.len())
                        .filter(|&r| (s..=e).contains(&lines[r]))
                        .filter(drawn),
                );
                if rows.is_empty() {
                    rows.extend(
                        (0..lines.len())
                            .rev()
                            .filter(drawn)
                            .find(|&r| lines[r] <= s),
                    );
                }
                rows.sort_unstable();
                rows.dedup();
                rows
            })
            .collect()
    }

    /// Every rendered row the gutter marks, sorted.
    pub fn review_marked_rows(&self) -> Vec<usize> {
        let mut rows: Vec<usize> = self.review_rows().concat();
        rows.sort_unstable();
        rows.dedup();
        rows
    }

    /// Whether the gutter marks rendered `row`.
    pub fn review_row_marked(&self, row: usize) -> bool {
        self.review_marked_rows().binary_search(&row).is_ok()
    }

    /// Rows `]r` / `[r` stop on: the first row of each comment, in order.
    fn review_targets(&self) -> Vec<usize> {
        let mut rows: Vec<usize> = self
            .review_rows()
            .iter()
            .filter_map(|r| r.first().copied())
            .collect();
        rows.sort_unstable();
        rows.dedup();
        rows
    }

    /// `]r` (true) / `[r`: jump to the next / previous commented line.
    /// `]r` / `[r` have a commented line to go to.
    pub(crate) fn can_review_jump(&self) -> bool {
        !self.review_targets().is_empty()
    }

    pub(super) fn review_jump(&mut self, forward: bool) {
        if !self.can_review_jump() {
            return self.set_status(NO_REVIEW);
        }
        let targets = self.review_targets();
        let row = self.cursor.row;
        let next = if forward {
            targets.iter().find(|&&r| r > row)
        } else {
            targets.iter().rev().find(|&&r| r < row)
        };
        match next {
            Some(&r) => self.goto_row(r),
            None => self.set_status(NO_MORE_REVIEW),
        }
    }
}
