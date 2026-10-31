//! `--follow`: editor cursor updates moving the view (`App::follow_event`)
//! and a socket round trip through `debrief-follow`.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ramble::app::{App, AppEvent, FOLLOW_GRACE, Mode, StartOptions, StartTarget};
use ramble::config::{Config, SidebarShow};
use tempfile::TempDir;

const ROWS: u16 = 12;
/// Content rows: ROWS minus the status line.
const VH: usize = 11;

/// 60 one-row paragraphs: paragraph i (1-based) is on source line 2i - 1.
fn source() -> String {
    (1..=60).map(|i| format!("para {i}\n\n")).collect()
}

fn line_of(i: usize) -> usize {
    2 * i - 1
}

fn opts(dir: &Path, target: StartTarget) -> StartOptions {
    let mut config = Config::default();
    config.sidebar.show = SidebarShow::Never;
    config.lsp.server = vec![];
    StartOptions {
        target,
        tree_root: dir.to_path_buf(),
        config,
        review_cache: None,
        mdroots: ramble::app::MdrootsOptions::memory(),
    }
}

fn setup() -> (TempDir, PathBuf, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("doc.md");
    std::fs::write(&path, source()).unwrap();
    let app = App::new(
        opts(dir.path(), StartTarget::File(path.clone())),
        (40, ROWS),
    )
    .unwrap();
    (dir, path, app)
}

fn cursor_text(app: &App) -> String {
    app.page().unwrap().rendered.lines[app.cursor().row]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect()
}

fn key(c: char) -> AppEvent {
    AppEvent::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
}

/// Well past any grace window started by setup or earlier keys.
fn later() -> Instant {
    Instant::now() + FOLLOW_GRACE * 2
}

#[test]
fn a_visible_line_moves_only_the_cursor() {
    let (_d, path, mut app) = setup();
    app.follow_event(&path, line_of(3), later());
    assert_eq!(cursor_text(&app), "para 3");
    assert_eq!(app.scroll(), 0, "already on screen: no scroll");
}

#[test]
fn an_off_screen_line_is_centred() {
    let (_d, path, mut app) = setup();
    app.follow_event(&path, line_of(40), later());
    assert_eq!(cursor_text(&app), "para 40");
    let row = app.cursor().row;
    assert_eq!(app.scroll(), row - (VH - 1) / 2, "centred");
    // Back up to a line still on screen: the view stays put.
    let scroll = app.scroll();
    app.follow_event(&path, line_of(39), later());
    assert_eq!(cursor_text(&app), "para 39");
    assert_eq!(app.scroll(), scroll);
}

#[test]
fn a_line_past_the_end_lands_on_the_last_row() {
    let (_d, path, mut app) = setup();
    app.follow_event(&path, 10_000, later());
    let last = app.page().unwrap().rendered.lines.len() - 1;
    assert_eq!(app.cursor().row, last);
}

#[test]
fn updates_in_the_grace_window_after_a_key_are_dropped() {
    let (_d, path, mut app) = setup();
    app.event(key('j'));
    let typed = Instant::now();
    let row = app.cursor().row;
    app.follow_event(&path, line_of(40), typed + FOLLOW_GRACE / 2);
    assert_eq!(app.cursor().row, row, "dropped inside the window");
    app.follow_event(
        &path,
        line_of(40),
        typed + FOLLOW_GRACE + Duration::from_millis(10),
    );
    assert_eq!(cursor_text(&app), "para 40", "followed after it");
}

#[test]
fn updates_are_dropped_outside_normal_mode() {
    let (_d, path, mut app) = setup();
    app.event(key('/'));
    assert_eq!(app.mode(), Mode::Search);
    let row = app.cursor().row;
    app.follow_event(&path, line_of(40), later());
    assert_eq!(app.cursor().row, row);
}

#[test]
fn another_markdown_file_opens_and_c_o_returns() {
    let (dir, path, mut app) = setup();
    let other = dir.path().join("other.md");
    std::fs::write(&other, "# Other\n\nfirst\n\nsecond\n").unwrap();
    app.follow_event(&other, 5, later());
    let shown = app.page().unwrap().path.clone().unwrap();
    assert_eq!(shown.canonicalize().unwrap(), other.canonicalize().unwrap());
    assert_eq!(cursor_text(&app), "second");
    app.event(AppEvent::Key(KeyEvent::new(
        KeyCode::Char('o'),
        KeyModifiers::CONTROL,
    )));
    let back = app.page().unwrap().path.clone().unwrap();
    assert_eq!(back.canonicalize().unwrap(), path.canonicalize().unwrap());
}

