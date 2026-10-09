//! Connects [mdroots](https://github.com/martintrojer/mdroots) to the app:
//! the in-process backend for a page no configured `[[lsp.server]]` serves
//! (spec docs/specs/2026-10-08-mdroots-migration.md, S2). It gives the
//! page's link targets, broken-link dimming and `gd` targets, and the
//! `mdroots ○` / `mdroots ●` status label.
//!
//! One worker thread owns [`mdroots::Workspaces`] (built on the worker, on
//! the first request: its constructor opens the cache registry) and answers
//! [`Request`]s; the UI drains [`Reply`]s with [`App::pump_mdroots`] each
//! loop iteration, next to `pump_lsp`. Every mdroots call runs on the
//! worker. A request carries the tag `(page_id << 32) | version` like the
//! LSP glue; a reply for another page or an older version is dropped.
//!
//! For a page whose root has no ready workspace the worker first answers
//! from [`mdroots::Workspace::open_single`] (label `mdroots ○`, no broken
//! dimming: its index is incomplete), then opens the root with
//! [`mdroots::Workspaces::for_path`] and answers again (label `mdroots ●`).
//! Each reply replaces the page's targets and broken links.
//!
//! Limitation: the worker is one thread with no per-call cancellation, so
//! a slow `for_path` (a big root's first index) delays every later request,
//! even for pages in other roots. Queued page requests are coalesced to the
//! latest. Dropping the app cancels the shared [`mdroots::Cancel`] so a
//! running open stops; the worker is never joined.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use mdroots::{Cancel, DocLink, IndexMode, LinkStatus, RootMode, Workspace, Workspaces};

use super::App;
use crate::doc::LinkKind;

/// How the mdroots backend stores its index; converted to
/// [`mdroots::Options`] on the worker. There is deliberately no `Default`:
/// every caller says where the cache goes (tests never touch the user's).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdrootsOptions {
    /// Cache dir instead of the user's (`MDROOTS_CACHE_DIR`).
    pub cache_dir: Option<PathBuf>,
    /// In memory only: no cache dir is touched.
    pub memory: bool,
}

impl MdrootsOptions {
    /// mdroots' own cache policy and the user's cache dir.
    pub fn user_cache() -> Self {
        Self {
            cache_dir: None,
            memory: false,
        }
    }

    /// In memory only.
    pub fn memory() -> Self {
        Self {
            cache_dir: None,
            memory: true,
        }
    }

    /// Cache in `dir`.
    pub fn cache_dir(dir: PathBuf) -> Self {
        Self {
            cache_dir: Some(dir),
            memory: false,
        }
    }

    fn to_mdroots(&self, cancel: Cancel) -> mdroots::Options {
        let mut o = mdroots::Options::default().cancel(cancel);
        if self.memory {
            o = o.index(IndexMode::Memory);
        }
        if let Some(d) = &self.cache_dir {
            o = o.cache_dir(d.clone());
        }
        o
    }
}

/// A page to answer for.
struct Request {
    tag: u64,
    path: PathBuf,
    /// The source ramble shows, set as the overlay while answering.
    text: String,
    /// Paths changed on disk, refreshed in a ready root workspace first.
    refresh: Vec<PathBuf>,
}

/// One answer for a page.
pub(super) struct Reply {
    tag: u64,
    /// From the root's workspace (`for_path`), not `open_single`.
    root: bool,
    /// The serving workspace is single-file (its index is the page only).
    single: bool,
    links: Result<Vec<DocLink>, String>,
}

/// Which workspace served the current page's last applied reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Serving {
    Single,
    Root,
}

/// mdroots state owned by [`App`].
pub(super) struct MdrootsState {
    opts: MdrootsOptions,
    cancel: Cancel,
    /// Started by the first request.
    tx: Option<Sender<Request>>,
    reply_tx: Sender<Reply>,
    rx: Receiver<Reply>,
    /// Bumped per request; with the page id it makes the tag.
    version: u32,
    // Per page, reset by `mdroots_reset`.
    /// The current page is served by mdroots (no LSP server selected).
    active: bool,
    /// Tag of the request in flight for this page.
    tag: Option<u64>,
    serving: Option<Serving>,
    /// When the request in flight was sent; cleared by the root reply.
    sent_at: Option<Instant>,
}

