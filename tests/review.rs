//! Review markers (spec § Review markers, § Testing): the review module
//! against a fake `tuicr` script, app markers, and one real tuicr e2e in an
//! isolated tmux (skipped without tuicr or tmux).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ramble::app::{App, AppEvent, NO_MORE_REVIEW, NO_REVIEW, StartOptions, StartTarget};
use ramble::config::{Config, SidebarMode};
use ramble::review::{
    self, FileMarks, Intervals, Markers, Poller, Tuicr, canonical, discovery_dir,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

/// A fake `tuicr` in `dir/bin`: `review list` prints `dir/list.json`;
/// `review comments --session S` prints `dir/comments-<S with / @ ~ as _>.json`.
/// Every call appends its argv to `dir/log`.
struct Fake {
    dir: PathBuf,
}

impl Fake {
    fn new(dir: &Path) -> Fake {
        let script = dir.join("tuicr");
        std::fs::write(
            &script,
            format!(
                r#"#!/bin/sh
D='{d}'
echo "$*" >> "$D/log"
case "$2" in
  list) cat "$D/list.json" ;;
  comments) s=$(printf %s "$4" | tr '/@~' '___'); cat "$D/comments-$s.json" 2>/dev/null || echo '[]' ;;
  *) exit 2 ;;
esac
"#,
                d = dir.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let f = Fake {
            dir: dir.to_path_buf(),
        };
        f.sessions(&[]);
        f
    }

    fn command(&self) -> PathBuf {
        self.dir.join("tuicr")
    }

    fn tuicr(&self) -> Tuicr {
        Tuicr {
            command: self.command(),
        }
    }

    fn sessions(&self, s: &[(&str, bool)]) {
        let items: Vec<_> = s
            .iter()
            .map(|(slug, active)| serde_json::json!({"slug": slug, "kind": "local", "active": active}))
            .collect();
        std::fs::write(
            self.dir.join("list.json"),
            serde_json::to_string(&items).unwrap(),
        )
        .unwrap();
    }

    fn comments(&self, slug: &str, json: &str) {
        let name: String = slug
            .chars()
            .map(|c| if "/@~".contains(c) { '_' } else { c })
            .collect();
        std::fs::write(self.dir.join(format!("comments-{name}.json")), json).unwrap();
    }

    fn log(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.join("log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }
}

fn line(path: &str, start: usize, end: usize, side: &str) -> String {
    format!(r#"{{"path":"{path}","start_line":{start},"end_line":{end},"side":"{side}"}}"#)
}

fn marks(count: usize, lines: &[(usize, usize)]) -> FileMarks {
    FileMarks {
        count,
        lines: lines.to_vec(),
    }
}

const SLOW: Intervals = Intervals {
    discovery: Duration::from_secs(5),
    comments: Duration::from_secs(2),
};

#[test]
fn only_active_sessions_are_read_and_paths_map_to_the_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new(tmp.path());
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(repo.join("sub")).unwrap();
    fake.sessions(&[("live", true), ("old", false)]);
    fake.comments(
        "live",
        &format!(
            "[{},{},{}]",
            line("sub/a.md", 3, 4, "new"),
            line("sub/a.md", 1, 1, "old"),
            r#"{"path":null,"start_line":null,"end_line":null,"side":null}"#
        ),
    );
    fake.comments("old", &format!("[{}]", line("b.md", 1, 1, "new")));
    let mut p = Poller::new(fake.tuicr(), SLOW);
    let m = p.step(Some(&repo), Instant::now(), false).unwrap();
    assert_eq!(m.files.len(), 1, "{m:?}");
    assert_eq!(m.get(&repo.join("sub/a.md")), Some(&marks(1, &[(3, 4)])));
    let log = fake.log();
    let repo_s = repo.display().to_string();
    assert_eq!(
        log,
        vec![
            format!("review list --repo {repo_s}"),
            format!("review comments --session live --repo {repo_s}"),
        ]
    );
}

#[test]
fn polls_on_schedule_and_reports_only_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new(tmp.path());
    let repo = tmp.path().to_path_buf();
    fake.sessions(&[("s", true)]);
    fake.comments("s", &format!("[{}]", line("a.md", 1, 1, "new")));
    let mut p = Poller::new(fake.tuicr(), SLOW);
    let t0 = Instant::now();
    assert!(p.step(Some(&repo), t0, false).is_some());
    assert_eq!(fake.log().len(), 2);
    // Nothing due before 2 s.
    assert!(
        p.step(Some(&repo), t0 + Duration::from_secs(1), false)
            .is_none()
    );
    assert_eq!(fake.log().len(), 2);
    // Comments at 2 s; unchanged, so no event.
    assert!(
        p.step(Some(&repo), t0 + Duration::from_secs(2), false)
            .is_none()
    );
    let log = fake.log();
    assert_eq!(log.len(), 3);
    assert!(log[2].starts_with("review comments"), "{log:?}");
    // Changed comments are reported.
    fake.comments("s", &format!("[{}]", line("a.md", 2, 2, "new")));
    let m = p
        .step(Some(&repo), t0 + Duration::from_secs(4), false)
        .unwrap();
    assert_eq!(m.get(&repo.join("a.md")), Some(&marks(1, &[(2, 2)])));
    // Discovery again at 5 s.
    p.step(Some(&repo), t0 + Duration::from_secs(5), false);
    assert!(fake.log()[4].starts_with("review list"), "{:?}", fake.log());
}

#[test]
fn no_session_means_no_comment_calls() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new(tmp.path());
    let mut p = Poller::new(fake.tuicr(), SLOW);
    let t0 = Instant::now();
    assert!(p.step(Some(tmp.path()), t0, false).is_none());
    p.step(Some(tmp.path()), t0 + Duration::from_secs(3), false);
    assert_eq!(fake.log().len(), 1, "{:?}", fake.log());
}

