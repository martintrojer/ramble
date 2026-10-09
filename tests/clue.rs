//! The key clue (spec `2026-10-07-key-clue.md`): rows, timing, rendering
//! and the `[keys] clue` setting.

use std::path::Path;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ramble::app::{App, CLUE_DELAY, Focus, Mode, StartOptions, StartTarget};
use ramble::config::{Config, ServerConfig, ServerKind, SidebarMode, SidebarSide};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::json;
use tempfile::TempDir;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn keys(app: &mut App, s: &str) {
    for c in s.chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
}

fn config() -> Config {
    let mut c = Config::default();
    c.sidebar.default = SidebarMode::Files;
    c.lsp.server = vec![];
    c
}

fn write_doc(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("doc.md");
    std::fs::write(&path, "# Title\n\nSee [b](b.md).\n\nMore text.\n").unwrap();
    path
}

fn app_sized(config: Config, size: (u16, u16)) -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = write_doc(dir.path());
    let opts = StartOptions {
        target: StartTarget::File(path),
        tree_root: dir.path().to_path_buf(),
        config,
        review_cache: None,
        mdroots: ramble::app::MdrootsOptions::memory(),
    };
    let app = App::new(opts, size).unwrap();
    (dir, app)
}

fn app() -> (TempDir, App) {
    app_sized(config(), (100, 30))
}

/// An app with a fake zk server running, so every picker is available.
fn zk_app() -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join(".fake"), "").unwrap();
    let path = write_doc(&root);
    let script = json!([
        {"expect": "initialize", "reply": {"capabilities": {
            "definitionProvider": true, "hoverProvider": true,
            "documentLinkProvider": {}, "referencesProvider": true}}},
        {"expect": "workspace/executeCommand", "reply": {}},
    ]);
    let script_path = root.join("script.json");
    std::fs::write(&script_path, script.to_string()).unwrap();
    let mut c = config();
    c.lsp.server = vec![ServerConfig {
        kind: ServerKind::Zk,
        command: vec![
            env!("CARGO_BIN_EXE_fake-lsp").into(),
            script_path.display().to_string(),
        ],
        root_markers: vec![".fake".into()],
        position_encoding: None,
    }];
    let opts = StartOptions {
        target: StartTarget::File(path),
        tree_root: root.clone(),
        config: c,
        review_cache: None,
        mdroots: ramble::app::MdrootsOptions::memory(),
    };
    let mut app = App::new(opts, (100, 30)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !app.lsp_label().ends_with('●') {
        assert!(Instant::now() < deadline, "server never ran");
        app.pump_lsp(Duration::from_millis(20));
    }
    (dir, app)
}

fn row_keys(app: &App) -> Vec<String> {
    app.clue_rows().into_iter().map(|r| r.key).collect()
}

fn desc(app: &App, k: &str) -> Option<String> {
    app.clue_rows()
        .into_iter()
        .find(|r| r.key == k)
        .map(|r| r.desc)
}

/// Let the pause pass.
fn wait(app: &mut App) {
    app.tick(Instant::now() + CLUE_DELAY + Duration::from_millis(1));
}

fn screen(app: &App) -> Vec<String> {
    let (w, h) = app.size();
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| ramble::ui::draw(f, app)).unwrap();
    let buf = term.backend().buffer().clone();
    (0..h)
        .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect()
}

// Rows.

#[test]
fn g_lists_its_keys_without_a_server() {
    let (_d, mut app) = app();
    keys(&mut app, "g");
    assert_eq!(row_keys(&app), ["?", "R", "d", "g", "x"]);
    assert_eq!(desc(&app, "d").unwrap(), "follow the link");
    assert_eq!(desc(&app, "g").unwrap(), "first line (or line N)");
}

#[test]
fn g_lists_v_after_a_selection() {
    let (_d, mut app) = app();
    app.handle_key(key(KeyCode::Char('v')));
    app.handle_key(key(KeyCode::Esc));
    keys(&mut app, "g");
    assert!(row_keys(&app).contains(&"v".to_string()));
}

