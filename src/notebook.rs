//! Notebook operations (notes, search, tags, backlinks, links) over the zk
//! adapter, standard LSP, or local fallbacks (spec § Notebook operations).
//! A page mdroots serves gets its items from `app::mdroots_glue` instead;
//! [`available`] knows that through [`Sources::mdroots`].
//!
//! Everything here is pure: it builds request parameters and turns replies
//! or local data into picker [`Item`]s. The app sends the requests and picks
//! the source in the spec's order: zk adapter, then the standard LSP request
//! (when the server reports the capability), then the local fallback.

use std::path::{Path, PathBuf};

use lsp_types::Position;
use serde_json::{Value, json};

use crate::doc::Document;
use crate::lsp::{Encoding, Kind, byte_to_position, canonical_uri, uri_to_path};
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
        Op::Search => Err("Search needs a zk notebook"),
        Op::Tags if zk => Ok(()),
        Op::Tags => Err("Tags needs a zk notebook"),
        Op::Backlinks => match s.server {
            Some(k) if s.references && (k == Kind::Zk || s.heading) => Ok(()),
            _ => Err("Backlinks need a language server"),
        },
    }
}

/// The zk adapter: one function per operation, each the parameters of a
/// `workspace/executeCommand` request ([`zk::EXECUTE`]) with the server
/// root as the notebook path (`arguments[0]`).
pub mod zk {
    use super::*;

    pub const EXECUTE: &str = "workspace/executeCommand";

    fn list(root: &Path, opts: Value) -> Value {
        json!({"command": "zk.list", "arguments": [root, opts]})
    }

    /// `zk.list {select: [title, absPath], sort: [modified]}`.
    pub fn notes(root: &Path) -> Value {
        list(
            root,
            json!({"select": ["title", "absPath"], "sort": ["modified"]}),
        )
    }

    /// `zk.list {select: [title, absPath], match: [q]}`; `q` passed unchanged.
    pub fn search(root: &Path, q: &str) -> Value {
        list(root, json!({"select": ["title", "absPath"], "match": [q]}))
    }

    /// `zk.tag.list`.
    pub fn tags(root: &Path) -> Value {
        json!({"command": "zk.tag.list", "arguments": [root]})
    }

    /// `zk.list {select: [title, absPath], tags: [t]}`.
    pub fn notes_by_tag(root: &Path, t: &str) -> Value {
        list(root, json!({"select": ["title", "absPath"], "tags": [t]}))
    }

    /// Items from a `zk.list` reply: label = `title` (file stem when
    /// empty), path = `absPath`, detail = the path relative to `root`.
    pub fn note_items(v: &Value, root: &Path) -> Vec<Item> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|n| {
                let path = PathBuf::from(n["absPath"].as_str()?);
                let title = n["title"].as_str().unwrap_or_default();
                Some(Item {
                    label: if title.is_empty() {
                        stem(&path)
                    } else {
                        title.to_string()
                    },
                    detail: relative(&path, root),
                    path: Some(path),
                    ..Item::default()
                })
            })
            .collect()
    }

    /// Items from a `zk.tag.list` reply: label = `name`, detail = `note_count`.
    pub fn tag_items(v: &Value) -> Vec<Item> {
        v.as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| {
                Some(Item {
                    label: t["name"].as_str()?.to_string(),
                    detail: t["note_count"]
                        .as_u64()
                        .map(|n| n.to_string())
                        .unwrap_or_default(),
                    ..Item::default()
                })
            })
            .collect()
    }
}

/// Where `textDocument/references` is sent for the backlinks of the
/// current page: 0:0 for zk (not on a link, so zk answers for the note);
/// otherwise the first heading, or `None` when there is none.
pub fn backlinks_position(kind: Kind, doc: &Document, enc: Encoding) -> Option<Position> {
    match kind {
        Kind::Zk => Some(Position::new(0, 0)),
        Kind::Marksman | Kind::Generic => {
            let h = doc.headings.first()?;
            Some(byte_to_position(&doc.source, h.range.start, enc))
        }
    }
}

/// Parameters of the backlinks `textDocument/references` request.
pub fn references_params(path: &Path, pos: Position) -> Value {
    json!({
        "textDocument": {"uri": canonical_uri(path)},
        "position": pos,
        "context": {"includeDeclaration": false},
    })
}

/// Items from a `references` reply (`Location[]`): label = the note's first
/// heading (else its file stem), detail = `<path relative to root>:<line>`,
/// line = 1-based `range.start.line + 1`. Only the line is used, so the
/// position encoding does not matter here.
pub fn location_items(v: &Value, root: &Path) -> Vec<Item> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|loc| {
            let uri = loc["uri"].as_str()?.parse().ok()?;
            let path = uri_to_path(&uri)?;
            let line = loc["range"]["start"]["line"].as_u64()? as usize + 1;
            Some(Item {
                label: first_heading(&path).unwrap_or_else(|| stem(&path)),
                detail: format!("{}:{line}", relative(&path, root)),
                path: Some(path),
                line: Some(line),
                link: None,
            })
        })
        .collect()
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

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `path` relative to `root`, comparing canonical forms (servers may
/// return `/private/tmp` for `/tmp`); absolute when outside.
fn relative(path: &Path, root: &Path) -> String {
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let (p, r) = (canon(path), canon(root));
    p.strip_prefix(&r).unwrap_or(&p).display().to_string()
}

/// The first heading of the file at `path` (front matter is metadata, not a
/// heading: see `doc`).
fn first_heading(path: &Path) -> Option<String> {
    let doc = crate::doc::from_bytes(&std::fs::read(path).ok()?)?;
    doc.headings.first().map(|h| h.text.clone())
}
