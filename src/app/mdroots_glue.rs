//! Connects [mdroots](https://github.com/martintrojer/mdroots) to the app:
//! the in-process backend for a page no configured `[[lsp.server]]` serves
//! (design: docs/specs/2026-10-08-mdroots-migration.md). It gives the
//! page's link targets, broken-link dimming and `gd` targets, the
//! `mdroots ○` / `mdroots ●` status label, the notes, search, tags and
//! backlinks pickers' items, and the `K` preview of a link's target note.
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
//! Other notes changing on disk (e.g. a new note a `[[Title]]` link on the
//! page names) come from mdroots' own watcher: the worker opens with
//! [`mdroots::Options::watch`], subscribes to each root workspace it
//! answers from, and re-answers the current page when a change lands under
//! that root. mdroots watches only roots it reconciles in a DB on a local
//! disk; in a lazy or single-file workspace (and in memory mode) other
//! notes' changes show once the page is reopened or reloaded (`R`).
//! ramble's own watcher still reloads the page itself, which refreshes it.
//!
//! A picker request carries the picker's seq and is answered from the
//! root workspace of the page (`for_path`: the cached one, or opened then;
//! on the one worker thread that is the open the page's request started).
//! The worker builds the picker [`Item`]s; the picker drops an answer for
//! another seq. Only marker and VCS roots are complete. Any other root
//! (loose at the page's folder, lazy working set, single file, ...) answers
//! search, tags and backlinks as a partial list (the picker title says so);
//! the notes picker walks the tree root instead.
//!
//! A preview request (`K`) carries its own seq, never the page tag, so it
//! cannot make the page's reply stale. It is answered from the root
//! workspace of the TARGET (`for_path(target)`: a note in another root,
//! or a lone file, gets its own workspace); the page's `open_single`
//! workspace indexes the page only. Like the LSP hover, its reply is
//! dropped only when the page changed (or a newer `K` replaced it), not
//! when the cursor moved.
//!
//! Limitation: the worker is one thread with no per-call cancellation, so
//! a slow `for_path` (a big root's first index) delays every later request,
//! even for pages in other roots (a picker shows loading meanwhile).
//! Queued page requests are coalesced to the latest; picker and preview
//! requests are answered in order, never dropped. Dropping the app cancels the shared
//! [`mdroots::Cancel`] so a running open or search stops; the worker is
//! never joined.

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use mdroots::{
    Cancel, DocLink, IndexMode, LinkStatus, NoteSummary, RootMode, Workspace, Workspaces,
};

use super::App;
use crate::doc::LinkKind;
use crate::nav;
use crate::notebook::{self, Item};

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
        let mut o = mdroots::Options::default()
            .cancel(cancel)
            .watch(!self.memory);
        if self.memory {
            o = o.index(IndexMode::Memory);
        }
        if let Some(d) = &self.cache_dir {
            o = o.cache_dir(d.clone());
        }
        o
    }
}

/// A request to the worker. Page requests are coalesced to the latest;
/// picker and preview requests are answered in order, each exactly once.
enum Request {
    Page(PageRequest),
    Picker(PickerRequest),
    Preview(PreviewRequest),
}

/// The `K` preview of a link's target note.
struct PreviewRequest {
    /// Matched against [`MdrootsState::preview`] by the reply.
    seq: u64,
    target: PathBuf,
}

/// A page to answer for.
struct PageRequest {
    tag: u64,
    path: PathBuf,
    /// The source ramble shows, set as the overlay while answering.
    text: String,
    /// Paths changed on disk, refreshed in a ready root workspace first.
    refresh: Vec<PathBuf>,
}

/// A picker's items, from the root workspace of `path` (the page).
struct PickerRequest {
    /// The picker's seq (see `picker.rs`).
    seq: u64,
    path: PathBuf,
    query: PickerQuery,
}

/// What a picker asks mdroots for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PickerQuery {
    /// Every note, newest first. A workspace that indexes a working set
    /// only (lazy or single-file) would list a fraction: the file walk of
    /// `tree_root` answers instead.
    Notes { tree_root: PathBuf },
    /// [`Workspace::full_text`]: mdroots' query syntax, passed unchanged.
    Search(String),
    /// Tags, merged case-insensitively.
    Tags,
    /// The notes carrying a tag (case-insensitive), newest first.
    NotesWithTag(String),
    /// Links to the page.
    Backlinks,
}