#[test]
fn g_and_leader_z_with_a_zk_server() {
    let (_d, mut app) = zk_app();
    keys(&mut app, "g");
    let r = app.clue_rows().into_iter().find(|r| r.key == "r").unwrap();
    assert_eq!((r.desc.as_str(), r.group), ("+pickers", true));
    keys(&mut app, "r");
    assert_eq!(row_keys(&app), ["r"]);
    assert_eq!(desc(&app, "r").unwrap(), "backlinks");
    app.handle_key(key(KeyCode::Esc));
    keys(&mut app, " z");
    assert_eq!(row_keys(&app), ["b", "f", "l", "s", "z"]);
}

#[test]
fn leader_z_without_a_server_hides_unavailable_pickers() {
    let (_d, mut app) = app();
    keys(&mut app, " z");
    assert_eq!(row_keys(&app), ["f", "l"]);
    assert_eq!(desc(&app, "f").unwrap(), "notes");
}

#[test]
fn leader_lists_groups_built_ins_and_launchers() {
    let mut c = config();
    for (name, key) in [("one", "<leader>rr"), ("two", "<leader>rw")] {
        c.launch.push(ramble::config::Launcher {
            name: name.into(),
            key: Some(key.into()),
            command: vec!["true".into()],
            needs_vcs: false,
            disabled: false,
        });
    }
    let (_d, mut app) = app_sized(c, (100, 30));
    keys(&mut app, " ");
    let rows = app.clue_rows();
    let find = |k: &str| rows.iter().find(|r| r.key == k);
    let z = find("z").expect("z row");
    assert_eq!((z.desc.as_str(), z.group), ("+pickers", true));
    assert!(find("e").is_some_and(|r| !r.group), "e row (any text)");
    assert_eq!(find("o").unwrap().desc, "edit");
    // `r` mixes the launchers with the built-in `<leader>rl`.
    let r = find("r").expect("prefix r");
    assert_eq!((r.desc.as_str(), r.group), ("+3 keys", true));
    assert_eq!(app.clue_title(), "Space");
    keys(&mut app, "r");
    assert_eq!(app.clue_title(), "Space r");
    assert_eq!(row_keys(&app), ["l", "r", "w"]);
    // The launcher on `<leader>rr` replaces the built-in send.
    assert_eq!(desc(&app, "r").unwrap(), "one");
    assert_eq!(desc(&app, "l").unwrap(), "list the review comments");
}

#[test]
fn mixed_groups_show_a_key_count() {
    // A launcher under `<leader>z` mixes Launchers with Pickers.
    let mut c = config();
    c.launch.push(ramble::config::Launcher {
        name: "zap".into(),
        key: Some("<leader>zq".into()),
        command: vec!["true".into()],
        needs_vcs: false,
        disabled: false,
    });
    let (_d, mut app) = app_sized(c, (100, 30));
    keys(&mut app, " ");
    assert_eq!(desc(&app, "z").unwrap(), "+3 keys", "zf, zl, zq");
}

#[test]
fn ctrl_w_lists_window_keys_per_focus() {
    let (_d, mut app) = app();
    assert!(app.sidebar_cols() > 0, "sidebar shown at 100 cols");
    app.handle_key(ctrl('w'));
    assert_eq!(row_keys(&app), ["W", "h", "j", "k", "p", "w"]);
    assert_eq!(app.clue_title(), "C-w");
    app.handle_key(key(KeyCode::Char('h')));
    assert_ne!(app.focus(), Focus::Content);
    app.handle_key(ctrl('w'));
    assert_eq!(row_keys(&app), ["W", "h", "j", "k", "l", "p", "w"]);
}

#[test]
fn ctrl_w_with_the_sidebar_hidden_shows_no_box() {
    for side in [SidebarSide::Left, SidebarSide::Right] {
        let mut c = config();
        c.sidebar.side = side;
        let (_d, mut app) = app_sized(c, (30, 20));
        assert_eq!(app.sidebar_cols(), 0, "too narrow for the sidebar");
        app.handle_key(ctrl('w'));
        wait(&mut app);
        assert!(
            app.clue_rows().is_empty(),
            "{side:?}: {:?}",
            app.clue_rows()
        );
        assert!(!app.clue_visible(), "{side:?}");
    }
}

#[test]
fn marks_give_one_placeholder_row() {
    let (_d, mut app) = app();
    keys(&mut app, "m");
    let rows = app.clue_rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (rows[0].key.as_str(), rows[0].desc.as_str()),
        ("{a-z}", "set a mark")
    );
}

