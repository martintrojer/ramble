//! Review markers (spec § Review markers, § Testing): the review module
//! against a fake `tuicr` script, app markers, and one real tuicr e2e in an
//! isolated tmux (skipped without tuicr or tmux).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ramble::app::{App, AppEvent, NO_MORE_REVIEW, NO_REVIEW, StartOptions, StartTarget};
use ramble::config::{Config, SidebarMode, SidebarShow};
use ramble::review::{
    self, FileMarks, Intervals, Markers, Poller, Tuicr, canonical, discovery_dirs,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

/// A fake `tuicr` in `dir/bin`: `review list` prints `dir/list.json`;
/// `review comments --session S` prints `dir/comments-<S with / @ ~ as _>.json`.
/// With [`Fake::only`], both print `[]` for any other `--repo`.
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
  list) repo=$4 ;;
  comments) repo=$6 ;;
  *) exit 2 ;;
esac
if [ -f "$D/only" ] && [ "$repo" != "$(cat "$D/only")" ]; then echo '[]'; exit 0; fi
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

    /// Sessions exist only under `--repo dir`, as tuicr keys them.
    fn only(&self, dir: &Path) {
        std::fs::write(self.dir.join("only"), dir.display().to_string()).unwrap();
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
    let m = p
        .step(std::slice::from_ref(&repo), Instant::now(), false)
        .unwrap();
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
    assert!(p.step(std::slice::from_ref(&repo), t0, false).is_some());
    assert_eq!(fake.log().len(), 2);
    // Nothing due before 2 s.
    assert!(
        p.step(
            std::slice::from_ref(&repo),
            t0 + Duration::from_secs(1),
            false
        )
        .is_none()
    );
    assert_eq!(fake.log().len(), 2);
    // Comments at 2 s; unchanged, so no event.
    assert!(
        p.step(
            std::slice::from_ref(&repo),
            t0 + Duration::from_secs(2),
            false
        )
        .is_none()
    );
    let log = fake.log();
    assert_eq!(log.len(), 3);
    assert!(log[2].starts_with("review comments"), "{log:?}");
    // Changed comments are reported.
    fake.comments("s", &format!("[{}]", line("a.md", 2, 2, "new")));
    let m = p
        .step(
            std::slice::from_ref(&repo),
            t0 + Duration::from_secs(4),
            false,
        )
        .unwrap();
    assert_eq!(m.get(&repo.join("a.md")), Some(&marks(1, &[(2, 2)])));
    // Discovery again at 5 s.
    p.step(
        std::slice::from_ref(&repo),
        t0 + Duration::from_secs(5),
        false,
    );
    assert!(fake.log()[4].starts_with("review list"), "{:?}", fake.log());
}

#[test]
fn no_session_means_no_comment_calls() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new(tmp.path());
    let mut p = Poller::new(fake.tuicr(), SLOW);
    let t0 = Instant::now();
    assert!(p.step(&[tmp.path().to_path_buf()], t0, false).is_none());
    p.step(
        &[tmp.path().to_path_buf()],
        t0 + Duration::from_secs(3),
        false,
    );
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
    p.step(std::slice::from_ref(&repo), t0, false).unwrap();
    // tuicr quit: the session is inactive, but stays sticky.
    fake.sessions(&[("one", false)]);
    p.step(std::slice::from_ref(&repo), t0, true);
    assert_eq!(p.sessions(), vec![(repo.clone(), "one".to_string())]);
    // A different session becomes active and replaces it.
    fake.sessions(&[("one", false), ("two", true)]);
    let m = p.step(std::slice::from_ref(&repo), t0, true).unwrap();
    assert_eq!(p.sessions(), vec![(repo.clone(), "two".to_string())]);
    assert!(m.get(&repo.join("a.md")).is_none());
    assert!(m.get(&repo.join("b.md")).is_some());
    fake.sessions(&[("two", false)]);
    p.step(std::slice::from_ref(&repo), t0, true);
    assert_eq!(p.sessions(), vec![(repo.clone(), "two".to_string())]);
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
    let m = p
        .step(std::slice::from_ref(&repo), Instant::now(), false)
        .unwrap();
    assert_eq!(
        m.get(&repo.join("a.md")),
        Some(&marks(2, &[(1, 1), (5, 6)]))
    );
}

