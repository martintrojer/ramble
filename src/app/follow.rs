//! Links and history: follow, open external, back/forward.

use std::path::PathBuf;
use std::sync::Arc;

use super::{App, Effect};
use crate::nav::{self, Entry, PageRef, Target};

impl App {
    /// The current page and position as a history entry.
    pub(super) fn entry(&self) -> Option<Entry> {
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
    pub(super) fn link_dir(&self) -> PathBuf {
        match self.page.as_ref().and_then(|p| p.path.as_deref()) {
            Some(path) => match path.parent() {
                Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
                _ => PathBuf::from("."),
            },
            None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }

    /// The link under the cursor, resolved, plus its destination as written.
    pub(super) fn target_under_cursor(&mut self) -> Option<(Target, String)> {
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
    pub(super) fn follow(&mut self) {
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
                    self.pending_effect = Some(Effect::Edit(path));
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
    pub(super) fn open_external(&mut self) {
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
    pub(super) fn heading_row(&self, slug: &str) -> Option<usize> {
        let p = self.page.as_ref()?;
        let h = p.doc.headings.iter().find(|h| h.slug == slug).or_else(|| {
            let lower = slug.to_lowercase();
            p.doc.headings.iter().find(|h| h.slug == lower)
        })?;
        p.rendered.srcmap.row_for(h.range.start)
    }

    /// Cursor to `row`, column 0, scrolled to the top of the view.
    pub(super) fn jump_to_row(&mut self, row: usize) {
        self.goto_row(row);
        self.scroll = self.cursor.row.min(self.max_scroll());
    }

    /// `C-o` / `C-t`.
    pub(super) fn back(&mut self) {
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
    pub(super) fn forward(&mut self) {
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
    pub(super) fn restore(&mut self, e: &Entry) -> Result<(), String> {
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
}
