//! Page history (back and forward) and link destination resolution.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::doc::LinkKind;

/// Which page an entry shows. Stdin pages have no path, so their text is
/// kept in memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageRef {
    File(PathBuf),
    Stdin(Arc<String>),
}

/// One visited page and where the user was on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub page: PageRef,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub scroll: usize,
}

/// Browser-style history. The current page is not stored; callers pass it
/// to [`History::back`] and [`History::forward`].
#[derive(Debug, Clone, Default)]
pub struct History {
    back: Vec<Entry>,
    forward: Vec<Entry>,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the page being left by a follow. Clears the forward entries.
    pub fn push(&mut self, entry: Entry) {
        self.back.push(entry);
        self.forward.clear();
    }

    /// Step back: returns the entry to show, remembering `current` for
    /// [`History::forward`]. `None` (and no change) at the start.
    pub fn back(&mut self, current: Entry) -> Option<Entry> {
        let prev = self.back.pop()?;
        self.forward.push(current);
        Some(prev)
    }

    /// Step forward: the mirror of [`History::back`].
    pub fn forward(&mut self, current: Entry) -> Option<Entry> {
        let next = self.forward.pop()?;
        self.back.push(current);
        Some(next)
    }

    /// Entries behind the current one.
    pub fn depth(&self) -> usize {
        self.back.len()
    }

    /// Entries ahead of the current one.
    pub fn forward_depth(&self) -> usize {
        self.forward.len()
    }
}

/// Where a link destination points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    File {
        path: PathBuf,
        anchor: Option<String>,
    },
    /// A heading on the current page.
    Anchor(String),
    /// A URL to hand to the system opener.
    External(String),
}

/// Resolve a link destination as written against `current_dir` (the
/// current file's directory, or the working directory for stdin).
///
/// Wikilinks without an extension get `.md`; markdown links never do.
pub fn resolve(dest: &str, kind: &LinkKind, current_dir: &Path) -> Target {
    if let Some(anchor) = dest.strip_prefix('#') {
        return Target::Anchor(percent_decode(anchor));
    }
    let mut rest = dest;
    if let Some(scheme) = scheme(dest) {
        if !scheme.eq_ignore_ascii_case("file") {
            return Target::External(dest.to_string());
        }
        let after = &dest[scheme.len() + 1..];
        // file:///abs, file://localhost/abs, or file:/abs
        rest = match after.strip_prefix("//") {
            Some(s) => s.strip_prefix("localhost").unwrap_or(s),
            None => after,
        };
    }
    let (path, anchor) = match rest.split_once('#') {
        Some((p, a)) => (p, (!a.is_empty()).then(|| percent_decode(a))),
        None => (rest, None),
    };
    if path.is_empty() {
        return match anchor {
            Some(a) => Target::Anchor(a),
            None => Target::File {
                path: current_dir.to_path_buf(),
                anchor: None,
            },
        };
    }
    let mut path = percent_decode(path);
    if *kind == LinkKind::Wiki && Path::new(&path).extension().is_none() {
        path.push_str(".md");
    }
    Target::File {
        path: current_dir.join(path),
        anchor,
    }
}

/// RFC 3986 scheme (`ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )`) before the
/// first `:`, at least two characters so `C:` drive letters stay paths.
fn scheme(dest: &str) -> Option<&str> {
    let (s, _) = dest.split_once(':')?;
    let mut chars = s.chars();
    let ok = s.len() >= 2
        && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    ok.then_some(s)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(h << 4 | l);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}

/// Extensions ramble opens itself; anything else goes to the editor.
pub fn is_markdown(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        ["md", "markdown", "mdown", "mkd"]
            .iter()
            .any(|m| e.eq_ignore_ascii_case(m))
    })
}
