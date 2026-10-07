//! Owns state, applies actions, runs the event loop
//! (key | fs-watch | lsp | review | resize).
//!
//! Interface fixed by the orchestrator: `main.rs` (owned by t04_cli) calls
//! [`run`] for interactive mode; t05_app implements it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::config::Config;
use crate::doc::{self, Document};
use crate::nav::{self, Entry, History, PageRef, Target};
use crate::render::{self, RenderedPage, SrcMap, Theme};
use crate::ui;

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

/// Cap on a typed count, so `n as isize` never wraps negative.
const MAX_COUNT: usize = 1_000_000;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Empty,
    Blank,
    Word,
    Punct,
}

/// One grapheme on a rendered row.
#[derive(Debug, Clone, Copy)]
struct Cell {
    col: usize,
    class: Class,
}

/// A word-motion position: row plus grapheme index (0 on an empty row).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pos {
    row: usize,
    idx: usize,
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
    count: Option<usize>,
    pending: Option<char>,
    quit: bool,
    history: History,
    /// Text of the current page when it came from stdin (kept for history).
    stdin: Option<Arc<String>>,
    /// Opens an external URL (`open` / `xdg-open` by default).
    opener: Box<dyn FnMut(&str)>,
    /// Edits a non-markdown file (`$VISUAL` / `$EDITOR` / `vi` by default).
    editor: Box<Editor>,
    /// A file to hand to the editor; `run` drains it outside the TUI.
    pending_editor: Option<PathBuf>,
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
            count: None,
            pending: None,
            quit: false,
            history: History::new(),
            stdin: None,
            opener: Box::new(system_open),
            editor: Box::new(system_edit),
            pending_editor: None,
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

    /// Load and render `path`, cursor to the top. An I/O error is returned;
    /// a binary file leaves the current page unchanged and sets the status.
    pub fn open_file(&mut self, path: &Path) -> anyhow::Result<()> {
        let bytes = std::fs::read(path).with_context(|| format!("{}", path.display()))?;
        self.open_bytes(path, &bytes);
        Ok(())
    }

    /// Show `bytes` as the page for `path`. False (page unchanged) when the
    /// bytes look binary.
    fn open_bytes(&mut self, path: &Path, bytes: &[u8]) -> bool {
        let Some(doc) = doc::from_bytes(bytes) else {
            self.set_status("looks binary");
            return false;
        };
        let lossy = doc.lossy;
        self.set_page(Some(path.to_path_buf()), doc);
        if lossy {
            self.set_status("not valid UTF-8");
        }
        true
    }

    fn open_stdin(&mut self, text: Arc<String>) {
        self.set_page(None, doc::parse(text.as_ref().clone()));
        self.stdin = Some(text);
    }

    fn set_page(&mut self, path: Option<PathBuf>, doc: Document) {
        let rendered = render::render(&doc, self.render_width(), &self.theme);
        self.page = Some(Page {
            path,
            doc,
            rendered,
        });
        self.placeholder = None;
        self.rebuild_rows();
        self.cursor = Cursor::default();
        self.want_col = 0;
        self.scroll = 0;
        self.status.clear();
        self.stdin = None;
    }

    fn render_width(&self) -> u16 {
        self.size.0.min(self.config.render.max_width).max(1)
    }

    fn rebuild_rows(&mut self) {
        self.rows = match &self.page {
            None => Vec::new(),
            Some(p) => p.rendered.lines.iter().map(row_cells).collect(),
        };
    }

    /// Re-render for a new terminal size, keeping the cursor on the same
    /// source byte.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let anchor = self.page.as_ref().and_then(|p| {
            let map = &p.rendered.srcmap;
            if let Some(b) = map.source_at(self.cursor.row, self.cursor.col) {
                return Some((b, false));
            }
            let n = p.rendered.lines.len();
            (0..self.cursor.row)
                .rev()
                .chain(self.cursor.row + 1..n)
                .find_map(|r| map.source_at(r, 0))
                .map(|b| (b, true))
        });
        self.size = (cols, rows);
        let width = self.render_width();
        if let Some(p) = &mut self.page {
            p.rendered = render::render(&p.doc, width, &self.theme);
        }
        self.rebuild_rows();
        if let (Some((byte, fallback)), Some(p)) = (anchor, &self.page) {
            let map = &p.rendered.srcmap;
            if let Some(row) = map.row_for(byte) {
                let col = if fallback { 0 } else { col_for(map, byte) };
                self.cursor.row = row;
                self.want_col = col;
            }
        }
        self.set_col(self.want_col);
        self.keep_visible();
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

    /// A non-markdown file waiting for the editor.
    pub fn pending_editor(&self) -> Option<&Path> {
        self.pending_editor.as_deref()
    }

    /// Hand the pending file (if any) to the editor. The caller must have
    /// left the TUI first; `run` does this.
    pub fn run_pending_editor(&mut self) {
        if let Some(path) = self.pending_editor.take()
            && let Err(e) = (self.editor)(&path)
        {
            self.set_status(format!("editor: {e:#}"));
        }
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

    // ---------------------------------------------------------------------
    // Keys

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if let Some(prefix) = self.pending.take() {
            let count = self.count.take();
            if let KeyCode::Char(c) = key.code
                && !ctrl
            {
                self.prefixed(prefix, c, count);
            }
            self.keep_visible();
            return;
        }
        if let KeyCode::Char(c @ '0'..='9') = key.code
            && !ctrl
            && (c != '0' || self.count.is_some())
        {
            let d = c as usize - '0' as usize;
            let n = self.count.unwrap_or(0).saturating_mul(10).saturating_add(d);
            self.count = Some(n.min(MAX_COUNT));
            return;
        }
        let count = self.count.take();
        let n = count.unwrap_or(1).max(1);
        if ctrl {
            match key.code {
                // C-] arrives as C-5 from crossterm.
                KeyCode::Char('5' | ']') => self.follow(),
                KeyCode::Char('o' | 't') => self.back(),
                KeyCode::Char('i') => self.forward(),
                KeyCode::Char(c) => self.scroll_key(c),
                _ => {}
            }
        } else {
            match key.code {
                KeyCode::Enter => self.follow(),
                KeyCode::Tab => self.forward(),
                KeyCode::Char(c @ ('g' | 'z' | 'Z')) => {
                    self.pending = Some(c);
                    self.count = count;
                }
                KeyCode::Char('q') => self.quit = true,
                KeyCode::Char('h') | KeyCode::Left => self.horizontal(-(n as isize)),
                KeyCode::Char('l') | KeyCode::Right => self.horizontal(n as isize),
                KeyCode::Char('j') | KeyCode::Down => self.vertical(n as isize),
                KeyCode::Char('k') | KeyCode::Up => self.vertical(-(n as isize)),
                KeyCode::Char('0') | KeyCode::Home => self.set_col(0),
                KeyCode::Char('$') | KeyCode::End => {
                    self.vertical(n as isize - 1);
                    self.want_col = usize::MAX;
                    self.clamp_cursor();
                }
                KeyCode::Char('w') => self.repeat(n, Self::word_forward),
                KeyCode::Char('b') => self.repeat(n, Self::word_backward),
                KeyCode::Char('e') => self.repeat(n, Self::word_end),
                KeyCode::Char('}') => self.repeat(n, Self::paragraph_forward),
                KeyCode::Char('{') => self.repeat(n, Self::paragraph_backward),
                KeyCode::Char('G') => {
                    let row = count.map_or(self.last_row(), |c| c - 1);
                    self.goto_row(row);
                }
                KeyCode::Char('H') => {
                    let bottom = self.visible_bottom();
                    self.goto_row((self.scroll + n - 1).min(bottom));
                }
                KeyCode::Char('L') => {
                    let bottom = self.visible_bottom();
                    self.goto_row(bottom.saturating_sub(n - 1).max(self.scroll));
                }
                KeyCode::Char('M') => {
                    let mid = self.scroll + (self.visible_bottom() - self.scroll) / 2;
                    self.goto_row(mid);
                }
                KeyCode::PageDown => self.scroll_key('f'),
                KeyCode::PageUp => self.scroll_key('b'),
                _ => {}
            }
        }
        self.keep_visible();
    }

    fn prefixed(&mut self, prefix: char, c: char, count: Option<usize>) {
        match (prefix, c) {
            ('g', 'g') => self.goto_row(count.map_or(0, |c| c - 1)),
            ('g', 'd') => self.follow(),
            ('g', 'x') => self.open_external(),
            ('z', 'z') => {
                let half = (self.viewport_height() - 1) / 2;
                self.scroll = self.cursor.row.saturating_sub(half).min(self.max_scroll());
            }
            ('z', 't') => self.scroll = self.cursor.row.min(self.max_scroll()),
            ('z', 'b') => {
                self.scroll = (self.cursor.row + 1).saturating_sub(self.viewport_height());
            }
            ('Z', 'Z') => self.quit = true,
            _ => {}
        }
    }

    fn scroll_key(&mut self, c: char) {
        let vh = self.viewport_height();
        let max = self.max_scroll();
        let last = self.last_row();
        match c {
            'd' | 'u' => {
                let half = (vh / 2).max(1);
                let down = c == 'd';
                let row = if down {
                    (self.cursor.row + half).min(last)
                } else {
                    self.cursor.row.saturating_sub(half)
                };
                self.scroll = if down {
                    (self.scroll + half).min(max)
                } else {
                    self.scroll.saturating_sub(half)
                };
                self.move_to_row(row);
            }
            'f' => {
                let new = (self.scroll + vh).min(max);
                if new == self.scroll {
                    self.move_to_row(last);
                } else {
                    self.scroll = new;
                    self.move_to_row(self.cursor.row.max(new));
                }
            }
            'b' => {
                let new = self.scroll.saturating_sub(vh);
                if new == self.scroll {
                    self.move_to_row(0);
                } else {
                    self.scroll = new;
                    self.move_to_row(self.cursor.row.min(new + vh - 1));
                }
            }
            'e' => {
                self.scroll = (self.scroll + 1).min(max);
                if self.cursor.row < self.scroll {
                    self.move_to_row(self.scroll);
                }
            }
            'y' => {
                self.scroll = self.scroll.saturating_sub(1);
                if self.cursor.row >= self.scroll + vh {
                    self.move_to_row(self.scroll + vh - 1);
                }
            }
            _ => {}
        }
    }

    fn visible_bottom(&self) -> usize {
        (self.scroll + self.viewport_height() - 1).min(self.last_row())
    }

    fn keep_visible(&mut self) {
        let vh = self.viewport_height();
        if self.cursor.row < self.scroll {
            self.scroll = self.cursor.row;
        } else if self.cursor.row >= self.scroll + vh {
            self.scroll = self.cursor.row + 1 - vh;
        }
    }

    // ---------------------------------------------------------------------
    // Navigation

    /// The current page and position as a history entry.
    fn entry(&self) -> Option<Entry> {
        let p = self.page.as_ref()?;
        let page = match &p.path {
            Some(path) => PageRef::File(path.clone()),
            None => PageRef::Stdin(
                self.stdin
                    .clone()
                    .unwrap_or_else(|| Arc::new(p.doc.source.clone())),
            ),
        };
        Some(Entry {
            page,
            cursor_row: self.cursor.row,
            cursor_col: self.cursor.col,
            scroll: self.scroll,
        })
    }

    /// Directory relative links resolve against: the current file's
    /// directory, or the working directory for stdin.
    fn link_dir(&self) -> PathBuf {
        match self.page.as_ref().and_then(|p| p.path.as_deref()) {
            Some(path) => match path.parent() {
                Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
                _ => PathBuf::from("."),
            },
            None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }

    /// The link under the cursor, resolved, plus its destination as written.
    fn target_under_cursor(&mut self) -> Option<(Target, String)> {
        let Some(i) = self.link_under_cursor() else {
            self.set_status("No link under cursor");
            return None;
        };
        let link = self.page.as_ref()?.doc.links.get(i)?.clone();
        Some((
            nav::resolve(&link.dest, &link.kind, &self.link_dir()),
            link.dest,
        ))
    }

    /// `gd` / `Enter` / `C-]`: follow the link under the cursor.
    fn follow(&mut self) {
        let Some((target, dest)) = self.target_under_cursor() else {
            return;
        };
        match target {
            Target::External(url) => {
                (self.opener)(&url);
                self.set_status(format!("Opened {url}"));
            }
            Target::Anchor(anchor) => {
                let Some(row) = self.heading_row(&anchor) else {
                    self.set_status(format!("No heading #{anchor}"));
                    return;
                };
                if let Some(e) = self.entry() {
                    self.history.push(e);
                }
                self.jump_to_row(row);
            }
            Target::File { path, anchor } => {
                let as_written = dest.split('#').next().unwrap_or("").to_string();
                let missing = || format!("No such file: {as_written}");
                if !path.is_file() {
                    self.set_status(missing());
                    return;
                }
                if !nav::is_markdown(&path) {
                    self.pending_editor = Some(path);
                    return;
                }
                let Ok(bytes) = std::fs::read(&path) else {
                    self.set_status(missing());
                    return;
                };
                let here = self.entry();
                if !self.open_bytes(&path, &bytes) {
                    return;
                }
                if let Some(e) = here {
                    self.history.push(e);
                }
                if let Some(anchor) = anchor {
                    match self.heading_row(&anchor) {
                        Some(row) => self.jump_to_row(row),
                        None => self.set_status(format!("No heading #{anchor}")),
                    }
                }
            }
        }
    }

    /// `gx`: open the URL under the cursor externally.
    fn open_external(&mut self) {
        match self.target_under_cursor() {
            Some((Target::External(url), _)) => {
                (self.opener)(&url);
                self.set_status(format!("Opened {url}"));
            }
            Some(_) => self.set_status("Not a URL"),
            None => {}
        }
    }

    /// Rendered row of the heading with `slug` on the current page.
    fn heading_row(&self, slug: &str) -> Option<usize> {
        let p = self.page.as_ref()?;
        let h = p.doc.headings.iter().find(|h| h.slug == slug).or_else(|| {
            let lower = slug.to_lowercase();
            p.doc.headings.iter().find(|h| h.slug == lower)
        })?;
        p.rendered.srcmap.row_for(h.range.start)
    }

    /// Cursor to `row`, column 0, scrolled to the top of the view.
    fn jump_to_row(&mut self, row: usize) {
        self.goto_row(row);
        self.scroll = self.cursor.row.min(self.max_scroll());
    }

    /// `C-o` / `C-t`.
    fn back(&mut self) {
        let Some(here) = self.entry() else { return };
        let Some(e) = self.history.back(here) else {
            self.set_status("At the start of history");
            return;
        };
        if let Err(msg) = self.restore(&e) {
            self.history.forward(e);
            self.set_status(msg);
        }
    }

    /// `C-i` / `Tab`.
    fn forward(&mut self) {
        let Some(here) = self.entry() else { return };
        let Some(e) = self.history.forward(here) else {
            self.set_status("At the end of history");
            return;
        };
        if let Err(msg) = self.restore(&e) {
            self.history.back(e);
            self.set_status(msg);
        }
    }

    /// Show the entry's page with its cursor and scroll.
    fn restore(&mut self, e: &Entry) -> Result<(), String> {
        match &e.page {
            PageRef::File(path) => {
                let bytes =
                    std::fs::read(path).map_err(|_| format!("No such file: {}", path.display()))?;
                if !self.open_bytes(path, &bytes) {
                    return Err("looks binary".into());
                }
            }
            PageRef::Stdin(text) => self.open_stdin(text.clone()),
        }
        self.cursor.row = e.cursor_row.min(self.last_row());
        self.set_col(e.cursor_col);
        self.scroll = e.scroll.min(self.max_scroll());
        self.keep_visible();
        Ok(())
    }

    // ---------------------------------------------------------------------
    // Cursor motions

    fn repeat(&mut self, n: usize, f: fn(&Self, Pos) -> Pos) {
        let mut p = self.pos();
        for _ in 0..n {
            p = f(self, p);
        }
        self.set_pos(p);
    }

    /// Vertical move to `row`, keeping the desired column.
    fn move_to_row(&mut self, row: usize) {
        self.cursor.row = row.min(self.last_row());
        self.clamp_cursor();
    }

    /// Jump to `row` at column 0 (gg, G, H, M, L).
    fn goto_row(&mut self, row: usize) {
        self.cursor.row = row.min(self.last_row());
        self.set_col(0);
    }

    fn vertical(&mut self, delta: isize) {
        let row = self.cursor.row.saturating_add_signed(delta);
        self.move_to_row(row);
    }

    fn horizontal(&mut self, delta: isize) {
        let cells = self.cells(self.cursor.row);
        if cells.is_empty() {
            return;
        }
        let idx = self.cell_index(self.cursor.row, self.cursor.col);
        let new = idx
            .saturating_add_signed(delta)
            .min(cells.len().saturating_sub(1));
        self.set_col(cells[new].col);
    }

    fn set_col(&mut self, col: usize) {
        self.want_col = col;
        self.clamp_cursor();
        self.want_col = self.cursor.col;
    }

    /// Put the cursor column at the grapheme under `want_col` on its row.
    fn clamp_cursor(&mut self) {
        self.cursor.row = self.cursor.row.min(self.last_row());
        let cells = self.cells(self.cursor.row);
        self.cursor.col = match cells.iter().rposition(|c| c.col <= self.want_col) {
            Some(i) => cells[i].col,
            None => 0,
        };
    }

    fn cells(&self, row: usize) -> &[Cell] {
        self.rows.get(row).map_or(&[], Vec::as_slice)
    }

    fn cell_index(&self, row: usize, col: usize) -> usize {
        self.cells(row)
            .iter()
            .rposition(|c| c.col <= col)
            .unwrap_or(0)
    }

    fn pos(&self) -> Pos {
        Pos {
            row: self.cursor.row,
            idx: self.cell_index(self.cursor.row, self.cursor.col),
        }
    }

    fn set_pos(&mut self, p: Pos) {
        self.cursor.row = p.row.min(self.last_row());
        let col = self.cells(self.cursor.row).get(p.idx).map_or(0, |c| c.col);
        self.set_col(col);
    }

    fn class(&self, p: Pos) -> Class {
        self.cells(p.row)
            .get(p.idx)
            .map_or(Class::Empty, |c| c.class)
    }

    fn next(&self, p: Pos) -> Option<Pos> {
        if p.idx + 1 < self.cells(p.row).len() {
            Some(Pos {
                row: p.row,
                idx: p.idx + 1,
            })
        } else if p.row + 1 < self.total_rows() {
            Some(Pos {
                row: p.row + 1,
                idx: 0,
            })
        } else {
            None
        }
    }

    fn prev(&self, p: Pos) -> Option<Pos> {
        if p.idx > 0 {
            Some(Pos {
                row: p.row,
                idx: p.idx - 1,
            })
        } else if p.row > 0 {
            let row = p.row - 1;
            Some(Pos {
                row,
                idx: self.cells(row).len().saturating_sub(1),
            })
        } else {
            None
        }
    }

    /// `w`: start of the next word; an empty row counts as a word.
    fn word_forward(&self, p: Pos) -> Pos {
        let start = self.class(p);
        let mut q = p;
        if matches!(start, Class::Word | Class::Punct) {
            loop {
                let Some(n) = self.next(q) else { return q };
                let crossed = n.row != q.row;
                q = n;
                if crossed || self.class(q) != start {
                    break;
                }
            }
        } else {
            match self.next(q) {
                Some(n) => q = n,
                None => return q,
            }
        }
        while self.class(q) == Class::Blank {
            match self.next(q) {
                Some(n) => q = n,
                None => return q,
            }
        }
        q
    }

    /// `e`: end of the current or next word.
    fn word_end(&self, p: Pos) -> Pos {
        let Some(mut q) = self.next(p) else { return p };
        while matches!(self.class(q), Class::Blank | Class::Empty) {
            match self.next(q) {
                Some(n) => q = n,
                None => return q,
            }
        }
        let c = self.class(q);
        while let Some(n) = self.next(q) {
            if n.row != q.row || self.class(n) != c {
                break;
            }
            q = n;
        }
        q
    }

    /// `b`: start of the current or previous word.
    fn word_backward(&self, p: Pos) -> Pos {
        let Some(mut q) = self.prev(p) else { return p };
        while self.class(q) == Class::Blank {
            match self.prev(q) {
                Some(n) => q = n,
                None => return q,
            }
        }
        let c = self.class(q);
        if c == Class::Empty {
            return q;
        }
        while let Some(n) = self.prev(q) {
            if n.row != q.row || self.class(n) != c {
                break;
            }
            q = n;
        }
        q
    }

    fn blank_row(&self, row: usize) -> bool {
        self.cells(row).iter().all(|c| c.class == Class::Blank)
    }

    /// `}`: the next blank row after the current paragraph (or the last row).
    fn paragraph_forward(&self, p: Pos) -> Pos {
        let n = self.total_rows();
        let mut r = p.row;
        while r < n && self.blank_row(r) {
            r += 1;
        }
        while r < n && !self.blank_row(r) {
            r += 1;
        }
        Pos {
            row: r.min(self.last_row()),
            idx: 0,
        }
    }

    /// `{`: the previous blank row before the current paragraph (or row 0).
    fn paragraph_backward(&self, p: Pos) -> Pos {
        let mut r = p.row;
        while r > 0 && self.blank_row(r) {
            r -= 1;
        }
        while r > 0 && !self.blank_row(r) {
            r -= 1;
        }
        Pos { row: r, idx: 0 }
    }
}

