//! Links and history: follow, open external, back/forward.

use std::path::PathBuf;
use std::sync::Arc;

use super::{App, Effect};
use crate::doc::LinkKind;
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
            anchor: self.cursor_anchor(),
            sidebar: Some(self.sidebar_mode()),
            raw: p.raw,
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

    /// `gd` / `Enter` / `C-]`: follow the link under the cursor. The
    /// language server answers first (see `lsp_glue`), else the local parse.
    pub(super) fn follow(&mut self) {
        if !self.lsp_follow() {
            self.follow_local();
        }
    }

    /// Follow the link under the cursor using only the local parse.
    pub(super) fn follow_local(&mut self) {
        let code_path = self
            .link_under_cursor()
            .and_then(|i| self.page.as_ref()?.doc.links.get(i))
            .filter(|l| l.kind == LinkKind::CodePath)
            .cloned();
        if let Some(link) = code_path {
            self.follow_code_path(&link);
            return;
        }
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
                self.open_link_file(path, anchor, &as_written);
            }
        }
    }

    /// Open `path` as a followed link (history push), then jump to `anchor`.
    /// `as_written` names the file in status messages.
    pub(super) fn open_link_file(
        &mut self,
        path: PathBuf,
        anchor: Option<String>,
        as_written: &str,
    ) {
        let missing = || format!("No such file: {as_written}");
        if !path.is_file() {
            self.set_status(missing());
            return;
        }
        if !nav::is_markdown(&path) {
            self.pending_effect = Some(Effect::Edit { path, line: None });
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
    /// There is a page behind the current one.
    pub(crate) fn can_back(&self) -> bool {
        self.page.is_some() && self.history.depth() > 0
    }

    /// There is a page ahead of the current one.
    pub(crate) fn can_forward(&self) -> bool {
        self.page.is_some() && self.history.forward_depth() > 0
    }

    pub(super) fn back(&mut self) {
        if !self.can_back() {
            if self.page.is_some() {
                self.set_status("At the start of history");
            }
            return;
        }
        let Some(here) = self.entry() else { return };
        let Some(e) = self.history.back(here) else {
            return;
        };
        if let Err(msg) = self.restore(&e) {
            self.history.forward(e);
            self.set_status(msg);
        }
    }

    /// `C-i` / `Tab`.
    pub(super) fn forward(&mut self) {
        if !self.can_forward() {
            if self.page.is_some() {
                self.set_status("At the end of history");
            }
            return;
        }
        let Some(here) = self.entry() else { return };
        let Some(e) = self.history.forward(here) else {
            return;
        };
        if let Err(msg) = self.restore(&e) {
            self.history.back(e);
            self.set_status(msg);
        }
    }

    /// Show the entry's page with its cursor (by source byte) and scroll.
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
        if let Some(m) = e.sidebar
            && m != self.sidebar_mode()
        {
            self.set_sidebar_mode(m);
        }
        self.relayout_raw(e.raw);
        // The layout may differ from when the entry was made (resize,
        // sidebar width): place the cursor by its source byte, at the same
        // screen row.
        let old = super::Cursor {
            row: e.cursor_row,
            col: e.cursor_col,
        };
        match self.reanchor(old, e.anchor) {
            Some(c) if c != old => {
                self.cursor = c;
                self.want_col = c.col;
                let screen_row = e.cursor_row.saturating_sub(e.scroll);
                self.scroll = c.row.saturating_sub(screen_row).min(self.max_scroll());
            }
            _ => {
                self.cursor.row = e.cursor_row.min(self.last_row());
                self.set_col(e.cursor_col);
                self.scroll = e.scroll.min(self.max_scroll());
            }
        }
        self.keep_visible();
        Ok(())
    }
}
