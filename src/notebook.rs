//! Notebook operations (notes, search, tags, backlinks, links) over the zk
//! adapter, standard LSP, or local fallbacks (spec § Notebook operations).
//! A page mdroots serves gets its items from `app::mdroots_glue` instead;
//! [`available`] knows that through [`Sources::mdroots`]. The zk adapter and
//! the LSP-only helpers live with the optional backend, in
//! [`crate::backend::lsp::notebook`].
//!
//! Everything here is pure: it turns local data into picker [`Item`]s. The
//! app sends the requests and picks the source in the spec's order: zk
//! adapter, then the standard LSP request (when the server reports the
//! capability), then the local fallback.

use std::path::{Path, PathBuf};

use crate::doc::Document;
use crate::lsp::Kind;
use crate::nav::is_markdown;

/// Depth of the local notes file walk below the tree root.
pub const WALK_DEPTH: usize = 8;
/// Most files the local notes walk lists (a tree root of `$HOME` is huge).
pub const WALK_LIMIT: usize = 5000;

/// One picker row.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Item {
    pub label: String,
    pub detail: String,
    /// The file Enter opens, if any.
    pub path: Option<PathBuf>,
    /// 1-based source line to move the cursor to after opening.
    pub line: Option<usize>,
    /// Index into the current page's `doc.links` (links picker).
    pub link: Option<usize>,
}

/// A notebook operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Notes,
    Search,
    Tags,
    Backlinks,
    Links,
}

impl Op {
    pub const ALL: [Op; 5] = [Op::Notes, Op::Search, Op::Tags, Op::Backlinks, Op::Links];

    /// Name as used by the command line (`:Notes`) and picker titles.
    pub fn name(self) -> &'static str {
        match self {
            Op::Notes => "Notes",
            Op::Search => "Search",
            Op::Tags => "Tags",
            Op::Backlinks => "Backlinks",
            Op::Links => "Links",
        }
    }
}

/// What the current page's language server offers, for [`available`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Sources {
    /// A page is loaded.
    pub page: bool,
    /// The page's server kind, when it is running.
    pub server: Option<Kind>,
    /// The running server reports `referencesProvider`.
    pub references: bool,
    /// The page has at least one heading.
    pub heading: bool,
    /// The page is served by [mdroots](https://github.com/martintrojer/mdroots)
    /// (it has a path and no language server was selected).
    pub mdroots: bool,
}

/// Whether `op` has a source, else the status message to show.
pub fn available(op: Op, s: Sources) -> Result<(), &'static str> {
    let zk = s.server == Some(Kind::Zk);
    match op {
        Op::Notes => Ok(()),
        Op::Links if s.page => Ok(()),
        Op::Search | Op::Tags | Op::Backlinks if s.mdroots => Ok(()),
        Op::Links => Err("Links need an open file"),
        Op::Search if zk => Ok(()),
        Op::Search => Err("Search needs mdroots or a zk server"),
        Op::Tags if zk => Ok(()),
        Op::Tags => Err("Tags need mdroots or a zk server"),
        Op::Backlinks => match s.server {
            Some(k) if s.references && (k == Kind::Zk || s.heading) => Ok(()),
            _ => Err("Backlinks need mdroots or a language server"),
        },
    }
}

/// The links picker: every link of `doc`; label = visible text, detail =
/// destination as written.
pub fn link_items(doc: &Document) -> Vec<Item> {
    doc.links
        .iter()
        .enumerate()
        .map(|(i, l)| Item {
            label: doc.source.get(l.text_range.clone()).unwrap_or("").into(),
            detail: l.dest.clone(),
            link: Some(i),
            ..Item::default()
        })
        .collect()
}

/// The local notes fallback: markdown files below `root` up to
/// [`WALK_DEPTH`] (at most [`WALK_LIMIT`]), with the file tree's rules
/// (ignore files respected, dotfiles hidden). Label = file stem, detail =
/// path relative to `root`; sorted by detail.
pub fn walk_notes(root: &Path) -> Vec<Item> {
    let mut items: Vec<Item> = ignore::WalkBuilder::new(root)
        .max_depth(Some(WALK_DEPTH))
        .require_git(false)
        .hidden(true)
        .git_global(false)
        .build()
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()) && is_markdown(e.path()))
        .take(WALK_LIMIT)
        .map(|e| {
            let path = e.into_path();
            Item {
                label: stem(&path),
                detail: relative(&path, root),
                path: Some(path),
                ..Item::default()
            }
        })
        .collect();
    items.sort_by(|a, b| a.detail.cmp(&b.detail));
    items
}

pub(crate) fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `path` relative to `root`, comparing canonical forms (servers may
/// return `/private/tmp` for `/tmp`); absolute when outside.
pub(crate) fn relative(path: &Path, root: &Path) -> String {
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let (p, r) = (canon(path), canon(root));
    p.strip_prefix(&r).unwrap_or(&p).display().to_string()
}