#[test]
fn discovery_dirs_walk_from_the_folder_to_the_vcs_root() {
    let tmp = tempfile::tempdir().unwrap();
    let plain = tmp.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    assert_eq!(discovery_dirs(&plain.join("a.md")), vec![plain]);
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(repo.join(".jj")).unwrap();
    std::fs::create_dir_all(repo.join("notes/x")).unwrap();
    assert_eq!(discovery_dirs(&repo.join("a.md")), vec![repo.clone()]);
    assert_eq!(
        discovery_dirs(&repo.join("notes/x/a.md")),
        vec![repo.join("notes/x"), repo.join("notes"), repo.clone()]
    );
    // The walk stops after 8 ancestors; the root is still queried.
    let deep = repo.join("1/2/3/4/5/6/7/8/9/10");
    let dirs = discovery_dirs(&deep.join("a.md"));
    assert_eq!(dirs.len(), 10, "{dirs:?}");
    assert_eq!(dirs[0], deep);
    assert_eq!(dirs[8], repo.join("1/2"));
    assert_eq!(dirs[9], repo);
}

#[test]
fn sessions_are_listed_per_dir_and_paths_resolve_against_it() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = Fake::new(tmp.path());
    let repo = tmp.path().join("repo");
    let notes = repo.join("notes");
    fake.only(&notes);
    fake.sessions(&[("s", true)]);
    fake.comments("s", &format!("[{}]", line("a.md", 2, 2, "new")));
    let mut p = Poller::new(fake.tuicr(), SLOW);
    let t0 = Instant::now();
    let m = p.step(&[notes.clone(), repo.clone()], t0, false).unwrap();
    assert_eq!(m.get(&notes.join("a.md")), Some(&marks(1, &[(2, 2)])));
    assert_eq!(m.files.len(), 1, "{m:?}");
    assert_eq!(p.sessions(), vec![(notes.clone(), "s".to_string())]);
    // Sticky per directory: inactive now, still counted under notes only.
    fake.sessions(&[("s", false)]);
    p.step(&[notes.clone(), repo.clone()], t0, true);
    assert_eq!(p.sessions(), vec![(notes.clone(), "s".to_string())]);
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
    h.control.set_dirs(vec![repo.clone()]);
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

fn config(review_cmd: Option<&Path>, sidebar: Option<SidebarMode>) -> Config {
    let mut c = Config::default();
    c.lsp.server = vec![];
    // None: the sidebar hidden.
    match sidebar {
        Some(m) => {
            c.sidebar.default = m;
            // COLS is below the default auto-hide threshold.
            c.sidebar.auto_hide_below = 0;
        }
        None => c.sidebar.show = SidebarShow::Never,
    }
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
        config(Some(&tmp.path().join("nope/tuicr")), None),
    );
    missing.set_sender(tx.clone());
    assert!(!missing.start_review());
    assert!(!missing.review_running());

    let mut c = config(Some(&fake.command()), None);
    c.review.enabled = false;
    let mut off = app_in(&tmp.path().join("o"), c);
    off.set_sender(tx.clone());
    assert!(!off.start_review());

    let mut on = app_in(&tmp.path().join("y"), config(Some(&fake.command()), None));
    on.set_sender(tx);
    assert!(on.start_review());
    assert!(on.review_running());
}

#[test]
fn gutter_status_and_jumps() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("nb");
    let mut app = app_in(&dir, config(None, None));
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

