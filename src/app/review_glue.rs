//! Review comments in the app (debrief spec §2): the debrief-review batch
//! for the current page's repo root, polled once a second from the tick,
//! and mapped to rendered rows for the gutter, the file tree, the status
//! line and `]r` / `[r`. Capture and the prompt live in `comment`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use debrief_review::{Comment, Review};

use super::App;

/// Status when `]r` / `[r` finds nothing at all.
pub const NO_REVIEW: &str = "No review comments";
/// Status when `]r` / `[r` is already at the last / first comment.
pub const NO_MORE_REVIEW: &str = "No more review comments";
/// The marker drawn in the gutter and the file tree.
pub const MARKER: char = '●';
/// Status when `<leader>rr` finds an empty batch.
pub const NOTHING_TO_SEND: &str = "No comments to send";

/// How often the batch file is re-read.
pub const REVIEW_POLL: Duration = Duration::from_secs(1);

/// Comment marks for one file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileMarks {
    /// Comments on the file.
    pub count: usize,
    /// 1-based inclusive source line ranges of the comments.
    pub lines: Vec<(usize, usize)>,
}

/// Comment marks per canonical file path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Markers {
    pub files: BTreeMap<PathBuf, FileMarks>,
}

impl Markers {
    /// Marks from the working-copy comments (no `rev`) of `review`.
    fn from_review(review: &Review) -> Markers {
        let mut m = Markers::default();
        for c in review.comments().iter().filter(|c| c.rev.is_none()) {
            let path = canonical(&review.root().join(&c.path));
            let f = m.files.entry(path).or_default();
            f.count += 1;
            f.lines.push((c.lines.0 as usize, c.lines.1 as usize));
        }
        m
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Marks for `path` (canonicalised first).
    pub fn get(&self, path: &Path) -> Option<&FileMarks> {
        self.files.get(&canonical(path))
    }

    /// Whether any marked file lies under the folder `dir`.
    pub fn under(&self, dir: &Path) -> bool {
        let dir = canonical(dir);
        self.files.keys().any(|f| f.starts_with(&dir) && *f != dir)
    }
}

/// The canonical path (symlinks resolved), else the absolute path. Marker
/// keys and comment paths use it so they compare equal to tree paths.
pub fn canonical(path: &Path) -> PathBuf {
    path.canonicalize()
        .unwrap_or_else(|_| std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()))
}

/// `[ISSUE] ` for a typed comment, `` for an untyped one: what the
/// picker, the `K` popup and the status preview put before it.
pub(super) fn kind_tag(c: &Comment) -> String {
    c.kind.as_deref().map_or(String::new(), |k| {
        format!("[{}] ", debrief_review::Kind::label(k))
    })
}

/// The first 8 characters of a commit id, as the export shows it.
pub(super) fn short_commit(commit: &str) -> String {
    commit.chars().take(8).collect()
}

#[derive(Debug, Default)]
pub(super) struct ReviewState {
    /// Cache dir override (tests); else `debrief_review::cache_dir()`.
    cache: Option<PathBuf>,
    /// The batch for the current page's repo root.
    store: Option<Review>,
    /// Why there is no store for the current page, if it has a path.
    error: Option<String>,
    markers: Markers,
    polled_at: Option<Instant>,
    /// Columns reserved for the gutter in the current layout (0 or 1).
    pub(super) gutter: u16,
}

impl ReviewState {
    pub(super) fn new(cache: Option<PathBuf>) -> ReviewState {
        ReviewState {
            cache,
            ..Default::default()
        }
    }
}

impl App {
    /// Whether comment mode is on (`[review] enabled`).
    pub(crate) fn review_enabled(&self) -> bool {
        self.config.review.enabled
    }

    /// The open batch for the current page's repo root (t4 edits it, then
    /// calls [`App::review_refresh_markers`]).
    pub(crate) fn review_store(&mut self) -> Option<&mut Review> {
        self.review.store.as_mut()
    }

    /// There is a batch to list or send (`<leader>rl`, `<leader>rr`).
    pub(crate) fn has_review_batch(&self) -> bool {
        self.review.store.is_some()
    }

    /// The open batch, read-only.
    pub(super) fn review_batch(&self) -> Option<&Review> {
        self.review.store.as_ref()
    }

    /// The repo root of the open batch.
    pub fn review_root(&self) -> Option<&Path> {
        self.review.store.as_ref().map(Review::root)
    }