/// Grapheme cells of one rendered line.
fn row_cells(line: &ratatui::text::Line<'_>) -> Vec<Cell> {
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    let mut col = 0;
    let mut cells = Vec::new();
    for g in text.graphemes(true) {
        let w = g.width();
        if w == 0 {
            continue;
        }
        let ch = g.chars().next().unwrap_or(' ');
        let class = if ch.is_whitespace() {
            Class::Blank
        } else if ch.is_alphanumeric() || ch == '_' {
            Class::Word
        } else {
            Class::Punct
        };
        cells.push(Cell { col, class });
        col += w;
    }
    cells
}

/// Screen column of `byte` on the first segment drawn at or after it.
fn col_for(map: &SrcMap, byte: usize) -> usize {
    let Some(s) = map.segments.iter().find(|s| s.src.end > byte) else {
        return 0;
    };
    if byte <= s.src.start {
        return s.span.col_start;
    }
    let cols = s.span.col_end - s.span.col_start;
    let len = (s.src.end - s.src.start).max(1);
    s.span.col_start + (byte - s.src.start) * cols / len
}

/// Default opener: `open` (macOS) or `xdg-open`, detached, with null stdio.
fn system_open(url: &str) {
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
fn system_edit(path: &Path) -> anyhow::Result<()> {
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

/// Run the interactive TUI until the user quits.
pub fn run(opts: StartOptions) -> anyhow::Result<()> {
    let size = crossterm::terminal::size().context("reading terminal size")?;
    let mut app = App::new(opts, size)?;
    // Installs a panic hook that restores the terminal before unwinding.
    let mut terminal = ratatui::try_init().context("initialising terminal")?;
    let result = event_loop(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, app: &mut App) -> anyhow::Result<()> {
    while !app.should_quit() {
        if app.pending_editor().is_some() {
            suspend()?;
            app.run_pending_editor();
            resume()?;
            terminal.clear()?;
        }
        terminal.draw(|f| ui::draw(f, app))?;
        if !event::poll(Duration::from_millis(250))? {
            continue;
        }
        match event::read()? {
            Event::Key(key) => app.handle_key(key),
            Event::Resize(cols, rows) => app.resize(cols, rows),
            _ => {}
        }
    }
    Ok(())
}

/// Leave raw mode and the alternate screen so a child can use the terminal.
fn suspend() -> anyhow::Result<()> {
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen)?;
    Ok(())
}

/// Undo [`suspend`].
fn resume() -> anyhow::Result<()> {
    crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen)?;
    crossterm::terminal::enable_raw_mode()?;
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
