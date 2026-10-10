//! Comment mode (debrief spec §2): `cc` comments on the cursor line,
//! visual `c` on the selection's source lines. Lines and excerpt are
//! frozen from the displayed document when `c` is pressed; a bordered box
//! anchored below the commented rows (above when it doesn't fit) takes
//! the body: Enter saves, `C-j` / `Alt-Enter` insert a newline, Esc
//! cancels, `C-e` moves it into the editor, Tab / S-Tab pick the kind
//! from `[review] kinds`, readline keys edit it (see [`super::textbox`]),
//! PageUp / PageDown and the wheel scroll the page behind it (the box
//! then places itself for that view, see [`App::comment_fit`]). The rows
//! stay highlighted while typing. Comments go to the
//! debrief-review batch for the page's repo root.

use std::cell::Cell;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use debrief_review::Kind;
use ratatui::layout::Rect;

use super::effect::Effect;
use super::keys::{Action, KeyResult};
use super::launch::{Exit, LaunchCommand, editor_words};
use super::textbox::{Edit, TextBox};
use super::{App, Mode};

/// Text rows the comment box grows to before it scrolls.
pub const BOX_ROWS: usize = 8;
/// Columns left free on each side of the comment box.
pub const BOX_MARGIN: u16 = 1;
/// Status when Enter (or the editor) leaves an empty body.
pub const EMPTY_COMMENT: &str = "Empty comment not saved";
/// Status when the cursor row stands for no source line.
pub const NO_SOURCE_LINE: &str = "No source line here";

/// Comment-box actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentAction {
    /// `cc` (false: the cursor row) or visual `c` (true: the selection).
    Start(bool),
    /// A key that edits the text or moves the cursor.
    Edit(Edit),
    /// Enter: save a non-empty body.
    Save,
    /// Esc: drop the comment.
    Cancel,
    /// `C-e`: edit the body in `$VISUAL` / `$EDITOR`.
    Editor,
    /// Tab (true) / S-Tab: the next / previous kind, through untyped.
    Kind(bool),
    /// `C-j`, `Alt-Enter`: a line break at the cursor.
    Newline,
    /// PageDown (true) / PageUp: scroll the page behind the box a page.
    Page(bool),
}

/// A comment being typed: where it goes, frozen at `c`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Draft {
    /// Relative to the batch root, `/`-separated.
    path: String,
    /// 1-based inclusive source lines.
    lines: (u32, u32),
    excerpt: String,
    text: TextBox,
    /// The kind id; new comments start untyped.
    kind: Option<String>,
    /// The first text row the box shows.
    scroll: Cell<usize>,
    /// The view's scroll when the box last fitted it (tells which way the
    /// user scrolls).
    view: usize,
    /// The user scrolled the page since the box opened: the view stays
    /// where they put it ([`App::comment_fit`]).
    scrolled: bool,
}

/// The comment box as drawn: where, its border titles, the visible text
/// rows and the terminal cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentBox {
    pub rect: Rect,
    /// ` comment [ISSUE] L12-18 `.
    pub title: String,
    /// The key hints on the bottom border.
    pub hint: String,
    pub lines: Vec<String>,
    pub cursor: (u16, u16),
}

/// The box's width in a pane `pane_w` wide.
fn box_width(pane_w: u16) -> u16 {
    if pane_w >= 2 * BOX_MARGIN + 3 {
        pane_w - 2 * BOX_MARGIN
    } else {
        pane_w
    }
}

/// The box's height for `d`'s text in a pane `pane_w` wide: the wrapped
/// rows up to [`BOX_ROWS`], and the border.
fn box_height(d: &Draft, pane_w: u16) -> u16 {
    let rows = d.text.rows(wrap_width(box_width(pane_w))).len();
    rows.min(BOX_ROWS) as u16 + 2
}

/// The wrap width of a box `w` wide: inside the border, with a spare cell
/// for the cursor at the end of a full row.
fn wrap_width(w: u16) -> usize {
    (w as usize).saturating_sub(3).max(1)
}