    /// Comments with no `rev` on the current file, matched by canonical
    /// path like the markers (a comment via a symlink alias counts).
    pub fn review_comments_here(&self) -> Vec<&Comment> {
        let store = self.review.store.as_ref();
        let path = self.page.as_ref().and_then(|p| p.path.as_deref());
        let (Some(store), Some(path)) = (store, path) else {
            return Vec::new();
        };
        let here = canonical(path);
        store
            .comments()
            .iter()
            .filter(|c| c.rev.is_none() && canonical(&store.root().join(&c.path)) == here)
            .collect()
    }

    /// The current page's path relative to the batch root, `/`-separated,
    /// from its canonical path. `Err` is the status to show.
    pub(super) fn review_rel_path(&self) -> Result<String, String> {
        let path = self.page.as_ref().and_then(|p| p.path.as_deref());
        let Some(path) = path else {
            return Err("Can't comment on stdin".into());
        };
        let Some(store) = &self.review.store else {
            let why = self.review.error.as_deref().unwrap_or("no review batch");
            return Err(format!("Can't comment: {why}"));
        };
        let root = store.root();
        let outside = || format!("Can't comment here: outside {}", root.display());
        let canon = path.canonicalize().map_err(|_| outside())?;
        let rel = canon.strip_prefix(root).map_err(|_| outside())?;
        let parts: Option<Vec<&str>> = rel.components().map(|c| c.as_os_str().to_str()).collect();
        match parts {
            Some(p) if !p.is_empty() => Ok(p.join("/")),
            _ => Err(outside()),
        }
    }

    /// Called for every page shown: open the batch for its repo root (kept
    /// when the root is the same), then re-read it.
    pub(super) fn review_page_changed(&mut self) {
        let path = self.page.as_ref().and_then(|p| p.path.clone());
        match path.filter(|_| self.review_enabled()) {
            None => {
                self.review.store = None;
                self.review.error = None;
            }
            Some(path) => {
                let root = debrief_review::repo_root(&canonical(&path));
                let same = self
                    .review
                    .store
                    .as_ref()
                    .is_some_and(|s| s.root() == canonical(&root));
                if same {
                    if let Some(s) = &mut self.review.store {
                        let _ = s.reload();
                    }
                } else {
                    self.review_open(&root);
                }
            }
        }
        self.review.polled_at = Some(self.now);
        self.review_refresh_markers();
    }

    fn review_open(&mut self, root: &Path) {
        self.review.store = None;
        self.review.error = None;
        let cache = self.review.cache.clone().or_else(debrief_review::cache_dir);
        let Some(cache) = cache else {
            self.review.error = Some("no cache directory".into());
            return;
        };
        match Review::open(&cache, root) {
            Ok(r) => self.review.store = Some(r),
            Err(e) => {
                let msg = format!("review: {e}");
                self.set_status(msg.clone());
                self.review.error = Some(msg);
            }
        }
    }

    /// From the tick: re-read the batch at most once per [`REVIEW_POLL`].
    pub(super) fn review_tick(&mut self, now: Instant) {
        let Some(store) = &mut self.review.store else {
            return;
        };
        if self.review.polled_at.is_some_and(|t| now < t + REVIEW_POLL) {
            return;
        }
        self.review.polled_at = Some(now);
        if let Ok(true) = store.reload() {
            self.review_refresh_markers();
        }
    }

    /// Rebuild the markers from the batch and re-lay out the gutter and
    /// the tree when they changed.
    pub(crate) fn review_refresh_markers(&mut self) {
        let markers = self
            .review
            .store
            .as_ref()
            .map(Markers::from_review)
            .unwrap_or_default();
        self.set_review_markers(markers);
    }

