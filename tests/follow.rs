//! `--follow`: editor cursor updates moving the view (`App::follow_event`)
//! and the real socket round trip (`follow_sock`).
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ramble::app::{App, AppEvent, FOLLOW_GRACE, Mode, StartOptions, StartTarget};
use ramble::config::{Config, SidebarShow};
use ramble::follow_sock;
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
    let dir = base.path().join("ramble-test");
    (base, dir)
}

#[test]
fn a_sent_line_reaches_the_listener_in_scope_only() {
    let (_base, dir) = sock_dir();
    let (tx, rx) = mpsc::channel();
    let listener = follow_sock::listen(&dir, "t1w2", move |p, l| {
        let _ = tx.send((p, l));
    })
    .unwrap();
    assert!(listener.path().exists());

    // Another window's sender doesn't reach it (negative window: 300 ms
    // is far longer than a local socket write).
    follow_sock::send_all(&dir, "t1w3", 7, Path::new("/n/a.md"));
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());

    follow_sock::send_all(&dir, "t1w2", 7, Path::new("/n/a b.md"));
    let got = rx.recv_timeout(Duration::from_secs(2)).expect("delivered");
    assert_eq!(got, (PathBuf::from("/n/a b.md"), 7));

    let sock = listener.path().to_path_buf();
    drop(listener);
    assert!(!sock.exists(), "dropping the listener removes the socket");
}

#[test]
fn a_dead_socket_is_removed_by_the_next_sender() {
    let (_base, dir) = sock_dir();
    follow_sock::ensure_dir(&dir).unwrap();
    let stale = dir.join(follow_sock::socket_name("any", 1));
    // Bind and drop: the file stays, nothing listens.
    drop(std::os::unix::net::UnixListener::bind(&stale).unwrap());
    assert!(stale.exists());
    follow_sock::send_all(&dir, "any", 1, Path::new("/x.md"));
    assert!(!stale.exists());
}

#[test]
fn a_socket_dir_others_can_open_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let (_base, dir) = sock_dir();
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    let err = follow_sock::listen(&dir, "any", |_, _| {}).unwrap_err();
    assert!(err.to_string().contains("other users"), "{err}");
}

#[test]
fn a_client_holding_its_connection_open_does_not_stall_others() {
    use std::io::Write;
    let (_base, dir) = sock_dir();
    let (tx, rx) = mpsc::channel();
    let listener = follow_sock::listen(&dir, "any", move |p, l| {
        let _ = tx.send((p, l));
    })
    .unwrap();
    // Keeps writing well inside the per-read timeout, never closes.
    let sock = listener.path().to_path_buf();
    let hog = std::thread::spawn(move || {
        let mut s = std::os::unix::net::UnixStream::connect(&sock).unwrap();
        for _ in 0..40 {
            if s.write_all(b"1 /hog.md\n").is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });
    std::thread::sleep(Duration::from_millis(50));
    follow_sock::send_all(&dir, "any", 7, Path::new("/n/a.md"));
    // The hog gets at most its own deadline (500 ms); the sender must be
    // served well before the hog's 4 s of writes end.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut got = None;
    while let Ok(m) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        if m.0 == Path::new("/n/a.md") {
            got = Some(m);
            break;
        }
    }
    assert_eq!(got, Some((PathBuf::from("/n/a.md"), 7)));
    hog.join().unwrap();
}

#[test]
fn senders_skip_a_socket_dir_others_can_open_and_non_sockets() {
    use std::os::unix::fs::PermissionsExt;
    let (_base, dir) = sock_dir();
    let (tx, rx) = mpsc::channel();
    let listener = follow_sock::listen(&dir, "any", move |p, l| {
        let _ = tx.send((p, l));
    })
    .unwrap();
    // A plain file named like a socket is never touched.
    let plain = dir.join(follow_sock::socket_name("any", 2));
    std::fs::write(&plain, "keep").unwrap();
    // Open the dir to others: as if someone else had made it.
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
    follow_sock::send_all(&dir, "any", 3, Path::new("/n/a.md"));
    // Negative window: a local socket write lands in well under 300 ms.
    assert!(
        rx.recv_timeout(Duration::from_millis(300)).is_err(),
        "sent into a dir others can open"
    );
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    follow_sock::send_all(&dir, "any", 3, Path::new("/n/a.md"));
    assert!(rx.recv_timeout(Duration::from_secs(2)).is_ok());
    assert!(plain.exists(), "a non-socket *.sock was removed");
    drop(listener);
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
