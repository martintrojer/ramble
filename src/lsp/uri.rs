//! File URIs, canonicalized so different spellings of one path compare equal
//! (for example `/tmp` and `/private/tmp` on macOS).

use std::path::{Path, PathBuf};
use std::str::FromStr;

use lsp_types::Uri;

/// Canonicalizes `path` as far as the filesystem allows: the longest existing
/// ancestor is resolved and the missing tail appended unchanged.
pub(crate) fn canonical_path(path: &Path) -> PathBuf {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|d| d.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    if let Ok(p) = abs.canonicalize() {
        return p;
    }
    let mut tail = Vec::new();
    let mut cur = abs.as_path();
    while let Some(parent) = cur.parent() {
        if let Some(name) = cur.file_name() {
            tail.push(name.to_owned());
        }
        if let Ok(mut p) = parent.canonicalize() {
            p.extend(tail.iter().rev());
            return p;
        }
        cur = parent;
    }
    abs
}

/// The `file://` URI of the canonical form of `path`.
pub fn canonical_uri(path: &Path) -> Uri {
    let p = canonical_path(path);
    let url = url::Url::from_file_path(&p)
        .unwrap_or_else(|()| panic!("not an absolute path: {}", p.display()));
    Uri::from_str(url.as_str()).expect("url::Url produces a valid URI")
}

/// The canonical path of a `file://` URI; `None` for other schemes.
pub fn uri_to_path(uri: &Uri) -> Option<PathBuf> {
    let url = url::Url::parse(uri.as_str()).ok()?;
    if url.scheme() != "file" {
        return None;
    }
    let path = url.to_file_path().ok()?;
    Some(canonical_path(&path))
}
