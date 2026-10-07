//! Owns state, applies actions, runs the event loop
//! (key | fs-watch | lsp | review | resize).
//!
//! Layout: `keys` (key → [`Action`] table and dispatch), `motion` (cursor
//! motions), `follow` (links and history), `page` (loading and layout),
//! `effect` (side effects run outside the TUI), `run` (terminal and loop),
//! `search` (`/ ? n N * #`), `marks` (marks and yank), `launch`
//! (launchers), `watch` (live reload).
//! Later units add a file and register in the tables here and in `keys`.

mod cmdline;
mod codepath;
mod effect;
mod follow;
mod hints;
mod keys;
mod launch;
mod lsp_glue;
mod marks;
mod motion;
mod page;
mod picker;
mod run;
mod scroll;
mod search;
pub mod sidebar;
mod watch;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::Instant;

use crossterm::event::KeyEvent;

use crate::config::Config;
use crate::doc::Document;
use crate::nav::History;
use crate::render::{RenderedPage, Theme};

pub use cmdline::CmdAction;
pub use codepath::resolve as resolve_code_path;
pub use effect::{Clipboard, Effect, osc52};
pub use hints::{HINT_ALPHABET, hint_labels};
pub use keys::{Action, KeyResult};
pub use launch::{Exit, LaunchCommand, LaunchVars, expand, parse_key, system_run, vcs_root};
pub use lsp_glue::{SPINNER_AFTER, server_spec, tag as lsp_tag};
pub use picker::{PICKER_TAG_BASE, PickerAction, PickerView, filter as picker_filter};
pub use run::{run, suspend_and_run};
pub use search::find_all;
pub use sidebar::Focus;
pub use watch::{DEBOUNCE, DELETED_BANNER, FileWatcher, FsEvent};

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
    "No file loaded. Pick one in the file tree (C-w h, then Enter; <leader>e shows it).";

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
    /// Typing a `/` or `?` pattern.
    Search,
    /// Typing a sidebar filter (`/` in a sidebar pane).
    Filter,
    /// The picker overlay is open.
    Picker,
    /// Typing a `:` command.
    Command,
    /// Typing a hint label after `s`.
    Hint,
}

/// Everything the event loop feeds the app. Later units add variants
/// (Lsp, FsWatch, Review) sent from background threads via [`App::sender`].
#[derive(Debug, Clone)]
pub enum AppEvent {
    Key(KeyEvent),
    Resize(u16, u16),
    Tick(Instant),
    /// The watched file changed or disappeared (debounced).
    FsWatch(PathBuf, FsEvent),
    /// An event from a language server (`run` drains the app's own LSP
    /// channel with [`App::pump_lsp`]; this variant injects one directly).
    Lsp(crate::lsp::LspEvent),
}

/// Runs an editor on a file (at a line, if given) outside the TUI.
type Editor = dyn FnMut(&Path, Option<usize>) -> anyhow::Result<()>;

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
    search: search::Search,
    marks: marks::Marks,
    clipboard: Box<dyn Clipboard>,
    /// Bytes for the terminal (OSC 52) that `run` writes to stdout.
    term_out: Rc<RefCell<Vec<u8>>>,
    /// Runs launcher commands (`launch::system_run` by default).
    runner: Box<launch::Runner>,
    /// Environment lookup for `${editor}`.
    env: Box<launch::EnvLookup>,
    /// Launcher key sequences after the leader, with indices into
    /// `config.launch`.
    leader_bindings: Vec<(Vec<char>, usize)>,
    /// When a launcher last reloaded the page (its own fs event is ignored).
    launch_reloaded_at: Option<Instant>,
    watcher: Option<FileWatcher>,
    /// Banner over the content, e.g. the file was deleted.
    banner: Option<String>,
    /// Language servers and per-page LSP state.
    lsp: lsp_glue::LspState,
    /// The clock, advanced by [`App::tick`].
    now: Instant,
    sidebar: sidebar::Sidebar,
    picker: picker::PickerState,
    /// The `:` command being typed.
    cmdline: Option<String>,
    hints: hints::Hints,
}

impl App {
    /// Load the start target and lay it out for a terminal of `size`
    /// (cols, rows).
    pub fn new(opts: StartOptions, size: (u16, u16)) -> anyhow::Result<App> {
        let term_out = Rc::new(RefCell::new(Vec::new()));
        let (leader_bindings, key_errors) = launch::bindings(&opts.config.launch);
        let sidebar = sidebar::Sidebar::new(&opts.config.sidebar, &opts.target);
        let lsp = lsp_glue::LspState::new(&opts.config.lsp.server);
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
            search: search::Search::default(),
            marks: marks::Marks::new(),
            clipboard: Box::new(effect::Osc52 {
                out: term_out.clone(),
            }),
            term_out,
            runner: Box::new(launch::system_run),
            env: Box::new(|k| std::env::var(k).ok()),
            leader_bindings,
            launch_reloaded_at: None,
            watcher: None,
            banner: None,
            lsp,
            now: Instant::now(),
            sidebar,
            picker: Default::default(),
            cmdline: None,
            hints: hints::Hints::default(),
        };
        match opts.target {
            StartTarget::File(path) => app.open_file(&path)?,
            StartTarget::Dir(_) => app.placeholder = Some(NO_FILE_MESSAGE),
            StartTarget::Stdin(text) => app.open_stdin(Arc::new(text)),
        }
        app.sync_tree();
        app.sidebar_fit();
        if !key_errors.is_empty() {
            app.set_status(key_errors.join("; "));
        }
        Ok(app)
    }

    /// Replace the clipboard sink (tests record).
    pub fn with_clipboard(mut self, clipboard: impl Clipboard + 'static) -> Self {
        self.clipboard = Box::new(clipboard);
        self
    }

    /// Replace the launcher runner (tests record argv and cwd).
    pub fn with_runner(
        mut self,
        runner: impl FnMut(&LaunchCommand) -> anyhow::Result<Exit> + 'static,
    ) -> Self {
        self.runner = Box::new(runner);
        self
    }

    /// Replace the environment lookup used for `${editor}`.
    pub fn with_env(mut self, env: impl Fn(&str) -> Option<String> + 'static) -> Self {
        self.env = Box::new(env);
        self
    }

    /// Take the bytes queued for the terminal (OSC 52 clipboard writes).
    pub fn take_terminal_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.term_out.borrow_mut())
    }

    /// Replace the side effects: `opener` gets external URLs, `editor`
    /// gets non-markdown files. Tests pass recording closures.
    pub fn with_effects(
        mut self,
        opener: impl FnMut(&str) + 'static,
        mut editor: impl FnMut(&Path) -> anyhow::Result<()> + 'static,
    ) -> Self {
        self.opener = Box::new(opener);
        self.editor = Box::new(move |path, _line| editor(path));
        self
    }

    /// Replace the editor with one that also gets the line to open at
    /// (code-path links with `:LINE`). Tests pass a recording closure.
    pub fn with_editor(
        mut self,
        editor: impl FnMut(&Path, Option<usize>) -> anyhow::Result<()> + 'static,
    ) -> Self {
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
            AppEvent::Resize(cols, rows) => {
                self.resize(cols, rows);
                self.sidebar_fit();
            }
            AppEvent::Tick(now) => self.tick(now),
            AppEvent::FsWatch(path, ev) => self.fs_event(&path, ev),
            AppEvent::Lsp(ev) => self.lsp_event(ev),
        }
    }

    /// Called once per loop iteration (at least every 100 ms): advances the
    /// clock used for the slow-request spinner.
    pub fn tick(&mut self, now: Instant) {
        self.now = now;
    }

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