impl MdrootsState {
    pub(super) fn new(opts: MdrootsOptions) -> Self {
        let (reply_tx, rx) = mpsc::channel();
        Self {
            opts,
            cancel: Cancel::new(),
            tx: None,
            reply_tx,
            rx,
            version: 0,
            active: false,
            tag: None,
            serving: None,
            sent_at: None,
        }
    }

    fn send(&mut self, req: Request) {
        let tx = self.tx.get_or_insert_with(|| {
            let (tx, rx) = mpsc::channel();
            let opts = self.opts.to_mdroots(self.cancel.clone());
            let cancel = self.cancel.clone();
            let out = self.reply_tx.clone();
            std::thread::spawn(move || worker(opts, cancel, rx, out));
            tx
        });
        let _ = tx.send(req);
    }
}

impl Drop for MdrootsState {
    fn drop(&mut self) {
        // Stops a running discovery or index; the worker then sees its
        // channel closed and exits.
        self.cancel.cancel();
    }
}

/// The worker loop: answer the latest queued page, until the app is gone.
fn worker(opts: mdroots::Options, cancel: Cancel, rx: Receiver<Request>, out: Sender<Reply>) {
    let workspaces = Workspaces::new(opts.clone());
    while let Ok(mut req) = rx.recv() {
        // Coalesce: only the latest page matters, but keep every refresh.
        while let Ok(next) = rx.try_recv() {
            let mut refresh = std::mem::take(&mut req.refresh);
            refresh.extend(next.refresh.iter().cloned());
            req = next;
            req.refresh = refresh;
        }
        if cancel.is_cancelled() {
            return;
        }
        let path = canonical(&req.path);
        let refresh: Vec<PathBuf> = req.refresh.iter().map(|p| canonical(p)).collect();
        let send = |ws: &Workspace, root: bool| {
            let reply = Reply {
                tag: req.tag,
                root,
                single: ws.root().mode == RootMode::SingleFile,
                links: answer(ws, &path, &req.text),
            };
            out.send(reply).is_ok()
        };
        if let Some(ws) = workspaces.get(&path) {
            if !refresh.is_empty() {
                let _ = ws.refresh_paths(&refresh, &cancel);
            }
            if !send(&ws, true) {
                return;
            }
            continue;
        }
        if let Ok(single) = Workspace::open_single(&path, opts.clone())
            && !send(&single, false)
        {
            return;
        }
        // A freshly opened root has read every note: no refresh needed.
        let ok = match workspaces.for_path(&path) {
            Ok(ws) => send(&ws, true),
            Err(e) => out
                .send(Reply {
                    tag: req.tag,
                    root: true,
                    single: true,
                    links: Err(e.to_string()),
                })
                .is_ok(),
        };
        if !ok {
            return;
        }
    }
}

/// The page's links in `ws`, read with ramble's text as the overlay. The
/// overlay is cleared again before returning, so it never outlives the
/// request (the page may be left at any time).
fn answer(ws: &Workspace, path: &Path, text: &str) -> Result<Vec<DocLink>, String> {
    let overlaid = ws.set_overlay(path, text).is_ok();
    let links = ws.document_links(path).map_err(|e| e.to_string());
    if overlaid {
        let _ = ws.clear_overlay(path);
    }
    links
}

/// `p` canonical; a missing file by its canonical folder and name
/// (`refresh_paths` does not canonicalize).
fn canonical(p: &Path) -> PathBuf {
    if let Ok(c) = p.canonicalize() {
        return c;
    }
    match (
        p.parent().and_then(|d| d.canonicalize().ok()),
        p.file_name(),
    ) {
        (Some(d), Some(n)) => d.join(n),
        _ => p.to_path_buf(),
    }
}

impl App {
    /// Forget the previous page's mdroots state (`lsp_page_changed`).
    pub(super) fn mdroots_reset(&mut self) {
        let m = &mut self.mdroots;
        m.active = false;
        m.tag = None;
        m.serving = None;
        m.sent_at = None;
    }

