//! Live reload with the real `notify` watcher (spec § Testing "live
//! reload, real watcher"): write to a temp file and wait up to 2s.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use ramble::app::{App, AppEvent, FileWatcher, FsEvent, StartOptions, StartTarget, content_hash};
use ramble::config::Config;

const WAIT: Duration = Duration::from_secs(2);

/// The default config without language servers (N5).
fn no_lsp() -> Config {
    let mut c = Config::default();
    c.lsp.server = vec![];
    c
}

#[test]
fn watcher_reports_change_then_removal() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.md");
    std::fs::write(&file, "one\n").unwrap();
    let (tx, rx) = mpsc::channel::<(PathBuf, FsEvent)>();
    let mut w = FileWatcher::new(move |p, e| {
        let _ = tx.send((p, e));
    })
    .unwrap();
    w.watch(Some((&file, content_hash(b"one\n"))));
    // A sibling file alone does not count.
    std::fs::write(dir.path().join("other.md"), "x\n").unwrap();
    assert!(
        rx.recv_timeout(Duration::from_millis(500)).is_err(),
        "sibling write reported"
    );
    std::fs::write(&file, "two\n").unwrap();
    let (path, ev) = rx.recv_timeout(WAIT).expect("change event within 2s");
    assert_eq!(ev, FsEvent::Changed);
    assert_eq!(path.file_name(), file.file_name());
    // Drain any trailing burst, then delete.
    while rx.recv_timeout(Duration::from_millis(300)).is_ok() {}
    std::fs::remove_file(&file).unwrap();
    let (_, ev) = rx.recv_timeout(WAIT).expect("remove event within 2s");
    assert_eq!(ev, FsEvent::Removed);
}

#[test]
fn events_that_leave_the_file_unchanged_are_not_reported() {
    // macOS FSEvents replays writes from just before the watch started;
    // an event is only reported when the bytes differ from what the
    // watcher last saw. Rewriting the same bytes has the same shape.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.md");
    std::fs::write(&file, "one\n").unwrap();
    let (tx, rx) = mpsc::channel::<(PathBuf, FsEvent)>();
    let mut w = FileWatcher::new(move |p, e| {
        let _ = tx.send((p, e));
    })
    .unwrap();
    w.watch(Some((&file, content_hash(b"one\n"))));
    std::fs::write(&file, "one\n").unwrap();
    assert!(
        rx.recv_timeout(Duration::from_millis(500)).is_err(),
        "unchanged file reported"
    );
    std::fs::write(&file, "two\n").unwrap();
    let (_, ev) = rx.recv_timeout(WAIT).expect("change event within 2s");
    assert_eq!(ev, FsEvent::Changed);
}

/// A watcher on `file` reporting into the returned channel.
fn watcher() -> (FileWatcher, mpsc::Receiver<(PathBuf, FsEvent)>) {
    let (tx, rx) = mpsc::channel();
    let w = FileWatcher::new(move |p, e| {
        let _ = tx.send((p, e));
    })
    .unwrap();
    (w, rx)
}

#[test]
fn a_write_between_load_and_watch_is_reported() {
    // The page was built from A; B landed before the watch started, so no
    // watcher event covers it. The baseline is A, so it still counts.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.md");
    std::fs::write(&file, "A\n").unwrap();
    let loaded = std::fs::read(&file).unwrap();
    std::fs::write(&file, "B\n").unwrap();
    let (mut w, rx) = watcher();
    w.watch(Some((&file, content_hash(&loaded))));
    let (_, ev) = rx.recv_timeout(WAIT).expect("change event within 2s");
    assert_eq!(ev, FsEvent::Changed);
}

#[test]
fn rewatching_the_same_file_reseeds_the_baseline() {
    // A reload of the same path built from older bytes than the file now
    // holds must be followed by a change, not swallowed.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.md");
    std::fs::write(&file, "B\n").unwrap();
    let (mut w, rx) = watcher();
    w.watch(Some((&file, content_hash(b"B\n"))));
    assert!(
        rx.recv_timeout(Duration::from_millis(500)).is_err(),
        "unchanged file reported"
    );
    w.watch(Some((&file, content_hash(b"A\n"))));
    let (_, ev) = rx.recv_timeout(WAIT).expect("change event within 2s");
    assert_eq!(ev, FsEvent::Changed);
}

#[test]
fn app_sees_a_write_that_lands_before_the_watcher_starts() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.md");
    std::fs::write(&file, "# A\n\nbefore\n").unwrap();
    let mut app = App::new(
        StartOptions {
            target: StartTarget::File(file.clone()),
            tree_root: dir.path().to_path_buf(),
            config: no_lsp(),
            review_cache: None,
            mdroots: ramble::app::MdrootsOptions::memory(),
        },
        (40, 10),
    )
    .unwrap();
    std::fs::write(&file, "# A\n\nafter\n").unwrap();
    let (tx, rx) = mpsc::channel::<AppEvent>();
    app.set_sender(tx);
    app.start_watcher().unwrap();
    let ev = rx.recv_timeout(WAIT).expect("fs event within 2s");
    assert!(
        matches!(ev, AppEvent::FsWatch(_, FsEvent::Changed)),
        "{ev:?}"
    );
    app.event(ev);
    assert!(app.page().unwrap().doc.source.contains("after"));
}

#[test]
fn app_reloads_through_real_watcher() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.md");
    std::fs::write(&file, "# A\n\nbefore\n").unwrap();
    let mut app = App::new(
        StartOptions {
            target: StartTarget::File(file.clone()),
            tree_root: dir.path().to_path_buf(),
            config: no_lsp(),
            review_cache: None,
            mdroots: ramble::app::MdrootsOptions::memory(),
        },
        (40, 10),
    )
    .unwrap();
    let (tx, rx) = mpsc::channel::<AppEvent>();
    app.set_sender(tx);
    app.start_watcher().unwrap();
    std::fs::write(&file, "# A\n\nafter\n").unwrap();
    let ev = rx.recv_timeout(WAIT).expect("fs event within 2s");
    assert!(
        matches!(ev, AppEvent::FsWatch(_, FsEvent::Changed)),
        "{ev:?}"
    );
    app.event(ev);
    assert!(app.page().unwrap().doc.source.contains("after"));
    // The reload re-seeds the watcher with the new bytes, so it settles:
    // a stale baseline would report the same write again, forever.
    assert!(
        rx.recv_timeout(Duration::from_millis(600)).is_err(),
        "watcher kept reporting after the reload"
    );
}

#[test]
fn opening_another_page_retargets_the_watcher() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.md");
    let b = dir.path().join("b.md");
    std::fs::write(&a, "# A\n").unwrap();
    std::fs::write(&b, "# B\n\nbefore\n").unwrap();
    let mut app = App::new(
        StartOptions {
            target: StartTarget::File(a.clone()),
            tree_root: dir.path().to_path_buf(),
            config: no_lsp(),
            review_cache: None,
            mdroots: ramble::app::MdrootsOptions::memory(),
        },
        (40, 10),
    )
    .unwrap();
    let (tx, rx) = mpsc::channel::<AppEvent>();
    app.set_sender(tx);
    app.start_watcher().unwrap();
    app.open_file(&b).unwrap();
    std::fs::write(&b, "# B\n\nafter\n").unwrap();
    let ev = rx.recv_timeout(WAIT).expect("fs event for b.md within 2s");
    assert!(
        matches!(&ev, AppEvent::FsWatch(p, FsEvent::Changed) if p.file_name() == b.file_name()),
        "{ev:?}"
    );
    app.event(ev);
    assert!(app.page().unwrap().doc.source.contains("after"));
}
