//! Code-path links: inline code naming an existing file is followable.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ramble::app::{App, Effect, StartOptions, StartTarget, resolve_code_path};
use ramble::config::{Config, ServerConfig, ServerKind, SidebarShow};
use ramble::doc::{Link, LinkKind};
use serde_json::json;
use tempfile::TempDir;

const COLS: u16 = 60;
const ROWS: u16 = 12;

fn opts(tree_root: &Path, target: StartTarget) -> StartOptions {
    let mut config = Config::default();
    config.sidebar.show = SidebarShow::Never;
    config.lsp.server = vec![];
    StartOptions {
        target,
        tree_root: tree_root.to_path_buf(),
        config,
        review_cache: None,
    }
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A temp dir holding `a.md` with `source` (plus `files`), opened as the tree root.
fn app_with(source: &str, files: &[&str]) -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    for f in files {
        write(&root.join(f), "x\n");
    }
    write(&root.join("a.md"), source);
    let app = App::new(
        opts(&root, StartTarget::File(root.join("a.md"))),
        (COLS, ROWS),
    )
    .unwrap();
    (dir, app)
}

fn code_paths(app: &App) -> Vec<Link> {
    let links = &app.page().unwrap().doc.links;
    links
        .iter()
        .filter(|l| l.kind == LinkKind::CodePath)
        .cloned()
        .collect()
}

fn dests(app: &App) -> Vec<String> {
    code_paths(app).into_iter().map(|l| l.dest).collect()
}

fn keys(app: &mut App, s: &str) {
    for c in s.chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
}

fn row_text(app: &App, row: usize) -> String {
    let line = &app.page().unwrap().rendered.lines[row];
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// Cursor onto the first drawn cell of `text` (rendered text).
fn goto(app: &mut App, text: &str) {
    let n = app.page().unwrap().rendered.lines.len();
    let (row, col) = (0..n)
        .find_map(|r| row_text(app, r).find(text).map(|c| (r, c)))
        .unwrap_or_else(|| panic!("no {text:?} on screen"));
    keys(app, "gg");
    for _ in 0..row {
        keys(app, "j");
    }
    keys(app, "0");
    for _ in 0..col {
        keys(app, "l");
    }
    assert_eq!((app.cursor().row, app.cursor().col), (row, col));
}

#[derive(Default)]
struct Edits(Vec<(PathBuf, Option<usize>)>);

fn recording(app: App) -> (App, Rc<RefCell<Edits>>) {
    let fx = Rc::new(RefCell::new(Edits::default()));
    let e = fx.clone();
    let app = app
        .with_effects(|_| panic!("no opener"), |_| panic!("plain editor unused"))
        .with_editor(move |p, line| {
            e.borrow_mut().0.push((p.to_path_buf(), line));
            Ok(())
        });
    (app, fx)
}

#[test]
fn resolution_order_file_dir_then_vcs_root_then_tree_root() {
    let dir = tempfile::tempdir().unwrap();
    let tree = dir.path().canonicalize().unwrap();
    let repo = tree.join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let docs = repo.join("docs");
    for d in [&docs, &repo, &tree] {
        write(&d.join("all.txt"), "x\n");
    }
    write(&repo.join("two.txt"), "x\n");
    write(&tree.join("two.txt"), "x\n");
    write(&tree.join("tree.txt"), "x\n");
    write(
        &docs.join("a.md"),
        "`all.txt` `two.txt` `tree.txt` `./all.txt` `../two.txt`\n",
    );
    let app = App::new(
        opts(&tree, StartTarget::File(docs.join("a.md"))),
        (COLS, ROWS),
    )
    .unwrap();
    let got: Vec<_> = code_paths(&app)
        .into_iter()
        .map(|l| (l.dest, l.resolved.unwrap()))
        .collect();
    assert_eq!(
        got,
        vec![
            ("all.txt".into(), docs.join("all.txt")),
            ("two.txt".into(), repo.join("two.txt")),
            ("tree.txt".into(), tree.join("tree.txt")),
            ("./all.txt".into(), docs.join("all.txt")),
            ("../two.txt".into(), docs.join("../two.txt")),
        ]
    );
}

#[test]
fn line_and_column_suffixes() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().to_path_buf();
    write(&d.join("a.rs"), "x\n");
    write(&d.join("odd:7"), "x\n");
    let dirs = [d.clone()];
    let r = |t: &str| resolve_code_path(t, &dirs, None);
    assert_eq!(r("a.rs"), Some((d.join("a.rs"), None)));
    assert_eq!(r("a.rs:12"), Some((d.join("a.rs"), Some(12))));
    assert_eq!(r("a.rs:12:4"), Some((d.join("a.rs"), Some(12))));
    assert_eq!(r("  a.rs:3  "), Some((d.join("a.rs"), Some(3))));
    // The full text wins when it names a file.
    assert_eq!(r("odd:7"), Some((d.join("odd:7"), None)));
    assert_eq!(r("a.rs:0"), None);
    assert_eq!(r("a.rs:x"), None);
    assert_eq!(r("a.rs:1:2:3"), None);
}