/// A picker's answer: items built on the worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PickerAnswer {
    /// The root the details are relative to.
    pub(super) root: PathBuf,
    pub(super) items: Vec<Item>,
    /// From a workspace that indexes a working set only: the items may be
    /// incomplete.
    pub(super) partial: bool,
}

/// A worker answer.
pub(super) enum Reply {
    Page(PageReply),
    Picker {
        seq: u64,
        result: Result<PickerAnswer, String>,
    },
    /// The popup text for preview `seq`.
    Preview {
        seq: u64,
        result: Result<String, String>,
    },
}

/// One answer for a page.
pub(super) struct PageReply {
    tag: u64,
    /// From the root's workspace (`for_path`), not `open_single`.
    root: bool,
    /// The serving workspace is single-file (its index is the page only).
    single: bool,
    /// The serving workspace runs mdroots' watcher.
    watching: bool,
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
    /// Bumped per preview request.
    preview_seq: u64,
    /// The preview in flight: the page id it was asked on and its seq.
    preview: Option<(u64, u64)>,
    // Per page, reset by `mdroots_reset`.
    /// The current page is served by mdroots (no LSP server selected).
    active: bool,
    /// Tag of the request in flight for this page.
    tag: Option<u64>,
    serving: Option<Serving>,
    /// When the request in flight was sent; cleared by the root reply.
    sent_at: Option<Instant>,
    /// The last applied reply's workspace runs mdroots' watcher.
    watching: bool,
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
            preview_seq: 0,
            preview: None,
            active: false,
            tag: None,
            serving: None,
            sent_at: None,
            watching: false,
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

/// How often the worker checks its root workspaces' change subscriptions
/// while no page request arrives.
const POLL: Duration = Duration::from_millis(100);

/// The worker loop: answer the latest queued page and every queued picker
/// request, and re-answer the page when mdroots' watcher reports changes
/// under its root, until the app is gone.
fn worker(opts: mdroots::Options, cancel: Cancel, rx: Receiver<Request>, out: Sender<Reply>) {
    let workspaces = Workspaces::new(opts.clone());
    // One change subscription per root workspace answered from, by root.
    let mut subs: HashMap<PathBuf, Receiver<Vec<PathBuf>>> = HashMap::new();
    // The current page's request, re-answered on changes under its root.
    let mut last: Option<PageRequest> = None;
    loop {
        // Block while there is nothing to watch; otherwise poll both.
        let next = if subs.is_empty() {
            rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
        } else {
            rx.recv_timeout(POLL)
        };
        let first = match next {
            Ok(req) => req,
            Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {
                let Some(req) = &last else { continue };
                let path = canonical(&req.path);
                let Some(ws) = workspaces.get(&path) else {
                    continue;
                };
                let root = &ws.root().path;
                // Drain every batch: each is a refresh of this root.
                let changed = subs.get(root).is_some_and(|sub| sub.try_iter().count() > 0);
                if changed && !send_page(&out, &ws, req, &path, true) {
                    return;
                }
                continue;
            }
        };
        // Coalesce pages: only the latest matters, but keep every refresh.
        // Picker and preview requests are kept in order.
        let mut page: Option<PageRequest> = None;
        let mut others = Vec::new();
        for req in std::iter::once(first).chain(std::iter::from_fn(|| rx.try_recv().ok())) {
            match req {
                Request::Page(mut next) => {
                    if let Some(prev) = page.take() {
                        let mut refresh = prev.refresh;
                        refresh.append(&mut next.refresh);
                        next.refresh = refresh;
                    }
                    page = Some(next);
                }
                other => others.push(other),
            }
        }
        if cancel.is_cancelled() {
            return;
        }
        // The page first: it opens the root a picker on it needs anyway.
        if let Some(mut req) = page {
            if !answer_page(&workspaces, &opts, &cancel, &mut subs, &mut req, &out) {
                return;
            }
            last = Some(req);
        }
        for req in others {
            let reply = match req {
                Request::Picker(p) => Reply::Picker {
                    seq: p.seq,
                    result: picker_answer(&workspaces, &cancel, &p),
                },
                Request::Preview(p) => Reply::Preview {
                    seq: p.seq,
                    result: preview_answer(&workspaces, &p.target),
                },
                Request::Page(_) => unreachable!("pages are coalesced above"),
            };
            if out.send(reply).is_err() {
                return;
            }
        }
    }
}

/// Send the page's links from `ws`. False when the app is gone.
fn send_page(
    out: &Sender<Reply>,
    ws: &Workspace,
    req: &PageRequest,
    path: &Path,
    root: bool,
) -> bool {
    let reply = PageReply {
        tag: req.tag,
        root,
        single: ws.root().mode == RootMode::SingleFile,
        watching: ws.watching(),
        links: answer(ws, path, &req.text),
    };
    out.send(Reply::Page(reply)).is_ok()
}

/// Answer a page: from its ready root workspace (refreshing the changed
/// paths first), else from `open_single` and then the freshly opened
/// root. False when the app is gone.
fn answer_page(
    workspaces: &Workspaces,
    opts: &mdroots::Options,
    cancel: &Cancel,
    subs: &mut HashMap<PathBuf, Receiver<Vec<PathBuf>>>,
    req: &mut PageRequest,
    out: &Sender<Reply>,
) -> bool {
    let path = canonical(&req.path);
    let refresh: Vec<PathBuf> = std::mem::take(&mut req.refresh)
        .iter()
        .map(|p| canonical(p))
        .collect();
    let ws = if let Some(ws) = workspaces.get(&path) {
        if !refresh.is_empty() {
            let _ = ws.refresh_paths(&refresh, cancel);
        }
        Ok(ws)
    } else {
        if let Ok(single) = Workspace::open_single(&path, opts.clone())
            && !send_page(out, &single, req, &path, false)
        {
            return false;
        }
        // A freshly opened root has read every note: no refresh needed.
        workspaces.for_path(&path)
    };
    match ws {
        Ok(ws) => {
            subs.entry(ws.root().path.clone())
                .or_insert_with(|| ws.subscribe());
            send_page(out, &ws, req, &path, true)
        }
        Err(e) => out
            .send(Reply::Page(PageReply {
                tag: req.tag,
                root: true,
                single: true,
                watching: false,
                links: Err(e.to_string()),
            }))
            .is_ok(),
    }
}

/// Most hits a search lists.
const SEARCH_LIMIT: usize = 200;

/// Whether a root of this mode indexes every note under it. Loose roots may
/// sit at the page's folder, lazy ones index a working set, and a single
/// file is just the page.
fn is_complete(mode: RootMode) -> bool {
    matches!(mode, RootMode::Marker | RootMode::Vcs)
}

/// A picker's items from the root workspace of the page (opened here if
/// the page's own request has not opened it). Shapes match the zk
/// adapter's (label = title, detail = path relative to the root) except
/// that search hits carry their line, so Enter lands on the hit.
fn picker_answer(
    workspaces: &Workspaces,
    cancel: &Cancel,
    req: &PickerRequest,
) -> Result<PickerAnswer, String> {
    let path = canonical(&req.path);
    let ws = workspaces.for_path(&path).map_err(|e| e.to_string())?;
    let root = ws.root().path;
    let partial = !is_complete(ws.root().mode);
    let rel = |p: &Path| p.strip_prefix(&root).unwrap_or(p).display().to_string();
    let note_items = |notes: Vec<NoteSummary>| -> Vec<Item> {
        newest_first(notes)
            .into_iter()
            .map(|n| Item {
                label: title_or_stem(&n.title, &n.path),
                detail: rel(&n.path),
                path: Some(n.path),
                ..Item::default()
            })
            .collect()
    };
    let items = match &req.query {
        PickerQuery::Notes { tree_root } if partial => {
            return Ok(PickerAnswer {
                root: tree_root.clone(),
                items: notebook::walk_notes(tree_root),
                partial: false,
            });
        }
        PickerQuery::Notes { .. } => note_items(ws.notes()),
        PickerQuery::NotesWithTag(tag) => note_items(ws.notes_with_tag(tag)),
        PickerQuery::Search(q) => {
            let hits = ws
                .full_text(q, SEARCH_LIMIT, cancel)
                .map_err(|e| e.to_string())?;
            let titles: HashMap<PathBuf, String> =
                ws.notes().into_iter().map(|n| (n.path, n.title)).collect();
            hits.into_iter()
                .map(|h| {
                    let line = h.line as usize + 1; // Hit.line is 0-based.
                    let title = titles.get(&h.path).map_or("", String::as_str);
                    Item {
                        label: title_or_stem(title, &h.path),
                        detail: format!("{}:{line}", rel(&h.path)),
                        path: Some(h.path),
                        line: Some(line),
                        link: None,
                    }
                })
                .collect()
        }
        PickerQuery::Tags => merge_tags(ws.tags())
            .into_iter()
            .map(|(name, n)| Item {
                label: name,
                detail: n.to_string(),
                ..Item::default()
            })
            .collect(),
        PickerQuery::Backlinks => ws
            .backlinks(&path)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|b| {
                let line = b.line as usize + 1; // Backlink.line is 0-based.
                Item {
                    label: b.from_title,
                    detail: format!("{}:{line}", rel(&b.from)),
                    path: Some(b.from),
                    line: Some(line),
                    link: None,
                }
            })
            .collect(),
    };
    Ok(PickerAnswer {
        root,
        items,
        partial,
    })
}