    /// The current page (at `path`) has no LSP server: ask mdroots. The
    /// page itself is refreshed in a ready root workspace first, so a
    /// reload (watcher, `C-l`) or a revisit sees the file as it is now.
    pub(super) fn mdroots_page_changed(&mut self, path: PathBuf) {
        self.mdroots.active = true;
        self.mdroots_request(path.clone(), vec![path]);
    }

    /// The watcher reported `path` (not the current page) changed: refresh
    /// it and re-ask the current page's links, e.g. a new note that a
    /// `[[Title]]` link on the page names.
    pub(super) fn mdroots_path_changed(&mut self, path: &Path) {
        if !self.mdroots.active {
            return;
        }
        let Some(page) = self.page.as_ref().and_then(|p| p.path.clone()) else {
            return;
        };
        self.mdroots_request(page, vec![path.to_path_buf()]);
    }

    fn mdroots_request(&mut self, path: PathBuf, refresh: Vec<PathBuf>) {
        let Some(page) = &self.page else { return };
        let page_id = self.page_id();
        let m = &mut self.mdroots;
        m.version = m.version.wrapping_add(1);
        let tag = (page_id << 32) | u64::from(m.version);
        m.tag = Some(tag);
        m.sent_at = Some(self.now);
        let text = page.doc.source.clone();
        m.send(Request {
            tag,
            path,
            text,
            refresh,
        });
    }

    /// Apply pending mdroots replies, blocking up to `timeout` for the
    /// first. Returns how many were applied. `run` calls it every loop
    /// iteration.
    pub fn pump_mdroots(&mut self, timeout: Duration) -> usize {
        let first = match self.mdroots.rx.recv_timeout(timeout) {
            Ok(r) => r,
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return 0,
        };
        self.mdroots_reply(first);
        let mut n = 1;
        while let Ok(r) = self.mdroots.rx.try_recv() {
            self.mdroots_reply(r);
            n += 1;
        }
        n
    }

    pub(super) fn mdroots_reply(&mut self, r: Reply) {
        if !self.mdroots.active || self.mdroots.tag != Some(r.tag) {
            return; // Another page or an older request: drop.
        }
        if r.root {
            self.mdroots.sent_at = None;
        }
        let Ok(links) = r.links else {
            if r.root && self.mdroots.serving.is_none() {
                self.mdroots.active = false; // Nothing serves this page.
            }
            return;
        };
        self.mdroots.serving = Some(if r.root {
            Serving::Root
        } else {
            Serving::Single
        });
        // Broken dimming only from the root's workspace: a single-file
        // index calls a `[[Title]]` it cannot see broken. (A root mdroots
        // itself opens single-file, e.g. a denied location, never dims.)
        let dim = r.root && !r.single;
        self.apply_doc_links(&links, dim);
        if let Some((from, at)) = self.lsp_backlink() {
            if self.cursor != at {
                self.lsp_await_backlink_clear();
            } else if self.land_on_link_to(&from) || r.root {
                // Landed, or the root's answer did not have it either.
                self.lsp_await_backlink_clear();
            }
        }
    }

    /// Replace the page's targets and broken links with `links`, matched
    /// to ramble's links by exact range. A DocLink with no equal ramble
    /// link is ignored (images, code mentions, code blocks); a ramble link
    /// with none gets no target and `gd` resolves it locally. That includes
    /// links in an unfenced header (`Title: x [l](a)` at the top): ramble
    /// parses them, mdroots' workspace parse does not.
    fn apply_doc_links(&mut self, links: &[DocLink], dim: bool) {
        let Some(page) = &self.page else { return };
        let mut targets = HashMap::new();
        let mut anchors = HashMap::new();
        let mut broken = BTreeSet::new();
        for dl in links {
            let Some(i) = page
                .doc
                .links
                .iter()
                .position(|l| l.kind != LinkKind::CodePath && l.range == dl.range)
            else {
                continue;
            };
            if let Some(t) = &dl.target {
                targets.insert(i, t.clone());
                if let Some(a) = &dl.anchor {
                    anchors.insert(i, a.clone());
                }
            }
            if dim && dl.status == LinkStatus::Broken {
                broken.insert(i);
            }
        }
        self.lsp_set_links(targets, anchors, broken);
    }