#[test]
fn yank_operator_lists_its_keys() {
    let (_d, mut app) = app();
    keys(&mut app, "y");
    assert_eq!(app.mode(), Mode::OpPending);
    assert_eq!(row_keys(&app), ["F", "f", "u", "y", "{motion}"]);
    assert_eq!(app.clue_title(), "y");
    keys(&mut app, "g");
    assert_eq!(app.clue_title(), "y g");
    assert_eq!(row_keys(&app), ["g"], "only motions after y g");
}

#[test]
fn visual_mode_keeps_only_keys_visual_accepts() {
    let (_d, mut app) = app();
    keys(&mut app, "vg");
    assert_eq!(row_keys(&app), ["g"]);
}

#[test]
fn count_alone_shows_nothing_and_count_prefix_is_not_titled() {
    let (_d, mut app) = app();
    keys(&mut app, "3");
    wait(&mut app);
    assert!(app.clue_rows().is_empty());
    assert!(!app.clue_visible());
    keys(&mut app, "g");
    wait(&mut app);
    assert!(app.clue_visible());
    assert_eq!(app.clue_title(), "g");
}

// Timing.

#[test]
fn opens_after_400_ms_not_before() {
    let (_d, mut app) = app();
    let before = Instant::now();
    keys(&mut app, "g");
    let after = Instant::now();
    app.tick(before + Duration::from_millis(399));
    assert!(!app.clue_visible(), "hidden at 399 ms");
    app.tick(after + CLUE_DELAY);
    assert!(app.clue_visible(), "shown at 400 ms");
}

#[test]
fn a_growing_sequence_updates_without_a_new_delay() {
    let (_d, mut app) = app();
    keys(&mut app, " ");
    wait(&mut app);
    assert!(app.clue_visible());
    keys(&mut app, "z");
    assert!(app.clue_visible(), "no second delay");
    assert_eq!(app.clue_title(), "Space z");
    assert_eq!(row_keys(&app), ["f", "l"]);
}

#[test]
fn a_fast_sequence_never_shows_and_restarts_the_timer() {
    let (_d, mut app) = app();
    keys(&mut app, "gg");
    wait(&mut app);
    assert!(!app.clue_visible());
    let before = Instant::now();
    keys(&mut app, "g");
    app.tick(before + Duration::from_millis(399));
    assert!(!app.clue_visible(), "the timer starts at the new g");
}

#[test]
fn esc_closes_the_box() {
    let (_d, mut app) = app();
    keys(&mut app, "g");
    wait(&mut app);
    assert!(app.clue_visible());
    app.handle_key(key(KeyCode::Esc));
    assert!(!app.clue_visible());
    let before = Instant::now();
    keys(&mut app, "z");
    app.tick(before + Duration::from_millis(100));
    assert!(!app.clue_visible(), "Esc forgot the old shown state");
}

#[test]
fn completing_the_sequence_closes_the_box() {
    let (_d, mut app) = app();
    keys(&mut app, "z");
    wait(&mut app);
    assert!(app.clue_visible());
    keys(&mut app, "z");
    assert!(!app.clue_visible());
}

// Config.

#[test]
fn clue_false_never_shows() {
    let mut c = config();
    c.keys.clue = false;
    let (_d, mut app) = app_sized(c, (100, 30));
    keys(&mut app, "g");
    wait(&mut app);
    assert!(!app.clue_visible());
    assert!(screen(&app).iter().all(|l| !l.contains("follow the link")));
}

// Rendering.

