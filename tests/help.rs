//! The `g?` help overlay (spec § Keymap) and the status-line hint.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ramble::app::{App, Focus, HelpLine, Mode, StartOptions, StartTarget};
use ramble::config::{Config, Launcher, SidebarMode};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tempfile::TempDir;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn keys(app: &mut App, s: &str) {
    for c in s.chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
}

fn config() -> Config {
    let mut c = Config::default();
    c.sidebar.default = SidebarMode::Off;
    c.lsp.server = vec![];
    c
}

fn launcher(name: &str, key: Option<&str>, needs_vcs: bool) -> Launcher {
    Launcher {
        name: name.into(),
        key: key.map(str::to_string),
        command: vec!["true".into()],
        needs_vcs,
        disabled: false,
    }
}

/// An app on `dir/<name>` (a short markdown file) at `size`.
fn app_named(name: &str, config: Config, size: (u16, u16)) -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::write(&path, "# Title\n\nSee [b](b.md).\n").unwrap();
    let app = App::new(
        StartOptions {
            target: StartTarget::File(path),
            tree_root: dir.path().to_path_buf(),
            config,
        },
        size,
    )
    .unwrap();
    (dir, app)
}

fn app_with(config: Config) -> (TempDir, App) {
    app_named("doc.md", config, (80, 24))
}

fn items(app: &App) -> Vec<(String, String, bool)> {
    app.help_lines()
        .into_iter()
        .filter_map(|l| match l {
            HelpLine::Item { keys, desc, dim } => Some((keys, desc, dim)),
            HelpLine::Group(_) => None,
        })
        .collect()
}

fn has_keys(app: &App, k: &str) -> bool {
    items(app).iter().any(|(keys, ..)| keys == k)
}

fn screen(app: &App) -> String {
    let (w, h) = app.size();
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| ramble::ui::draw(f, app)).unwrap();
    term.backend().to_string()
}

fn status_row(app: &App) -> String {
    screen(app).lines().last().unwrap().to_string()
}

#[test]
fn g_question_opens_and_q_esc_g_question_close() {
    for close in ["q", "\u{1b}", "g?"] {
        let (_d, mut app) = app_with(config());
        keys(&mut app, "g?");
        assert_eq!(app.mode(), Mode::Help);
        assert!(app.help_view().is_some());
        match close {
            "\u{1b}" => app.handle_key(key(KeyCode::Esc)),
            s => keys(&mut app, s),
        }
        assert_eq!(app.mode(), Mode::Normal, "closed by {close:?}");
        assert!(!app.should_quit(), "q closes help, not the app");
    }
}

#[test]
fn opening_help_drops_the_count_and_bare_question_still_searches() {
    let (_d, mut app) = app_with(config());
    keys(&mut app, "3g?q");
    assert_eq!(app.mode(), Mode::Normal);
    keys(&mut app, "j");
    assert_eq!(app.cursor().row, 1, "the 3 typed before g? was dropped");
    keys(&mut app, "?");
    assert_eq!(app.mode(), Mode::Search);
}

#[test]
fn g_question_works_from_the_sidebar_and_lists_its_keys() {
    let mut c = config();
    c.sidebar.default = SidebarMode::Files;
    let (_d, mut app) = app_with(c);
    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    keys(&mut app, "h");
    assert_eq!(app.focus(), Focus::Files);
    keys(&mut app, "g?");
    assert_eq!(app.mode(), Mode::Help);
    let all = items(&app);
    assert!(
        all.iter()
            .any(|(k, d, _)| k == "o, Enter" && d.contains("open"))
    );
    assert!(
        !has_keys(&app, "]l"),
        "content-only keys hidden in the sidebar"
    );
    keys(&mut app, "q");
    assert_eq!(app.mode(), Mode::Normal);
    assert_eq!(app.focus(), Focus::Files);
}

#[test]
fn unavailable_actions_are_hidden() {
    let (_d, app) = app_with(config());
    assert!(!has_keys(&app, "K"), "no LSP: no hover row");
    assert!(!has_keys(&app, "Space zs"), "no zk: no search row");
    assert!(!has_keys(&app, "Space zz"), "no zk: no tags row");
    assert!(!has_keys(&app, "Space zb"), "no server: no backlinks row");
    assert!(!has_keys(&app, "grr"));
    assert!(!has_keys(&app, ":Search <query>"));
    assert!(!has_keys(&app, ":Tags"));
    assert!(!has_keys(&app, "]r"), "no review comments: no ]r row");
    assert!(!has_keys(&app, "C-o, C-t"), "no history yet");
    assert!(!has_keys(&app, "n"), "no previous search");
    assert!(has_keys(&app, "Space zf"), "notes always work");
    assert!(has_keys(&app, "Space zl"));
    assert!(has_keys(&app, ":Notes"));
    assert!(has_keys(&app, "g?"));
}

#[test]
fn rows_appear_once_they_become_available() {
    let (_d, mut app) = app_with(config());
    keys(&mut app, "/Title");
    app.handle_key(key(KeyCode::Enter));
    assert!(has_keys(&app, "n"), "after a search n repeats it");
}

