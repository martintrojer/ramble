//! tuicr session discovery and comment polling, producing review markers
//! (spec § Review markers).
//!
//! Only the `tuicr review` CLI is used. A background thread ([`spawn`])
//! polls it for the discovery directories the app sets in [`Control`], and
//! emits a new [`Markers`] whenever the comment set changes. No dependency
//! on rendering or the LSP.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;

/// How often sessions are listed.
pub const DISCOVERY_EVERY: Duration = Duration::from_secs(5);
/// How often comments are fetched while a session exists.
pub const COMMENTS_EVERY: Duration = Duration::from_secs(2);
/// Sleep between checks of the control flags.
const STEP: Duration = Duration::from_millis(50);

/// Comment marks for one file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileMarks {
    /// Every kept comment on the file, file-level ones included.
    pub count: usize,
    /// 1-based inclusive source line ranges of line comments.
    pub lines: Vec<(usize, usize)>,
}

/// Comment marks per canonical file path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Markers {
    pub files: BTreeMap<PathBuf, FileMarks>,
}

impl Markers {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Marks for `path` (canonicalised first).
    pub fn get(&self, path: &Path) -> Option<&FileMarks> {
        self.files.get(&canonical(path))
    }

    /// Whether any marked file lies under the folder `dir`.
    pub fn under(&self, dir: &Path) -> bool {
        let dir = canonical(dir);
        self.files.keys().any(|f| f.starts_with(&dir) && *f != dir)
    }
}

fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The canonical path (symlinks resolved), else the absolute path. Marker
/// keys use it so they compare equal to file-tree paths.
pub fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| absolute(path))
}

/// One entry of `tuicr review list`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Session {
    pub slug: String,
    #[serde(default)]
    pub active: bool,
}

/// One entry of `tuicr review comments`. Review-level comments have no
/// path; file-level comments have no lines.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Comment {
    pub path: Option<PathBuf>,
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
    pub side: Option<String>,
}

/// Parse `tuicr review list` output.
pub fn parse_sessions(json: &str) -> anyhow::Result<Vec<Session>> {
    Ok(serde_json::from_str(json)?)
}

/// Parse `tuicr review comments` output.
pub fn parse_comments(json: &str) -> anyhow::Result<Vec<Comment>> {
    Ok(serde_json::from_str(json)?)
}

/// Fold comments whose paths are relative to `dir` into `markers`. Drops
/// comments without a path, and line comments not on the `new` side.
pub fn add_comments(markers: &mut Markers, dir: &Path, comments: &[Comment]) {
    for c in comments {
        let Some(path) = &c.path else { continue };
        let lines = match c.start_line {
            Some(start) if c.side.as_deref() == Some("new") => {
                Some((start, c.end_line.unwrap_or(start).max(start)))
            }
            Some(_) => continue,
            None => None,
        };
        let m = markers.files.entry(canonical(&dir.join(path))).or_default();
        m.count += 1;
        m.lines.extend(lines);
    }
}

/// Most ancestors of the file's folder [`discovery_dirs`] walks.
pub const MAX_ANCESTORS: usize = 8;

/// The directories whose tuicr sessions belong to a file, in order: its
/// folder, then each ancestor up to and including its VCS root (at most
/// [`MAX_ANCESTORS`] above the folder), then the VCS root itself. Outside a
/// VCS repository, just the folder. `tuicr --file` keys a session to the
/// file's folder (or the folder passed to it); `tuicr -w -p` to the repo.
pub fn discovery_dirs(file: &Path) -> Vec<PathBuf> {
    let file = absolute(file);
    let Some(folder) = file.parent() else {
        return vec![file];
    };
    let Some(root) = crate::app::vcs_root(folder) else {
        return vec![folder.to_path_buf()];
    };
    let mut dirs: Vec<PathBuf> = folder
        .ancestors()
        .take(MAX_ANCESTORS + 1)
        .take_while(|d| d.starts_with(&root))
        .map(Path::to_path_buf)
        .collect();
    if !dirs.contains(&root) {
        dirs.push(root);
    }
    dirs
}