/// The box's top row and height, `h` rows wanted, for selected rows at
/// screen rows `lo..=hi` (relative to the pane's top, may lie outside it)
/// in a pane `ph` rows high: below the rows, else above them, else on the
/// roomier side, shrunk to fit (at least one text row), else at the
/// bottom ([`fit_scroll`] scrolls the view so before the user scrolls
/// this only happens in a pane too short for the box and a row; after,
/// also when the selection leaves no room). Rows scrolled off pin the box
/// to that edge.
pub(super) fn place(lo: i64, hi: i64, h: u16, ph: u16) -> (u16, u16) {
    let (h, ph64) = (h.min(ph), i64::from(ph));
    let hh = i64::from(h);
    if hi < 0 {
        return (0, h);
    }
    if lo >= ph64 {
        return (ph - h, h);
    }
    let below = (ph64 - hi - 1).max(0);
    let above = lo.max(0);
    if hh <= below {
        return ((hi + 1) as u16, h);
    }
    if hh <= above {
        return ((lo - hh) as u16, h);
    }
    let room = below.max(above);
    if room >= 3 {
        let y = if below >= above { hi + 1 } else { 0 };
        return (y as u16, room as u16);
    }
    (ph - h, h)
}

/// Whether the box [`place`] puts in a pane `ph` high, the selected rows
/// at screen rows `lo..=hi`, gets its full height (`h`, at most `ph - 1`)
/// and covers none of them.
fn fits(lo: i64, hi: i64, h: u16, ph: u16) -> bool {
    let (y, got) = place(lo, hi, h, ph);
    let (y, end) = (i64::from(y), i64::from(y) + i64::from(got));
    let covers = lo.max(0) < end && y <= hi.min(i64::from(ph) - 1);
    got >= h.min(ph.saturating_sub(1)) && !covers
}

/// Until the user scrolls, the scroll (at most `max`) for a box `h` rows
/// high and the selected rows `lo..=hi` in a pane `ph` high: `s` when the box fits there
/// ([`fits`]; rows scrolled off fit); after a scroll by the user, the
/// next scroll that way (`dir` above 0 down, below 0 up) where it does
/// with the last selected row on screen; else the nearest such. A selection
/// taller than the pane so ends up with its last rows just above the box.
/// No scroll fitting (a pane too short for the box and one row), the one
/// putting the last selected row above the box at the bottom. The view
/// may scroll the box's height past the last row (see
/// [`App::comment_overscroll`]).
pub(super) fn fit_scroll(
    lo: usize,
    hi: usize,
    h: u16,
    ph: u16,
    s: usize,
    max: usize,
    dir: i8,
) -> usize {
    let ok = |s: usize| fits(lo as i64 - s as i64, hi as i64 - s as i64, h, ph);
    if ok(s) {
        return s;
    }
    // Moving, keep the last selected row on screen.
    let shown = |t: usize| t <= hi && hi < t + usize::from(ph);
    let down = (s + 1..=max).find(|&t| shown(t) && ok(t));
    let up = (0..s).rev().find(|&t| shown(t) && ok(t));
    let pick = match (dir, down, up) {
        (1.., Some(t), _) | (..=-1, _, Some(t)) => Some(t),
        (_, Some(d), Some(u)) => Some(if d - s <= s - u { d } else { u }),
        (_, d, u) => d.or(u),
    };
    pick.unwrap_or_else(|| {
        let bh = usize::from(h.min(ph.saturating_sub(1)));
        (hi + 1 + bh).saturating_sub(usize::from(ph)).min(max)
    })
}

/// The scroll after the user scrolled the view to `s` (at most `max`,
/// `dir` above 0 down, below 0 up; `n` rows in all): `s` when [`place`]
/// puts the box off the selected rows on screen (beside them, or shrunk
/// into the rows that are not selected) with a row left beside it for
/// the cursor, or the selection fills the pane (the box then at the
/// bottom over it); else (a selection edge 1-2 rows from the pane's edge
/// leaving no room for a box, or the box over the last rows of a view
/// scrolled past the end) the nearest scroll that way where one of
/// those holds (the other way at the end).
#[allow(clippy::too_many_arguments)]
pub(super) fn user_scroll(
    lo: usize,
    hi: usize,
    h: u16,
    ph: u16,
    s: usize,
    max: usize,
    dir: i8,
    n: usize,
) -> usize {
    let ok = |t: usize| {
        let fills = lo <= t && hi + 1 >= t + usize::from(ph);
        let shown = usize::from(ph).min(n.saturating_sub(t));
        let (lo, hi) = (lo as i64 - t as i64, hi as i64 - t as i64);
        let (y, got) = place(lo, hi, h, ph);
        let free = off_the_box(0, y, got, shown).is_some_and(|r| r < shown);
        let (y, end) = (i64::from(y), i64::from(y) + i64::from(got));
        fills || (free && !(lo.max(0) < end && y <= hi.min(i64::from(ph) - 1)))
    };
    let reach = usize::from(ph);
    let down = || (s..=max.min(s + reach)).find(|&t| ok(t));
    let up = || (s.saturating_sub(reach)..=s).rev().find(|&t| ok(t));
    let found = if dir < 0 {
        up().or_else(down)
    } else {
        down().or_else(up)
    };
    found.unwrap_or(s)
}

