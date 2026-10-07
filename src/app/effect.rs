//! Side effects that need the real terminal: queued by actions as an
//! [`Effect`], drained by `run` outside the TUI. Later units add variants
//! (e.g. `Launch`).

use std::path::{Path, PathBuf};

use anyhow::Context;

use super::App;

/// A side effect to run with the TUI suspended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Open this (non-markdown) file in the editor.
    Edit(PathBuf),
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
            Some(Effect::Edit(path)) => {
                if let Err(e) = (self.editor)(&path) {
                    self.set_status(format!("editor: {e:#}"));
                }
            }
            None => {}
        }
    }

    /// A non-markdown file waiting for the editor.
    pub fn pending_editor(&self) -> Option<&Path> {
        match &self.pending_effect {
            Some(Effect::Edit(path)) => Some(path),
            None => None,
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

/// Default editor: `$VISUAL`, else `$EDITOR`, else `vi`, in the terminal.
pub(super) fn system_edit(path: &Path) -> anyhow::Result<()> {
    use std::io::IsTerminal;
    use std::process::Stdio;
    let cmd = ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "vi".into());
    let mut words = cmd.split_whitespace();
    let prog = words.next().unwrap_or("vi");
    let stdin = match editor_stdin(std::io::stdin().is_terminal()) {
        EditorStdin::Tty => {
            std::fs::File::open("/dev/tty").map_or_else(|_| Stdio::inherit(), Stdio::from)
        }
        EditorStdin::Inherit => Stdio::inherit(),
    };
    let status = std::process::Command::new(prog)
        .args(words)
        .arg(path)
        .stdin(stdin)
        .status()
        .with_context(|| format!("running {prog}"))?;
    anyhow::ensure!(status.success(), "{prog} exited with {status}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_stdin_is_tty_when_stdin_is_a_pipe() {
        assert_eq!(editor_stdin(false), EditorStdin::Tty);
        assert_eq!(editor_stdin(true), EditorStdin::Inherit);
    }
}