/// `command` as an executable path: itself if it names a path that exists,
/// else the first match on `$PATH`. `None` means the thread does not start.
pub fn resolve_command(command: &str) -> Option<PathBuf> {
    if command.is_empty() {
        return None;
    }
    let p = Path::new(command);
    if p.components().count() > 1 || p.is_absolute() {
        return p.is_file().then(|| p.to_path_buf());
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(command))
        .find(|c| c.is_file())
}

/// Runs `tuicr review ...`.
#[derive(Debug, Clone)]
pub struct Tuicr {
    pub command: PathBuf,
}

impl Tuicr {
    fn run(&self, args: &[&str], dir: &Path) -> anyhow::Result<String> {
        let out = Command::new(&self.command)
            .arg("review")
            .args(args)
            .arg("--repo")
            .arg(dir)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()?;
        anyhow::ensure!(out.status.success(), "tuicr exited with {}", out.status);
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    pub fn list(&self, dir: &Path) -> anyhow::Result<Vec<Session>> {
        parse_sessions(&self.run(&["list"], dir)?)
    }

    pub fn comments(&self, slug: &str, dir: &Path) -> anyhow::Result<Vec<Comment>> {
        parse_comments(&self.run(&["comments", "--session", slug], dir)?)
    }
}

/// Polling intervals (tests shorten them).
#[derive(Debug, Clone, Copy)]
pub struct Intervals {
    pub discovery: Duration,
    pub comments: Duration,
}

impl Default for Intervals {
    fn default() -> Self {
        Intervals {
            discovery: DISCOVERY_EVERY,
            comments: COMMENTS_EVERY,
        }
    }
}

/// The polling state machine, driven by [`Poller::step`] with an explicit
/// clock.
pub struct Poller {
    tuicr: Tuicr,
    every: Intervals,
    dirs: Vec<PathBuf>,
    /// Active `(dir, slug)` sessions at the last discovery, in `dirs` order.
    active: Vec<(PathBuf, String)>,
    /// Per directory: the last session seen active there.
    sticky: HashMap<PathBuf, String>,
    discovered_at: Option<Instant>,
    fetched_at: Option<Instant>,
    last: Markers,
}

impl Poller {
    pub fn new(tuicr: Tuicr, every: Intervals) -> Poller {
        Poller {
            tuicr,
            every,
            dirs: Vec::new(),
            active: Vec::new(),
            sticky: HashMap::new(),
            discovered_at: None,
            fetched_at: None,
            last: Markers::default(),
        }
    }

    /// Sessions whose comments count, as `(dir listed under, slug)`: per
    /// directory, the active ones plus the sticky one.
    pub fn sessions(&self) -> Vec<(PathBuf, String)> {
        let mut out = Vec::new();
        for d in &self.dirs {
            out.extend(self.active.iter().filter(|(ad, _)| ad == d).cloned());
            if let Some(s) = self.sticky.get(d) {
                let pair = (d.clone(), s.clone());
                if !out.contains(&pair) {
                    out.push(pair);
                }
            }
        }
        out
    }

    /// List sessions under `dir`, updating `active` and the sticky session.
    /// Returns whether the listing succeeded.
    fn discover(&mut self, dir: &Path) -> bool {
        let Ok(sessions) = self.tuicr.list(dir) else {
            return false;
        };
        let active: Vec<String> = sessions
            .into_iter()
            .filter(|s| s.active)
            .map(|s| s.slug)
            .collect();
        let keep = self.sticky.get(dir).is_some_and(|s| active.contains(s));
        if let (false, Some(first)) = (keep, active.first()) {
            self.sticky.insert(dir.to_path_buf(), first.clone());
        }
        self.active.retain(|(d, _)| d != dir);
        self.active
            .extend(active.into_iter().map(|s| (dir.to_path_buf(), s)));
        true
    }

