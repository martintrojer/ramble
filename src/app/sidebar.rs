//! The sidebar (spec § Sidebar, docs/specs/2026-10-07-sidebar-layout.md):
//! visibility (`<leader>e`, narrow auto-hide), modes files / outline /
//! split (`<leader>E`), the lazily walked file tree, the outline of the
//! current page, focus moves (`C-w h/l/w/W/j/k/p`), the per-pane `/`
//! filter, and the `auto` mode switching from files to `sidebar.reading`
//! when a page is first shown.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::keys::{Action, KeyResult};
use super::sidebar_width::{self, MIN_CONTENT};
use super::{App, Mode, StartTarget};
use crate::config::{SidebarConfig, SidebarMode, SidebarShow, SidebarSide, SidebarWidth};
use crate::nav::is_markdown;

/// Status when the sidebar is meant to show but the content would get
/// fewer than `MIN_CONTENT` columns (the last-resort guard).
pub const NARROW_MESSAGE: &str = "Window too narrow for the sidebar";
/// Status for a focus move while the sidebar is hidden.
pub const HIDDEN_MESSAGE: &str = "Sidebar is hidden";
/// Status for `<leader>e` with no page: the tree is all there is.
pub const TREE_STAYS_MESSAGE: &str = "No file loaded: the file tree stays shown";
/// Status for `C-w j` without a pane below.
pub const NO_PANE_BELOW: &str = "No pane below";
/// Status for `C-w k` without a pane above.
pub const NO_PANE_ABOVE: &str = "No pane above";
/// Status for `-` when the tree root is already `/`.
pub const ROOT_TOP_MESSAGE: &str = "At the filesystem root";

/// Which pane receives keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    Content,
    Files,
    Outline,
}

/// Sidebar actions, bound in [`App::keymap`] and run by [`App::apply`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarAction {
    /// `<leader>e`: show or hide the sidebar.
    Toggle,
    /// `<leader>E`: outline → files → split → outline, and show it.
    Cycle,
    /// `C-w h` (`C-w l` with the sidebar on the right): the last sidebar
    /// pane.
    FocusSidebar,
    /// `C-w l` (`C-w h` with the sidebar on the right).
    FocusContent,
    /// `C-w w`
    FocusNext,
    /// `C-w W`
    FocusPrev,
    /// `C-w j`: the pane below (outline in split mode).
    FocusBelow,
    /// `C-w k`: the pane above (files in split mode).
    FocusAbove,
    /// `C-w p`: the previously focused pane.
    FocusLast,
    Down,
    Up,
    /// `C-d` / `C-u`: half the pane's drawn rows down / up.
    HalfDown,
    HalfUp,
    Top,
    Bottom,
    /// `h`: collapse the directory, or select the parent.
    Collapse,
    /// `l`: expand the directory.
    Expand,
    /// `-` (and `h` on a top-level row): re-root the tree at the parent
    /// of its root (D10).
    RootUp,
    /// `.` on a folder row: make that folder the tree root (D10).
    RootHere,
    /// `o` / `Enter`: open a file, toggle a directory, jump to a heading.
    Open,
    /// `/` in a sidebar pane.
    FilterStart,
    FilterInput(char),
    FilterBackspace,
    /// Enter in the filter prompt: keep the filter, select the first match.
    FilterCommit,
    /// Esc in the filter prompt: clear the filter.
    FilterCancel,
    /// Esc in a sidebar pane: clear its filter.
    FilterClear,
}

/// Sidebar state owned by [`App`].
#[derive(Debug, Clone)]
pub(super) struct Sidebar {
    /// Never `Auto`: resolved at start.
    mode: SidebarMode,
    /// `sidebar.default` is `auto`: switch to `reading` when a page shows.
    auto: bool,
    /// `sidebar.reading` as a mode (outline or split).
    reading: SidebarMode,
    /// The screen edge it is drawn at (`sidebar.side`, `:Sidebar
    /// left|right`).
    side: SidebarSide,
    /// The user picked a mode this session (`<leader>E`, `:Sidebar`),
    /// which stops the `auto` switching.
    manual: bool,
    /// The user's visibility flag (D3 step 4); starts at `sidebar.show`.
    /// With `show = "auto"` it means pinned (D11).
    shown: bool,
    /// `show = "auto"`: an unpinned peek is open (D11).
    peeked: bool,
    /// Where the open peek is drawn: over the page (true) or beside it.
    /// Decided when the peek opens and on a terminal resize (D11, P9).
    overlay: bool,
    /// A visibility choice made while the terminal was narrower than
    /// `auto_hide_below` (D3 step 2); cleared by a terminal resize.
    narrow_override: Option<bool>,
    /// `set_page` is laying out a page: count it as loaded.
    loading: bool,
    /// The auto width (D5), not counting the border; see
    /// [`App::sidebar_refit`].
    width: u16,
    focus: Focus,
    /// The focus before the last focus change, for `C-w p`.
    prev_focus: Focus,
    /// The sidebar pane `C-w h` returns to.
    last_side: Focus,
    /// Built when the files pane is first shown.
    tree: Option<Tree>,
    outline_sel: usize,
    /// First drawn row of the files / outline list: saved from each frame
    /// (`App::set_layout`) and moved by the wheel over an unfocused
    /// outline, which shows no selection to move.
    files_top: Cell<usize>,
    outline_top: Cell<usize>,
    outline_filter: Option<String>,
    /// The filter being typed (`Mode::Filter`).
    prompt: Option<String>,
}

impl Sidebar {
    pub(super) fn new(config: &SidebarConfig, target: &StartTarget) -> Sidebar {
        let reading = SidebarMode::from(config.reading);
        let mode = match (config.default, target) {
            (SidebarMode::Auto, StartTarget::Dir(_)) => SidebarMode::Files,
            (SidebarMode::Auto, StartTarget::File(_) | StartTarget::Stdin(_)) => reading,
            (m, _) => m,
        };
        Sidebar {
            mode,
            auto: config.default == SidebarMode::Auto,
            reading,
            side: config.side,
            manual: false,
            // `auto` starts hidden and unpinned (D11).
            shown: config.show == SidebarShow::Always,
            peeked: false,
            overlay: false,
            narrow_override: None,
            loading: false,
            width: config.min_width,
            focus: Focus::Content,
            prev_focus: Focus::Content,
            last_side: Focus::Files,
            tree: None,
            outline_sel: 0,
            files_top: Cell::new(0),
            outline_top: Cell::new(0),
            outline_filter: None,
            prompt: None,
        }
    }

    /// The panes the mode shows, top to bottom. With no page the files
    /// pane is always among them (an outline alone becomes files).
    fn panes(&self, no_page: bool) -> &'static [Focus] {
        match self.mode {
            SidebarMode::Outline if no_page => &[Focus::Files],
            SidebarMode::Files => &[Focus::Files],
            SidebarMode::Outline => &[Focus::Outline],
            SidebarMode::Split => &[Focus::Files, Focus::Outline],
            SidebarMode::Auto => &[],
        }
    }

    /// The next mode for `<leader>E`.
    fn next_mode(&self) -> SidebarMode {
        match self.mode {
            SidebarMode::Outline => SidebarMode::Files,
            SidebarMode::Files => SidebarMode::Split,
            SidebarMode::Split | SidebarMode::Auto => SidebarMode::Outline,
        }
    }
}