#[test]
fn last_active_session_stays_after_it_ends_until_another_starts() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new(tmp.path());
    let repo = tmp.path().to_path_buf();
    fake.sessions(&[("one", true)]);
    fake.comments("one", &format!("[{}]", line("a.md", 1, 1, "new")));
    fake.comments("two", &format!("[{}]", line("b.md", 1, 1, "new")));
    let mut p = Poller::new(fake.tuicr(), SLOW);
    let t0 = Instant::now();
    p.step(Some(&repo), t0, false).unwrap();
    // tuicr quit: the session is inactive, but stays sticky.
    fake.sessions(&[("one", false)]);
    p.step(Some(&repo), t0, true);
    assert_eq!(p.sessions(), vec!["one".to_string()]);
    // A different session becomes active and replaces it.
    fake.sessions(&[("one", false), ("two", true)]);
    let m = p.step(Some(&repo), t0, true).unwrap();
    assert_eq!(p.sessions(), vec!["two".to_string()]);
    assert!(m.get(&repo.join("a.md")).is_none());
    assert!(m.get(&repo.join("b.md")).is_some());
    fake.sessions(&[("two", false)]);
    p.step(Some(&repo), t0, true);
    assert_eq!(p.sessions(), vec!["two".to_string()]);
}

#[test]
fn several_active_sessions_are_unioned() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new(tmp.path());
    let repo = tmp.path().to_path_buf();
    fake.sessions(&[("a/x@~file", true), ("b", true)]);
    fake.comments("a/x@~file", &format!("[{}]", line("a.md", 1, 1, "new")));
    fake.comments("b", &format!("[{}]", line("a.md", 5, 6, "new")));
    let mut p = Poller::new(fake.tuicr(), SLOW);
    let m = p.step(Some(&repo), Instant::now(), false).unwrap();
    assert_eq!(
        m.get(&repo.join("a.md")),
        Some(&marks(2, &[(1, 1), (5, 6)]))
    );
}

#[test]
fn discovery_dir_is_the_vcs_root_else_the_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let plain = tmp.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    assert_eq!(discovery_dir(&plain.join("a.md")), plain);
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(repo.join(".jj")).unwrap();
    std::fs::create_dir_all(repo.join("notes")).unwrap();
    assert_eq!(discovery_dir(&repo.join("notes/a.md")), repo);
}

