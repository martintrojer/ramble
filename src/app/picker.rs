//! The picker overlay (spec § Keymap notes, § Notebook operations): opens
//! a notebook operation, fuzzy-filters its items with nucleo, `C-n`/`C-p`
//! move, `Enter` opens, `Esc` closes. `search` first prompts for a query.
//!
//! Picker requests use their own tag space: `PICKER_TAG_BASE | seq`. A reply
//! whose seq is not the open picker's current request is dropped.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use nucleo::{Config, Matcher, Utf32Str};
use serde_json::Value;

use super::keys::{Action, KeyResult};
use super::{App, Mode};
use crate::lsp::Kind;
use crate::nav::{self, Target};
use crate::notebook::{self, Item, Op, Sources, zk};

fn canonical(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// High bit set on every picker request tag.
pub const PICKER_TAG_BASE: u64 = 1 << 63;

/// Keys while the picker is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerAction {
    Input(char),
    Backspace,
    Next,
    Prev,
    Accept,
    Close,
}

/// How a reply becomes items, and what Enter does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Content {
    /// Notes (open the path).
    Notes,
    /// zk tags (Enter opens the notes with that tag).
    Tags,
    /// references Locations (open path at line).
    Locations,
    /// Links of the current page (Enter follows).
    Links,
}

#[derive(Debug, Clone)]
struct Picker {
    title: String,
    content: Content,
    items: Vec<Item>,
    /// Indices into `items` matching `input`, best first.
    filtered: Vec<usize>,
    selected: usize,
    input: String,
    /// Typing the search query (Enter sends it) rather than a filter.
    prompting: bool,
    /// Waiting for the reply to request `seq`.
    loading: bool,
    seq: u64,
    /// Server root the request went to (relative paths in details).
    root: PathBuf,
    /// Backlinks: the canonical path of the page the picker was opened
    /// from; Enter lands on the first link back to it.
    from: Option<PathBuf>,
}

impl Picker {
    fn new(title: impl Into<String>, content: Content, root: PathBuf) -> Picker {
        Picker {
            title: title.into(),
            content,
            items: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
            input: String::new(),
            prompting: false,
            loading: false,
            seq: 0,
            root,
            from: None,
        }
    }

    fn set_items(&mut self, items: Vec<Item>) {
        self.items = items;
        self.loading = false;
        self.refilter();
    }

    fn refilter(&mut self) {
        self.filtered = filter(&self.items, &self.input);
        self.selected = 0;
    }
}

/// Indices of `items` matching `query` (nucleo fuzzy match over label and
/// detail), best score first; all items in order for an empty query.
pub fn filter(items: &[Item], query: &str) -> Vec<usize> {
    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut buf = Vec::new();
    let mut scored: Vec<(usize, u32)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| {
            let hay = format!("{} {}", it.label, it.detail);
            pattern
                .score(Utf32Str::new(&hay, &mut buf), &mut matcher)
                .map(|s| (i, s))
        })
        .collect();
    scored.sort_by_key(|&(i, s)| (std::cmp::Reverse(s), i));
    scored.into_iter().map(|(i, _)| i).collect()
}

/// What the UI draws for the open picker.
#[derive(Debug, Clone)]
pub struct PickerView<'a> {
    pub title: &'a str,
    /// The query being typed (search prompt) or the filter.
    pub input: &'a str,
    pub prompting: bool,
    pub loading: bool,
    /// Matching items, best first.
    pub items: Vec<&'a Item>,
    pub selected: usize,
}

/// Picker state owned by [`App`].
#[derive(Debug, Clone, Default)]
pub(super) struct PickerState {
    open: Option<Picker>,
    seq: u64,
}