/// A reflowed paragraph: rows are labelled with the first source line they
/// draw, so lines 2..n of a paragraph must reach their rows via the srcmap.
const PARA: &str = "# P\n\nfirst line\nsecond line\nthird line\n\nnext para\n\nlast para\nmore\n";

fn para_app(dir: &Path, cols: u16) -> App {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("p.md"), PARA).unwrap();
    App::new(
        StartOptions {
            target: StartTarget::File(dir.join("p.md")),
            tree_root: dir.to_path_buf(),
            config: config(None, None),
        },
        (cols, ROWS),
    )
    .unwrap()
}

fn gutter_rows(app: &App, cols: u16) -> Vec<usize> {
    let mut term = Terminal::new(TestBackend::new(cols, ROWS)).unwrap();
    term.draw(|f| ramble::ui::draw(f, app)).unwrap();
    let buf = term.backend().buffer();
    (0..ROWS - 1)
        .filter(|&y| buf[(0, y)].symbol() == "●")
        .map(|y| app.scroll() + y as usize)
        .collect()
}

#[test]
fn comments_inside_a_reflowed_paragraph_mark_and_target_its_rows() {
    for cols in [80, 12] {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("nb");
        let mut app = para_app(&dir, cols);

        // Line 4 ("second line"), the middle of a 3-line paragraph.
        app.event(AppEvent::Review(markers(
            &dir,
            &[("p.md", marks(1, &[(4, 4)]))],
        )));
        let second = row_of(&app, "second");
        assert_eq!(gutter_rows(&app, cols), vec![second], "cols {cols}");
        keys(&mut app, "gg]r");
        assert_eq!(app.cursor().row, second, "cols {cols}");
        keys(&mut app, "]r");
        assert_eq!(app.status(), NO_MORE_REVIEW, "cols {cols}");

        // Line 10 ("more"), the last line of the last paragraph.
        app.event(AppEvent::Review(markers(
            &dir,
            &[("p.md", marks(1, &[(10, 10)]))],
        )));
        let more = row_of(&app, "more");
        assert_eq!(gutter_rows(&app, cols), vec![more], "cols {cols}");
        keys(&mut app, "gg]r");
        assert_eq!(app.cursor().row, more, "cols {cols}");
    }
}

#[test]
fn file_level_comments_count_but_get_no_gutter() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("nb");
    let mut app = app_in(&dir, config(None, None));
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
    let mut app = app_in(&dir, config(None, Some(SidebarMode::Files)));
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
    let app = app_in(&dir, config(None, None));
    assert_eq!(app.review_dirs(), vec![std::path::absolute(&dir).unwrap()]);
    let s = App::new(
        StartOptions {
            target: StartTarget::Stdin("# S\n".into()),
            tree_root: dir.clone(),
            config: config(None, None),
        },
        (COLS, ROWS),
    )
    .unwrap();
    assert!(s.review_dirs().is_empty());
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
    let app = app_in(&dir, config(Some(&fake.command()), None));
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

#[test]
fn a_session_keyed_to_the_files_folder_below_the_repo_root_is_found() {
    // `tuicr --file notes/doc.md` keys its session to `<repo>/notes`, not
    // the repo root (tuicr 0.25.0).
    let tmp = tempfile::tempdir().unwrap();
    let fake_dir = tmp.path().join("fake");
    std::fs::create_dir_all(&fake_dir).unwrap();
    let fake = Fake::new(&fake_dir);
    let repo = canonical(&{
        let r = tmp.path().join("repo");
        std::fs::create_dir_all(r.join(".git")).unwrap();
        r
    });
    let notes = repo.join("notes");
    fake.only(&notes);
    fake.sessions(&[("rr", true)]);
    fake.comments("rr", &format!("[{}]", line("doc.md", 5, 5, "new")));
    let mut app = app_in(&notes, config(Some(&fake.command()), None));
    let (tx, rx) = mpsc::channel();
    app.set_sender(tx);
    let short = Intervals {
        discovery: Duration::from_millis(50),
        comments: Duration::from_millis(50),
    };
    assert!(app.start_review_with(short));
    let m = recv_review(&rx, Duration::from_secs(5))
        .unwrap_or_else(|| panic!("no markers; tuicr calls: {:?}", fake.log()));
    app.event(AppEvent::Review(m));
    assert_eq!(app.review_count(), 1);
}

