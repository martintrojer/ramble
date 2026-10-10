//! LSP document lifecycle: every `didOpen` is balanced by a `didClose`
//! before the same document is opened again, driven by the scripted fake
//! server (`tests/support/fake_lsp.rs`).

use std::path::Path;
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

#[test]
fn did_open_is_balanced_by_did_close_across_reload_navigation_and_back() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join(".fake"), "").unwrap();
    let a = root.join("a.md");
    std::fs::write(&a, "# A\n\n[b](b.md)\n").unwrap();
    std::fs::write(root.join("b.md"), "# B\n\nback to [a](a.md)\n").unwrap();
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
        target: StartTarget::File(a.clone()),
        tree_root: root.clone(),
        config,
        review_cache: None,
        mdroots: ramble::app::MdrootsOptions::memory(),
    };
    let mut app = App::new(opts, (40, 12)).unwrap();
    pump_until(&mut app, "server running", |a| a.lsp_label().ends_with('●'));
    wait_lifecycle(&log, 1);

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