/// One row of the file tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeItem {
    pub path: PathBuf,
    pub name: String,
    /// 0 for entries directly under the root.
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
    pub markdown: bool,
}

/// One heading of the outline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineItem {
    pub level: u8,
    pub text: String,
    /// Rendered row of the heading.
    pub row: usize,
    /// The heading containing the cursor (drawn with `◂`).
    pub current: bool,
}

#[derive(Debug, Clone)]
struct Node {
    path: PathBuf,
    name: String,
    is_dir: bool,
}

/// The file tree under a root. Directories are read one level at a time,
/// when expanded, so a root of `$HOME` costs one directory read at start.
///
/// The root is a browsing view (D10): `-` and `.` move it, while
/// `App::tree_root` (review discovery, notebook root, `yF`) never moves.
#[derive(Debug, Clone)]
pub struct Tree {
    root: PathBuf,
    show_all: bool,
    /// Walked directories and their sorted children.
    children: HashMap<PathBuf, Vec<Node>>,
    expanded: HashSet<PathBuf>,
    selected: Option<PathBuf>,
    /// The followed file when it lies outside the root.
    outside: Option<PathBuf>,
    /// The deepest root left with `-` (D10). It and its ancestors are
    /// listed even when the walk would skip them (dotfiles, ignored,
    /// no markdown), so you can see where you came from.
    origin: Option<PathBuf>,
    filter: Option<String>,
    /// Entries the walker has yielded below a walked dir, at any depth.
    entries_read: usize,
}

fn canonical(p: &Path) -> PathBuf {
    p.canonicalize()
        .or_else(|_| std::path::absolute(p))
        .unwrap_or_else(|_| p.to_path_buf())
}

