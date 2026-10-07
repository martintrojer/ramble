//! Owns state, applies actions, runs the event loop
//! (key | fs-watch | lsp | review | resize).
//!
//! Layout: `keys` (key → [`Action`] table and dispatch), `motion` (cursor
//! motions), `follow` (links and history), `page` (loading and layout),
//! `effect` (side effects run outside the TUI), `run` (terminal and loop).
//! Later units add a file and register in the tables here and in `keys`.

mod effect;
mod follow;
mod keys;
mod motion;
mod page;
mod run;
mod scroll;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::Instant;

use crossterm::event::KeyEvent;

use crate::config::Config;
use crate::doc::Document;
use crate::nav::History;
use crate::render::{RenderedPage, Theme};

pub use effect::Effect;
pub use keys::{Action, KeyResult};
pub use run::{run, suspend_and_run};

use motion::Cell;

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

/// Message shown in the content area when no file is loaded.
pub const NO_FILE_MESSAGE: &str =
    "No file loaded. The file tree arrives with the sidebar (step 8). Run: ramble <file.md>";

/// Cursor position in rendered-row coordinates (display columns).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cursor {
    pub row: usize,
    pub col: usize,
}

/// A loaded document and its layout at the current width.
#[derive(Debug, Clone)]
pub struct Page {
    /// `None` for a stdin document.
    pub path: Option<PathBuf>,
    pub doc: Document,
    pub rendered: RenderedPage,
}

/// Input mode; selects the key table in [`App::keymap`]. Later units add
/// Search / Command / Picker / Hint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Normal,
}

/// Everything the event loop feeds the app. Later units add variants
/// (Lsp, FsWatch, Review) sent from background threads via [`App::sender`].
#[derive(Debug, Clone)]
pub enum AppEvent {
    Key(KeyEvent),
    Resize(u16, u16),
    Tick(Instant),
}

/// Runs an editor on a file outside the TUI.
type Editor = dyn FnMut(&Path) -> anyhow::Result<()>;

pub struct App {
    config: Config,
    tree_root: PathBuf,
    theme: Theme,
    size: (u16, u16),
    page: Option<Page>,
    /// Grapheme cells per rendered row of the current page.
    rows: Vec<Vec<Cell>>,
    placeholder: Option<&'static str>,
    cursor: Cursor,
    /// Desired column for vertical motions (`usize::MAX` after `$`).
    want_col: usize,
    scroll: usize,
    status: String,
    mode: Mode,
    /// Typed count, shared by all modes.
    count: Option<usize>,
    /// Keys of an unfinished multi-key sequence, shared by all modes.
    pending: Vec<KeyEvent>,
    quit: bool,
    history: History,
    /// Text of the current page when it came from stdin (kept for history).
    stdin: Option<Arc<String>>,
    /// Opens an external URL (`open` / `xdg-open` by default).
    opener: Box<dyn FnMut(&str)>,
    /// Edits a non-markdown file (`$VISUAL` / `$EDITOR` / `vi` by default).
    editor: Box<Editor>,
    /// A side effect to run outside the TUI; `run` drains it.
    pending_effect: Option<Effect>,
    /// Feeds [`AppEvent`]s back into the loop; set by `run`.
    sender: Option<Sender<AppEvent>>,
}

impl App {
    /// Load the start target and lay it out for a terminal of `size`
    /// (cols, rows).
    pub fn new(opts: StartOptions, size: (u16, u16)) -> anyhow::Result<App> {
        let mut app = App {
            config: opts.config,
            tree_root: opts.tree_root,
            theme: Theme::catppuccin_mocha(),
            size,
            page: None,
            rows: Vec::new(),
            placeholder: None,
            cursor: Cursor::default(),
            want_col: 0,
            scroll: 0,
            status: String::new(),
            mode: Mode::Normal,
            count: None,
            pending: Vec::new(),
            quit: false,
            history: History::new(),
            stdin: None,
            opener: Box::new(effect::system_open),
            editor: Box::new(effect::system_edit),
            pending_effect: None,
            sender: None,
        };
        match opts.target {
            StartTarget::File(path) => app.open_file(&path)?,
            StartTarget::Dir(_) => app.placeholder = Some(NO_FILE_MESSAGE),
            StartTarget::Stdin(text) => app.open_stdin(Arc::new(text)),
        }
        Ok(app)
    }

