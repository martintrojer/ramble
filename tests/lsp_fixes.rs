//! LSP document lifecycle: every `didOpen` is balanced by a `didClose`
//! before the same document is opened again, driven by the scripted fake
//! server (`tests/support/fake_lsp.rs`).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ramble::app::{App, AppEvent, FsEvent, StartOptions, StartTarget};
use ramble::config::{Config, ServerConfig, ServerKind, SidebarShow};
use serde_json::{Value, json};

fn key(c: char, m: KeyModifiers) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), m)
}

fn keys(app: &mut App, s: &str) {
    for c in s.chars() {
        app.handle_key(key(c, KeyModifiers::NONE));
    }
}

fn row_text(app: &App, row: usize) -> String {
    app.page().unwrap().rendered.lines[row]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect()
}

/// Move the cursor to the row reading `text`.
fn goto_text(app: &mut App, text: &str) {
    let rows = app.page().unwrap().rendered.lines.len();
    let row = (0..rows).find(|&r| row_text(app, r) == text).unwrap();
    keys(app, "gg");
    for _ in 0..row {
        keys(app, "j");
    }
}

fn pump_until(app: &mut App, what: &str, pred: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !pred(app) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        app.pump_lsp(Duration::from_millis(20));
    }
}

/// `(method, file name, version)` of every didOpen / didClose fake-lsp
/// logged, in order. A line still being written does not parse and is
/// skipped.
fn lifecycle(log: &Path) -> Vec<(String, String, Option<i64>)> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|m| m["method"] == "textDocument/didOpen" || m["method"] == "textDocument/didClose")
        .map(|m| {
            let td = &m["params"]["textDocument"];
            let name = td["uri"].as_str().unwrap().rsplit('/').next().unwrap();
            let method = m["method"].as_str().unwrap();
            let method = method.trim_start_matches("textDocument/");
            (method.to_string(), name.to_string(), td["version"].as_i64())
        })
        .collect()
}

fn wait_lifecycle(log: &Path, n: usize) -> Vec<(String, String, Option<i64>)> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let got = lifecycle(log);
        if got.len() >= n {
            return got;
        }
        assert!(Instant::now() < deadline, "only got {got:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// `(method, uri)` of every didOpen / didClose fake-lsp logged, in order.
fn lifecycle_uris(log: &Path) -> Vec<(String, String)> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|m| m["method"] == "textDocument/didOpen" || m["method"] == "textDocument/didClose")
        .map(|m| {
            let method = m["method"].as_str().unwrap();
            let uri = m["params"]["textDocument"]["uri"].as_str().unwrap();
            (
                method.trim_start_matches("textDocument/").into(),
                uri.into(),
            )
        })
        .collect()
}

/// Starts an app on `file` under `root` (which holds `.fake`) with the fake
/// server; returns it once the server runs and the first didOpen is logged,
/// with the log path.
fn start(root: &Path, file: &Path) -> (App, PathBuf) {
    let script = root.join("script.json");
    let caps = json!({"capabilities": {"definitionProvider": true, "documentLinkProvider": {}}});
    std::fs::write(
        &script,
        json!([{"expect": "initialize", "reply": caps}]).to_string(),
    )
    .unwrap();
    let log = root.join("log.jsonl");
    let mut config = Config::default();
    config.sidebar.show = SidebarShow::Never;
    config.lsp.server = vec![ServerConfig {
        kind: ServerKind::Generic,
        command: vec![
            env!("CARGO_BIN_EXE_fake-lsp").into(),
            script.display().to_string(),
            log.display().to_string(),
        ],
        root_markers: vec![".fake".into()],
        position_encoding: None,
    }];
    let opts = StartOptions {
        target: StartTarget::File(file.to_path_buf()),
        tree_root: root.to_path_buf(),
        config,
        review_cache: None,
        mdroots: ramble::app::MdrootsOptions::memory(),
    };
    let mut app = App::new(opts, (40, 12)).unwrap();
    pump_until(&mut app, "server running", |a| a.lsp_label().ends_with('●'));
    wait_lifecycle(&log, 1);
    (app, log)
}