/// Folders first, then by name, case-insensitively.
fn sort_nodes(nodes: &mut [Node]) {
    nodes.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

fn matches(name: &str, filter: Option<&str>) -> bool {
    filter.is_none_or(|f| name.to_lowercase().contains(&f.to_lowercase()))
}

impl Tree {
    /// A tree rooted at `root`, with the root's own entries read.
    /// `show_all` lists non-markdown files too (dimmed when drawn).
    pub fn new(root: &Path, show_all: bool) -> Tree {
        let root = canonical(root);
        let mut tree = Tree {
            root: root.clone(),
            show_all,
            children: HashMap::new(),
            expanded: HashSet::new(),
            selected: None,
            outside: None,
            origin: None,
            filter: None,
            entries_read: 0,
        };
        tree.walk(&root);
        tree
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Re-root the tree at `root` (D10). Expansion is kept (absolute
    /// paths), the filter and selection are cleared. An
    /// [`Tree::outside`] file that the new root covers is shown and
    /// selected. Moving the root away from the followed file does not make
    /// it outside: the title keeps showing the root you browse.
    pub fn set_root(&mut self, root: &Path) {
        self.root = canonical(root);
        self.walk(&self.root.clone());
        self.filter = None;
        self.selected = None;
        if let Some(f) = self.outside.clone()
            && f.starts_with(&self.root)
        {
            self.reveal(&f);
        }
    }

    /// Re-root at the parent of the root, expanding and selecting the old
    /// root (unless a followed file just came into view, which is
    /// selected instead). False at `/`.
    pub fn root_up(&mut self) -> bool {
        let Some(parent) = self.root.parent().map(Path::to_path_buf) else {
            return false;
        };
        let old = self.root.clone();
        let was_outside = self.outside.is_some();
        if !self.origin.as_ref().is_some_and(|o| o.starts_with(&old)) {
            self.origin = Some(old.clone());
        }
        self.set_root(&parent);
        self.expand(&old);
        if !was_outside || self.outside.is_some() {
            self.selected = Some(old);
        }
        true
    }

    /// How many directory entries have been read so far; shows that a
    /// walk reads one level only.
    pub fn entries_read(&self) -> usize {
        self.entries_read
    }

    /// Read one level of `dir`, respecting ignore files (including those
    /// in parent directories) and hiding dotfiles.
    fn walk(&mut self, dir: &Path) {
        if self.children.contains_key(dir) {
            return;
        }
        let mut nodes: Vec<Node> = ignore::WalkBuilder::new(dir)
            .max_depth(Some(1))
            .require_git(false)
            .hidden(true)
            .git_global(false)
            .build()
            .flatten()
            .filter(|e| e.depth() >= 1)
            .inspect(|_| self.entries_read += 1)
            .filter(|e| e.depth() == 1)
            .map(|e| Node {
                path: e.path().to_path_buf(),
                name: e.file_name().to_string_lossy().into_owned(),
                is_dir: e.path().is_dir(),
            })
            .collect();
        sort_nodes(&mut nodes);
        self.children.insert(dir.to_path_buf(), nodes);
    }

    /// The children of `dir` to list: the walked ones, plus the next
    /// directory down towards [`Tree::origin`] when the walk skipped it.
    fn listed(&self, dir: &Path) -> Option<std::borrow::Cow<'_, [Node]>> {
        let nodes = self.children.get(dir)?;
        let pinned = self
            .origin
            .as_deref()
            .and_then(|o| o.ancestors().find(|a| a.parent() == Some(dir)))
            .filter(|p| p.is_dir() && !nodes.iter().any(|n| n.path == *p));
        let Some(p) = pinned else {
            return Some(std::borrow::Cow::Borrowed(nodes));
        };
        let mut nodes = nodes.clone();
        nodes.push(Node {
            path: p.to_path_buf(),
            name: p
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            is_dir: true,
        });
        sort_nodes(&mut nodes);
        Some(std::borrow::Cow::Owned(nodes))
    }

    /// `path` is the origin or one of its ancestors (D10).
    fn on_origin_path(&self, path: &Path) -> bool {
        self.origin.as_ref().is_some_and(|o| o.starts_with(path))
    }

    pub fn expand(&mut self, dir: &Path) {
        self.walk(dir);
        self.expanded.insert(dir.to_path_buf());
    }

    /// Re-read the root and every directory walked so far (expanded or
    /// not), keeping expansion state, the filter, `outside` and the
    /// selection. Vanished entries and directories drop out; a vanished
    /// selection moves to its nearest surviving ancestor, else row 0.
    /// [`Tree::entries_read`] keeps counting across refreshes.
    pub fn refresh(&mut self) {
        let walked: Vec<PathBuf> = self.children.drain().map(|(d, _)| d).collect();
        self.expanded.retain(|d| d.is_dir());
        for dir in walked {
            if dir == self.root || dir.is_dir() {
                self.walk(&dir);
            }
        }
        let had = self.selected.is_some();
        let mut sel = self.selected.take();
        while let Some(p) = &sel
            && !p.exists()
        {
            sel = p
                .parent()
                .filter(|d| d.starts_with(&self.root) && *d != self.root)
                .map(Path::to_path_buf);
        }
        self.selected = sel;
        if had && self.selected.is_none() {
            self.select_index(0);
        }
    }

    /// The expansion and selection, for [`Tree::restore_view`].
    pub(super) fn view(&self) -> (HashSet<PathBuf>, Option<PathBuf>) {
        (self.expanded.clone(), self.selected.clone())
    }

    /// Undo a [`Tree::reveal`] that should not stick (a reload).
    pub(super) fn restore_view(
        &mut self,
        (expanded, selected): (HashSet<PathBuf>, Option<PathBuf>),
    ) {
        self.expanded = expanded;
        self.selected = selected;
    }

    pub fn collapse(&mut self, dir: &Path) {
        self.expanded.remove(dir);
    }

    pub fn is_expanded(&self, dir: &Path) -> bool {
        self.expanded.contains(dir)
    }

    /// Follow `file`: expand its ancestors and select it. A file outside
    /// the root leaves no selection and is remembered for the title.
    pub fn reveal(&mut self, file: &Path) {
        let file = canonical(file);
        let Ok(rel) = file.strip_prefix(&self.root) else {
            self.selected = None;
            self.outside = Some(file);
            return;
        };
        let mut dir = self.root.clone();
        if let Some(parent) = rel.parent() {
            for c in parent.components() {
                dir.push(c);
                self.expand(&dir.clone());
            }
        }
        self.selected = Some(file);
        self.outside = None;
    }

    /// The followed file when it is outside the root.
    pub fn outside(&self) -> Option<&Path> {
        self.outside.as_deref()
    }

    pub fn selected(&self) -> Option<&Path> {
        self.selected.as_deref()
    }

    pub fn select(&mut self, path: Option<PathBuf>) {
        self.selected = path;
    }

    pub fn filter(&self) -> Option<&str> {
        self.filter.as_deref()
    }

    pub fn set_filter(&mut self, filter: Option<String>) {
        self.filter = filter;
    }

    /// True unless `dir` was read and holds no markdown at any read depth.
    /// A directory not read yet might hold some, so it counts.
    fn has_markdown(&self, dir: &Path) -> bool {
        let Some(nodes) = self.children.get(dir) else {
            return true;
        };
        nodes.iter().any(|n| {
            if n.is_dir {
                self.has_markdown(&n.path)
            } else {
                is_markdown(&n.path)
            }
        })
    }

    /// The rows to draw: expanded directories' children in order, minus
    /// hidden entries, minus rows not matching the filter.
    pub fn visible_items(&self) -> Vec<TreeItem> {
        let mut out = Vec::new();
        self.push_items(&self.root, 0, &mut out);
        out
    }

    fn push_items(&self, dir: &Path, depth: usize, out: &mut Vec<TreeItem>) {
        let Some(nodes) = self.listed(dir) else {
            return;
        };
        for n in nodes.iter() {
            let markdown = !n.is_dir && is_markdown(&n.path);
            let shown = self.show_all
                || self.on_origin_path(&n.path)
                || if n.is_dir {
                    self.has_markdown(&n.path)
                } else {
                    markdown
                };
            if !shown {
                continue;
            }
            let expanded = n.is_dir && self.expanded.contains(&n.path);
            if matches(&n.name, self.filter.as_deref()) {
                out.push(TreeItem {
                    path: n.path.clone(),
                    name: n.name.clone(),
                    depth,
                    is_dir: n.is_dir,
                    expanded,
                    markdown,
                });
            }
            if expanded {
                self.push_items(&n.path, depth + 1, out);
            }
        }
    }

    /// Index of the selection among `items`.
    pub fn selected_index(&self, items: &[TreeItem]) -> Option<usize> {
        let sel = self.selected.as_deref()?;
        items.iter().position(|i| i.path == sel)
    }

    /// Move the selection by `delta` rows (from the top when none).
    fn move_by(&mut self, delta: isize) {
        let items = self.visible_items();
        if items.is_empty() {
            return;
        }
        let i = match self.selected_index(&items) {
            Some(i) => i.saturating_add_signed(delta).min(items.len() - 1),
            None if delta < 0 => items.len() - 1,
            None => 0,
        };
        self.selected = Some(items[i].path.clone());
    }

    fn select_index(&mut self, i: usize) {
        let items = self.visible_items();
        self.selected = items
            .get(i.min(items.len().saturating_sub(1)))
            .map(|i| i.path.clone());
    }
}

fn is_ctrl(key: &KeyEvent, c: char) -> bool {
    key.code == KeyCode::Char(c) && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn plain(key: &KeyEvent) -> Option<char> {
    match key.code {
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => Some(c),
        _ => None,
    }
}

fn act(a: SidebarAction) -> KeyResult {
    KeyResult::Action(Action::Sidebar(a))
}

/// `C-w h l w W j k p` (the second key with or without Ctrl). `h` and `l`
/// follow the screen: the one pointing at the sidebar's `side` focuses
/// it, the other the content (D9).
pub(super) fn window_keymap(keys: &[KeyEvent], side: SidebarSide) -> Option<KeyResult> {
    let (first, rest) = keys.split_first()?;
    if !is_ctrl(first, 'w') {
        return None;
    }
    let Some(k) = rest.first() else {
        return Some(KeyResult::Pending);
    };
    let (left, right) = match side {
        SidebarSide::Left => (SidebarAction::FocusSidebar, SidebarAction::FocusContent),
        SidebarSide::Right => (SidebarAction::FocusContent, SidebarAction::FocusSidebar),
    };
    Some(match k.code {
        KeyCode::Char('h') | KeyCode::Left => act(left),
        KeyCode::Char('l') | KeyCode::Right => act(right),
        KeyCode::Char('w') => act(SidebarAction::FocusNext),
        KeyCode::Char('W') => act(SidebarAction::FocusPrev),
        KeyCode::Char('j') | KeyCode::Down => act(SidebarAction::FocusBelow),
        KeyCode::Char('k') | KeyCode::Up => act(SidebarAction::FocusAbove),
        KeyCode::Char('p') => act(SidebarAction::FocusLast),
        _ => KeyResult::None,
    })
}

/// Keys in the filter prompt.
pub(super) fn filter_keymap(keys: &[KeyEvent]) -> KeyResult {
    let Some(key) = keys.last() else {
        return KeyResult::None;
    };
    match key.code {
        KeyCode::Enter => act(SidebarAction::FilterCommit),
        KeyCode::Esc => act(SidebarAction::FilterCancel),
        KeyCode::Backspace => act(SidebarAction::FilterBackspace),
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            act(SidebarAction::FilterInput(c))
        }
        _ => KeyResult::None,
    }
}

impl App {
    /// Keys while a sidebar pane has focus.
    pub(super) fn sidebar_keymap(&self, keys: &[KeyEvent]) -> KeyResult {
        use SidebarAction as S;
        if let Some(r) = self.leader_keymap(keys) {
            return r;
        }
        if let Some(r) = window_keymap(keys, self.sidebar_side()) {
            return r;
        }
        match keys {
            [a, b] if plain(a) == Some('g') && plain(b) == Some('g') => act(S::Top),
            [a, b] if plain(a) == Some('g') && plain(b) == Some('?') => {
                KeyResult::Action(Action::Help(super::HelpAction::Open))
            }
            [k] if is_ctrl(k, 'd') => act(S::HalfDown),
            [k] if is_ctrl(k, 'u') => act(S::HalfUp),
            [k] => match (k.code, plain(k)) {
                (_, Some('j')) | (KeyCode::Down, _) => act(S::Down),
                (_, Some('k')) | (KeyCode::Up, _) => act(S::Up),
                (_, Some('g')) => KeyResult::Pending,
                (_, Some('G')) => act(S::Bottom),
                (_, Some('h')) | (KeyCode::Left, _) => act(S::Collapse),
                (_, Some('l')) | (KeyCode::Right, _) => act(S::Expand),
                (_, Some('o')) | (KeyCode::Enter, _) => act(S::Open),
                (_, Some('-')) => act(S::RootUp),
                (_, Some('.')) => act(S::RootHere),
                (_, Some('/')) => act(S::FilterStart),
                (_, Some(':')) => KeyResult::Action(Action::Cmd(super::CmdAction::Start)),
                (KeyCode::Esc, _) => act(S::FilterClear),
                (_, Some('q')) => KeyResult::Action(Action::Quit),
                _ => KeyResult::None,
            },
            _ => KeyResult::None,
        }
    }

    /// The screen edge the sidebar is drawn at.
    pub fn sidebar_side(&self) -> SidebarSide {
        self.sidebar.side
    }

    /// `:Sidebar left|right`: move the sidebar to that edge. Width,
    /// visibility and focus are unchanged.
    pub(super) fn set_sidebar_side(&mut self, side: SidebarSide) {
        self.sidebar.side = side;
    }

    /// The resolved sidebar mode (never `Auto`).
    pub fn sidebar_mode(&self) -> SidebarMode {
        self.sidebar.mode
    }

    /// Switch the sidebar mode (`Auto` means files without a page, else
    /// `sidebar.reading`) and re-lay out the content for the new width.
    /// Does not count as a manual pick and never shows or hides the
    /// sidebar (history restore calls it).
    pub fn set_sidebar_mode(&mut self, mode: SidebarMode) {
        self.sidebar.mode = match mode {
            SidebarMode::Auto if self.page.is_none() => SidebarMode::Files,
            SidebarMode::Auto => self.sidebar.reading,
            m => m,
        };
        self.sync_tree();
        self.sidebar_refit(false);
        let (cols, rows) = self.size;
        self.resize(cols, rows);
        self.sidebar_fit();
    }

    /// A mode picked by the user (`<leader>E`, `:Sidebar`): stops the
    /// `auto` switching for the rest of the session, and shows the sidebar.
    /// With `show = "auto"` it opens a peek (not a pin) and focuses the
    /// first pane (D11, P7).
    pub(super) fn pick_sidebar_mode(&mut self, mode: SidebarMode) {
        self.sidebar.manual = true;
        self.sidebar.mode = mode;
        if !self.show_is_auto() {
            self.show_sidebar(true);
            return;
        }
        if self.sidebar_visible() {
            self.sync_tree();
            self.sidebar_relayout(false);
            self.sidebar_fit();
        } else {
            self.sidebar_peek_open();
        }
        if self.can_focus_sidebar() {
            self.focus_pane(self.sidebar_panes()[0]);
        }
    }

    /// `sidebar.show` is `auto` (peek, D11).
    pub(crate) fn show_is_auto(&self) -> bool {
        self.config.sidebar.show == SidebarShow::Auto
    }

    /// In `auto`, the sidebar is hidden (unpinned, no peek) while a page is
    /// shown, so a focus move into it would open a peek.
    pub(crate) fn peek_could_open(&self) -> bool {
        self.show_is_auto() && !self.sidebar_visible() && !self.sidebar_panes().is_empty()
    }

    /// Focus can move into the sidebar now, or a peek would open for it:
    /// the `avail` of the `C-w` help rows (P3).
    pub(crate) fn can_focus_or_peek(&self) -> bool {
        self.can_focus_sidebar() || self.peek_could_open()
    }

    /// Set the `auto` pin and peek, then lay out again: the page is
    /// re-laid out only when the columns beside it changed (an overlay
    /// never resizes it, P10).
    fn set_auto_visibility(&mut self, pinned: bool, peeked: bool) {
        let before = self.sidebar_cols();
        let opening = peeked && !self.sidebar.peeked && !pinned;
        self.sidebar.shown = pinned;
        self.sidebar.peeked = peeked && !pinned;
        self.sync_tree();
        self.sidebar_refit(false);
        if opening {
            self.sidebar.overlay = !self.peek_fits_beside();
        }
        if self.sidebar_cols() != before {
            let (cols, rows) = self.size;
            self.resize(cols, rows);
        }
        self.sidebar_fit();
    }

    /// A peek goes beside the page when the page keeps its full
    /// `render.max_width` next to it: `w + 1 <= cols - review gutter -
    /// max_width` (P9). A narrow terminal overlays regardless; that is
    /// [`App::sidebar_overlay`].
    fn peek_fits_beside(&self) -> bool {
        let room = self
            .size
            .0
            .saturating_sub(self.review_gutter())
            .saturating_sub(self.config.render.max_width);
        self.sidebar_width_cols() < room
    }

    /// Open an unpinned peek (D11): `auto` only, while hidden.
    fn sidebar_peek_open(&mut self) {
        if self.peek_could_open() {
            self.set_auto_visibility(false, true);
        }
    }

    /// Close an unpinned peek (D11). Focus is left to the caller.
    fn sidebar_peek_close(&mut self) {
        if self.show_is_auto() && self.sidebar.peeked {
            self.set_auto_visibility(self.sidebar.shown, false);
        }
    }

    /// The user's focus went back to the content: close a peek (D11).
    fn focus_returned(&mut self) {
        if self.sidebar.focus == Focus::Content {
            self.sidebar_peek_close();
        }
    }

    /// A visibility choice by the user (`<leader>e`, `<leader>E`,
    /// `:Sidebar`). While the terminal is narrow it holds until the next
    /// resize (D3 step 2).
    /// With `show = "auto"` showing pins and hiding unpins (D11); the
    /// narrow override is never set there.
    pub(super) fn show_sidebar(&mut self, show: bool) {
        if self.show_is_auto() {
            self.set_auto_visibility(show, false);
            return;
        }
        self.sidebar.shown = show;
        if self.narrow() {
            self.sidebar.narrow_override = Some(show);
        }
        self.sync_tree();
        self.sidebar_refit(false);
        let (cols, rows) = self.size;
        self.resize(cols, rows);
        self.sidebar_fit();
    }

    /// A terminal resize from the event loop: a change of width ends the
    /// step-2 override; the width is refitted, then the page laid out.
    pub(super) fn sidebar_terminal_resized(&mut self, cols: u16, rows: u16) {
        if cols != self.size.0 {
            self.sidebar.narrow_override = None;
        }
        self.size = (cols, rows);
        self.sidebar_refit(false);
        if self.show_is_auto() && self.sidebar.peeked {
            self.sidebar.overlay = !self.peek_fits_beside();
        }
        self.resize(cols, rows);
        self.sidebar_fit();
    }

    /// Recompute the auto width (D5) for the rows shown now. `widen_only`
    /// (a folder expanded, a filter changed) never narrows it, so the
    /// sidebar doesn't shift while you browse; it narrows at the next page
    /// change. True when the width changed; the caller re-lays out.
    pub(super) fn sidebar_refit(&mut self, widen_only: bool) -> bool {
        let c = &self.config.sidebar;
        if c.width != SidebarWidth::Auto {
            return false;
        }
        let need = self.sidebar_need();
        let w = sidebar_width::sidebar_width(
            &need,
            self.size.0,
            self.config.render.max_width,
            c.min_width,
            c.max_width,
        );
        let w = if widen_only {
            w.max(self.sidebar.width)
        } else {
            w
        };
        std::mem::replace(&mut self.sidebar.width, w) != w
    }

    /// [`App::sidebar_refit`], then re-lay out the page through `resize`
    /// (srcmap, hints, review gutter) when the width changed.
    pub(super) fn sidebar_relayout(&mut self, widen_only: bool) {
        if self.sidebar_refit(widen_only) && self.sidebar_cols() > 0 {
            let (cols, rows) = self.size;
            self.resize(cols, rows);
        }
        self.sidebar_fit_focus();
    }

    /// The columns each row of the shown panes needs, titles included,
    /// measured as drawn (D5 inputs).
    fn sidebar_need(&self) -> Vec<u16> {
        let mut need = Vec::new();
        for pane in self.sidebar_panes() {
            match pane {
                Focus::Files => {
                    let Some(t) = self.tree() else { continue };
                    // The root in the title is clipped from the left to
                    // fit (D10), so only `Files` counts, not the path.
                    let title = match t.outside() {
                        Some(_) => self.sidebar_title(),
                        None => FILES_TITLE.into(),
                    };
                    need.push(sidebar_width::cols(&files_title(&title, t.filter())));
                    for i in t.visible_items() {
                        let mark = match self.file_marker(&i.path) {
                            Some((c, Some(n))) => format!(" {c} {n}"),
                            Some((c, None)) => format!(" {c}"),
                            None => String::new(),
                        };
                        need.push(sidebar_width::tree_row_need(i.depth, &i.name, &mark));
                    }
                }
                Focus::Outline => {
                    need.push(sidebar_width::cols(OUTLINE_TITLE));
                    if let Some(p) = &self.page {
                        need.extend(
                            p.doc
                                .headings
                                .iter()
                                .map(|h| sidebar_width::outline_row_need(h.level, &h.text)),
                        );
                    }
                }
                Focus::Content => {}
            }
        }
        need
    }

    /// The terminal is narrower than `sidebar.auto_hide_below`.
    fn narrow(&self) -> bool {
        let below = self.config.sidebar.auto_hide_below;
        below > 0 && self.size.0 < below
    }

    /// No page is loaded or being laid out (D3 step 1).
    fn no_page(&self) -> bool {
        self.page.is_none() && !self.sidebar.loading
    }

    /// Whether the sidebar should be drawn, in the order of D3: no page
    /// (always), a choice made while narrow, narrow auto-hide, then the
    /// user's flag. The `MIN_CONTENT` guard is applied by
    /// [`App::sidebar_cols`], not here.
    ///
    /// With `show = "auto"`: no page, pinned, or peeked (D11, P11).
    pub fn sidebar_visible(&self) -> bool {
        if self.no_page() {
            return true;
        }
        if self.show_is_auto() {
            return self.sidebar.shown || self.sidebar.peeked;
        }
        if let Some(v) = self.sidebar.narrow_override {
            return v;
        }
        !self.narrow() && self.sidebar.shown
    }

    /// The panes shown when the sidebar is visible, top to bottom.
    pub fn sidebar_panes(&self) -> &'static [Focus] {
        self.sidebar.panes(self.no_page())
    }

    /// Called by `set_page` before the page is laid out; returns whether
    /// this is the first page (none was loaded).
    /// With `show = "auto"` the first page hides the sidebar here, before
    /// the first render: `loading` ends the no-page rule, and with no pin
    /// and no peek (none can open while the tree is all there is) the page
    /// is laid out at its reading width (D11, P6).
    pub(super) fn sidebar_page_loading(&mut self) -> bool {
        self.sidebar.loading = true;
        self.page.is_none()
    }

    /// Called by `set_page` once the page is set. The first page moves
    /// focus to the content.
    pub(super) fn sidebar_page_loaded(&mut self, first: bool) {
        self.sidebar.loading = false;
        self.sidebar.outline_top.set(0);
        if first {
            self.set_focus(Focus::Content);
        }
        self.sidebar_relayout(false);
    }

    /// Start with no page: focus the files pane, first row selected.
    pub(super) fn sidebar_focus_tree(&mut self) {
        if self.no_page()
            && self.can_focus_sidebar()
            && self.sidebar_panes().contains(&Focus::Files)
        {
            self.focus_pane(Focus::Files);
        }
    }

    /// Called by `set_page` before the page is laid out: with
    /// `sidebar.default = "auto"` and no manual pick, show
    /// `sidebar.reading`. No re-layout (the caller renders next) and no
    /// narrow-window status.
    pub(super) fn sidebar_auto_reading(&mut self) {
        let s = &self.sidebar;
        if !s.auto || s.manual || s.mode == s.reading {
            return;
        }
        self.sidebar.mode = self.sidebar.reading;
        self.sidebar_fit_focus();
    }

    /// Which pane receives keys.
    pub fn focus(&self) -> Focus {
        self.sidebar.focus
    }

    /// Columns the sidebar takes, its border included; 0 when it is hidden
    /// or does not fit. The width is the auto width (D5) or the fixed
    /// `sidebar.width`, cut so the page keeps `MIN_CONTENT` columns; when
    /// that leaves less than `min_width` (or the fixed width, if smaller)
    /// the sidebar is dropped. With no page the tree is all there is to
    /// show, so it skips the guard and only clamps to the terminal.
    ///
    /// These are the layout columns: 0 for an overlay (D11), which is drawn
    /// over the page; [`App::sidebar_drawn_cols`] is what is drawn.
    pub fn sidebar_cols(&self) -> u16 {
        if !self.sidebar_visible() || self.sidebar_panes().is_empty() || self.sidebar_overlay() {
            return 0;
        }
        let cols = self.size.0;
        let c = &self.config.sidebar;
        let w = self.sidebar_width_cols();
        if self.no_page() {
            return w.min(cols.saturating_sub(1)) + 1;
        }
        let fit = w.min(cols.saturating_sub(1 + MIN_CONTENT));
        if fit == 0 || fit < c.min_width.min(w) {
            0
        } else {
            fit + 1
        }
    }

    /// The sidebar width before any guard: the auto width or the fixed
    /// `sidebar.width`, not counting the border.
    fn sidebar_width_cols(&self) -> u16 {
        match self.config.sidebar.width {
            SidebarWidth::Auto => self.sidebar.width,
            SidebarWidth::Fixed(n) => n,
        }
    }

    /// The sidebar is drawn over the page instead of beside it (D11): a
    /// peek without spare room, or anything shown in `auto` while the
    /// terminal is narrow. Never with no page.
    pub fn sidebar_overlay(&self) -> bool {
        self.show_is_auto()
            && !self.no_page()
            && self.sidebar_visible()
            && (self.narrow() || (self.sidebar.peeked && self.sidebar.overlay))
    }

    /// Columns the sidebar is drawn in, border included, beside the page
    /// or over it. An overlay is `min(width, cols - 1)` plus the border;
    /// neither `MIN_CONTENT` nor the `min_width` drop applies (P11).
    pub fn sidebar_drawn_cols(&self) -> u16 {
        if !self.sidebar_overlay() || self.sidebar_panes().is_empty() {
            return self.sidebar_cols();
        }
        match self.size.0 {
            0 => 0,
            cols => self.sidebar_width_cols().min(cols - 1) + 1,
        }
    }

    /// A sidebar pane is drawn, so focus can move to it.
    pub(crate) fn can_focus_sidebar(&self) -> bool {
        self.sidebar_drawn_cols() > 0
    }

    /// Share of the sidebar height given to files in split mode.
    pub fn sidebar_split_ratio(&self) -> f32 {
        self.config.sidebar.split_ratio.clamp(0.0, 1.0)
    }

    /// The file tree, once the files pane has been shown.
    pub fn tree(&self) -> Option<&Tree> {
        self.sidebar.tree.as_ref()
    }

    /// Headings of the current page matching the outline filter.
    pub fn outline(&self) -> Vec<OutlineItem> {
        let Some(p) = &self.page else {
            return Vec::new();
        };
        let rows: Vec<Option<usize>> = p
            .doc
            .headings
            .iter()
            .map(|h| p.rendered.srcmap.row_for(h.range.start))
            .collect();
        let current = rows
            .iter()
            .rposition(|r| r.is_some_and(|r| r <= self.cursor.row));
        p.doc
            .headings
            .iter()
            .zip(&rows)
            .enumerate()
            .filter_map(|(i, (h, row))| {
                Some(OutlineItem {
                    level: h.level,
                    text: h.text.clone(),
                    row: (*row)?,
                    current: current == Some(i),
                })
            })
            .filter(|o| matches(&o.text, self.sidebar.outline_filter.as_deref()))
            .collect()
    }

    /// Selected row of the outline pane (while it has focus).
    pub fn outline_selected(&self) -> Option<usize> {
        (self.sidebar.focus == Focus::Outline && !self.outline().is_empty())
            .then_some(self.sidebar.outline_sel)
    }

    /// Title of the files pane: `Files <root>` (D10), or the current
    /// file's path when it lies outside the tree root.
    pub fn sidebar_title(&self) -> String {
        match (
            self.tree().and_then(Tree::outside),
            self.sidebar_root_label(),
        ) {
            (Some(p), _) => p.display().to_string(),
            (None, Some(root)) => format!("{FILES_TITLE} {root}"),
            (None, None) => FILES_TITLE.into(),
        }
    }

    /// The tree root as shown in the title, with `~` for `$HOME`.
    pub fn sidebar_root_label(&self) -> Option<String> {
        let root = self.tree()?.root();
        let home = (self.env)("HOME")
            .filter(|h| !h.is_empty())
            .map(|h| canonical(Path::new(&h)));
        let rel = home.as_deref().and_then(|h| root.strip_prefix(h).ok());
        Some(match rel {
            Some(r) if r.as_os_str().is_empty() => "~".into(),
            Some(r) => format!("~/{}", r.display()),
            None => root.display().to_string(),
        })
    }

    /// The current page's file, canonical like the tree's paths: the
    /// files pane marks its row (D8).
    pub fn sidebar_current_file(&self) -> Option<PathBuf> {
        self.page.as_ref()?.path.as_deref().map(canonical)
    }

    /// The files pane title as drawn: [`App::sidebar_title`] and the
    /// filter.
    pub fn sidebar_files_title(&self) -> String {
        files_title(&self.sidebar_title(), self.tree().and_then(Tree::filter))
    }

    /// The filter prompt (`/text`) while typing one.
    pub fn filter_prompt(&self) -> Option<String> {
        self.sidebar.prompt.as_ref().map(|p| format!("/{p}"))
    }

    /// Build the tree if the files pane is shown, and follow the current
    /// file in it.
    pub(super) fn sync_tree(&mut self) {
        if !self.sidebar_panes().contains(&Focus::Files) {
            return;
        }
        let tree = self
            .sidebar
            .tree
            .get_or_insert_with(|| Tree::new(&self.tree_root, self.config.sidebar.show_all));
        if let Some(path) = self.page.as_ref().and_then(|p| p.path.as_deref()) {
            tree.reveal(path);
        }
    }

    /// The file tree, for the refresh in `watch.rs`.
    pub(super) fn tree_mut(&mut self) -> Option<&mut Tree> {
        self.sidebar.tree.as_mut()
    }

    /// After a mode change or resize: give focus back to the content when
    /// its pane is gone, and say so when the sidebar should show but
    /// leaves the content fewer than `MIN_CONTENT` columns. Auto-hide is
    /// silent.
    pub(super) fn sidebar_fit(&mut self) {
        self.sidebar_fit_focus();
        if self.sidebar_drawn_cols() == 0
            && self.sidebar_visible()
            && !self.sidebar_panes().is_empty()
        {
            self.set_status(NARROW_MESSAGE);
        }
    }

    /// Give focus back to the content when its pane is gone or the
    /// sidebar does not fit.
    fn sidebar_fit_focus(&mut self) {
        let shown = self.can_focus_sidebar();
        if !shown || !self.sidebar_panes().contains(&self.sidebar.focus) {
            self.set_focus(Focus::Content);
        }
        if self.sidebar.focus == Focus::Content && self.mode == Mode::Filter {
            self.sidebar.prompt = None;
            self.mode = Mode::Normal;
        }
    }

    /// Change focus, remembering the old one for `C-w p`.
    fn set_focus(&mut self, f: Focus) {
        if f != self.sidebar.focus {
            self.sidebar.prev_focus = self.sidebar.focus;
            self.sidebar.focus = f;
        }
    }

    fn focus_pane(&mut self, f: Focus) {
        self.set_focus(f);
        if f == Focus::Content {
            return;
        }
        self.sidebar.last_side = f;
        if f == Focus::Outline {
            // Always the cursor's heading; the draw keeps the list where it
            // is when that row is shown, else scrolls the minimum to it.
            self.sidebar.outline_sel = self.outline().iter().position(|o| o.current).unwrap_or(0);
        }
        if f == Focus::Files
            && let Some(t) = &mut self.sidebar.tree
            && t.selected_index(&t.visible_items()).is_none()
        {
            t.select_index(0);
        }
    }

    /// Run a sidebar action.
    pub(super) fn sidebar_action(&mut self, a: SidebarAction) {
        use SidebarAction as S;
        match a {
            S::Toggle if self.no_page() => self.set_status(TREE_STAYS_MESSAGE),
            // In `auto` a visible unpinned peek hides and stays unpinned
            // (P8); a hidden sidebar is pinned (`show_sidebar`).
            S::Toggle => self.show_sidebar(!self.sidebar_visible()),
            S::Cycle => self.pick_sidebar_mode(self.sidebar.next_mode()),
            S::FocusSidebar
            | S::FocusNext
            | S::FocusPrev
            | S::FocusBelow
            | S::FocusAbove
            | S::FocusLast
                if self.peek_could_open() && self.focus_enters_sidebar(a) =>
            {
                self.sidebar_peek_open();
                if self.can_focus_sidebar() {
                    self.sidebar_action(a);
                }
            }
            S::FocusSidebar
            | S::FocusNext
            | S::FocusPrev
            | S::FocusBelow
            | S::FocusAbove
            | S::FocusLast
                if !self.can_focus_sidebar() =>
            {
                let hidden = !self.sidebar_visible();
                self.set_status(if hidden {
                    HIDDEN_MESSAGE
                } else {
                    NARROW_MESSAGE
                });
            }
            S::FocusSidebar => {
                let panes = self.sidebar_panes();
                let f = if panes.contains(&self.sidebar.last_side) {
                    self.sidebar.last_side
                } else {
                    panes[0]
                };
                self.focus_pane(f);
            }
            S::FocusContent => {
                self.focus_pane(Focus::Content);
                self.focus_returned();
            }
            S::FocusNext => {
                let mut order = self.sidebar_panes().to_vec();
                order.push(Focus::Content);
                let i = order.iter().position(|&f| f == self.sidebar.focus);
                let next = order[i.map_or(0, |i| (i + 1) % order.len())];
                self.focus_pane(next);
                self.focus_returned();
            }
            S::FocusPrev => {
                let mut order = self.sidebar_panes().to_vec();
                order.push(Focus::Content);
                let n = order.len();
                let i = order.iter().position(|&f| f == self.sidebar.focus);
                let prev = order[i.map_or(n - 1, |i| (i + n - 1) % n)];
                self.focus_pane(prev);
                self.focus_returned();
            }
            S::FocusBelow => {
                if self.sidebar_panes().len() < 2 {
                    self.set_status(NO_PANE_BELOW);
                } else if self.sidebar.focus != Focus::Outline {
                    self.focus_pane(Focus::Outline);
                }
            }
            S::FocusAbove => {
                if self.sidebar_panes().len() < 2 {
                    self.set_status(NO_PANE_ABOVE);
                } else if self.sidebar.focus != Focus::Files {
                    self.focus_pane(Focus::Files);
                }
            }
            S::FocusLast => {
                let p = self.sidebar.prev_focus;
                let p = if p == Focus::Content || self.sidebar_panes().contains(&p) {
                    p
                } else {
                    Focus::Content
                };
                if p != self.sidebar.focus {
                    self.focus_pane(p);
                    self.focus_returned();
                }
            }
            S::Down => self.pane_move(1),
            S::Up => self.pane_move(-1),
            S::HalfDown => self.pane_move(self.half_pane()),
            S::HalfUp => self.pane_move(-self.half_pane()),
            S::Top => self.pane_move(isize::MIN),
            S::Bottom => self.pane_move(isize::MAX),
            S::Collapse => self.tree_collapse(),
            S::Expand => self.tree_expand(),
            S::RootUp => self.tree_root_up(),
            S::RootHere => self.tree_root_here(),
            S::Open => self.pane_open(),
            S::FilterStart => {
                let current = self.pane_filter().unwrap_or_default();
                self.sidebar.prompt = Some(current);
                self.mode = Mode::Filter;
            }
            S::FilterInput(c) => {
                if let Some(p) = &mut self.sidebar.prompt {
                    p.push(c);
                    let p = p.clone();
                    self.set_pane_filter(Some(p));
                }
            }
            S::FilterBackspace => match &mut self.sidebar.prompt {
                Some(p) if p.is_empty() => self.sidebar_action(S::FilterCancel),
                Some(p) => {
                    p.pop();
                    let p = p.clone();
                    self.set_pane_filter(Some(p));
                }
                None => {}
            },
            S::FilterCommit => {
                let typed = self.sidebar.prompt.take().unwrap_or_default();
                self.mode = Mode::Normal;
                self.set_pane_filter((!typed.is_empty()).then_some(typed));
                self.pane_move(isize::MIN);
            }
            S::FilterCancel => {
                self.sidebar.prompt = None;
                self.mode = Mode::Normal;
                self.set_pane_filter(None);
            }
            // The second Esc (no filter left) closes a peek (D11, P13).
            S::FilterClear if self.pane_filter().is_none() && self.sidebar.peeked => {
                self.focus_pane(Focus::Content);
                self.focus_returned();
            }
            S::FilterClear => self.set_pane_filter(None),
        }
    }

    /// Whether focus action `a` would land in a sidebar pane, from a
    /// hidden sidebar (focus on the content): a peek opens for it (P2).
    fn focus_enters_sidebar(&self, a: SidebarAction) -> bool {
        use SidebarAction as S;
        let panes = self.sidebar_panes();
        match a {
            S::FocusSidebar | S::FocusNext | S::FocusPrev => true,
            S::FocusBelow | S::FocusAbove => panes.len() >= 2,
            S::FocusLast => panes.contains(&self.sidebar.prev_focus),
            _ => false,
        }
    }

    fn pane_filter(&self) -> Option<String> {
        match self.sidebar.focus {
            Focus::Files => self.tree().and_then(Tree::filter).map(str::to_string),
            Focus::Outline => self.sidebar.outline_filter.clone(),
            Focus::Content => None,
        }
    }

    /// Pasted text (one line) appended to the filter being typed.
    pub(super) fn filter_insert(&mut self, text: &str) {
        if let Some(p) = &mut self.sidebar.prompt {
            p.push_str(text);
            let p = p.clone();
            self.set_pane_filter(Some(p));
        }
    }

    fn set_pane_filter(&mut self, filter: Option<String>) {
        match self.sidebar.focus {
            Focus::Files => {
                if let Some(t) = &mut self.sidebar.tree {
                    t.set_filter(filter);
                }
            }
            Focus::Outline => {
                self.sidebar.outline_filter = filter;
                self.sidebar.outline_sel = 0;
            }
            Focus::Content => {}
        }
        self.sidebar_relayout(true);
    }

    /// Move the focused pane's selection; `isize::MIN` / `MAX` mean the
    /// first / last row.
    /// Half the rows the focused list pane drew in the last frame (at
    /// least 1), as `C-d` / `C-u` move in the page.
    fn half_pane(&self) -> isize {
        let l = self.layout();
        let area = match self.sidebar.focus {
            Focus::Files => l.files,
            Focus::Outline => l.outline,
            Focus::Content => None,
        };
        let rows = area.map_or(0, |a| a.items.height);
        isize::from(i16::try_from(rows / 2).unwrap_or(i16::MAX)).max(1)
    }

    fn pane_move(&mut self, delta: isize) {
        self.pane_move_in(self.sidebar.focus, delta);
    }

    /// Focus pane `f` (the mouse) and select its row `i`.
    pub(super) fn pane_select(&mut self, f: Focus, i: usize) {
        self.focus_pane(f);
        match f {
            Focus::Files => {
                if let Some(t) = &mut self.sidebar.tree {
                    t.select_index(i);
                }
            }
            Focus::Outline => {
                self.sidebar.outline_sel = i.min(self.outline().len().saturating_sub(1));
            }
            Focus::Content => {}
        }
    }

    /// The files pane's row `i`, if it is a folder: its depth.
    pub(super) fn folder_depth(&self, i: usize) -> Option<usize> {
        let item = self.tree()?.visible_items().into_iter().nth(i)?;
        item.is_dir.then_some(item.depth)
    }

    /// First drawn row of sidebar list `f` (files or outline).
    pub fn sidebar_list_top(&self, f: Focus) -> usize {
        match f {
            Focus::Files => self.sidebar.files_top.get(),
            Focus::Outline => self.sidebar.outline_top.get(),
            Focus::Content => 0,
        }
    }

    /// Record the first row the last frame drew for list `f`.
    pub(super) fn set_sidebar_list_top(&self, f: Focus, top: usize) {
        match f {
            Focus::Files => self.sidebar.files_top.set(top),
            Focus::Outline => self.sidebar.outline_top.set(top),
            Focus::Content => {}
        }
    }

    /// The wheel over a sidebar pane: move the selection when the pane
    /// draws one (files always; outline while focused), else scroll the
    /// list by `delta` rows, clamped to the last full page of `body` rows.
    pub(super) fn pane_wheel(&mut self, f: Focus, delta: isize, body: usize) {
        if f == Focus::Outline && self.outline_selected().is_none() {
            let max = self.outline().len().saturating_sub(body);
            let top = &self.sidebar.outline_top;
            top.set(top.get().saturating_add_signed(delta).min(max));
        } else {
            self.pane_move_in(f, delta);
        }
    }

    /// Move pane `f`'s selection without focusing it (the wheel).
    pub(super) fn pane_move_in(&mut self, f: Focus, delta: isize) {
        match f {
            Focus::Files => {
                let Some(t) = &mut self.sidebar.tree else {
                    return;
                };
                match delta {
                    isize::MIN => t.select_index(0),
                    isize::MAX => t.select_index(usize::MAX),
                    d => t.move_by(d),
                }
            }
            Focus::Outline => {
                let last = self.outline().len().saturating_sub(1);
                let sel = &mut self.sidebar.outline_sel;
                *sel = match delta {
                    isize::MIN => 0,
                    isize::MAX => last,
                    d => sel.saturating_add_signed(d).min(last),
                };
            }
            Focus::Content => {}
        }
    }

    fn selected_item(&self) -> Option<TreeItem> {
        let t = self.tree()?;
        let items = t.visible_items();
        let i = t.selected_index(&items)?;
        items.into_iter().nth(i)
    }

    fn tree_collapse(&mut self) {
        let Some(item) = self.selected_item() else {
            return;
        };
        let Some(t) = &mut self.sidebar.tree else {
            return;
        };
        if item.is_dir && item.expanded {
            t.collapse(&item.path);
        } else if let Some(parent) = item.path.parent()
            && parent != t.root()
        {
            let parent = parent.to_path_buf();
            t.collapse(&parent);
            t.select(Some(parent));
        } else {
            self.tree_root_up();
        }
    }

    /// `-`: re-root the files pane one folder up.
    fn tree_root_up(&mut self) {
        if self.sidebar.focus != Focus::Files {
            return;
        }
        let Some(t) = &mut self.sidebar.tree else {
            return;
        };
        if t.root_up() {
            self.sidebar_relayout(false);
        } else {
            self.set_status(ROOT_TOP_MESSAGE);
        }
    }

    /// `.`: make the selected folder the root of the files pane.
    fn tree_root_here(&mut self) {
        if self.sidebar.focus != Focus::Files {
            return;
        }
        let Some(item) = self.selected_item().filter(|i| i.is_dir) else {
            return;
        };
        let Some(t) = &mut self.sidebar.tree else {
            return;
        };
        t.set_root(&item.path);
        if t.selected().is_none() {
            t.select_index(0);
        }
        self.sidebar_relayout(false);
    }

    fn tree_expand(&mut self) {
        if let Some(item) = self.selected_item()
            && item.is_dir
            && let Some(t) = &mut self.sidebar.tree
        {
            t.expand(&item.path);
            self.sidebar_relayout(true);
        }
    }

    fn pane_open(&mut self) {
        match self.sidebar.focus {
            Focus::Files => {
                let Some(item) = self.selected_item() else {
                    return;
                };
                if item.is_dir {
                    if let Some(t) = &mut self.sidebar.tree {
                        if item.expanded {
                            t.collapse(&item.path);
                        } else {
                            t.expand(&item.path);
                        }
                    }
                    self.sidebar_relayout(true);
                } else if !item.markdown {
                    self.pending_effect = Some(super::Effect::Edit {
                        path: item.path,
                        line: None,
                    });
                } else {
                    self.open_from_tree(&item.path);
                }
            }
            Focus::Outline => {
                if let Some(o) = self.outline().get(self.sidebar.outline_sel) {
                    let row = o.row;
                    // Close a peek first, so a beside peek's re-layout
                    // doesn't move the row jumped to (D11, P4).
                    self.sidebar_peek_close();
                    self.jump_to_row(row);
                    self.set_focus(Focus::Content);
                }
            }
            Focus::Content => {}
        }
    }

    /// Open a markdown file picked in the tree, pushing the page left.
    fn open_from_tree(&mut self, path: &Path) {
        let Ok(bytes) = std::fs::read(path) else {
            self.set_status(format!("Cannot read {}", path.display()));
            return;
        };
        let here = self.entry();
        // Close a peek before the page is laid out, so a beside peek
        // doesn't lay it out twice (D11, P5); a page that can't be shown
        // brings it back.
        let peeked = self.sidebar.peeked;
        self.sidebar_peek_close();
        if !self.open_bytes(path, &bytes) {
            if peeked {
                self.sidebar_peek_open();
                self.focus_pane(Focus::Files);
            }
            return;
        }
        if let Some(e) = here {
            self.history.push(e);
        }
        self.set_focus(Focus::Content);
    }
}

/// Title of the outline pane.
pub const OUTLINE_TITLE: &str = "Outline";
/// The files pane title before the root.
pub const FILES_TITLE: &str = "Files";

/// The files pane title with the filter, if any.
fn files_title(title: &str, filter: Option<&str>) -> String {
    match filter {
        Some(f) => format!("{title} /{f}"),
        None => title.to_string(),
    }
}

/// Paths of `items`, for tests and debugging.
pub fn item_paths(items: &[TreeItem]) -> Vec<PathBuf> {
    items.iter().map(|i| i.path.clone()).collect()
}