#[test]
fn box_sits_bottom_right_above_the_status_line() {
    let (_d, mut app) = app();
    let status_before = screen(&app).last().unwrap().clone();
    keys(&mut app, "g");
    let hidden = screen(&app);
    assert!(
        hidden.iter().all(|l| !l.contains("follow the link")),
        "not before the pause"
    );
    wait(&mut app);
    let s = screen(&app);
    let (w, h) = app.size();
    let rows: Vec<usize> = (0..h as usize)
        .filter(|&y| s[y].contains("follow the link"))
        .collect();
    assert_eq!(rows.len(), 1);
    // Bottom border on the row above the status line, 1 column inset.
    let bottom = &s[h as usize - 2];
    let chars: Vec<char> = bottom.chars().collect();
    assert_eq!(chars[w as usize - 1], ' ', "1 column inset");
    assert_eq!(chars[w as usize - 2], '┘');
    let title_row = s
        .iter()
        .position(|l| l.contains("┐") && l.contains(" g "))
        .unwrap();
    assert_eq!(title_row, h as usize - 2 - 6, "5 rows + borders");
    assert!(s[title_row + 1].contains("?  show this help"));
    // The status line is not covered (it may show the pending keys).
    let status = s.last().unwrap();
    assert!(!status.contains('┘') && !status.contains('│'));
    assert_eq!(
        status.split_whitespace().next(),
        status_before.split_whitespace().next()
    );
}

#[test]
fn overflow_wraps_into_columns() {
    // 100x10: area of 9 rows, box at most 4 high, 2 rows per column.
    let (_d, mut app) = app_sized(config(), (100, 10));
    keys(&mut app, "g");
    wait(&mut app);
    let s = screen(&app);
    let line = s.iter().find(|l| l.contains("show this help")).unwrap();
    assert!(
        line.contains("follow the link"),
        "second column beside the first: {line:?}"
    );
    assert!(s.iter().any(|l| l.contains("x  open the URL externally")));
    assert!(s.iter().all(|l| !l.contains("more")));
}

#[test]
fn overflow_that_still_does_not_fit_ends_with_more() {
    // 60x10: room for one 2-row column only.
    let (_d, mut app) = app_sized(config(), (60, 10));
    keys(&mut app, "g");
    wait(&mut app);
    let s = screen(&app);
    assert!(s.iter().any(|l| l.contains("?  show this help")));
    assert!(s.iter().any(|l| l.contains("…  4 more")), "{s:#?}");
    assert!(s.iter().all(|l| !l.contains("follow the link")));
}

#[test]
fn keys_clue_parses_and_defaults_true() {
    let load = |src: &str| {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, src).unwrap();
        Config::load(&path).unwrap()
    };
    assert!(Config::default().keys.clue);
    assert!(load("[keys]\nleader = \",\"\n").keys.clue);
    assert!(!load("[keys]\nclue = false\n").keys.clue);
}

#[test]
fn a_launcher_on_a_built_in_leader_key_wins() {
    let mut c = config();
    c.launch.push(ramble::config::Launcher {
        name: "mine".into(),
        key: Some("<leader>e".into()),
        command: vec!["true".into()],
        needs_vcs: false,
        disabled: false,
    });
    let (_d, mut app) = app_sized(c, (100, 30));
    keys(&mut app, " ");
    assert_eq!(desc(&app, "e").unwrap(), "mine");
}

#[test]
fn launchers_that_need_a_repo_say_so() {
    let mut c = config();
    let l = |name: &str, key: &str, needs_vcs| ramble::config::Launcher {
        name: name.into(),
        key: Some(key.into()),
        command: vec!["true".into()],
        needs_vcs,
        disabled: false,
    };
    c.launch.push(l("browse", "<leader>rd", false));
    c.launch.push(l("changes", "<leader>rw", true));
    let (_d, mut app) = app_sized(c, (100, 30));
    keys(&mut app, " r");
    assert_eq!(desc(&app, "d").unwrap(), "browse");
    assert_eq!(desc(&app, "w").unwrap(), "changes (needs a repo)");
}

#[test]
fn ctrl_w_rows_follow_the_sidebar_side() {
    let mut c = config();
    c.sidebar.side = SidebarSide::Right;
    let (_d, mut app) = app_sized(c, (100, 30));
    app.handle_key(ctrl('w'));
    assert_eq!(row_keys(&app), ["W", "j", "k", "l", "p", "w"]);
    assert_eq!(desc(&app, "l").unwrap(), "to the sidebar");
    app.handle_key(key(KeyCode::Char('l')));
    assert_ne!(app.focus(), Focus::Content);
    app.handle_key(ctrl('w'));
    assert_eq!(row_keys(&app), ["W", "h", "j", "k", "l", "p", "w"]);
    assert_eq!(desc(&app, "h").unwrap(), "to the content");
    assert_eq!(desc(&app, "l").unwrap(), "to the sidebar");
}