#[test]
fn did_open_is_balanced_by_did_close_across_reload_navigation_and_back() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join(".fake"), "").unwrap();
    let a = root.join("a.md");
    std::fs::write(&a, "# A\n\n[b](b.md)\n").unwrap();
    std::fs::write(root.join("b.md"), "# B\n\nback to [a](a.md)\n").unwrap();
    let (mut app, log) = start(&root, &a);

    // Reload: close, then reopen at the next version.
    std::fs::write(&a, "# A\n\n[b](b.md)\n\nmore\n").unwrap();
    app.event(AppEvent::FsWatch(a.clone(), FsEvent::Changed));
    wait_lifecycle(&log, 3);

    // Navigation: leaving a.md closes it.
    goto_text(&mut app, "b");
    keys(&mut app, "gd");
    pump_until(&mut app, "on b.md", |a| a.history_depth() == 1);
    wait_lifecycle(&log, 5);

    // C-o: leaving b.md closes it; a.md reopens.
    app.handle_key(key('o', KeyModifiers::CONTROL));
    let got = wait_lifecycle(&log, 7);

    let ev = |m: &str, f: &str, v: Option<i64>| (m.to_string(), f.to_string(), v);
    assert_eq!(
        got,
        vec![
            ev("didOpen", "a.md", Some(1)),
            ev("didClose", "a.md", None),
            ev("didOpen", "a.md", Some(2)),
            ev("didClose", "a.md", None),
            ev("didOpen", "b.md", Some(1)),
            ev("didClose", "b.md", None),
            ev("didOpen", "a.md", Some(3)),
        ]
    );
}

/// A page reached through the link `alias.md -> a.md`: start.md links to
/// alias.md, a.md links on to b.md. Returns the app on alias.md (its path
/// kept as the alias, its didOpen sent for a.md) and the log.
fn on_followed_symlink(root: &Path) -> (App, PathBuf) {
    std::fs::write(root.join(".fake"), "").unwrap();
    let start_md = root.join("start.md");
    std::fs::write(&start_md, "# Start\n\n[alias](alias.md)\n").unwrap();
    std::fs::write(root.join("a.md"), "# A\n\n[b](b.md)\n").unwrap();
    std::fs::write(root.join("b.md"), "# B\n").unwrap();
    std::os::unix::fs::symlink(root.join("a.md"), root.join("alias.md")).unwrap();
    let (mut app, log) = start(root, &start_md);
    goto_text(&mut app, "alias");
    keys(&mut app, "gd");
    pump_until(&mut app, "on alias.md", |a| a.history_depth() == 1);
    assert_eq!(
        app.page().unwrap().path.as_deref(),
        Some(root.join("alias.md").as_path())
    );
    let got = wait_lifecycle(&log, 3);
    assert_eq!(got[2], ("didOpen".into(), "a.md".into(), Some(1)));
    (app, log)
}

fn uri(p: &Path) -> String {
    ramble::lsp::canonical_uri(p).as_str().to_string()
}

#[test]
fn did_close_reuses_the_did_open_uri_after_the_symlink_is_retargeted() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let (mut app, log) = on_followed_symlink(&root);

    let alias = root.join("alias.md");
    std::fs::remove_file(&alias).unwrap();
    std::os::unix::fs::symlink(root.join("b.md"), &alias).unwrap();
    app.reload();
    wait_lifecycle(&log, 5);

    let got = lifecycle_uris(&log);
    let ev = |m: &str, f: &str| (m.to_string(), uri(&root.join(f)));
    assert_eq!(
        got[2..],
        [
            ev("didOpen", "a.md"),
            ev("didClose", "a.md"),
            ev("didOpen", "b.md"),
        ]
    );
}

#[test]
fn did_close_reuses_the_did_open_uri_after_the_symlink_is_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let (mut app, log) = on_followed_symlink(&root);

    std::fs::remove_file(root.join("alias.md")).unwrap();
    // The page still shows a.md's text; follow its link to b.md.
    goto_text(&mut app, "b");
    keys(&mut app, "gd");
    pump_until(&mut app, "on b.md", |a| a.history_depth() == 2);
    wait_lifecycle(&log, 5);

    let got = lifecycle_uris(&log);
    let ev = |m: &str, f: &str| (m.to_string(), uri(&root.join(f)));
    assert_eq!(
        got[2..],
        [
            ev("didOpen", "a.md"),
            ev("didClose", "a.md"),
            ev("didOpen", "b.md"),
        ]
    );
}
