//! The sidebar (spec § Sidebar): modes off / files / outline / split, the
//! lazily walked file tree, the outline of the current page, focus moves
//! (`C-w h/l/w`) and the per-pane `/` filter.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::keys::{Action, KeyResult};
use super::{App, Mode, StartTarget};
use crate::config::{SidebarConfig, SidebarMode};
use crate::nav::is_markdown;

/// Status when the terminal cannot fit the sidebar next to the content.
pub const NARROW_MESSAGE: &str = "Window too narrow for the sidebar";
/// Status for a focus move while no sidebar is shown.
pub const OFF_MESSAGE: &str = "Sidebar is off";
/// Columns the content keeps at least before the sidebar is dropped.
const MIN_CONTENT: u16 = 10;

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
    /// `<leader>e`: off → files → outline → split → off.
    Cycle,
    /// `C-w h`
    FocusLeft,
    /// `C-w l`
    FocusRight,
    /// `C-w w`
    FocusNext,
    Down,
    Up,
    Top,
    Bottom,
    /// `h`: collapse the directory, or select the parent.
    Collapse,
    /// `l`: expand the directory.
    Expand,
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
    /// What `Auto` means for this start target.
    auto: SidebarMode,
    focus: Focus,
    /// The sidebar pane `C-w h` returns to.
    last_side: Focus,
    /// Built when the files pane is first shown.
    tree: Option<Tree>,
    outline_sel: usize,
    outline_filter: Option<String>,
    /// The filter being typed (`Mode::Filter`).
    prompt: Option<String>,
}

impl Sidebar {
    pub(super) fn new(config: &SidebarConfig, target: &StartTarget) -> Sidebar {
        let auto = match target {
            StartTarget::Dir(_) => SidebarMode::Files,
            StartTarget::File(_) | StartTarget::Stdin(_) => SidebarMode::Outline,
        };
        let mode = match config.default {
            SidebarMode::Auto => auto,
            m => m,
        };
        Sidebar {
            mode,
            auto,
            focus: Focus::Content,
            last_side: Focus::Files,
            tree: None,
            outline_sel: 0,
            outline_filter: None,
            prompt: None,
        }
    }

