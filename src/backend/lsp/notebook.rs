//! The zk adapter and the LSP-only notebook helpers: request parameters
//! for zk's commands and backlinks `textDocument/references`, and the
//! picker [`Item`]s from their replies (docs/design.md § Notebook operations).

use std::path::{Path, PathBuf};

use lsp_types::Position;
use serde_json::{Value, json};

use super::{Encoding, Kind, byte_to_position, canonical_uri, uri_to_path};
use crate::doc::Document;
use crate::notebook::{Item, relative, stem};

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

/// The first heading of the file at `path` (front matter is metadata, not a
/// heading: see `doc`).
fn first_heading(path: &Path) -> Option<String> {
    let doc = crate::doc::from_bytes(&std::fs::read(path).ok()?)?;
    doc.headings.first().map(|h| h.text.clone())
}
