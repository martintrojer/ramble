//! Live reload: a `notify` watcher on the current file's folder, debounced
//! into [`FsEvent`]s, plus the app's reload and deleted-file banner.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{RecursiveMode, Watcher as _};

use super::sidebar::Tree;
use super::{App, AppEvent};

/// Quiet period before a burst of raw events becomes one [`FsEvent`].
pub const DEBOUNCE: Duration = Duration::from_millis(100);

/// Watcher events for this long after a launcher's own reload are ignored.
const LAUNCH_QUIET: Duration = Duration::from_millis(500);

/// Banner shown while the current file is gone.
pub const DELETED_BANNER: &str = "File deleted; showing last version";

/// What happened to the watched file (decided by re-statting it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsEvent {
    Changed,
    Removed,
}

/// The watched file: its canonical folder, its name, and its state at the
/// last emit (or the loaded bytes given to `watch`).
type Target = Arc<Mutex<Option<(PathBuf, std::ffi::OsString, FileState)>>>;

/// A hash of the file's bytes; `None` while it does not exist.
type FileState = Option<u64>;

/// The hash the watcher compares file contents by; pass the hash of the
/// bytes a page was built from to [`FileWatcher::watch`].
pub fn content_hash(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

fn file_state(path: &Path) -> FileState {
    std::fs::read(path).ok().map(|b| content_hash(&b))
}

/// Watches one file at a time through its parent folder (non-recursive),
/// so editors that replace the file by rename still trigger a reload.
pub struct FileWatcher {
    inner: notify::RecommendedWatcher,
    target: Target,
    /// Feeds the debouncer a synthetic event (see `watch`).
    kick: mpsc::Sender<Vec<PathBuf>>,
    /// The folder currently watched.
    dir: Option<PathBuf>,
    /// The file as given (reported back in events).
    file: Option<PathBuf>,
}

impl FileWatcher {
    /// Start a watcher; `emit` gets `(file, event)` after each debounced
    /// burst of changes to the watched file.
    pub fn new(emit: impl Fn(PathBuf, FsEvent) + Send + 'static) -> notify::Result<Self> {
        let target: Target = Arc::new(Mutex::new(None));
        let (raw_tx, raw_rx) = mpsc::channel::<Vec<PathBuf>>();
        let kick = raw_tx.clone();
        let inner = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                let _ = raw_tx.send(ev.paths);
            }
        })?;
        let shared = target.clone();
        std::thread::spawn(move || debounce(raw_rx, shared, emit));
        Ok(Self {
            inner,
            target,
            kick,
            dir: None,
            file: None,
        })
    }

    /// Watch `file`, whose page was built from bytes hashing to `loaded`
    /// (see [`content_hash`]); `None` stops watching. Called again for the
    /// same file (a reload), it only re-seeds the baseline.
    ///
    /// The baseline is the loaded bytes, not a fresh read: a write landing
    /// between the page's read and this call must still count as a change.
    /// Every call kicks the debouncer once, so such a write is reported even
    /// when its event predates the watch (inotify never sees it).
    pub fn watch(&mut self, file: Option<(&Path, u64)>) {
        let (path, loaded) = file.unzip();
        if self.file.as_deref() != path {
            self.retarget(path);
        }
        let mut t = self.target.lock().unwrap_or_else(|e| e.into_inner());
        let Some((dir, name, last)) = t.as_mut() else {
            return;
        };
        *last = loaded;
        let _ = self.kick.send(vec![dir.join(&*name)]);
    }

    fn retarget(&mut self, file: Option<&Path>) {
        if let Some(dir) = self.dir.take() {
            let _ = self.inner.unwatch(&dir);
        }
        *self.target.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.file = file.map(Path::to_path_buf);
        let Some(file) = file else { return };
        let Some(name) = file.file_name() else { return };
        let parent = match file.parent() {
            Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
            _ => PathBuf::from("."),
        };
        let Ok(dir) = parent.canonicalize() else {
            return;
        };
        if self.inner.watch(&dir, RecursiveMode::NonRecursive).is_ok() {
            // The baseline is set by `watch`. macOS FSEvents also replays
            // writes from just before the watch started; the debouncer drops
            // those when the bytes match the baseline.
            *self.target.lock().unwrap_or_else(|e| e.into_inner()) =
                Some((dir.clone(), name.to_os_string(), None));
            self.dir = Some(dir);
        }
    }
}