fn recv_review(rx: &mpsc::Receiver<AppEvent>, within: Duration) -> Option<Markers> {
    let deadline = Instant::now() + within;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(AppEvent::Review(m)) => return Some(m),
            Ok(_) => {}
            Err(_) => return None,
        }
    }
    None
}

#[test]
fn thread_skips_polls_while_paused_and_polls_at_once_on_resume() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new(tmp.path());
    let repo = tmp.path().to_path_buf();
    fake.sessions(&[("s", true)]);
    fake.comments("s", &format!("[{}]", line("a.md", 1, 1, "new")));
    let (tx, rx) = mpsc::channel();
    // Long intervals: after the first poll, only a resume polls again.
    let long = Intervals {
        discovery: Duration::from_secs(600),
        comments: Duration::from_secs(600),
    };
    let h = review::spawn(fake.tuicr(), long, move |m| {
        let _ = tx.send(AppEvent::Review(m));
    });
    h.control.set_dir(Some(repo.clone()));
    assert!(recv_review(&rx, Duration::from_secs(5)).is_some());
    h.control.set_paused(true);
    // Let a poll already in flight finish.
    std::thread::sleep(Duration::from_millis(500));
    let calls = fake.log().len();
    fake.comments("s", &format!("[{}]", line("a.md", 9, 9, "new")));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(fake.log().len(), calls, "no polls while paused");
    h.control.set_paused(false);
    let m = recv_review(&rx, Duration::from_secs(5)).expect("poll on resume");
    assert_eq!(m.get(&repo.join("a.md")), Some(&marks(1, &[(9, 9)])));
}

// ---- App ------------------------------------------------------------------

const COLS: u16 = 60;
const ROWS: u16 = 14;

/// Line numbers: 1 "# Title", 3 "alpha", 5 "beta", 7 "gamma", 9 "delta".
const DOC: &str = "# Title\n\nalpha\n\nbeta\n\ngamma\n\ndelta\n";

fn config(review_cmd: Option<&Path>, sidebar: SidebarMode) -> Config {
    let mut c = Config::default();
    c.lsp.server = vec![];
    c.sidebar.default = sidebar;
    if let Some(cmd) = review_cmd {
        c.review.command = cmd.display().to_string();
    }
    c
}

fn app_in(dir: &Path, config: Config) -> App {
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("doc.md"), DOC).unwrap();
    std::fs::write(dir.join("sub/b.md"), "# B\n").unwrap();
    App::new(
        StartOptions {
            target: StartTarget::File(dir.join("doc.md")),
            tree_root: dir.to_path_buf(),
            config,
        },
        (COLS, ROWS),
    )
    .unwrap()
}

fn markers(dir: &Path, entries: &[(&str, FileMarks)]) -> Markers {
    let mut m = Markers::default();
    for (p, f) in entries {
        m.files.insert(canonical(&dir.join(p)), f.clone());
    }
    m
}

fn screen(app: &App) -> Vec<String> {
    let mut term = Terminal::new(TestBackend::new(COLS, ROWS)).unwrap();
    term.draw(|f| ramble::ui::draw(f, app)).unwrap();
    let buf = term.backend().buffer().clone();
    (0..ROWS)
        .map(|y| {
            (0..COLS)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect()
        })
        .collect()
}

fn row_of(app: &App, text: &str) -> usize {
    let lines = &app.page().unwrap().rendered.lines;
    lines
        .iter()
        .position(|l| l.spans.iter().any(|s| s.content.contains(text)))
        .unwrap()
}