    /// Poll what is due at `now` for `dirs`; `force` polls everything now.
    /// Returns the markers when they changed.
    pub fn step(&mut self, dirs: &[PathBuf], now: Instant, force: bool) -> Option<Markers> {
        let changed = self.dirs != dirs;
        let force = force || changed;
        if changed {
            self.dirs = dirs.to_vec();
            self.active.clear();
        }
        if self.dirs.is_empty() {
            return self.publish(Markers::default());
        }
        let due = |at: Option<Instant>, every: Duration| at.is_none_or(|t| now >= t + every);
        let mut fetch = force || due(self.fetched_at, self.every.comments);
        if force || due(self.discovered_at, self.every.discovery) {
            self.discovered_at = Some(now);
            for dir in self.dirs.clone() {
                fetch |= self.discover(&dir);
            }
        }
        let sessions = self.sessions();
        if sessions.is_empty() {
            return self.publish(Markers::default());
        }
        if !fetch {
            return None;
        }
        self.fetched_at = Some(now);
        let mut markers = Markers::default();
        for (dir, slug) in &sessions {
            if let Ok(cs) = self.tuicr.comments(slug, dir) {
                add_comments(&mut markers, dir, &cs);
            }
        }
        self.publish(markers)
    }

    fn publish(&mut self, markers: Markers) -> Option<Markers> {
        if markers == self.last {
            return None;
        }
        self.last = markers.clone();
        Some(markers)
    }
}

/// Shared between the app and the review thread.
#[derive(Debug, Default)]
pub struct Control {
    /// Set while a launcher runs; the thread skips polls, then polls once
    /// immediately when it clears.
    paused: AtomicBool,
    /// Bumped on every resume, so a pause shorter than one thread step
    /// still forces a poll.
    resumed: AtomicU64,
    /// The discovery directories (empty for stdin or no page).
    dirs: Mutex<Vec<PathBuf>>,
    stop: AtomicBool,
}

impl Control {
    pub fn set_dirs(&self, dirs: Vec<PathBuf>) {
        *self.dirs.lock().unwrap_or_else(|e| e.into_inner()) = dirs;
    }

    pub fn dirs(&self) -> Vec<PathBuf> {
        self.dirs.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
        if !paused {
            self.resumed.fetch_add(1, Ordering::SeqCst);
        }
    }

    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }
}

/// The running review thread; dropping it stops the thread.
#[derive(Debug)]
pub struct Handle {
    pub control: Arc<Control>,
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.control.stop.store(true, Ordering::SeqCst);
    }
}

/// Start the review thread. `emit` gets every changed marker set.
pub fn spawn(tuicr: Tuicr, every: Intervals, emit: impl Fn(Markers) + Send + 'static) -> Handle {
    let control = Arc::new(Control::default());
    let c = control.clone();
    std::thread::spawn(move || {
        let mut poller = Poller::new(tuicr, every);
        let mut seen = 0;
        while !c.stop.load(Ordering::SeqCst) {
            if !c.paused() {
                let resumed = c.resumed.load(Ordering::SeqCst);
                let force = resumed != seen;
                seen = resumed;
                if let Some(m) = poller.step(&c.dirs(), Instant::now(), force) {
                    emit(m);
                }
            }
            std::thread::sleep(STEP);
        }
    });
    Handle { control }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_keep_new_side_and_file_level() {
        let json = r#"[
          {"path":null,"start_line":null,"end_line":null,"side":null},
          {"path":"a.md","start_line":null,"end_line":null,"side":null},
          {"path":"a.md","start_line":1,"end_line":1,"side":"old"},
          {"path":"a.md","start_line":3,"end_line":4,"side":"new"},
          {"path":"sub/b.md","start_line":2,"end_line":null,"side":"new"}
        ]"#;
        let mut m = Markers::default();
        add_comments(&mut m, Path::new("/r"), &parse_comments(json).unwrap());
        let a = &m.files[&canonical(Path::new("/r/a.md"))];
        assert_eq!((a.count, a.lines.clone()), (2, vec![(3, 4)]));
        let b = &m.files[&canonical(Path::new("/r/sub/b.md"))];
        assert_eq!((b.count, b.lines.clone()), (1, vec![(2, 2)]));
        assert_eq!(m.files.len(), 2);
    }

    #[test]
    fn resolve_command_needs_an_existing_file() {
        assert_eq!(resolve_command(""), None);
        assert_eq!(resolve_command("/nonexistent/tuicr"), None);
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("t");
        std::fs::write(&f, "").unwrap();
        assert_eq!(resolve_command(f.to_str().unwrap()), Some(f));
    }
}