/// Collect raw events touching the target, wait for a quiet period, then
/// re-read the file and emit Changed or Removed if its state differs from
/// the last emit (events alone are not proof of a change: see `watch`).
fn debounce(rx: mpsc::Receiver<Vec<PathBuf>>, target: Target, emit: impl Fn(PathBuf, FsEvent)) {
    let mut due: Option<Instant> = None;
    loop {
        let wait = due.map_or(Duration::from_secs(3600), |d| {
            d.saturating_duration_since(Instant::now())
        });
        match rx.recv_timeout(wait) {
            Ok(paths) => {
                let t = target.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let Some((_, name, _)) = t else { continue };
                // Compare names only: macOS reports /private/... paths.
                if paths
                    .iter()
                    .any(|p| p.file_name() == Some(name.as_os_str()))
                {
                    due = Some(Instant::now() + DEBOUNCE);
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                due = None;
                let mut t = target.lock().unwrap_or_else(|e| e.into_inner());
                let Some((dir, name, last)) = t.as_mut() else {
                    continue;
                };
                let file = dir.join(&*name);
                let now = file_state(&file);
                if now == *last {
                    continue;
                }
                *last = now;
                drop(t);
                let ev = if now.is_some() {
                    FsEvent::Changed
                } else {
                    FsEvent::Removed
                };
                emit(file, ev);
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        // A deleted file cannot be canonicalized: compare folder + name.
        _ => {
            let parent = |p: &Path| {
                p.parent()
                    .and_then(|d| d.canonicalize().ok())
                    .map(|d| d.join(p.file_name().unwrap_or_default()))
            };
            parent(a).is_some() && parent(a) == parent(b)
        }
    }
}

impl App {
    /// Start watching the current file; events arrive as
    /// [`AppEvent::FsWatch`] through the sender. Needs a sender (set by
    /// `run`, or by tests).
    pub fn start_watcher(&mut self) -> anyhow::Result<()> {
        let Some(tx) = self.sender.clone() else {
            anyhow::bail!("no event sender");
        };
        let w = FileWatcher::new(move |path, ev| {
            let _ = tx.send(AppEvent::FsWatch(path, ev));
        })?;
        self.watcher = Some(w);
        self.retarget_watch();
        Ok(())
    }

    /// Point the watcher at the current page's file (none for stdin).
    pub(super) fn retarget_watch(&mut self) {
        let file = self.page.as_ref().and_then(|p| p.path.clone());
        let loaded = self.loaded_hash;
        if let Some(w) = &mut self.watcher {
            w.watch(file.as_deref().zip(loaded));
        }
    }

    /// A watcher event for `path`.
    pub(super) fn fs_event(&mut self, path: &Path, ev: FsEvent) {
        let Some(current) = self.page.as_ref().and_then(|p| p.path.clone()) else {
            return;
        };
        if !same_file(path, &current) {
            return;
        }
        match ev {
            FsEvent::Changed => {
                let quiet = self
                    .launch_reloaded_at
                    .is_some_and(|t| t.elapsed() < LAUNCH_QUIET);
                if !quiet {
                    self.reload();
                }
            }
            FsEvent::Removed => self.banner = Some(DELETED_BANNER.into()),
        }
    }

    /// Banner over the content (e.g. the file was deleted).
    pub fn banner(&self) -> Option<&str> {
        self.banner.as_deref()
    }

    /// Re-read the current file, keeping the cursor on the same source
    /// byte and the scroll offset. A missing file shows the banner and
    /// keeps the content. Stdin pages do not reload.
    pub fn reload(&mut self) {
        let Some(path) = self.page.as_ref().and_then(|p| p.path.clone()) else {
            return;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            self.banner = Some(DELETED_BANNER.into());
            return;
        };
        let anchor = self.cursor_anchor();
        let (scroll, status, raw) = (self.scroll, self.status.clone(), self.raw());
        let fm_expanded = self.front_matter_expanded();
        let view = self.tree().map(Tree::view);
        if !self.open_bytes(&path, &bytes) {
            return;
        }
        // open_bytes reveals the file in the tree, expanding its ancestors
        // and selecting it; a reload is not navigation, so the user's
        // expansion and selection win.
        if let (Some(v), Some(t)) = (view, self.tree_mut()) {
            t.restore_view(v);
        }
        // Set before the raw layout, which reads it; a no-op when folded.
        if let Some(p) = &mut self.page {
            p.fm_expanded = fm_expanded;
        }
        if fm_expanded && !raw {
            let (cols, rows) = self.size;
            self.resize(cols, rows);
        }
        self.relayout_raw(raw);
        self.banner = None;
        if self.status.is_empty() {
            self.status = status;
        }
        self.scroll = scroll.min(self.max_scroll());
        self.restore_anchor(anchor);
        self.keep_visible();
        self.on_reloaded();
    }

    /// `C-l` / `:e` / `:Refresh`: re-read the current file (not stdin),
    /// re-read the file tree if built, and ask `run` to clear the
    /// terminal. A status set by the reload (binary, not UTF-8) or the
    /// deleted-file banner is kept.
    pub fn refresh(&mut self) {
        let file = self.page.as_ref().is_some_and(|p| p.path.is_some());
        if file {
            self.status.clear();
            self.reload();
        }
        // reload() keeps the tree's expansion and selection.
        let tree = match self.tree_mut() {
            Some(t) => {
                t.refresh();
                true
            }
            None => false,
        };
        self.sidebar_relayout(false);
        self.clear_request = true;
        let stdin = self.page.as_ref().is_some_and(|p| p.path.is_none());
        let msg = match (file, stdin, tree) {
            (true, ..) if !self.status.is_empty() || self.banner.is_some() => return,
            (true, ..) => "Refreshed",
            (false, true, true) => "Refreshed tree; stdin page unchanged",
            (false, true, false) => "Nothing to refresh (stdin)",
            (false, false, true) => "Refreshed tree",
            (false, false, false) => "Nothing to refresh",
        };
        self.set_status(msg);
    }

    /// Whether the terminal should be cleared before the next draw (set by
    /// [`App::refresh`]); taking it resets it.
    pub fn take_clear_request(&mut self) -> bool {
        std::mem::take(&mut self.clear_request)
    }

    /// Called after every reload of the current page. LSP needs nothing
    /// here: reload goes through `open_bytes` -> `set_page` ->
    /// `lsp_page_changed`, which already re-sends `didOpen` at the next
    /// version and asks for documentLink (or asks mdroots, which refreshes
    /// the page first). Do not re-open the document here.
    pub fn on_reloaded(&mut self) {}
}