    /// The panes the mode shows, top to bottom.
    fn panes(&self) -> &'static [Focus] {
        match self.mode {
            SidebarMode::Files => &[Focus::Files],
            SidebarMode::Outline => &[Focus::Outline],
            SidebarMode::Split => &[Focus::Files, Focus::Outline],
            SidebarMode::Off | SidebarMode::Auto => &[],
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
    filter: Option<String>,
}

fn canonical(p: &Path) -> PathBuf {
    p.canonicalize()
        .or_else(|_| std::path::absolute(p))
        .unwrap_or_else(|_| p.to_path_buf())
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
            filter: None,
        };
        tree.walk(&root);
        tree
    }

    pub fn root(&self) -> &Path {
        &self.root
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
            .filter(|e| e.depth() == 1)
            .map(|e| Node {
                path: e.path().to_path_buf(),
                name: e.file_name().to_string_lossy().into_owned(),
                is_dir: e.path().is_dir(),
            })
            .collect();
        nodes.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        self.children.insert(dir.to_path_buf(), nodes);
    }

    pub fn expand(&mut self, dir: &Path) {
        self.walk(dir);
        self.expanded.insert(dir.to_path_buf());
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
        let Some(nodes) = self.children.get(dir) else {
            return;
        };
        for n in nodes {
            let markdown = !n.is_dir && is_markdown(&n.path);
            let shown = self.show_all
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

/// `C-w h` / `C-w l` / `C-w w` (the second key with or without Ctrl).
pub(super) fn window_keymap(keys: &[KeyEvent]) -> Option<KeyResult> {
    let (first, rest) = keys.split_first()?;
    if !is_ctrl(first, 'w') {
        return None;
    }
    let Some(k) = rest.first() else {
        return Some(KeyResult::Pending);
    };
    Some(match k.code {
        KeyCode::Char('h') | KeyCode::Left => act(SidebarAction::FocusLeft),
        KeyCode::Char('l') | KeyCode::Right => act(SidebarAction::FocusRight),
        KeyCode::Char('w') => act(SidebarAction::FocusNext),
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
        if let Some(r) = window_keymap(keys) {
            return r;
        }
        match keys {
            [a, b] if plain(a) == Some('g') && plain(b) == Some('g') => act(S::Top),
            [k] => match (k.code, plain(k)) {
                (_, Some('j')) | (KeyCode::Down, _) => act(S::Down),
                (_, Some('k')) | (KeyCode::Up, _) => act(S::Up),
                (_, Some('g')) => KeyResult::Pending,
                (_, Some('G')) => act(S::Bottom),
                (_, Some('h')) | (KeyCode::Left, _) => act(S::Collapse),
                (_, Some('l')) | (KeyCode::Right, _) => act(S::Expand),
                (_, Some('o')) | (KeyCode::Enter, _) => act(S::Open),
                (_, Some('/')) => act(S::FilterStart),
                (KeyCode::Esc, _) => act(S::FilterClear),
                (_, Some('q')) => KeyResult::Action(Action::Quit),
                _ => KeyResult::None,
            },
            _ => KeyResult::None,
        }
    }

    /// The resolved sidebar mode (never `Auto`).
    pub fn sidebar_mode(&self) -> SidebarMode {
        self.sidebar.mode
    }

    /// Switch the sidebar mode (`Auto` means the start target's default)
    /// and re-lay out the content for the new width.
    pub fn set_sidebar_mode(&mut self, mode: SidebarMode) {
        self.sidebar.mode = match mode {
            SidebarMode::Auto => self.sidebar.auto,
            m => m,
        };
        self.sync_tree();
        let (cols, rows) = self.size;
        self.resize(cols, rows);
        self.sidebar_fit();
    }

    /// Which pane receives keys.
    pub fn focus(&self) -> Focus {
        self.sidebar.focus
    }

    /// Columns the sidebar takes, its border included; 0 when it is off or
    /// does not fit.
    pub fn sidebar_cols(&self) -> u16 {
        if self.sidebar.panes().is_empty() {
            return 0;
        }
        let w = self.config.sidebar.width.saturating_add(1);
        if w >= self.size.0.saturating_sub(MIN_CONTENT) {
            0
        } else {
            w
        }
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

    /// Title of the files pane: `Files`, or the current file's path when
    /// it lies outside the tree root.
    pub fn sidebar_title(&self) -> String {
        match self.tree().and_then(Tree::outside) {
            Some(p) => p.display().to_string(),
            None => "Files".into(),
        }
    }

    /// The filter prompt (`/text`) while typing one.
    pub fn filter_prompt(&self) -> Option<String> {
        self.sidebar.prompt.as_ref().map(|p| format!("/{p}"))
    }

    /// Build the tree if the files pane is shown, and follow the current
    /// file in it.
    pub(super) fn sync_tree(&mut self) {
        if !self.sidebar.panes().contains(&Focus::Files) {
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

    /// After a mode change or resize: give focus back to the content when
    /// its pane is gone, and say so when the sidebar does not fit.
    pub(super) fn sidebar_fit(&mut self) {
        let shown = self.sidebar_cols() > 0;
        if !shown || !self.sidebar.panes().contains(&self.sidebar.focus) {
            self.sidebar.focus = Focus::Content;
        }
        if self.sidebar.focus == Focus::Content && self.mode == Mode::Filter {
            self.sidebar.prompt = None;
            self.mode = Mode::Normal;
        }
        if !shown && !self.sidebar.panes().is_empty() {
            self.set_status(NARROW_MESSAGE);
        }
    }

    fn focus_pane(&mut self, f: Focus) {
        self.sidebar.focus = f;
        if f == Focus::Content {
            return;
        }
        self.sidebar.last_side = f;
        if f == Focus::Outline {
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
            S::Cycle => self.set_sidebar_mode(match self.sidebar.mode {
                SidebarMode::Off | SidebarMode::Auto => SidebarMode::Files,
                SidebarMode::Files => SidebarMode::Outline,
                SidebarMode::Outline => SidebarMode::Split,
                SidebarMode::Split => SidebarMode::Off,
            }),
            S::FocusLeft | S::FocusNext if self.sidebar_cols() == 0 => {
                let off = self.sidebar.panes().is_empty();
                self.set_status(if off { OFF_MESSAGE } else { NARROW_MESSAGE });
            }
            S::FocusLeft => {
                let panes = self.sidebar.panes();
                let f = if panes.contains(&self.sidebar.last_side) {
                    self.sidebar.last_side
                } else {
                    panes[0]
                };
                self.focus_pane(f);
            }
            S::FocusRight => self.focus_pane(Focus::Content),
            S::FocusNext => {
                let mut order = self.sidebar.panes().to_vec();
                order.push(Focus::Content);
                let i = order.iter().position(|&f| f == self.sidebar.focus);
                let next = order[i.map_or(0, |i| (i + 1) % order.len())];
                self.focus_pane(next);
            }
            S::Down => self.pane_move(1),
            S::Up => self.pane_move(-1),
            S::Top => self.pane_move(isize::MIN),
            S::Bottom => self.pane_move(isize::MAX),
            S::Collapse => self.tree_collapse(),
            S::Expand => self.tree_expand(),
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
            S::FilterClear => self.set_pane_filter(None),
        }
    }

    fn pane_filter(&self) -> Option<String> {
        match self.sidebar.focus {
            Focus::Files => self.tree().and_then(Tree::filter).map(str::to_string),
            Focus::Outline => self.sidebar.outline_filter.clone(),
            Focus::Content => None,
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
    }

    /// Move the focused pane's selection; `isize::MIN` / `MAX` mean the
    /// first / last row.
    fn pane_move(&mut self, delta: isize) {
        match self.sidebar.focus {
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
        }
    }

    fn tree_expand(&mut self) {
        if let Some(item) = self.selected_item()
            && item.is_dir
            && let Some(t) = &mut self.sidebar.tree
        {
            t.expand(&item.path);
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
                } else if !item.markdown {
                    self.pending_effect = Some(super::Effect::Edit(item.path));
                } else {
                    self.open_from_tree(&item.path);
                }
            }
            Focus::Outline => {
                if let Some(o) = self.outline().get(self.sidebar.outline_sel) {
                    let row = o.row;
                    self.jump_to_row(row);
                    self.sidebar.focus = Focus::Content;
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
        if !self.open_bytes(path, &bytes) {
            return;
        }
        if let Some(e) = here {
            self.history.push(e);
        }
        self.sidebar.focus = Focus::Content;
    }
}

/// Paths of `items`, for tests and debugging.
pub fn item_paths(items: &[TreeItem]) -> Vec<PathBuf> {
    items.iter().map(|i| i.path.clone()).collect()
}