/// Keys in the picker: typed text filters, no counts.
pub(super) fn picker_keymap(keys: &[KeyEvent]) -> KeyResult {
    use PickerAction as P;
    let Some(key) = keys.last() else {
        return KeyResult::None;
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let a = match key.code {
        KeyCode::Char('n') if ctrl => P::Next,
        KeyCode::Char('p') if ctrl => P::Prev,
        KeyCode::Down => P::Next,
        KeyCode::Up => P::Prev,
        KeyCode::Enter => P::Accept,
        KeyCode::Esc => P::Close,
        KeyCode::Backspace => P::Backspace,
        KeyCode::Char(c) if !ctrl => P::Input(c),
        _ => return KeyResult::None,
    };
    KeyResult::Action(Action::Picker(a))
}

impl App {
    /// The open picker, for drawing and tests.
    pub fn picker(&self) -> Option<PickerView<'_>> {
        let p = self.picker.open.as_ref()?;
        Some(PickerView {
            title: &p.title,
            input: &p.input,
            prompting: p.prompting,
            loading: p.loading,
            items: p.filtered.iter().map(|&i| &p.items[i]).collect(),
            selected: p.selected,
        })
    }

    /// What the current page offers (see [`notebook::available`]).
    fn sources(&self) -> Sources {
        let running = self.lsp_running();
        Sources {
            page: self.page.is_some(),
            server: running.as_ref().map(|r| r.1),
            references: running.as_ref().is_some_and(|(c, ..)| {
                c.capabilities()
                    .references_provider
                    .as_ref()
                    .is_some_and(|p| !matches!(p, lsp_types::OneOf::Left(false)))
            }),
            heading: self
                .page
                .as_ref()
                .is_some_and(|p| !p.doc.headings.is_empty()),
        }
    }

    /// Notebook operations with a source right now, in table order.
    pub fn available_ops(&self) -> Vec<Op> {
        let s = self.sources();
        Op::ALL
            .into_iter()
            .filter(|&op| notebook::available(op, s).is_ok())
            .collect()
    }

    /// Open the picker for `op`. `query` is the search query (`:Search q`);
    /// without one, search prompts for it first.
    /// `op` has a source right now.
    pub(crate) fn can_open(&self, op: Op) -> bool {
        notebook::available(op, self.sources()).is_ok()
    }

    pub fn open_op(&mut self, op: Op, query: Option<&str>) {
        if let Err(msg) = notebook::available(op, self.sources()) {
            self.set_status(msg);
            return;
        }
        let zk_root = self.lsp_running().filter(|r| r.1 == Kind::Zk).map(|r| r.2);
        match op {
            Op::Notes => match zk_root {
                Some(root) => {
                    let params = zk::notes(&root);
                    self.picker_request("Notes", Content::Notes, root, zk::EXECUTE, params);
                }
                None => {
                    let root = self.tree_root.clone();
                    let mut p = Picker::new("Notes", Content::Notes, root.clone());
                    p.set_items(notebook::walk_notes(&root));
                    self.show_picker(p);
                }
            },
            Op::Search => {
                let root = zk_root.unwrap_or_default();
                match query.map(str::trim).filter(|q| !q.is_empty()) {
                    Some(q) => self.send_search(root, q),
                    None => {
                        let mut p = Picker::new("Search", Content::Notes, root);
                        p.prompting = true;
                        self.show_picker(p);
                    }
                }
            }
            Op::Tags => {
                let root = zk_root.unwrap_or_default();
                let params = zk::tags(&root);
                self.picker_request("Tags", Content::Tags, root, zk::EXECUTE, params);
            }
            Op::Backlinks => self.open_backlinks(),
            Op::Links => {
                let Some(page) = &self.page else { return };
                let mut p = Picker::new("Links", Content::Links, self.tree_root.clone());
                p.set_items(notebook::link_items(&page.doc));
                self.show_picker(p);
            }
        }
    }

    fn send_search(&mut self, root: PathBuf, q: &str) {
        let params = zk::search(&root, q);
        let title = format!("Search: {q}");
        self.picker_request(&title, Content::Notes, root, zk::EXECUTE, params);
    }

    fn open_backlinks(&mut self) {
        let Some((client, kind, root)) = self.lsp_running() else {
            return;
        };
        let Some(page) = &self.page else { return };
        let Some(path) = page.path.clone() else {
            return;
        };
        let Some(pos) = notebook::backlinks_position(kind, &page.doc, client.encoding()) else {
            self.set_status("Backlinks need a language server");
            return;
        };
        let params = notebook::references_params(&path, pos);
        let sent = self.picker_request(
            "Backlinks",
            Content::Locations,
            root,
            "textDocument/references",
            params,
        );
        if sent && let Some(p) = self.picker.open.as_mut() {
            p.from = Some(canonical(&path));
        }
    }

    /// Send a picker request and show the picker loading. False when it
    /// could not be sent (status set, no picker shown).
    fn picker_request(
        &mut self,
        title: &str,
        content: Content,
        root: PathBuf,
        method: &str,
        params: Value,
    ) -> bool {
        self.picker.seq += 1;
        let seq = self.picker.seq;
        let Some((client, ..)) = self.lsp_running() else {
            self.set_status(format!("{title}: no language server"));
            return false;
        };
        if let Err(e) = client.request(method, params, PICKER_TAG_BASE | seq) {
            self.set_status(format!("{title}: {e:#}"));
            return false;
        }
        let mut p = Picker::new(title, content, root);
        p.loading = true;
        p.seq = seq;
        self.show_picker(p);
        true
    }

    fn show_picker(&mut self, p: Picker) {
        self.picker.open = Some(p);
        self.mode = Mode::Picker;
    }

    fn close_picker(&mut self) {
        self.picker.open = None;
        if self.mode == Mode::Picker {
            self.mode = Mode::Normal;
        }
    }

    /// A reply tagged with [`PICKER_TAG_BASE`]. Dropped unless it answers
    /// the open picker's current request.
    pub(super) fn picker_response(&mut self, tag: u64, result: Result<Value, String>) {
        let seq = tag & !PICKER_TAG_BASE;
        let Some(p) = self.picker.open.as_mut() else {
            return;
        };
        if !p.loading || p.seq != seq {
            return;
        }
        let v = match result {
            Ok(v) => v,
            Err(e) => {
                let msg = format!("{}: {e}", p.title);
                self.close_picker();
                self.set_status(msg);
                return;
            }
        };
        let items = match p.content {
            Content::Notes => zk::note_items(&v, &p.root),
            Content::Tags => zk::tag_items(&v),
            Content::Locations => notebook::location_items(&v, &p.root),
            Content::Links => Vec::new(),
        };
        p.set_items(items);
    }

    pub(super) fn picker_action(&mut self, a: PickerAction) {
        let Some(p) = self.picker.open.as_mut() else {
            self.mode = Mode::Normal;
            return;
        };
        match a {
            PickerAction::Input(c) => {
                p.input.push(c);
                if !p.prompting {
                    p.refilter();
                }
            }
            PickerAction::Backspace => {
                p.input.pop();
                if !p.prompting {
                    p.refilter();
                }
            }
            PickerAction::Next if !p.filtered.is_empty() => {
                p.selected = (p.selected + 1) % p.filtered.len();
            }
            PickerAction::Prev if !p.filtered.is_empty() => {
                p.selected = (p.selected + p.filtered.len() - 1) % p.filtered.len();
            }
            PickerAction::Next | PickerAction::Prev => {}
            PickerAction::Close => self.close_picker(),
            PickerAction::Accept => self.picker_accept(),
        }
    }

    /// Select the open picker's item `i` (the mouse).
    pub(super) fn picker_select(&mut self, i: usize) {
        if let Some(p) = self.picker.open.as_mut()
            && i < p.filtered.len()
        {
            p.selected = i;
        }
    }

    fn picker_accept(&mut self) {
        let Some(p) = self.picker.open.as_ref() else {
            return;
        };
        if p.prompting {
            let q = p.input.trim().to_string();
            if !q.is_empty() {
                let root = p.root.clone();
                self.send_search(root, &q);
            }
            return;
        }
        let Some(item) = p.filtered.get(p.selected).map(|&i| p.items[i].clone()) else {
            return;
        };
        let (content, root, from) = (p.content, p.root.clone(), p.from.clone());
        self.close_picker();
        match content {
            Content::Tags => {
                let params = zk::notes_by_tag(&root, &item.label);
                let title = format!("Tag: {}", item.label);
                self.picker_request(&title, Content::Notes, root, zk::EXECUTE, params);
            }
            Content::Links => {
                if let Some(i) = item.link {
                    self.follow_link_index(i);
                }
            }
            Content::Notes => {
                if let Some(path) = &item.path {
                    self.open_path_at(path, item.line);
                }
            }
            Content::Locations => {
                if let Some(path) = &item.path {
                    self.open_backlink(path, item.line, from.as_deref());
                }
            }
        }
    }

    /// Open a backlink: the cursor goes to the first link in the opened
    /// page that resolves to `from` (zk reports the line of the first
    /// substring hit of the target's name, often not the link), else to
    /// the server's `line`.
    /// If no link resolves yet (it may need the page's documentLink reply),
    /// the search is redone when that reply arrives, unless the cursor has
    /// moved by then.
    fn open_backlink(&mut self, path: &Path, line: Option<usize>, from: Option<&Path>) {
        let before = self.page_id();
        self.open_path_at(path, line);
        if self.page_id() == before {
            return;
        }
        let Some(from) = from else { return };
        if !self.land_on_link_to(from) {
            self.lsp_await_backlink(from.to_path_buf(), self.cursor);
        }
    }

    /// Put the cursor on the first link to `target` (see
    /// [`App::first_link_to`]). False when there is none.
    pub(super) fn land_on_link_to(&mut self, target: &Path) -> bool {
        let Some(i) = self.first_link_to(target) else {
            return false;
        };
        let Some(page) = &self.page else { return false };
        let Some(seg) = page
            .rendered
            .srcmap
            .segments
            .iter()
            .find(|s| s.link == Some(i))
        else {
            return false;
        };
        let (row, col) = (seg.span.row, seg.span.col_start);
        self.jump_to_row(row);
        self.set_col(col);
        true
    }

    /// Index of the first link on the current page whose target is the
    /// canonical path `target`: the documentLink target when the server
    /// gave one, else local resolution.
    fn first_link_to(&self, target: &Path) -> Option<usize> {
        let page = self.page.as_ref()?;
        let dir = self.link_dir();
        page.doc.links.iter().enumerate().position(|(i, link)| {
            let path = match self.link_target(i) {
                Some(p) => p.to_path_buf(),
                None => match nav::resolve(&link.dest, &link.kind, &dir) {
                    Target::File { path, .. } => path,
                    _ => return false,
                },
            };
            canonical(&path) == target
        })
    }

    /// Move the cursor onto link `i` and follow it like `gd`.
    fn follow_link_index(&mut self, i: usize) {
        let Some(page) = &self.page else { return };
        let Some(seg) = page
            .rendered
            .srcmap
            .segments
            .iter()
            .find(|s| s.link == Some(i))
        else {
            self.set_status("Link is not drawn");
            return;
        };
        let (row, col) = (seg.span.row, seg.span.col_start);
        self.cursor.row = row;
        self.set_col(col);
        self.keep_visible();
        self.follow();
    }

    /// Open `path` as a followed link (existence check, history push), then
    /// put the cursor on the first row of 1-based source `line`. Used by the
    /// pickers and `:e`.
    pub(super) fn open_path_at(&mut self, path: &Path, line: Option<usize>) {
        let before = self.page_id();
        let shown = path.display().to_string();
        self.open_link_file(path.to_path_buf(), None, &shown);
        if self.page_id() == before {
            return; // Not opened (missing, binary, or handed to the editor).
        }
        let Some(line) = line else { return };
        let Some(p) = &self.page else { return };
        if let Some(row) = p.rendered.source_lines.iter().position(|&l| l >= line) {
            self.jump_to_row(row);
        }
    }
}

