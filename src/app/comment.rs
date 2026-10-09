//! Comment mode (debrief spec §2): `cc` comments on the cursor line,
//! visual `c` on the selection's source lines. Lines and excerpt are
//! frozen from the displayed document when `c` is pressed; a bordered box
//! anchored below the commented rows (above when it doesn't fit) takes
//! the body: Enter saves, `C-j` / `Alt-Enter` insert a newline, Esc
//! cancels, `C-e` moves it into the editor, Tab / S-Tab pick the kind
//! from `[review] kinds`, readline keys edit it (see [`super::textbox`]).
//! The rows stay highlighted while typing. Comments go to the
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

/// The wrap width of a box `w` wide: inside the border, with a spare cell
/// for the cursor at the end of a full row.
fn wrap_width(w: u16) -> usize {
    (w as usize).saturating_sub(3).max(1)
}

/// The box's top row and height, `h` rows wanted, for selected rows at
/// screen rows `lo..=hi` (relative to the pane's top, may lie outside it)
/// in a pane `ph` rows high: below the rows, else above them, else on the
/// roomier side, shrunk to fit (at least one text row), else over the
/// bottom. Rows scrolled off pin the box to that edge.
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

/// Keys while typing a comment.
pub(super) fn comment_keymap(keys: &[KeyEvent]) -> KeyResult {
    let Some(key) = keys.last() else {
        return KeyResult::None;
    };
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
        let want = rows.len().min(BOX_ROWS) as u16 + 2;
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
            }),
            Err(e) => self.set_status(format!("editor: {e}; comment not saved")),
        }
    }

    /// Write `initial` to a temp file, run the editor on it through the
    /// launcher runner (TUI suspended), and return what it leaves. `Err`
    /// says why nothing came back.
    pub(super) fn edit_in_editor(&mut self, initial: &str) -> Result<String, String> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let file =
            std::env::temp_dir().join(format!("ramble-comment-{}-{nanos}.md", std::process::id()));
        std::fs::write(&file, initial).map_err(|e| e.to_string())?;
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

#[cfg(test)]
mod tests {
    use super::{paste_text, place};

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
    fn paste_keeps_line_breaks() {
        assert_eq!(paste_text("a\r\nb\rc\nd\te\x07"), "a\nb\nc\nd e");
    }
}