#[test]
fn review_thread_needs_enabled_and_a_resolvable_command() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new(tmp.path());
    let (tx, _rx) = mpsc::channel();

    let mut missing = app_in(
        &tmp.path().join("m"),
        config(Some(&tmp.path().join("nope/tuicr")), SidebarMode::Off),
    );
    missing.set_sender(tx.clone());
    assert!(!missing.start_review());
    assert!(!missing.review_running());

    let mut c = config(Some(&fake.command()), SidebarMode::Off);
    c.review.enabled = false;
    let mut off = app_in(&tmp.path().join("o"), c);
    off.set_sender(tx.clone());
    assert!(!off.start_review());

    let mut on = app_in(
        &tmp.path().join("y"),
        config(Some(&fake.command()), SidebarMode::Off),
    );
    on.set_sender(tx);
    assert!(on.start_review());
    assert!(on.review_running());
}

#[test]
fn gutter_status_and_jumps() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("nb");
    let mut app = app_in(&dir, config(None, SidebarMode::Off));
    let before = screen(&app);
    assert!(!before.iter().any(|l| l.contains('●')));
    assert_eq!(app.review_gutter(), 0);
    let (alpha, beta, gamma, delta) = (
        row_of(&app, "alpha"),
        row_of(&app, "beta"),
        row_of(&app, "gamma"),
        row_of(&app, "delta"),
    );

    // No markers yet.
    keys(&mut app, "]r");
    assert_eq!(app.status(), NO_REVIEW);

    // Lines 5-7 (beta..gamma) and a line past the end (shown on the last).
    app.event(AppEvent::Review(markers(
        &dir,
        &[("doc.md", marks(3, &[(5, 7), (40, 40)]))],
    )));
    assert_eq!(app.review_gutter(), 1);
    assert_eq!(app.review_count(), 3);
    let s = screen(&app);
    for (row, text) in [(beta, "beta"), (gamma, "gamma"), (delta, "delta")] {
        assert_eq!(s[row].trim_end(), format!("●{text}"), "row {row}");
    }
    assert!(!s[alpha].starts_with('●'), "{:?}", s[alpha]);
    assert!(
        s[alpha].starts_with(" alpha"),
        "content moved right: {:?}",
        s[alpha]
    );
    assert!(s[(ROWS - 1) as usize].contains("review 3"), "{s:?}");

    keys(&mut app, "]r");
    assert_eq!(app.cursor().row, beta);
    keys(&mut app, "]r");
    assert_eq!(app.cursor().row, delta);
    keys(&mut app, "]r");
    assert_eq!(app.cursor().row, delta);
    assert_eq!(app.status(), NO_MORE_REVIEW);
    keys(&mut app, "[r");
    assert_eq!(app.cursor().row, beta);
    keys(&mut app, "[r");
    assert_eq!(app.status(), NO_MORE_REVIEW);

    // Markers gone: the gutter goes and the layout is as before.
    app.event(AppEvent::Review(Markers::default()));
    assert_eq!(app.review_gutter(), 0);
    let s = screen(&app);
    assert!(!s.iter().any(|l| l.contains('●')));
    assert!(!s[(ROWS - 1) as usize].contains("review 3"));
}

#[test]
fn file_level_comments_count_but_get_no_gutter() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("nb");
    let mut app = app_in(&dir, config(None, SidebarMode::Off));
    app.event(AppEvent::Review(markers(
        &dir,
        &[("doc.md", marks(1, &[]))],
    )));
    assert_eq!(app.review_gutter(), 0);
    assert_eq!(app.review_count(), 1);
    assert!(screen(&app)[(ROWS - 1) as usize].contains("review 1"));
}

#[test]
fn tree_marks_files_with_counts_and_folders() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("nb");
    let mut app = app_in(&dir, config(None, SidebarMode::Files));
    app.event(AppEvent::Review(markers(
        &dir,
        &[("sub/b.md", marks(2, &[(1, 1)]))],
    )));
    assert_eq!(app.file_marker(&dir.join("sub")), Some(('●', None)));
    assert_eq!(app.file_marker(&dir.join("sub/b.md")), Some(('●', Some(2))));
    assert_eq!(app.file_marker(&dir.join("doc.md")), None);
    let s = screen(&app);
    let sub = s.iter().find(|l| l.contains("sub")).unwrap();
    assert!(sub.contains("sub ●"), "{sub:?}");
    assert!(!s.iter().any(|l| l.contains("doc.md ●")), "{s:?}");
}