/// `<leader>z` (pending) and `<leader>z{f,s,z,b,l}`.
pub(super) fn leader_op(typed: &[char]) -> Option<KeyResult> {
    let op = match typed {
        ['z'] => return Some(KeyResult::Pending),
        ['z', 'f'] => Op::Notes,
        ['z', 's'] => Op::Search,
        ['z', 'z'] => Op::Tags,
        ['z', 'b'] => Op::Backlinks,
        ['z', 'l'] => Op::Links,
        _ => return None,
    };
    Some(KeyResult::Action(Action::OpenOp(op)))
}

/// Normal-mode keys of this unit: `:` and `grr`.
pub(super) fn normal_keys(keys: &[KeyEvent]) -> Option<KeyResult> {
    let plain = |k: &KeyEvent| match k.code {
        KeyCode::Char(c) if !k.modifiers.contains(KeyModifiers::CONTROL) => Some(c),
        _ => None,
    };
    let typed: Option<Vec<char>> = keys.iter().map(plain).collect();
    Some(match typed?.as_slice() {
        [':'] => KeyResult::Action(Action::Cmd(super::CmdAction::Start)),
        ['g', 'r'] => KeyResult::Pending,
        ['g', 'r', 'r'] => KeyResult::Action(Action::OpenOp(Op::Backlinks)),
        ['g', 'r', _] => KeyResult::None,
        _ => return None,
    })
}