#[test]
fn home_expansion_uses_the_injected_env() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().canonicalize().unwrap().join("home");
    write(&home.join("notes/x.rs"), "x\n");
    assert_eq!(
        resolve_code_path("~/notes/x.rs:2", &[], Some(&home)),
        Some((home.join("notes/x.rs"), Some(2)))
    );
    assert_eq!(resolve_code_path("~/notes/x.rs", &[], None), None);
    assert_eq!(resolve_code_path("~bob/x.rs", &[], Some(&home)), None);

    // In the app: HOME comes from the env lookup, applied on reload.
    let (_d, app) = app_with("`~/notes/x.rs`\n", &[]);
    assert!(dests(&app).is_empty());
    let h = home.clone();
    let mut app = app.with_env(move |k| (k == "HOME").then(|| h.display().to_string()));
    app.reload();
    let links = code_paths(&app);
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].resolved, Some(home.join("notes/x.rs")));
}

#[test]
fn only_existing_regular_files_are_links() {
    let long = format!("`{}`", "a".repeat(600));
    let src = format!(
        "`there.txt` `gone.txt` `sub` `has space.txt` `https://x.io/there.txt` \
         `let x = 1` [`there.txt`](there.txt) {long}\n"
    );
    let (_d, app) = app_with(&src, &["there.txt", "has space.txt", "sub/inner.txt"]);
    assert_eq!(dests(&app), ["there.txt"]);
    // The code inside the markdown link stays that link's text.
    let links = &app.page().unwrap().doc.links;
    assert_eq!(links.len(), 2);
    assert_eq!(links[1].kind, LinkKind::Markdown);
}

#[test]
fn absolute_paths_resolve() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().canonicalize().unwrap().join("abs.txt");
    write(&f, "x\n");
    let (_d, app) = app_with(&format!("`{}:9`\n", f.display()), &[]);
    let links = code_paths(&app);
    assert_eq!(links[0].resolved, Some(f));
    assert_eq!(links[0].line, Some(9));
}

#[test]
fn code_path_segments_are_link_styled() {
    use ratatui::style::Modifier;
    let (_d, app) = app_with("see `b.txt` and `nope.txt`\n", &["b.txt"]);
    let p = app.page().unwrap();
    let i = p
        .doc
        .links
        .iter()
        .position(|l| l.kind == LinkKind::CodePath);
    let seg = p.rendered.srcmap.segments.iter().find(|s| s.link.is_some());
    assert_eq!(seg.and_then(|s| s.link), i);
    let line = &p.rendered.lines[0];
    let span = |t: &str| line.spans.iter().find(|s| s.content.contains(t)).unwrap();
    let linked = span("b.txt").style;
    let plain = span("nope.txt").style;
    assert!(linked.add_modifier.contains(Modifier::UNDERLINED));
    assert_eq!(linked.fg, plain.fg, "keeps the code colour");
    assert!(!plain.add_modifier.contains(Modifier::UNDERLINED));
}

