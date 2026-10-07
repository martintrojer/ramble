//! Review markers in the app (spec § Review markers): starts the review
//! thread, keeps its discovery directory on the current file, pauses it
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
        handle.control.set_dir(self.review_dir());
        self.review.handle = Some(handle);
        true
    }

    /// Whether the review thread is running.
    pub fn review_running(&self) -> bool {
        self.review.handle.is_some()
    }

    /// The discovery directory for the current page (none for stdin).
    pub fn review_dir(&self) -> Option<PathBuf> {
        let path = self.page.as_ref()?.path.as_deref()?;
        Some(review::discovery_dir(path))
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
            h.control.set_dir(self.review_dir());
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

    /// Whether rendered `row` shows a source line inside a comment range.
    /// Lines past the end of the file count as the last line; rows with no
    /// text are never marked.
    pub fn review_row_marked(&self, row: usize) -> bool {
        let (Some(m), Some(p)) = (self.current_marks(), &self.page) else {
            return false;
        };
        if self.cells(row).is_empty() {
            return false;
        }
        let Some(&line) = p.rendered.source_lines.get(row) else {
            return false;
        };
        let last = p.doc.source.lines().count().max(1);
        m.lines
            .iter()
            .any(|&(s, e)| (s.min(last)..=e.min(last)).contains(&line))
    }

    /// Rows `]r` / `[r` stop on: the first drawn row of each comment's
    /// start line, in order.
    fn review_targets(&self) -> Vec<usize> {
        let (Some(m), Some(p)) = (self.current_marks(), &self.page) else {
            return Vec::new();
        };
        let last = p.doc.source.lines().count().max(1);
        let lines = &p.rendered.source_lines;
        let mut rows: Vec<usize> = m
            .lines
            .iter()
            .filter_map(|&(s, _)| {
                let s = s.min(last);
                (0..lines.len()).find(|&r| lines[r] >= s && !self.cells(r).is_empty())
            })
            .collect();
        rows.sort_unstable();
        rows.dedup();
        rows
    }

    /// `]r` (true) / `[r`: jump to the next / previous commented line.
    pub(super) fn review_jump(&mut self, forward: bool) {
        let targets = self.review_targets();
        if targets.is_empty() {
            return self.set_status(NO_REVIEW);
        }
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
