//! Following the editor's cursor (`--follow`): what an
//! [`AppEvent::Follow`](super::AppEvent::Follow) does to the view. The
//! socket side is [`crate::follow_sock`]; the spec is
//! docs/specs/2026-10-22-follow-cursor.md.

use std::path::Path;
use std::time::{Duration, Instant};

use super::{App, Mode};
use crate::nav;

/// After a key, mouse event or paste in ramble, editor updates are
/// dropped for this long, so following never fights the reader.
pub const FOLLOW_GRACE: Duration = Duration::from_secs(2);

impl App {
    /// The reader used ramble at `now`: start the grace window.
    pub(super) fn follow_touch(&mut self, now: Instant) {
        self.follow_quiet_until = Some(now + FOLLOW_GRACE);
    }

    /// The editor's cursor is on 1-based source `line` of `path` at `now`.
    /// Dropped during the grace window, outside Normal mode, or with a
    /// key sequence or count half typed; another markdown file is opened
    /// as a followed link (`C-o` returns). Never sets a status: the hook
    /// fires on every cursor move.
    pub fn follow_event(&mut self, path: &Path, line: usize, now: Instant) {
        let busy = self.mode != Mode::Normal || !self.pending.is_empty() || self.count.is_some();
        if busy || self.follow_quiet_until.is_some_and(|t| now < t) {
            return;
        }
        let current = self.page.as_ref().and_then(|p| p.path.clone());
        if !current.is_some_and(|c| super::watch::same_file(&c, path)) {
            if !nav::is_markdown(path) || !path.is_file() {
                return;
            }
            // Unreadable or binary: open_link_file would say so, every move.
            let Ok(bytes) = std::fs::read(path) else {
                return;
            };
            if bytes[..bytes.len().min(8192)].contains(&0) {
                return; // doc::from_bytes's binary test
            }
            let before = self.page_id();
            let shown = path.display().to_string();
            self.open_link_file(path.to_path_buf(), None, &shown);
            if self.page_id() == before {
                return;
            }
        }
        self.clearing_stale_status(|a| a.follow_land(line));
    }

    /// Cursor to the first row of source `line`; scroll only when that row
    /// is off screen, then centre it.
    fn follow_land(&mut self, line: usize) {
        let Some(p) = &self.page else { return };
        let row = p
            .rendered
            .source_lines
            .iter()
            .position(|&l| l >= line)
            .unwrap_or(usize::MAX);
        self.goto_row(row);
        let vh = self.viewport_height();
        let row = self.cursor.row;
        if row < self.scroll || row >= self.scroll + vh {
            self.center_cursor();
        }
    }
}