#[test]
fn non_markdown_and_missing_files_are_ignored_silently() {
    let (dir, path, mut app) = setup();
    let rs = dir.path().join("main.rs");
    std::fs::write(&rs, "fn main() {}\n").unwrap();
    app.follow_event(&rs, 1, later());
    app.follow_event(&dir.path().join("gone.md"), 1, later());
    assert_eq!(app.page().unwrap().path.as_deref(), Some(path.as_path()));
    assert_eq!(app.status(), "");
}

/// A short socket dir: Unix socket paths are capped near 104 bytes.
fn sock_dir() -> (TempDir, PathBuf) {
    let base = tempfile::Builder::new()
        .prefix("f")
        .tempdir_in("/tmp")
        .unwrap();
    let dir = base.path().join("follow-test");
    (base, dir)
}

/// The socket side lives in debrief-follow (tested there); this checks
/// ramble's use of it: a sent line arrives, named as ramble's socket.
#[test]
fn a_sent_line_reaches_a_ramble_listener() {
    let (_base, dir) = sock_dir();
    let (tx, rx) = mpsc::channel();
    let listener = debrief_follow::listen(&dir, "any", "ramble", move |p, l| {
        let _ = tx.send((p, l));
    })
    .unwrap();
    let name = listener
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert!(name.starts_with("any.ramble."), "{name}");
    debrief_follow::send_all(&dir, "any", 7, Path::new("/n/a b.md"));
    let got = rx.recv_timeout(Duration::from_secs(2)).expect("delivered");
    assert_eq!(got, (PathBuf::from("/n/a b.md"), 7));
}

fn mouse(kind: MouseEventKind, now: Instant) -> AppEvent {
    AppEvent::Mouse(
        MouseEvent {
            kind,
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        },
        now,
    )
}

#[test]
fn a_click_or_paste_starts_the_grace_window_but_hovering_does_not() {
    let (_d, path, mut app) = setup();
    let t = later();
    app.event(mouse(MouseEventKind::Moved, t));
    app.follow_event(&path, line_of(40), t + Duration::from_millis(10));
    assert_eq!(cursor_text(&app), "para 40", "hover is not using ramble");

    app.event(mouse(MouseEventKind::ScrollDown, t));
    let row = app.cursor().row;
    app.follow_event(&path, line_of(3), t + FOLLOW_GRACE / 2);
    assert_eq!(app.cursor().row, row, "dropped after a scroll");

    let (_d, path, mut app) = setup();
    app.event(AppEvent::Paste("x".into()));
    let pasted = Instant::now();
    app.follow_event(&path, line_of(40), pasted + FOLLOW_GRACE / 2);
    assert_ne!(cursor_text(&app), "para 40", "dropped after a paste");
}

#[test]
fn a_half_typed_key_sequence_or_count_pauses_following() {
    for typed in ['g', '5'] {
        let (_d, path, mut app) = setup();
        app.event(key(typed));
        let row = app.cursor().row;
        app.follow_event(&path, line_of(40), later());
        assert_eq!(app.cursor().row, row, "after {typed:?}");
    }
}

#[test]
fn an_unreadable_or_binary_markdown_file_is_ignored_silently() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, path, mut app) = setup();
    let bin = dir.path().join("bin.md");
    std::fs::write(&bin, b"a\0b").unwrap();
    let locked = dir.path().join("locked.md");
    std::fs::write(&locked, "# x\n").unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    app.follow_event(&bin, 1, later());
    app.follow_event(&locked, 1, later());
    assert_eq!(app.page().unwrap().path.as_deref(), Some(path.as_path()));
    assert_eq!(app.status(), "");
}

/// `App`'s listener (`--follow`, or the first follow launcher): started
/// once (no rebind), removed with the app, and a bind failure is the
/// "follow off" status.
#[test]
fn the_listener_starts_once_and_a_failure_is_a_status() {
    use std::os::unix::fs::PermissionsExt;
    let (_base, dir) = sock_dir();
    let (_d, _path, mut app) = setup();
    let (tx, _rx) = mpsc::channel();
    app.set_sender(tx);
    assert!(!app.following());
    app.follow_start_in(&dir);
    assert!(app.following(), "{}", app.status());
    let socks = || std::fs::read_dir(&dir).unwrap().count();
    assert_eq!(socks(), 1);
    app.follow_start_in(&dir);
    assert_eq!(socks(), 1, "no rebind");
    drop(app);
    assert_eq!(socks(), 0, "dropping the app removes the socket");

    let (_d, _path, mut app) = setup();
    let (tx, _rx) = mpsc::channel();
    app.set_sender(tx);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    app.follow_start_in(&dir);
    assert!(!app.following());
    assert!(app.status().starts_with("follow off: "), "{}", app.status());
}