#[test]
fn launchers_are_listed_with_their_keys() {
    let mut c = config();
    c.launch = vec![
        launcher("edit", Some("<leader>o"), false),
        launcher("changes", Some("<leader>rw"), true),
        launcher("bare", None, false),
        launcher("broken", Some("x"), false),
    ];
    let (_d, app) = app_with(c);
    let all = items(&app);
    let find = |k: &str| all.iter().find(|(keys, ..)| keys == k).cloned();
    assert_eq!(
        find("Space o"),
        Some(("Space o".into(), "edit".into(), false))
    );
    let (_, desc, dim) = find("Space rw").expect("needs_vcs launcher listed");
    assert!(dim, "greyed without a repo");
    assert_eq!(desc, "changes (needs a repo)");
    let (_, desc, _) = find(":Launch bare").expect("keyless launcher under Commands");
    assert!(desc.contains("bare"));
    assert!(
        !all.iter()
            .any(|(k, d, _)| k.contains("broken") || d.contains("broken")),
        "a launcher with a key error is omitted"
    );
}

#[test]
fn launcher_rows_render_a_non_space_leader_as_is() {
    let mut c = config();
    c.keys.leader = ',';
    c.launch = vec![launcher("edit", Some("<leader>o"), false)];
    let (_d, app) = app_with(c);
    assert!(has_keys(&app, ",o"));
    assert!(has_keys(&app, ",e"));
}

#[test]
fn a_launcher_on_leader_e_replaces_the_sidebar_row() {
    let mut c = config();
    c.launch = vec![launcher("explore", Some("<leader>e"), false)];
    let (_d, app) = app_with(c);
    let all = items(&app);
    let rows: Vec<_> = all.iter().filter(|(k, ..)| k == "Space e").collect();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!(rows[0].1, "explore");
}

#[test]
fn filter_narrows_rows_and_esc_clears_then_closes() {
    let (_d, mut app) = app_with(config());
    keys(&mut app, "g?");
    let all = app.help_view().unwrap().lines.len();
    keys(&mut app, "/heading");
    let v = app.help_view().unwrap();
    assert!(v.filtering);
    assert!(v.lines.len() < all);
    assert!(v.lines.iter().all(|l| match l {
        HelpLine::Group(_) => true,
        HelpLine::Item { keys, desc, .. } =>
            (keys.clone() + desc).to_lowercase().contains("heading"),
    }));
    assert!(
        v.lines
            .iter()
            .any(|l| matches!(l, HelpLine::Item { keys, .. } if keys == "]]"))
    );
    app.handle_key(key(KeyCode::Enter));
    let v = app.help_view().unwrap();
    assert!(!v.filtering);
    assert_eq!(v.filter, "heading", "Enter keeps the filter");
    keys(&mut app, "q");
    assert_eq!(app.mode(), Mode::Normal, "q closes with a kept filter");

    keys(&mut app, "g?/zz");
    app.handle_key(key(KeyCode::Enter));
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.mode(), Mode::Help, "first Esc clears the kept filter");
    assert_eq!(app.help_view().unwrap().lines.len(), all);
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.mode(), Mode::Normal);

    keys(&mut app, "g?/x");
    app.handle_key(key(KeyCode::Esc));
    let v = app.help_view().expect("Esc while typing only clears");
    assert_eq!(v.filter, "");
}

#[test]
fn scrolls_and_never_panics_at_a_small_size() {
    let (_d, mut app) = app_named("doc.md", config(), (20, 8));
    keys(&mut app, "g?");
    screen(&app);
    keys(&mut app, "G");
    let max = app.help_view().unwrap().scroll;
    assert!(max > 0, "the list is longer than the box");
    keys(&mut app, "j");
    assert_eq!(app.help_view().unwrap().scroll, max, "j stops at the end");
    keys(&mut app, "k");
    assert_eq!(app.help_view().unwrap().scroll, max - 1);
    keys(&mut app, "gg");
    assert_eq!(app.help_view().unwrap().scroll, 0);
    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
    let half = app.help_view().unwrap().scroll;
    assert!(half > 0);
    app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert_eq!(app.help_view().unwrap().scroll, 0);
    screen(&app);
    for (w, h) in [(1, 1), (3, 3), (5, 2), (20, 8)] {
        app.event(ramble::app::AppEvent::Resize(w, h));
        screen(&app);
    }
}

#[test]
fn snapshot_help_modal() {
    let (_d, mut app) = app_with(config());
    keys(&mut app, "g?");
    insta::assert_snapshot!(screen(&app));
}

#[test]
fn status_line_shows_the_hint_when_it_fits() {
    let (_d, app) = app_named("doc.md", config(), (100, 10));
    let row = status_row(&app);
    assert!(row.contains("g? help"), "{row}");
    assert!(row.contains(" doc.md"), "{row}");
}

#[test]
fn status_line_drops_the_hint_before_truncating_the_path() {
    let name = "a-rather-long-file-name.md";
    let (_d, app) = app_named(name, config(), (40, 10));
    let row = status_row(&app);
    assert!(!row.contains("g? help"), "{row}");
    assert!(row.contains(name), "path kept whole: {row}");
    let (_d, short) = app_named("a.md", config(), (40, 10));
    assert!(status_row(&short).contains("g? help"), "short title fits");
}

#[test]
fn status_line_hint_is_hidden_while_a_prompt_owns_the_row() {
    let (_d, mut app) = app_with(config());
    keys(&mut app, "/");
    assert!(!status_row(&app).contains("g? help"));
}