    /// The mdroots status label, if mdroots is meant to serve the page:
    /// `—` before its first answer, `mdroots ○` from the single-file
    /// workspace, `mdroots ●` from the root's.
    pub(super) fn mdroots_label(&self) -> Option<String> {
        let m = &self.mdroots;
        if !m.active {
            return None;
        }
        Some(match m.serving {
            None => "—".into(),
            Some(Serving::Single) => "mdroots ○".into(),
            Some(Serving::Root) => "mdroots ●".into(),
        })
    }

    /// How long the mdroots request in flight has waited.
    pub(super) fn mdroots_waiting(&self) -> Option<Duration> {
        let m = &self.mdroots;
        m.active
            .then_some(m.sent_at)
            .flatten()
            .map(|t| self.now.saturating_duration_since(t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{StartOptions, StartTarget};

    fn app(dir: &Path, text: &str) -> App {
        let path = dir.join("a.md");
        std::fs::write(&path, text).unwrap();
        std::fs::write(dir.join("b.md"), "# B\n").unwrap();
        App::new(
            StartOptions {
                target: StartTarget::File(path),
                tree_root: dir.to_path_buf(),
                config: crate::config::Config {
                    lsp: crate::config::LspConfig { server: vec![] },
                    ..Default::default()
                },
                review_cache: None,
                mdroots: MdrootsOptions::memory(),
            },
            (40, 10),
        )
        .unwrap()
    }

    /// Real DocLinks for the page (DocLink is non_exhaustive), from an
    /// in-memory single-file workspace.
    fn real_links(dir: &Path) -> Vec<DocLink> {
        let ws = Workspace::open_single(
            &dir.join("a.md"),
            mdroots::Options::default().index(IndexMode::Memory),
        )
        .unwrap();
        ws.document_links(&dir.join("a.md")).unwrap()
    }

    #[test]
    fn a_reply_for_an_older_page_does_not_change_targets() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), "# A\n\n[b](b)\n");
        let old = app.mdroots.tag.unwrap();
        // A new page visit: the earlier request's replies are stale.
        app.reload();
        let links = real_links(dir.path());
        assert!(links[0].target.is_some(), "the fixture link resolves");
        app.mdroots_reply(Reply {
            tag: old,
            root: true,
            single: false,
            links: Ok(links.clone()),
        });
        assert_eq!(app.link_target(0), None);
        assert_eq!(app.lsp_label(), "—");
        let cur = app.mdroots.tag.unwrap();
        app.mdroots_reply(Reply {
            tag: cur,
            root: false,
            single: true,
            links: Ok(links),
        });
        assert!(app.link_target(0).is_some());
        assert_eq!(app.lsp_label(), "mdroots ○");
    }

    #[test]
    fn a_pending_backlink_survives_a_single_file_reply_and_lands_on_the_root_reply() {
        let dir = tempfile::tempdir().unwrap();
        // `[[Note B]]` names b.md by title: the local parse cannot land on it.
        let mut app = app(dir.path(), "# A\n\ntext\n\nSee [[Note B]].\n");
        std::fs::write(dir.path().join("b.md"), "# Note B\n").unwrap();
        let b = dir.path().canonicalize().unwrap().join("b.md");
        assert!(!app.land_on_link_to(&b));
        app.lsp_await_backlink(b.clone(), app.cursor);
        let tag = app.mdroots.tag.unwrap();
        let a = dir.path().join("a.md");
        let single = real_links(dir.path());
        assert_eq!(single[0].target, None, "single-file cannot see titles");
        app.mdroots_reply(Reply {
            tag,
            root: false,
            single: true,
            links: Ok(single),
        });
        assert!(app.lsp_backlink().is_some(), "kept for the root reply");
        assert_eq!(app.cursor.row, 0);
        let ws = Workspace::open_at(
            dir.path(),
            mdroots::Options::default().index(IndexMode::Memory),
        )
        .unwrap();
        let rooted = ws.document_links(&a).unwrap();
        assert_eq!(rooted[0].target.as_deref(), Some(b.as_path()));
        app.mdroots_reply(Reply {
            tag,
            root: true,
            single: false,
            links: Ok(rooted),
        });
        assert!(app.lsp_backlink().is_none());
        assert_eq!(app.link_under_cursor(), Some(0), "landed on the link");
    }
}