/// Lines of a target note's excerpt the `K` popup shows (raw lines, blank
/// ones included). The popup is as tall as its wrapped text plus the
/// border, clamped to the content area (`ui/hover.rs`), so 12 lines plus a
/// title and a few front matter lines fit a normal terminal.
const PREVIEW_LINES: usize = 12;

/// The popup text for `target`, from the root workspace of the target.
fn preview_answer(workspaces: &Workspaces, target: &Path) -> Result<String, String> {
    let target = canonical(target);
    let ws = workspaces.for_path(&target).map_err(|e| e.to_string())?;
    let p = ws
        .preview(&target, PREVIEW_LINES)
        .map_err(|e| e.to_string())?;
    Ok(preview_text(&p.title, &p.frontmatter, &p.excerpt))
}

/// The `K` popup text for a note: `# <title>` unless the excerpt already
/// starts with a heading line (mdroots' excerpt usually starts with the
/// note's H1), then the front matter as `key: value` lines (the `title`
/// key left out: it is the title), then the excerpt. Empty sections are
/// skipped.
fn preview_text(title: &str, frontmatter: &[(String, String)], excerpt: &str) -> String {
    let excerpt = excerpt.trim();
    let mut parts = Vec::new();
    if !excerpt.lines().next().is_some_and(is_heading_line) {
        parts.push(format!("# {title}"));
    }
    let fm: Vec<String> = frontmatter
        .iter()
        .filter(|(k, _)| k != "title")
        .map(|(k, v)| format!("{k}: {v}").trim_end().to_string())
        .collect();
    if !fm.is_empty() {
        parts.push(fm.join("\n"));
    }
    if !excerpt.is_empty() {
        parts.push(excerpt.to_string());
    }
    parts.join("\n\n")
}