#[test]
fn app_tracks_the_discovery_dir_and_stdin_has_none() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("nb");
    let app = app_in(&dir, config(None, SidebarMode::Off));
    assert_eq!(app.review_dir(), Some(std::path::absolute(&dir).unwrap()));
    let s = App::new(
        StartOptions {
            target: StartTarget::Stdin("# S\n".into()),
            tree_root: dir.clone(),
            config: config(None, SidebarMode::Off),
        },
        (COLS, ROWS),
    )
    .unwrap();
    assert_eq!(s.review_dir(), None);
}

#[test]
fn a_launcher_pauses_the_thread_and_it_polls_on_return() {
    let tmp = tempfile::tempdir().unwrap();
    let fake_dir = tmp.path().join("fake");
    std::fs::create_dir_all(&fake_dir).unwrap();
    let fake = Fake::new(&fake_dir);
    let dir = canonical(&{
        let d = tmp.path().join("nb");
        std::fs::create_dir_all(&d).unwrap();
        d
    });
    fake.sessions(&[]);
    // Short intervals: without the pause, the thread would poll while the
    // launcher runs.
    let short = Intervals {
        discovery: Duration::from_millis(50),
        comments: Duration::from_millis(50),
    };
    let app = app_in(&dir, config(Some(&fake.command()), SidebarMode::Off));
    let (tx, rx) = mpsc::channel();
    let fake_for_runner = Fake {
        dir: fake_dir.clone(),
    };
    let calls_during = std::rc::Rc::new(std::cell::Cell::new(usize::MAX));
    let seen = calls_during.clone();
    let mut app = app.with_runner(move |_cmd| {
        // Like `<leader>rr`: tuicr runs (and a session appears) while
        // ramble is suspended.
        // Let a poll already in flight finish.
        std::thread::sleep(Duration::from_millis(500));
        let before = fake_for_runner.log().len();
        fake_for_runner.sessions(&[("rr", false)]);
        fake_for_runner.comments("rr", &format!("[{}]", line("doc.md", 3, 3, "new")));
        std::thread::sleep(Duration::from_millis(300));
        seen.set(fake_for_runner.log().len() - before);
        Ok(ramble::app::Exit::Code(0))
    });
    app.set_sender(tx);
    assert!(app.start_review_with(short));
    // Polling with nothing found.
    wait_until("two polls", || fake.log().len() > 1);
    assert!(fake.log().iter().all(|l| l.starts_with("review list")));
    // Mark it active so the next poll discovers it; the runner then makes
    // it inactive, as tuicr does on quit, and the sticky rule keeps it.
    fake.sessions(&[("rr", true)]);
    wait_until("the session is found", || {
        fake.log().iter().any(|l| l.starts_with("review comments"))
    });
    app.launch("review");
    app.run_pending_effect();
    assert_eq!(calls_during.get(), 0, "no polls while the launcher ran");
    let m = recv_review(&rx, Duration::from_secs(5)).expect("poll on return");
    app.event(AppEvent::Review(m));
    assert_eq!(app.review_count(), 1);
}

