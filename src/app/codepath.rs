//! Code-path links: inline code spans whose text names an existing file
//! (`templates/Model.lean`, `~/x.rs:12`, `/etc/hosts:3:1`) become links.
//! Checked once per page load or reload; nothing else is a link.

use std::path::{Component, Path, PathBuf};

use super::{App, Effect};
use crate::doc::{Document, Link, LinkKind};
use crate::nav;

/// Longer code spans are never paths.
const MAX_LEN: usize = 512;

/// Resolve code text to an existing regular file and an optional line.
/// Relative paths are tried against each of `dirs` in order; `~/` uses
/// `home`. The full text is tried first, then with a trailing `:LINE:COL`
/// or `:LINE` stripped (COL is dropped).
pub fn resolve(
    text: &str,
    dirs: &[PathBuf],
    home: Option<&Path>,
) -> Option<(PathBuf, Option<usize>)> {
    let text = text.trim();
    if text.is_empty()
        || text.len() > MAX_LEN
        || text.contains(char::is_whitespace)
        || text.contains("://")
    {
        return None;
    }
    let find = |p: &str| locate(p, dirs, home);
    if let Some(path) = find(text) {
        return Some((path, None));
    }
    let (rest, line) = strip_position(text)?;
    find(rest).map(|path| (path, Some(line)))
}

/// `path:LINE:COL` or `path:LINE` (positive integers) to `(path, LINE)`.
fn strip_position(text: &str) -> Option<(&str, usize)> {
    let num = |s: &str| {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse::<usize>().ok())
            .flatten()
            .filter(|&n| n > 0)
    };
    let (head, last) = text.rsplit_once(':')?;
    num(last)?;
    if let Some((path, line)) = head.rsplit_once(':')
        && let Some(line) = num(line)
        && !path.is_empty()
    {
        return Some((path, line));
    }
    let line = num(last)?;
    (!head.is_empty()).then_some((head, line))
}

fn locate(text: &str, dirs: &[PathBuf], home: Option<&Path>) -> Option<PathBuf> {
    let candidates: Vec<PathBuf> = if let Some(rest) = text.strip_prefix("~/") {
        home.map(|h| h.join(rest)).into_iter().collect()
    } else if text.starts_with('~') {
        Vec::new()
    } else if Path::new(text).is_absolute() {
        vec![PathBuf::from(text)]
    } else {
        dirs.iter().map(|d| d.join(text)).collect()
    };
    candidates
        .into_iter()
        .map(|p| clean(&p))
        .find(|p| p.is_file())
}

/// Drop `.` components (keeps `..`: symlinks make folding it unsafe).
fn clean(p: &Path) -> PathBuf {
    p.components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect()
}

impl App {
    /// Directories relative code paths resolve against, in order: the
    /// page's directory (cwd for stdin), its VCS root, the tree root.
    fn code_path_dirs(&self, path: Option<&Path>) -> Vec<PathBuf> {
        let dir = match path.and_then(Path::parent) {
            Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
            Some(_) => PathBuf::from("."),
            None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        };
        let mut dirs = vec![dir.clone()];
        dirs.extend(super::vcs_root(&dir));
        dirs.push(self.tree_root.clone());
        let mut seen = Vec::new();
        dirs.retain(|d| {
            let key = std::path::absolute(d).unwrap_or_else(|_| d.clone());
            let new = !seen.contains(&key);
            seen.push(key);
            new
        });
        dirs
    }

    /// Turn the page's code spans that name an existing file into
    /// [`LinkKind::CodePath`] links. Called before the page is rendered.
    pub(super) fn add_code_path_links(&self, path: Option<&Path>, doc: &mut Document) {
        if doc.code_spans.is_empty() {
            return;
        }
        let dirs = self.code_path_dirs(path);
        let home = (self.env)("HOME").map(PathBuf::from);
        let links: Vec<Link> = doc
            .code_spans
            .iter()
            .filter_map(|c| {
                let (resolved, line) = resolve(&c.text, &dirs, home.as_deref())?;
                Some(Link {
                    kind: LinkKind::CodePath,
                    dest: c.text.trim().to_string(),
                    range: c.range.clone(),
                    text_range: c.text_range.clone(),
                    resolved: Some(resolved),
                    line,
                })
            })
            .collect();
        doc.add_links(links);
    }

    /// Follow a code-path link: markdown opens here at its line, anything
    /// else goes to the editor with the line.
    pub(super) fn follow_code_path(&mut self, link: &Link) {
        let Some(path) = link.resolved.clone().filter(|p| p.is_file()) else {
            self.set_status(format!("No such file: {}", link.dest));
            return;
        };
        if !nav::is_markdown(&path) {
            self.pending_effect = Some(Effect::Edit {
                path,
                line: link.line,
            });
            return;
        }
        self.open_link_file(path, None, &link.dest);
        let Some(line) = link.line else { return };
        let Some(p) = self.page.as_ref() else { return };
        if p.path.as_deref() != link.resolved.as_deref() {
            return; // Did not open (binary, unreadable).
        }
        let lines = &p.rendered.source_lines;
        match lines.iter().position(|&l| l >= line) {
            Some(row) => self.jump_to_row(row),
            None => {
                self.jump_to_row(self.last_row());
                self.set_status(format!("Line {line} is past the end"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_suffixes() {
        assert_eq!(strip_position("a.rs:12"), Some(("a.rs", 12)));
        assert_eq!(strip_position("a.rs:12:3"), Some(("a.rs", 12)));
        assert_eq!(strip_position("a.rs:0"), None);
        assert_eq!(strip_position("a.rs:x"), None);
        assert_eq!(strip_position("a.rs:"), None);
        assert_eq!(strip_position(":3"), None);
        assert_eq!(strip_position("a.rs"), None);
    }

    #[test]
    fn clean_drops_dot_components() {
        assert_eq!(clean(Path::new("d/./a/b.rs")), PathBuf::from("d/a/b.rs"));
        assert_eq!(clean(Path::new("./a")), PathBuf::from("a"));
    }
}