/// An ATX heading line: up to three spaces, one to six `#`, then a space
/// or the end of the line.
fn is_heading_line(line: &str) -> bool {
    let t = line.trim_start_matches(' ');
    if line.len() - t.len() > 3 {
        return false;
    }
    let hashes = t.len() - t.trim_start_matches('#').len();
    (1..=6).contains(&hashes)
        && t[hashes..]
            .chars()
            .next()
            .is_none_or(|c| c == ' ' || c == '\t')
}

/// `notes` by modification time, newest first (zk's `sort: [modified]`);
/// unknown times last, then by path.
fn newest_first(mut notes: Vec<NoteSummary>) -> Vec<NoteSummary> {
    notes.sort_by(|a, b| {
        let by_time = match (a.modified, b.modified) {
            (Some(x), Some(y)) => y.cmp(&x),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        };
        by_time.then_with(|| a.path.cmp(&b.path))
    });
    notes
}

/// `title`, or the file stem when it is empty.
fn title_or_stem(title: &str, path: &Path) -> String {
    if title.is_empty() {
        path.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        title.to_string()
    }
}

/// `tags` with names that differ only in case merged into one row (the
/// first spelling, the summed count): `notes_with_tag` matches
/// case-insensitively, so each spelling would open the same notes.
fn merge_tags(tags: Vec<(String, usize)>) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for (name, n) in tags {
        match at.get(&name.to_lowercase()) {
            Some(&i) => out[i].1 += n,
            None => {
                at.insert(name.to_lowercase(), out.len());
                out.push((name, n));
            }
        }
    }
    out
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
        m.preview = None;
        m.serving = None;
        m.sent_at = None;
        m.watching = false;
    }

    /// The current page (at `path`) has no LSP server: ask mdroots. The
    /// page itself is refreshed in a ready root workspace first, so a
    /// reload (watcher, `R`) or a revisit sees the file as it is now.
    pub(super) fn mdroots_page_changed(&mut self, path: PathBuf) {
        self.mdroots.active = true;
        self.mdroots_request(path.clone(), vec![path]);
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
        m.send(Request::Page(PageRequest {
            tag,
            path,
            text,
            refresh,
        }));
    }

    /// Whether mdroots serves the current page (it has a path and no LSP
    /// server was selected), so the pickers ask it.
    pub(super) fn mdroots_active(&self) -> bool {
        self.mdroots.active
    }

    /// Ask the worker for picker `seq`'s items, from the root workspace of
    /// the current page. The answer arrives through
    /// [`App::mdroots_picker_reply`]. False when the page has no path.
    pub(super) fn mdroots_picker_request(&mut self, seq: u64, query: PickerQuery) -> bool {
        let Some(path) = self.page.as_ref().and_then(|p| p.path.clone()) else {
            return false;
        };
        self.mdroots
            .send(Request::Picker(PickerRequest { seq, path, query }));
        true
    }

    /// Apply pending mdroots replies, blocking up to `timeout` for the
    /// first. Returns how many were applied. `run` calls it every loop
    /// iteration.
    pub fn pump_mdroots(&mut self, timeout: Duration) -> usize {
        let first = match self.mdroots.rx.recv_timeout(timeout) {
            Ok(r) => r,
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return 0,
        };
        self.mdroots_dispatch(first);
        let mut n = 1;
        while let Ok(r) = self.mdroots.rx.try_recv() {
            self.mdroots_dispatch(r);
            n += 1;
        }
        n
    }

    fn mdroots_dispatch(&mut self, r: Reply) {
        match r {
            Reply::Page(r) => self.mdroots_reply(r),
            Reply::Picker { seq, result } => self.mdroots_picker_reply(seq, result),
            Reply::Preview { seq, result } => self.mdroots_preview_reply(seq, result),
        }
    }

    /// The note `K` would preview on an mdroots page: the target mdroots
    /// gave the link under the cursor, when it is a markdown file other
    /// than the page. The same links `gd` follows through mdroots (not
    /// code paths, anchors on this page or URLs).
    fn mdroots_preview_target(&self) -> Option<PathBuf> {
        if !self.mdroots.active {
            return None;
        }
        let i = self.link_under_cursor()?;
        let page = self.page.as_ref()?;
        let link = page.doc.links.get(i)?;
        if link.kind == LinkKind::CodePath
            || !matches!(
                nav::resolve(&link.dest, &link.kind, &self.link_dir()),
                nav::Target::File { .. }
            )
        {
            return None;
        }
        let target = self.link_target(i)?;
        let here = page.path.as_deref().map(canonical);
        (nav::is_markdown(target) && here.as_deref() != Some(target)).then(|| target.to_path_buf())
    }

    /// `K` can preview the link under the cursor through mdroots.
    pub(super) fn mdroots_can_hover(&self) -> bool {
        self.mdroots_preview_target().is_some()
    }

    /// `K` on an mdroots page: ask the worker for the link target's
    /// preview, replacing any preview in flight. Before the page's links
    /// arrive, or for a link with no note to preview, the status says
    /// "No hover information" and an open preview closes.
    pub(super) fn mdroots_hover(&mut self) {
        let Some(target) = self.mdroots_preview_target() else {
            self.mdroots.preview = None;
            self.hover_close();
            self.set_status("No hover information");
            return;
        };
        let page_id = self.page_id();
        let m = &mut self.mdroots;
        m.preview_seq += 1;
        let seq = m.preview_seq;
        m.preview = Some((page_id, seq));
        m.send(Request::Preview(PreviewRequest { seq, target }));
    }

    /// Forget the preview in flight (`K` showed review comments instead).
    pub(super) fn mdroots_hover_cancel(&mut self) {
        self.mdroots.preview = None;
    }

    fn mdroots_preview_reply(&mut self, seq: u64, result: Result<String, String>) {
        if self.mdroots.preview != Some((self.page_id(), seq)) {
            return; // Another page, a newer `K`, or cancelled: drop.
        }
        self.mdroots.preview = None;
        match result {
            Ok(text) if !text.trim().is_empty() => self.set_hover_popup(text),
            _ => self.set_status("No hover information"),
        }
    }

    pub(super) fn mdroots_reply(&mut self, r: PageReply) {
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
        self.mdroots.watching = r.watching;
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

    /// Whether the workspace that last answered for the page runs mdroots'
    /// watcher (other notes' changes then update the page's links).
    pub fn mdroots_watching(&self) -> bool {
        self.mdroots.active && self.mdroots.watching
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
        app.mdroots_reply(PageReply {
            tag: old,
            root: true,
            single: false,
            watching: false,
            links: Ok(links.clone()),
        });
        assert_eq!(app.link_target(0), None);
        assert_eq!(app.lsp_label(), "—");
        let cur = app.mdroots.tag.unwrap();
        app.mdroots_reply(PageReply {
            tag: cur,
            root: false,
            single: true,
            watching: false,
            links: Ok(links),
        });
        assert!(app.link_target(0).is_some());
        assert_eq!(app.lsp_label(), "mdroots ○");
    }

    #[test]
    fn a_picker_request_between_page_requests_is_answered() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path(), "# A\n\n[b](b)\n");
        // Page, picker, page, page: the pages coalesce, the picker does not.
        assert!(app.mdroots_picker_request(7, PickerQuery::Tags));
        app.reload();
        app.reload();
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut picker = None;
        while picker.is_none() {
            assert!(Instant::now() < deadline, "no picker reply");
            if let Ok(Reply::Picker { seq, result }) =
                app.mdroots.rx.recv_timeout(Duration::from_millis(20))
            {
                picker = Some((seq, result));
            }
        }
        let (seq, result) = picker.unwrap();
        assert_eq!(seq, 7);
        let a = result.unwrap();
        assert_eq!(a.root, dir.path().canonicalize().unwrap());
        assert!(a.items.is_empty(), "no tags");
    }

    #[test]
    fn preview_text_shows_the_title_once_and_skips_empty_sections() {
        let fm = |kv: &[(&str, &str)]| -> Vec<(String, String)> {
            kv.iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        // The excerpt starts with the H1: no title line.
        assert_eq!(
            preview_text("Note B", &[], "# Note B\n\nBack."),
            "# Note B\n\nBack."
        );
        // No heading: the title first; the `title` key left out.
        assert_eq!(
            preview_text("T", &fm(&[("title", "T"), ("tags", "a, b")]), "body\n"),
            "# T\n\ntags: a, b\n\nbody"
        );
        // An empty note: the title alone.
        assert_eq!(preview_text("T", &[], ""), "# T");
        assert!(is_heading_line("## x") && is_heading_line("#") && is_heading_line("   # x"));
        assert!(
            !is_heading_line("#tag")
                && !is_heading_line("    # x")
                && !is_heading_line("####### x")
        );
    }

    #[test]
    fn tags_merge_case_insensitively_keeping_the_first_spelling() {
        let tags = vec![
            ("Project".into(), 1),
            ("project".into(), 2),
            ("x".into(), 1),
        ];
        assert_eq!(
            merge_tags(tags),
            [("Project".to_string(), 3), ("x".to_string(), 1)]
        );
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
        app.mdroots_reply(PageReply {
            tag,
            root: false,
            single: true,
            watching: false,
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
        app.mdroots_reply(PageReply {
            tag,
            root: true,
            single: false,
            watching: false,
            links: Ok(rooted),
        });
        assert!(app.lsp_backlink().is_none());
        assert_eq!(app.link_under_cursor(), Some(0), "landed on the link");
    }
}