#[test]
fn markdown_target_opens_at_line() {
    let mut b = String::from("# B\n\n");
    for i in 1..=30 {
        b.push_str(&format!("Line {i}\n\n"));
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    write(&root.join("b.md"), &b);
    write(&root.join("a.md"), "go `b.md:23` or `b.md:999`\n");
    let mut app = App::new(
        opts(&root, StartTarget::File(root.join("a.md"))),
        (COLS, ROWS),
    )
    .unwrap();
    goto(&mut app, "b.md:23");
    keys(&mut app, "gd");
    assert_eq!(
        app.page().unwrap().path.as_deref(),
        Some(root.join("b.md").as_path())
    );
    assert_eq!(app.history_depth(), 1);
    // Source line 23 is "Line 11" (lines 1-2 are the heading and a blank).
    assert_eq!(row_text(&app, app.cursor().row), "Line 11");

    send_ctrl_o(&mut app);
    goto(&mut app, "b.md:999");
    keys(&mut app, "gd");
    assert_eq!(app.status(), "Line 999 is past the end");
    let last = app.page().unwrap().rendered.lines.len() - 1;
    assert_eq!(app.cursor().row, last);
}

fn send_ctrl_o(app: &mut App) {
    app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
}

#[test]
fn non_markdown_target_goes_to_the_editor_with_line() {
    let (dir, app) = app_with("`src/m.rs:42:7` and `src/m.rs`\n", &["src/m.rs"]);
    let f = dir.path().canonicalize().unwrap().join("src/m.rs");
    let (mut app, fx) = recording(app);
    goto(&mut app, "src/m.rs:42");
    keys(&mut app, "gd");
    assert_eq!(
        app.pending_effect(),
        Some(&Effect::Edit {
            path: f.clone(),
            line: Some(42)
        })
    );
    app.run_pending_effect();
    keys(&mut app, "]l");
    keys(&mut app, "gd");
    app.run_pending_effect();
    assert_eq!(fx.borrow().0, [(f.clone(), Some(42)), (f, None)]);
    assert_eq!(app.history_depth(), 0);
}

#[test]
fn deleted_since_load_is_a_status() {
    let (dir, app) = app_with("`gone.txt`\n", &["gone.txt"]);
    let (mut app, fx) = recording(app);
    std::fs::remove_file(dir.path().join("gone.txt")).unwrap();
    goto(&mut app, "gone.txt");
    keys(&mut app, "gd");
    assert_eq!(app.status(), "No such file: gone.txt");
    assert_eq!(app.pending_effect(), None);
    assert!(fx.borrow().0.is_empty());
}

#[test]
fn link_motion_and_hints_include_code_paths() {
    let (_d, mut app) = app_with("# A\n\nplain `x.txt` then [b](b.md)\n", &["x.txt"]);
    keys(&mut app, "gg]l");
    let row = app.cursor().row;
    assert_eq!(&row_text(&app, row)[app.cursor().col..][..5], "x.txt");
    keys(&mut app, "]l");
    assert_eq!(&row_text(&app, row)[app.cursor().col..][..1], "b");
    keys(&mut app, "gg");
    keys(&mut app, "s");
    assert_eq!(app.hints().len(), 2);
}

#[test]
fn stdin_resolves_against_cwd() {
    // `cargo test` runs in the crate root; never change cwd (tests share it).
    let dir = tempfile::tempdir().unwrap();
    let app = App::new(
        opts(dir.path(), StartTarget::Stdin("`Cargo.toml:3`\n".into())),
        (COLS, ROWS),
    )
    .unwrap();
    let cwd = std::env::current_dir().unwrap();
    let links = code_paths(&app);
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].resolved, Some(cwd.join("Cargo.toml")));
    assert_eq!(links[0].line, Some(3));
}

/// documentLink and diagnostics ranges never land on a code-path link.
#[test]
fn lsp_results_skip_code_path_links() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    write(&root.join(".fake"), "");
    write(&root.join("x.txt"), "x\n");
    write(&root.join("b.md"), "# B\n");
    // Link 0 is the code path `x.txt` (cols 0..7), link 1 is [b](b) (8..14).
    write(&root.join("a.md"), "`x.txt` [b](b)\n");
    let uri = |p: &Path| ramble::lsp::canonical_uri(p).as_str().to_string();
    let range = |a: u32, b: u32| json!({"start": {"line": 0, "character": a}, "end": {"line": 0, "character": b}});
    let script = json!([
        {"expect": "initialize", "reply": {"capabilities": {
            "definitionProvider": true, "documentLinkProvider": {}}}},
        {"expect": "textDocument/documentLink", "reply": [
            {"range": range(0, 14), "target": uri(&root.join("b.md"))},
        ]},
        {"send": {"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics",
            "params": {"uri": uri(&root.join("a.md")), "version": 1,
                "diagnostics": [{"range": range(0, 14), "message": "dead"}]}}},
    ]);
    let script_path = root.join("script.json");
    std::fs::write(&script_path, script.to_string()).unwrap();
    let mut o = opts(&root, StartTarget::File(root.join("a.md")));
    o.config.lsp.server = vec![ServerConfig {
        kind: ServerKind::Generic,
        command: vec![
            env!("CARGO_BIN_EXE_fake-lsp").into(),
            script_path.display().to_string(),
        ],
        root_markers: vec![".fake".into()],
        position_encoding: None,
    }];
    let mut app = App::new(o, (COLS, ROWS)).unwrap();
    let links = &app.page().unwrap().doc.links;
    assert_eq!(links[0].kind, LinkKind::CodePath);
    assert_eq!(links[1].kind, LinkKind::Markdown);
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.link_target(1).is_none() {
        assert!(
            Instant::now() < deadline,
            "no documentLink; {}",
            app.status()
        );
        app.pump_lsp(Duration::from_millis(20));
    }
    while app.broken_links().is_empty() {
        assert!(
            Instant::now() < deadline,
            "no diagnostics; {}",
            app.status()
        );
        app.pump_lsp(Duration::from_millis(20));
    }
    assert_eq!(app.link_target(0), None);
    assert_eq!(app.link_target(1), Some(root.join("b.md").as_path()));
    // The diagnostic covers both; only the markdown link is broken.
    assert_eq!(app.broken_links().iter().copied().collect::<Vec<_>>(), [1]);
    // gd on the code path is local even with a definition provider.
    let (mut app, fx) = recording(app);
    goto(&mut app, "x.txt");
    keys(&mut app, "gd");
    app.run_pending_effect();
    assert_eq!(fx.borrow().0, [(root.join("x.txt"), None)]);
}
