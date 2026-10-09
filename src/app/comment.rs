//! Comment mode (debrief spec §2): `cc` comments on the cursor line,
//! visual `c` on the selection's source lines. Lines and excerpt are
//! frozen from the displayed document when `c` is pressed; a one-line
//! prompt takes the body (Enter saves, Esc cancels, `C-e` moves it into
//! the editor, Tab / S-Tab pick the kind from `[review] kinds`, readline
//! keys edit it: see [`super::textbox`]). Comments go to the
//! debrief-review batch for the page's repo root.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use debrief_review::Kind;

use super::effect::Effect;
use super::keys::{Action, KeyResult};
use super::launch::{Exit, LaunchCommand, editor_words};
use super::textbox::{Edit, TextBox};
use super::{App, Mode};

/// The prompt's label.
pub const COMMENT_PROMPT: &str = "comment: ";
/// Status when Enter (or the editor) leaves an empty body.
pub const EMPTY_COMMENT: &str = "Empty comment not saved";
/// Status when the cursor row stands for no source line.
pub const NO_SOURCE_LINE: &str = "No source line here";

/// Comment-mode actions.
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
}

/// Keys while typing a comment.
pub(super) fn comment_keymap(keys: &[KeyEvent]) -> KeyResult {
    let Some(key) = keys.last() else {
        return KeyResult::None;
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let a = match key.code {
        KeyCode::Char('e') if ctrl => CommentAction::Editor,
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

/// The prompt's label, with the kind: `comment [ISSUE]: `.
fn comment_label(d: &Draft) -> String {
    match &d.kind {
        Some(k) => format!("comment [{}]: ", Kind::label(k)),
        None => COMMENT_PROMPT.to_string(),
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

    /// The typed comment as the prompt shows it, its kind in the label
    /// (`comment [ISSUE]: `).
    pub fn comment_prompt(&self) -> Option<String> {
        let d = self.comment.as_ref()?;
        Some(format!("{}{}", comment_label(d), d.text.text()))
    }

    /// The prompt in `width` cells: the label, the part of the text around
    /// the cursor that fits, and the cursor's column.
    pub fn comment_prompt_view(&self, width: usize) -> Option<(String, usize)> {
        let d = self.comment.as_ref()?;
        let label = comment_label(d);
        let lw = unicode_width::UnicodeWidthStr::width(label.as_str());
        let (range, col) = d.text.line_view(width.saturating_sub(lw));
        Some((format!("{label}{}", &d.text.text()[range]), lw + col))
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
                if let Some(d) = &mut self.comment {
                    // One line: Up/Down have no row to go to.
                    d.text.apply(e, 0);
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

    /// Pasted text (one line) typed at the cursor.
    pub(super) fn comment_insert(&mut self, text: &str) {
        if let Some(d) = &mut self.comment {
            d.text.insert_line(text);
        }
    }

    fn comment_end(&mut self) -> Option<Draft> {
        if self.mode == Mode::Comment {
            self.mode = Mode::Normal;
        }
        self.comment.take()
    }

    /// Freeze the target lines and excerpt, then open the prompt.
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