/// The row nearest screen row `row` (of the `n` holding rows) outside the
/// box at rows `y..y + h`, above it on a tie; `None` when there is none.
fn off_the_box(row: usize, y: u16, h: u16, n: usize) -> Option<usize> {
    let (y, end) = (usize::from(y), usize::from(y) + usize::from(h));
    if row < y || row >= end {
        return Some(row);
    }
    let above = y.checked_sub(1);
    let below = (end < n).then_some(end);
    match (above, below) {
        (Some(a), Some(b)) => Some(if row - a <= b - row { a } else { b }),
        (a, b) => a.or(b),
    }
}

/// Pasted text for the box: CRLF and CR as LF, a tab as a space, other
/// control characters dropped.
fn paste_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .chars()
        .filter_map(|c| match c {
            '\r' | '\n' => Some('\n'),
            '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}

/// Keys while typing a comment: the box's, and PageUp / PageDown scroll
/// the page behind it (the box follows its rows).
pub(super) fn comment_keymap(keys: &[KeyEvent]) -> KeyResult {
    let Some(key) = keys.last() else {
        return KeyResult::None;
    };
    match key.code {
        KeyCode::PageDown => return KeyResult::Action(Action::Comment(CommentAction::Page(true))),
        KeyCode::PageUp => return KeyResult::Action(Action::Comment(CommentAction::Page(false))),
        _ => {}
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let a = match key.code {
        KeyCode::Char('e') if ctrl => CommentAction::Editor,
        KeyCode::Char('j') if ctrl => CommentAction::Newline,
        KeyCode::Enter if alt || ctrl => CommentAction::Newline,
        KeyCode::Enter => CommentAction::Save,
        KeyCode::Esc => CommentAction::Cancel,
        KeyCode::Tab | KeyCode::Char('\t') => CommentAction::Kind(true),
        KeyCode::BackTab => CommentAction::Kind(false),
        _ => match Edit::from_key(key) {
            Some(e) => CommentAction::Edit(e),
            None => return KeyResult::None,
        },
    };
    KeyResult::Action(Action::Comment(a))
}

impl Draft {
    /// The box title: ` comment [ISSUE] L12-18 `.
    fn title(&self) -> String {
        let kind = self
            .kind
            .as_deref()
            .map_or(String::new(), |k| format!(" [{}]", Kind::label(k)));
        let (a, b) = self.lines;
        let range = if a == b {
            format!("L{a}")
        } else {
            format!("L{a}-{b}")
        };
        format!(" comment{kind} {range} ")
    }
}

impl App {
    /// Comment mode is on and the page has a file to comment on.
    pub(crate) fn can_comment(&self) -> bool {
        self.review_enabled() && self.page.as_ref().is_some_and(|p| p.path.is_some())
    }

    /// Comment mode is on and `[review] kinds` lists some (Tab cycles).
    pub(crate) fn has_comment_kinds(&self) -> bool {
        self.can_comment() && !self.config.review.kinds.is_empty()
    }

    /// The comment box's title (` comment [ISSUE] L12-18 `).
    pub fn comment_title(&self) -> Option<String> {
        Some(self.comment.as_ref()?.title())
    }

    /// The comment's text so far.
    pub fn comment_text(&self) -> Option<&str> {
        Some(self.comment.as_ref()?.text.text())
    }

    /// The rendered rows a comment being typed is on (drawn as selected,
    /// the box's anchor): the rows its source lines are drawn on now, so
    /// they follow a reflow on resize.
    pub fn comment_rows(&self) -> Option<(usize, usize)> {
        let d = self.comment.as_ref()?;
        let (a, b) = d.lines;
        let rows = self.rows_for_lines((a as usize, b as usize));
        match (rows.first(), rows.last()) {
            (Some(&lo), Some(&hi)) => Some((lo, hi)),
            _ => Some((self.cursor.row, self.cursor.row)),
        }
    }

    /// The content pane's width (gutter and text), from the terminal size.
    fn comment_pane_width(&self) -> u16 {
        self.size.0.saturating_sub(self.sidebar_cols())
    }

    /// The comment box in the content pane `pane` (gutter and text):
    /// [`BOX_MARGIN`] in from its sides, as tall as the wrapped text up to
    /// [`BOX_ROWS`] rows (then it scrolls to the cursor), placed by
    /// [`place`] next to the rows.
    pub fn comment_box(&self, pane: Rect) -> Option<CommentBox> {
        let d = self.comment.as_ref()?;
        let (r0, r1) = self.comment_rows()?;
        if pane.width < 3 || pane.height < 3 {
            return None;
        }
        let w = box_width(pane.width);
        let wrap = wrap_width(w);
        let rows = d.text.rows(wrap);
        let want = box_height(d, pane.width);
        let scroll = self.scroll() as i64;
        let (lo, hi) = (r0 as i64 - scroll, r1 as i64 - scroll);
        let (y, h) = place(lo, hi, want, pane.height);
        let vis = (h as usize).saturating_sub(2).max(1);
        let (row, col) = d.text.cursor_cell(wrap);
        let mut s = d.scroll.get();
        if row < s {
            s = row;
        } else if row >= s + vis {
            s = row + 1 - vis;
        }
        s = s.min(rows.len().saturating_sub(vis));
        d.scroll.set(s);
        let lines = rows
            .iter()
            .skip(s)
            .take(vis)
            .map(|r| d.text.text()[r.clone()].to_string())
            .collect();
        let rect = Rect::new(pane.x + (pane.width - w) / 2, pane.y + y, w, h);
        let inner_w = w.saturating_sub(2).max(1);
        let cursor = (
            rect.x + 1 + (col as u16).min(inner_w - 1),
            rect.y + 1 + (row.saturating_sub(s)).min(vis - 1) as u16,
        );
        let tab = if self.has_comment_kinds() {
            " · Tab kind"
        } else {
            ""
        };
        Some(CommentBox {
            rect,
            title: d.title(),
            hint: format!(" Enter save · C-j newline{tab} · C-e editor · Esc cancel "),
            lines,
            cursor,
        })
    }

    /// Rows the page may scroll past its last row while the box is open:
    /// the box's height, so the last rows can sit above it.
    pub(super) fn comment_overscroll(&self) -> usize {
        let (w, h) = (self.comment_pane_width(), self.viewport_height() as u16);
        match &self.comment {
            Some(d) if w >= 3 && h >= 3 => usize::from(box_height(d, w).min(h - 1)),
            _ => 0,
        }
    }

    /// Until the user scrolls, scroll the page so the box covers none of
    /// the commented rows on screen ([`fit_scroll`]). After PageUp /
    /// PageDown or the wheel the view stays where the user put it
    /// ([`user_scroll`]) and [`place`] puts the box beside the rows on
    /// screen, or in the unselected rows, or (the selection filling the
    /// pane) at the bottom over the rows. The cursor stays in the view,
    /// off the box.
    pub(super) fn comment_fit(&mut self) {
        let (w, ph) = (self.comment_pane_width(), self.viewport_height() as u16);
        let Some((lo, hi)) = self.comment_rows() else {
            return;
        };
        let max = self.max_scroll();
        let Some(d) = &mut self.comment else { return };
        if w < 3 || ph < 3 {
            return;
        }
        let h = box_height(d, w);
        let dir = self.scroll.cmp(&d.view) as i8;
        let n = self.rows.len();
        let s = if d.scrolled {
            user_scroll(lo, hi, h, ph, self.scroll, max, dir, n)
        } else {
            fit_scroll(lo, hi, h, ph, self.scroll, max, dir)
        };
        d.view = s;
        self.scroll = s;
        let (y, bh) = place(lo as i64 - s as i64, hi as i64 - s as i64, h, ph);
        let on = self.cursor.row.clamp(s, s + usize::from(ph) - 1) - s;
        let shown = usize::from(ph).min(n.saturating_sub(s));
        let row = s + off_the_box(on, y, bh, shown).unwrap_or(on);
        if row != self.cursor.row {
            self.move_to_row(row);
        }
    }

    /// The user scrolled the page behind the box (PageUp / PageDown, the
    /// wheel): from now on the view stays where they put it.
    pub(super) fn comment_scrolled(&mut self) {
        if let Some(d) = &mut self.comment {
            d.scrolled = true;
        }
    }

    /// Tab (`forward`) / S-Tab: move the draft's kind along untyped ->
    /// `[review] kinds` in order -> untyped. No kinds: nothing happens.
    fn comment_cycle_kind(&mut self, forward: bool) {
        let ids: Vec<&str> = self
            .config
            .review
            .kinds
            .iter()
            .map(|k| k.id.as_str())
            .collect();
        let Some(d) = &mut self.comment else { return };
        if ids.is_empty() {
            return;
        }
        // Positions 0..n are the kinds, n is untyped.
        let n = ids.len();
        let at = d
            .kind
            .as_deref()
            .and_then(|k| ids.iter().position(|i| *i == k))
            .unwrap_or(n);
        let next = if forward {
            (at + 1) % (n + 1)
        } else {
            (at + n) % (n + 1)
        };
        d.kind = ids.get(next).map(|s| s.to_string());
    }

    pub(super) fn comment_action(&mut self, a: CommentAction) {
        match a {
            CommentAction::Start(visual) => self.comment_start(visual),
            CommentAction::Edit(e) => {
                let wrap = wrap_width(box_width(self.comment_pane_width()));
                if let Some(d) = &mut self.comment {
                    d.text.apply(e, wrap);
                }
            }
            CommentAction::Newline => {
                if let Some(d) = &mut self.comment {
                    d.text.insert('\n');
                }
            }
            CommentAction::Kind(forward) => self.comment_cycle_kind(forward),
            CommentAction::Page(down) => {
                if down {
                    self.page_down();
                } else {
                    self.page_up();
                }
                self.comment_scrolled();
            }
            CommentAction::Cancel => {
                self.comment_end();
            }
            CommentAction::Save => {
                if let Some(d) = self.comment_end() {
                    self.comment_save(d);
                }
            }
            CommentAction::Editor => {
                if let Some(d) = self.comment_end() {
                    self.pending_effect = Some(Effect::EditComment {
                        path: d.path,
                        lines: d.lines,
                        excerpt: d.excerpt,
                        initial: d.text.into_text(),
                        kind: d.kind,
                    });
                }
            }
        }
    }

    /// Pasted text typed at the cursor, its line breaks kept.
    pub(super) fn comment_paste(&mut self, text: &str) {
        if let Some(d) = &mut self.comment {
            d.text.insert_str(&paste_text(text));
        }
    }

    fn comment_end(&mut self) -> Option<Draft> {
        if self.mode == Mode::Comment {
            self.mode = Mode::Normal;
        }
        self.comment.take()
    }

    /// Freeze the target lines and excerpt, then open the box.
    fn comment_start(&mut self, visual: bool) {
        let (lo, hi) = match (visual, self.visual_rows()) {
            (true, Some(rows)) => rows,
            _ => (self.cursor.row, self.cursor.row),
        };
        self.visual_leave();
        if self.page.is_none() {
            return;
        }
        let path = match self.review_rel_path() {
            Ok(p) => p,
            Err(msg) => return self.set_status(msg),
        };
        let Some((a, b)) = self.source_line_range(lo, hi, true) else {
            return self.set_status(NO_SOURCE_LINE);
        };
        let Some(p) = &self.page else { return };
        let src = p.doc.source.as_str();
        let excerpt = src[super::visual::line_bytes(src, a, b)].trim_end_matches(['\n', '\r']);
        self.comment = Some(Draft {
            path,
            lines: (a as u32, b as u32),
            excerpt: excerpt.to_string(),
            text: TextBox::new(),
            kind: None,
            scroll: Cell::new(0),
            view: self.scroll,
            scrolled: false,
        });
        self.status.clear();
        self.mode = Mode::Comment;
    }

    /// Save `d` with `body` (trimmed); an empty body saves nothing.
    fn comment_save(&mut self, d: Draft) {
        let body = d.text.text().trim();
        if body.is_empty() {
            return self.set_status(EMPTY_COMMENT);
        }
        let Some(store) = self.review_store() else {
            return self.set_status("Can't comment: no review batch");
        };
        let c = debrief_review::Comment::on_file(d.path, d.lines.0..=d.lines.1, d.excerpt, body)
            .with_kind(d.kind);
        let msg = match store.add(c) {
            Ok(_) => format!("Comment added ({} in batch)", store.comments().len()),
            Err(e) => format!("comment not saved: {e}"),
        };
        self.set_status(msg);
        self.review_refresh_markers();
    }

    /// `C-e` (TUI suspended): edit the text in the editor, save what it
    /// leaves.
    pub(super) fn run_comment_editor(
        &mut self,
        path: String,
        lines: (u32, u32),
        excerpt: String,
        initial: String,
        kind: Option<String>,
    ) {
        match self.edit_in_editor(&initial) {
            Ok(text) => self.comment_save(Draft {
                path,
                lines,
                excerpt,
                text: TextBox::from_text(&text),
                kind,
                scroll: Cell::new(0),
                view: 0,
                scrolled: false,
            }),
            Err(e) => self.set_status(format!("editor: {e}; comment not saved")),
        }
    }

    /// Write `initial` to a temp file, run the editor on it through the
    /// launcher runner (TUI suspended), and return what it leaves. `Err`
    /// says why nothing came back.
    pub(super) fn edit_in_editor(&mut self, initial: &str) -> Result<String, String> {
        let file = write_private_temp(initial).map_err(|e| e.to_string())?;
        let mut argv = editor_words(self.env.as_ref());
        argv.push(file.display().to_string());
        let cwd = self
            .review_root()
            .map_or_else(|| self.link_dir(), std::path::Path::to_path_buf);
        let cmd = LaunchCommand {
            name: "editor".into(),
            argv,
            cwd,
        };
        let result = (self.runner)(&cmd);
        let text = std::fs::read_to_string(&file);
        let _ = std::fs::remove_file(&file);
        match result {
            Ok(Exit::Code(0)) => text.map_err(|e| e.to_string()),
            Ok(Exit::Code(n)) => Err(format!("exited with status {n}")),
            Ok(Exit::Signal(n)) => Err(format!("killed by signal {n}")),
            Err(e) => Err(format!("{e:#}")),
        }
    }
}

/// Create a fresh `ramble-comment-*.md` in the temp dir holding `text`:
/// exclusively (never follows or reuses an existing file) and, on unix,
/// readable by the owner only.
fn write_private_temp(text: &str) -> std::io::Result<std::path::PathBuf> {
    use std::io::Write;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut attempt = 0u32;
    loop {
        let file = std::env::temp_dir().join(format!(
            "ramble-comment-{}-{nanos}-{attempt}.md",
            std::process::id()
        ));
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        match opts.open(&file) {
            Ok(mut f) => {
                if let Err(e) = f.write_all(text.as_bytes()) {
                    let _ = std::fs::remove_file(&file);
                    return Err(e);
                }
                return Ok(file);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && attempt < 100 => {
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{fit_scroll, off_the_box, paste_text, place, user_scroll};

    #[test]
    fn place_below_else_above_never_over_the_rows() {
        assert_eq!(place(5, 6, 3, 20), (7, 3), "below");
        assert_eq!(place(17, 19, 3, 20), (14, 3), "above");
        assert_eq!(place(4, 15, 6, 20), (16, 4), "shrunk on the roomier side");
        assert_eq!(place(10, 18, 6, 20), (4, 6), "above");
        assert_eq!(place(4, 17, 6, 20), (0, 4), "shrunk above");
        assert_eq!(place(0, 19, 3, 20), (17, 3), "no room: over the bottom");
    }

    #[test]
    fn place_pins_to_the_edge_the_rows_scrolled_off() {
        assert_eq!(place(-9, -2, 4, 20), (0, 4), "scrolled off the top");
        assert_eq!(place(25, 26, 4, 20), (16, 4), "off the bottom");
        assert_eq!(place(-3, 2, 4, 20), (3, 4), "partly visible: below");
        assert_eq!(place(1, 1, 30, 20), (2, 18), "taller than the pane");
    }

    #[test]
    fn fit_scroll_moves_the_rows_out_of_the_box() {
        // Rows 10..=12 at scroll 10 in a 7-row pane, a 3-row box: fits.
        assert_eq!(fit_scroll(10, 12, 3, 7, 10, 90, 0), 10);
        // Rows 0..=49 (taller than the pane): the last row above the box.
        assert_eq!(fit_scroll(0, 49, 3, 11, 0, 90, 0), 42);
        // One row at the top of a 4-row pane, a 3-row box: below it.
        assert_eq!(fit_scroll(5, 5, 3, 4, 5, 90, 0), 5);
        // One row in the middle of a 5-row pane: the nearest scroll, the
        // row then one off the edge with the box beside it.
        assert_eq!(fit_scroll(7, 7, 3, 5, 5, 90, 0), 6);
        // The user scrolled: on that way, never back.
        assert_eq!(fit_scroll(7, 7, 3, 5, 5, 90, 1), 6);
        assert_eq!(fit_scroll(7, 7, 3, 5, 5, 90, -1), 4);
        // A 3-row pane: no scroll fits; the row above a box at the bottom.
        assert_eq!(fit_scroll(7, 7, 3, 3, 6, 90, 0), 7);
        // Scrolled off: the box pins to the edge, the view stays.
        assert_eq!(fit_scroll(7, 7, 3, 5, 9, 90, 1), 9);
    }

    #[test]
    fn a_user_scroll_stays_unless_the_box_has_no_room() {
        // Rows 0..=49 at scroll 10 in an 11-row pane: they fill it.
        assert_eq!(user_scroll(0, 49, 3, 11, 10, 90, -1, 100), 10);
        // Rows 20..=59 at scroll 15: the box above them, the scroll kept.
        assert_eq!(user_scroll(20, 59, 3, 11, 15, 90, -1, 100), 15);
        // Rows 20..=59 at scroll 51: 2 free rows below, too few. Down
        // opens a third; up, the rows fill the pane at 49.
        assert_eq!(user_scroll(20, 59, 3, 11, 51, 90, 1, 100), 52);
        assert_eq!(user_scroll(20, 59, 3, 11, 51, 90, -1, 100), 49);
        // A 6-row box shrinks into the 3 free rows (a text row).
        assert_eq!(user_scroll(20, 59, 6, 11, 51, 90, 1, 100), 52);
        // At the end of the diff, back the other way.
        assert_eq!(user_scroll(20, 59, 3, 11, 51, 51, 1, 100), 49);
        // Scrolled off: pinned to the edge, kept.
        assert_eq!(user_scroll(20, 29, 3, 11, 40, 90, 1, 100), 40);
        // Past the end, the box over all 3 rows still shown: back one,
        // a row left for the cursor.
        assert_eq!(user_scroll(20, 29, 3, 11, 97, 97, 1, 100), 96);
    }

    #[test]
    fn the_cursor_moves_off_the_box_to_the_nearest_free_row() {
        assert_eq!(off_the_box(2, 5, 3, 10), Some(2), "not under it");
        assert_eq!(off_the_box(5, 5, 3, 10), Some(4), "above");
        assert_eq!(off_the_box(7, 5, 3, 10), Some(8), "below");
        assert_eq!(off_the_box(6, 5, 3, 10), Some(4), "a tie: above");
        assert_eq!(off_the_box(9, 7, 3, 10), Some(6), "box at the bottom");
        assert_eq!(off_the_box(0, 0, 3, 10), Some(3), "box at the top");
        assert_eq!(off_the_box(1, 0, 3, 3), None, "box fills the pane");
        assert_eq!(off_the_box(8, 7, 3, 9), Some(6), "no row below it");
    }

    #[test]
    fn paste_keeps_line_breaks() {
        assert_eq!(paste_text("a\r\nb\rc\nd\te\x07"), "a\nb\nc\nd e");
    }
}
