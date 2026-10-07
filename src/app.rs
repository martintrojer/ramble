//! Owns state, applies actions, runs the event loop
//! (key | fs-watch | lsp | review | resize).
//!
//! Interface fixed by the orchestrator: `main.rs` (owned by t04_cli) calls
//! [`run`] for interactive mode; t05_app implements it.

use std::path::PathBuf;

use crate::config::Config;

/// What the interactive TUI opens with. Produced by `cli`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartTarget {
    /// Open this markdown file.
    File(PathBuf),
    /// Open the file tree here with nothing loaded.
    Dir(PathBuf),
    /// A document read from stdin (keys then come from /dev/tty).
    Stdin(String),
}

#[derive(Debug, Clone)]
pub struct StartOptions {
    pub target: StartTarget,
    /// Tree root per spec § cli (argument dir, else VCS root, else $HOME).
    pub tree_root: PathBuf,
    pub config: Config,
}

/// Run the interactive TUI until the user quits.
pub fn run(opts: StartOptions) -> anyhow::Result<()> {
    let _ = opts;
    todo!("t05_app")
}