#[test]
fn back_jump_goes_to_the_nearest_previous_comment() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("nb");
    let mut app = app_in(&dir, config(None, None));
    let (alpha, beta, gamma) = (
        row_of(&app, "alpha"),
        row_of(&app, "beta"),
        row_of(&app, "gamma"),
    );
    app.event(AppEvent::Review(markers(
        &dir,
        &[("doc.md", marks(3, &[(3, 3), (5, 5), (7, 7)]))],
    )));
    keys(&mut app, "]r]r]r");
    assert_eq!(app.cursor().row, gamma);
    keys(&mut app, "[r");
    assert_eq!(app.cursor().row, beta);
    keys(&mut app, "[r");
    assert_eq!(app.cursor().row, alpha);
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
    real_tuicr_e2e("real_tuicr_comment_shows_in_ramble", false);
}

/// The default `<leader>rr` on a file below the repo root: tuicr runs in
/// the repo root with `--file notes/a.md` and keys the session to `notes`.
#[test]
fn real_tuicr_comment_in_a_repo_subfolder_shows_in_ramble() {
    if !has("git", "--version") {
        eprintln!("skipping: git not found on PATH");
        return;
    }
    real_tuicr_e2e(
        "real_tuicr_comment_in_a_repo_subfolder_shows_in_ramble",
        true,
    );
}

/// Start tuicr on `a.md` in tmux (from the notebook folder, or with `repo`
/// from the root of a git repo holding `notes/a.md`), add a comment with
/// `tuicr review add`, then wait for ramble to show it. HOME and XDG dirs
/// point into a temp dir.
fn real_tuicr_e2e(name: &str, repo: bool) {
    if !has("tuicr", "--version") || !has("tmux", "-V") {
        eprintln!("skipping {name}: tuicr or tmux not found on PATH");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = canonical(tmp.path());
    let home = root.join("home");
    let (cwd, nb) = if repo {
        let r = root.join("repo");
        (r.clone(), r.join("notes"))
    } else {
        (root.join("nb"), root.join("nb"))
    };
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&nb).unwrap();
    let doc = nb.join("a.md");
    std::fs::write(&doc, DOC).unwrap();
    let cfg = root.join("ramble.toml");
    std::fs::write(&cfg, "[lsp]\nserver = []\n[sidebar]\nshow = \"never\"\n").unwrap();
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
    if repo {
        let o = Command::new("git")
            .args(["init", "-q"])
            .current_dir(&cwd)
            .envs(envs.iter().map(|(k, v)| (*k, v)))
            .output()
            .unwrap();
        assert!(o.status.success(), "git init: {o:?}");
    }
    let file_arg = doc.strip_prefix(&cwd).unwrap().to_path_buf();
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
        cwd.to_str().unwrap(),
        &format!("env {env_prefix}tuicr --file {}", quote(&file_arg)),
    ]);
    assert!(out.status.success(), "tmux: {out:?}");

    // Wait for tuicr's active session.
    let nb_s = nb.to_str().unwrap();
    // Event-driven: exits as soon as the session appears; the long deadline
    // only absorbs startup latency under heavy load.
    let deadline = Instant::now() + Duration::from_secs(60);
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
        if Instant::now() >= deadline {
            let pane = (tmux.0)(&["capture-pane", "-p", "-t", "e2e"]);
            panic!(
                "no active tuicr session: {o:?}\ntuicr pane:\n{}",
                String::from_utf8_lossy(&pane.stdout)
            );
        }
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