    /// Replace the side effects: `opener` gets external URLs, `editor`
    /// gets non-markdown files. Tests pass recording closures.
    pub fn with_effects(
        mut self,
        opener: impl FnMut(&str) + 'static,
        editor: impl FnMut(&Path) -> anyhow::Result<()> + 'static,
    ) -> Self {
        self.opener = Box::new(opener);
        self.editor = Box::new(editor);
        self
    }

    /// Give the app the loop's event sender (for background threads).
    pub fn set_sender(&mut self, tx: Sender<AppEvent>) {
        self.sender = Some(tx);
    }

    /// A sender into the event loop, once `run` has set one.
    pub fn sender(&self) -> Option<Sender<AppEvent>> {
        self.sender.clone()
    }

    /// Process one event from the loop.
    pub fn event(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Key(key) => self.handle_key(key),
            AppEvent::Resize(cols, rows) => self.resize(cols, rows),
            AppEvent::Tick(now) => self.tick(now),
        }
    }

    /// Called once per loop iteration (at least every 100 ms). No-op for now.
    pub fn tick(&mut self, _now: Instant) {}

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    /// First visible rendered row.
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    pub fn page(&self) -> Option<&Page> {
        self.page.as_ref()
    }

    /// Terminal size (cols, rows).
    pub fn size(&self) -> (u16, u16) {
        self.size
    }

    /// Rows available for content (the last row is the status line).
    pub fn viewport_height(&self) -> usize {
        (self.size.1 as usize).saturating_sub(1).max(1)
    }

    /// Centered message for the content area when nothing is loaded.
    pub fn placeholder(&self) -> Option<&str> {
        self.placeholder
    }

    /// Index into the page's `doc.links` for the link under the cursor.
    pub fn link_under_cursor(&self) -> Option<usize> {
        let p = self.page.as_ref()?;
        let Cursor { row, col } = self.cursor;
        p.rendered
            .srcmap
            .segments
            .iter()
            .find(|s| s.span.row == row && (s.span.col_start..s.span.col_end).contains(&col))
            .and_then(|s| s.link)
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status = msg.into();
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn should_quit(&self) -> bool {
        self.quit
    }

    /// Status-line title: path relative to the tree root, or `[stdin]`.
    pub fn title(&self) -> String {
        match &self.page {
            Some(Page { path: None, .. }) => "[stdin]".into(),
            Some(Page {
                path: Some(path), ..
            }) => path
                .strip_prefix(&self.tree_root)
                .unwrap_or(path)
                .display()
                .to_string(),
            None => "[no file]".into(),
        }
    }

    /// LSP state for the status line. No LSP until step 10.
    pub fn lsp_label(&self) -> &str {
        "—"
    }

    /// Back-history depth: entries behind the current page.
    pub fn history_depth(&self) -> usize {
        self.history.depth()
    }

    /// Forward-history depth: entries ahead of the current page.
    pub fn forward_depth(&self) -> usize {
        self.history.forward_depth()
    }

    /// Position through the page: `All`, `Top`, or `NN%`.
    pub fn position_label(&self) -> String {
        let total = self.total_rows();
        let vh = self.viewport_height();
        if total <= vh {
            "All".into()
        } else if self.scroll == 0 {
            "Top".into()
        } else {
            format!("{}%", ((self.scroll + vh) * 100 / total).min(100))
        }
    }

    fn total_rows(&self) -> usize {
        self.rows.len()
    }

    fn last_row(&self) -> usize {
        self.total_rows().saturating_sub(1)
    }

    fn max_scroll(&self) -> usize {
        self.total_rows().saturating_sub(self.viewport_height())
    }
}
