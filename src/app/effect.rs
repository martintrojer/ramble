//! Side effects that need the real terminal: queued by actions as an
//! [`Effect`], drained by `run` outside the TUI.

use std::path::{Path, PathBuf};

use anyhow::Context;

use super::App;
use super::launch::LaunchCommand;

/// A side effect to run with the TUI suspended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Open this (non-markdown) file in the editor, at `line` if given.
    Edit { path: PathBuf, line: Option<usize> },
    /// Run a launcher, then reload the page.
    Launch(LaunchCommand),
    /// `C-e` in the comment prompt: edit `initial` in the editor, then
    /// save it as a comment on `lines` of `path` (batch-relative).
    EditComment {
        path: String,
        lines: (u32, u32),
        excerpt: String,
        initial: String,
    },
    /// `C-e` in the review picker: edit comment `id`'s body.
    EditReviewComment { id: u64, initial: String },
    /// `<leader>rr`: export the batch and run the hand-back command.
    SendReview,
}

impl App {
    /// The queued side effect, if any.
    pub fn pending_effect(&self) -> Option<&Effect> {
        self.pending_effect.as_ref()
    }

    /// Run the queued side effect (if any). The caller must have left the
    /// TUI first; `run` does this.
    pub fn run_pending_effect(&mut self) {
        match self.pending_effect.take() {
            Some(Effect::Edit { path, line }) => {
                if let Err(e) = (self.editor)(&path, line) {
                    self.set_status(format!("editor: {e:#}"));
                }
            }
            Some(Effect::Launch(cmd)) => self.run_launch(&cmd),
            Some(Effect::EditComment {
                path,
                lines,
                excerpt,
                initial,
            }) => self.run_comment_editor(path, lines, excerpt, initial),
            Some(Effect::EditReviewComment { id, initial }) => {
                self.run_review_comment_editor(id, &initial)
            }
            Some(Effect::SendReview) => self.run_review_send(),
            None => {}
        }
    }

    /// A non-markdown file waiting for the editor.
    pub fn pending_editor(&self) -> Option<&Path> {
        match &self.pending_effect {
            Some(Effect::Edit { path, .. }) => Some(path),
            _ => None,
        }
    }

    /// Hand the pending file (if any) to the editor. The caller must have
    /// left the TUI first; `run` does this.
    pub fn run_pending_editor(&mut self) {
        if self.pending_editor().is_some() {
            self.run_pending_effect();
        }
    }
}

/// Default opener: `open` (macOS) or `xdg-open`, detached, with null stdio.
pub(super) fn system_open(url: &str) {
    use std::process::{Command, Stdio};
    let prog = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    if let Ok(mut child) = Command::new(prog)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        // Reap it in the background so it never becomes a zombie.
        std::thread::spawn(move || child.wait());
    }
}

/// Where the editor child's stdin comes from.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum EditorStdin {
    /// Inherit ours: stdin is the terminal.
    Inherit,
    /// Open `/dev/tty`: stdin is the (exhausted) document pipe.
    Tty,
}

pub(crate) fn editor_stdin(stdin_is_tty: bool) -> EditorStdin {
    if stdin_is_tty {
        EditorStdin::Inherit
    } else {
        EditorStdin::Tty
    }
}

/// Stdin for a child run in the terminal: ours when it is the terminal,
/// else `tty()` (normally `/dev/tty`), falling back to ours if that fails.
pub(crate) fn child_stdin(
    stdin_is_tty: bool,
    tty: impl FnOnce() -> std::io::Result<std::fs::File>,
) -> std::process::Stdio {
    use std::process::Stdio;
    match editor_stdin(stdin_is_tty) {
        EditorStdin::Tty => tty().map_or_else(|_| Stdio::inherit(), Stdio::from),
        EditorStdin::Inherit => Stdio::inherit(),
    }
}

pub(crate) fn open_tty() -> std::io::Result<std::fs::File> {
    std::fs::File::open("/dev/tty")
}

/// Default editor: `$VISUAL`, else `$EDITOR`, else `vi`, in the terminal,
/// given `+LINE` before the file when `line` is set.
pub(super) fn system_edit(path: &Path, line: Option<usize>) -> anyhow::Result<()> {
    use std::io::IsTerminal;
    let cmd = ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "vi".into());
    let mut words = cmd.split_whitespace();
    let prog = words.next().unwrap_or("vi");
    let status = std::process::Command::new(prog)
        .args(words)
        .args(line.map(|n| format!("+{n}")))
        .arg(path)
        .stdin(child_stdin(std::io::stdin().is_terminal(), open_tty))
        .status()
        .with_context(|| format!("running {prog}"))?;
    anyhow::ensure!(status.success(), "{prog} exited with {status}");
    Ok(())
}

/// Copies text to the system clipboard.
pub trait Clipboard {
    fn copy(&mut self, text: &str) -> anyhow::Result<()>;
}

/// Default clipboard: an OSC 52 sequence queued for the terminal. `run`
/// writes the queue to stdout (which ratatui owns) after each event.
pub(super) struct Osc52 {
    pub(super) out: std::rc::Rc<std::cell::RefCell<Vec<u8>>>,
}

impl Clipboard for Osc52 {
    fn copy(&mut self, text: &str) -> anyhow::Result<()> {
        self.out.borrow_mut().extend(osc52(text).into_bytes());
        Ok(())
    }
}

/// The OSC 52 "set clipboard" sequence for `text`.
pub fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_stdin_is_tty_when_stdin_is_a_pipe() {
        assert_eq!(editor_stdin(false), EditorStdin::Tty);
        assert_eq!(editor_stdin(true), EditorStdin::Inherit);
    }

    #[test]
    fn base64_matches_rfc4648_vectors() {
        for (i, o) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(i.as_bytes()), o);
        }
        assert_eq!(osc52("hi"), "\x1b]52;c;aGk=\x07");
    }
}