    /// Show `markers` (the batch's, or injected by layout tests) and re-lay
    /// out the gutter and the tree. The next batch change replaces them.
    pub fn set_review_markers(&mut self, markers: Markers) {
        if markers == self.review.markers {
            return self.review_relayout();
        }
        self.review.markers = markers;
        self.review_relayout();
        // Marks add ` ● N` to tree rows.
        self.sidebar_relayout(true);
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

    /// Whether rendered `row` stands for source: drawn text, or a source
    /// anchor (blank code lines, empty raw lines). Separators do not.
    pub(super) fn row_has_source(&self, row: usize) -> bool {
        !self.cells(row).is_empty()
            || self
                .page
                .as_ref()
                .is_some_and(|p| p.rendered.anchored.get(row).copied().unwrap_or(false))
    }

    /// Rendered rows showing each line comment of the current file, in
    /// comment order; each list is sorted (see [`App::rows_for_lines`]).
    fn review_rows(&self) -> Vec<Vec<usize>> {
        let Some(m) = self.current_marks() else {
            return Vec::new();
        };
        m.lines.iter().map(|&l| self.rows_for_lines(l)).collect()
    }

    /// Rendered rows showing 1-based source lines `(s, e)`, sorted. Lines
    /// map to rows through the srcmap (so a line in the middle of a
    /// reflowed paragraph finds its row), plus rows with a source anchor
    /// that start on one of those lines (blank code lines draw no
    /// segments). A range that reaches no row falls back to the last such
    /// row at or before its start line. Lines past the end of the file
    /// count as the last line; separator rows are never included.
    pub(super) fn rows_for_lines(&self, (s, e): (usize, usize)) -> Vec<usize> {
        let Some(p) = &self.page else {
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
        let sourced = |r: &usize| self.row_has_source(*r);
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
                .filter(sourced),
        );
        if rows.is_empty() {
            rows.extend(
                (0..lines.len())
                    .rev()
                    .filter(sourced)
                    .find(|&r| lines[r] <= s),
            );
        }
        rows.sort_unstable();
        rows.dedup();
        rows
    }

    /// Working-copy comments on the current file whose lines are drawn on
    /// the cursor row (the rows the gutter marks for them), in batch order.
    pub fn review_comments_at_cursor(&self) -> Vec<&Comment> {
        let row = self.cursor.row;
        self.review_comments_here()
            .into_iter()
            .filter(|c| {
                let lines = (c.lines.0 as usize, c.lines.1 as usize);
                self.rows_for_lines(lines).binary_search(&row).is_ok()
            })
            .collect()
    }

    /// The status-line preview for the cursor row: `● `, the kind
    /// (`[ISSUE] `) and the first line of the first comment on it.
    pub fn review_preview(&self) -> Option<String> {
        let c = *self.review_comments_at_cursor().first()?;
        let first = c.body.lines().next().unwrap_or("");
        Some(format!("{MARKER} {}{first}", kind_tag(c)))
    }

    /// `K` on a commented line: the popup text, every comment on the row
    /// as `● line a` / `● lines a-b` (then ` [ISSUE]`) over its body.
    pub(super) fn review_hover_text(&self) -> Option<String> {
        let cs = self.review_comments_at_cursor();
        if cs.is_empty() {
            return None;
        }
        let parts: Vec<String> = cs
            .iter()
            .map(|c| {
                let (a, b) = c.lines;
                let at = if a == b {
                    format!("line {a}")
                } else {
                    format!("lines {a}-{b}")
                };
                let kind = kind_tag(c);
                let head = format!("{MARKER} {at} {kind}");
                format!("{}\n{}", head.trim_end(), c.body.trim_end())
            })
            .collect();
        Some(parts.join("\n\n"))
    }

    /// The cursor is on a commented line (`K` shows the comments).
    pub(crate) fn has_comment_here(&self) -> bool {
        !self.review_comments_at_cursor().is_empty()
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

    /// `]r` / `[r` have a commented line to go to.
    pub(crate) fn can_review_jump(&self) -> bool {
        !self.review_targets().is_empty()
    }

    /// `]r` (true) / `[r`: jump to the next / previous commented line.
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

    /// Enter in the review picker: open a working-copy comment's file (if
    /// it isn't the current page) and put the cursor on the first row of
    /// its lines; a diff comment only says where it lives.
    pub(super) fn review_goto(&mut self, id: u64) {
        let Some(store) = &self.review.store else {
            return;
        };
        let Some(c) = store.comments().iter().find(|c| c.id == id) else {
            return self.set_status("Comment is gone");
        };
        if let Some(rev) = &c.rev {
            let msg = format!(
                "Comment on {} — open it in debrief",
                short_commit(&rev.commit)
            );
            return self.set_status(msg);
        }
        let path = store.root().join(&c.path);
        let lines = (c.lines.0 as usize, c.lines.1 as usize);
        let here = self.page.as_ref().and_then(|p| p.path.as_deref());
        if here.is_none_or(|h| canonical(h) != canonical(&path)) {
            let before = self.page_id();
            self.open_path_at(&path, None);
            if self.page_id() == before {
                return; // Not opened; open_path_at set the status.
            }
        }
        if let Some(&row) = self.rows_for_lines(lines).first() {
            self.jump_to_row(row);
        }
    }

    /// `<leader>rr`: queue the hand-back unless there is nothing to send.
    pub(super) fn review_send(&mut self) {
        match &self.review.store {
            None => self.set_status(super::picker::NO_BATCH),
            Some(s) if s.comments().is_empty() => self.set_status(NOTHING_TO_SEND),
            Some(_) => self.pending_effect = Some(super::Effect::SendReview),
        }
    }

    /// `q`, `ZZ`, `:q`: quit, unless the batch holds comments not sent
    /// yet. The batch is re-read first, so comments another handle sent or
    /// added count as they are now. Then warn; quitting again while that
    /// same warning shows quits. A batch that can't be read warns every
    /// time (nothing is known to be safe to drop). `:q!` and `C-c` quit
    /// without asking.
    pub(super) fn quit_checked(&mut self) {
        let n = match &mut self.review.store {
            None => 0,
            Some(store) => match store.reload() {
                Ok(_) => store.comments().len(),
                Err(e) => {
                    return self
                        .set_status(format!("Review batch unreadable ({e}); :q! quits anyway"));
                }
            },
        };
        if n == 0 {
            self.quit = true;
            return;
        }
        let warning = match n {
            1 => "1 comment not sent: <leader>rr sends it; q again or :q! quits".to_string(),
            n => format!("{n} comments not sent: <leader>rr sends them; q again or :q! quits"),
        };
        if self.status == warning {
            self.quit = true;
            return;
        }
        self.set_status(warning);
    }

    /// The hand-back (TUI suspended): re-read and export the batch, pipe it
    /// to `[send] command`, falling back to the clipboard, then remove the
    /// exported comments. Comments added meanwhile stay in the batch; a
    /// failed hand-back keeps all of it.
    pub(super) fn run_review_send(&mut self) {
        let Some(store) = &mut self.review.store else {
            return;
        };
        if let Err(e) = store.reload() {
            return self.set_status(format!("Review not sent: {e}"));
        }
        let ids: Vec<u64> = store.comments().iter().map(|c| c.id).collect();
        if ids.is_empty() {
            self.set_status(NOTHING_TO_SEND);
            return self.review_refresh_markers();
        }
        let n = ids.len();
        let md = store.to_markdown(
            self.config.send.preamble.as_deref(),
            &self.config.review.kinds,
        );
        let root = store.root().to_path_buf();
        let clipboard = &mut self.clipboard;
        let mut copy = |text: &str| clipboard.copy(text).map_err(std::io::Error::other);
        let sent = debrief_review::handback::send(&self.config.send.command, &md, &root, &mut copy);
        let how = match sent {
            Ok(debrief_review::handback::Sent::Command) => "sent",
            Ok(debrief_review::handback::Sent::Clipboard) => "copied",
            Err(e) => return self.set_status(format!("Review not sent: {e}")),
        };
        let Some(store) = &mut self.review.store else {
            return;
        };
        let msg = match store.remove_all(&ids) {
            Ok(_) if how == "sent" => format!("Review sent ({n} comments)"),
            Ok(_) => format!("Review copied to clipboard ({n} comments)"),
            Err(e) => format!(
                "Review {how}, but the batch wasn't cleared: {e} (sending again repeats it)"
            ),
        };
        self.set_status(msg);
        self.review_refresh_markers();
    }

    /// `C-e` in the review picker (TUI suspended): edit comment `id`'s body
    /// in the editor. An emptied body or a failing editor changes nothing.
    pub(super) fn run_review_comment_editor(&mut self, id: u64, initial: &str) {
        let text = match self.edit_in_editor(initial) {
            Ok(t) => t,
            Err(e) => return self.set_status(format!("editor: {e}; comment not changed")),
        };
        let body = text.trim();
        if body.is_empty() {
            return self.set_status("Empty comment not saved (C-d removes it)");
        }
        let Some(store) = &mut self.review.store else {
            return;
        };
        let msg = match store.edit(id, body) {
            Ok(true) => "Comment edited".to_string(),
            Ok(false) => "Comment is gone".to_string(),
            Err(e) => format!("comment not changed: {e}"),
        };
        self.set_status(msg);
        self.review_refresh_markers();
        self.review_picker_refresh();
    }
}