fn wait_until(what: &str, pred: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !pred() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn keys(app: &mut App, s: &str) {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    for c in s.chars() {
        app.event(AppEvent::Key(KeyEvent::new(
            KeyCode::Char(c),
            KeyModifiers::NONE,
        )));
    }
}

// ---- Real tuicr e2e -------------------------------------------------------

fn has(cmd: &str, arg: &str) -> bool {
    Command::new(cmd)
        .arg(arg)
        .output()
        .is_ok_and(|o| o.status.success())
}

fn quote(p: &Path) -> String {
    format!("'{}'", p.display().to_string().replace('\'', r"'\''"))
}

#[test]
fn real_tuicr_comment_shows_in_ramble() {
    if !has("tuicr", "--version") || !has("tmux", "-V") {
        eprintln!("skipping real_tuicr_comment_shows_in_ramble: tuicr or tmux not found on PATH");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = canonical(tmp.path());
    let home = root.join("home");
    let nb = root.join("nb");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&nb).unwrap();
    let doc = nb.join("a.md");
    std::fs::write(&doc, DOC).unwrap();
    let cfg = root.join("ramble.toml");
    std::fs::write(&cfg, "[lsp]\nserver = []\n[sidebar]\ndefault = \"off\"\n").unwrap();
    let envs = [
        ("HOME", home.clone()),
        ("XDG_DATA_HOME", root.join("data")),
        ("XDG_CONFIG_HOME", root.join("config")),
        ("XDG_STATE_HOME", root.join("state")),
        ("XDG_CACHE_HOME", root.join("cache")),
    ];
    let env_prefix: String = envs
        .iter()
        .map(|(k, v)| format!("{k}={} ", quote(v)))
        .collect();
    let sock = root.join("sock");
    let tmux = |args: &[&str]| {
        Command::new("tmux")
            .arg("-S")
            .arg(&sock)
            .args(args)
            .envs(envs.iter().map(|(k, v)| (*k, v)))
            .env_remove("TMUX")
            .output()
            .unwrap()
    };
    struct Kill<F: Fn(&[&str]) -> std::process::Output>(F);
    impl<F: Fn(&[&str]) -> std::process::Output> Drop for Kill<F> {
        fn drop(&mut self) {
            (self.0)(&["kill-server"]);
        }
    }
    let tmux = Kill(tmux);
    let tuicr = |args: &[&str]| {
        Command::new("tuicr")
            .args(args)
            .current_dir(&nb)
            .envs(envs.iter().map(|(k, v)| (*k, v)))
            .output()
            .unwrap()
    };

    let out = (tmux.0)(&[
        "-f",
        "/dev/null",
        "new-session",
        "-d",
        "-s",
        "e2e",
        "-x",
        "80",
        "-y",
        "24",
        "-c",
        nb.to_str().unwrap(),
        &format!("env {env_prefix}tuicr --file {}", quote(&doc)),
    ]);
    assert!(out.status.success(), "tmux: {out:?}");

    // Wait for tuicr's active session.
    let nb_s = nb.to_str().unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let slug = loop {
        let o = tuicr(&["review", "list", "--repo", nb_s]);
        let list: Vec<serde_json::Value> = serde_json::from_slice(&o.stdout).unwrap_or_default();
        if let Some(s) = list.iter().find(|s| s["active"] == true) {
            let path = s["path"].as_str().unwrap_or_default();
            assert!(
                Path::new(path).starts_with(&root),
                "session file outside the isolated HOME: {path}"
            );
            break s["slug"].as_str().unwrap().to_string();
        }
        assert!(Instant::now() < deadline, "no active tuicr session: {o:?}");
        std::thread::sleep(Duration::from_millis(100));
    };
    let o = tuicr(&[
        "review",
        "add",
        "--session",
        &slug,
        "--repo",
        nb_s,
        "--target-file",
        "a.md",
        "--line",
        "5",
        "--username",
        "e2e",
        "marker please",
    ]);
    assert!(o.status.success(), "tuicr review add: {o:?}");

    let bin = env!("CARGO_BIN_EXE_ramble");
    let out = (tmux.0)(&[
        "new-window",
        "-t",
        "e2e",
        "-n",
        "ramble",
        &format!(
            "env {env_prefix}{} --config {} {}",
            quote(Path::new(bin)),
            quote(&cfg),
            quote(&doc)
        ),
    ]);
    assert!(out.status.success(), "tmux: {out:?}");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let screen =
            String::from_utf8_lossy(&(tmux.0)(&["capture-pane", "-p", "-t", "e2e:ramble"]).stdout)
                .into_owned();
        let beta = screen.lines().find(|l| l.contains("beta"));
        if screen.contains("review 1") && beta.is_some_and(|l| l.starts_with('●')) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the review marker; screen:\n{screen}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    (tmux.0)(&["send-keys", "-t", "e2e:ramble", "q"]);
}
