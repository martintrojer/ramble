//! Raw view (`gR`, `:Raw`): the page's markdown source instead of the
//! rendered layout.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ramble::app::{App, Effect, Exit, LaunchCommand, StartOptions, StartTarget};
use ramble::config::{Config, SidebarMode};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tempfile::TempDir;

const COLS: u16 = 60;
const ROWS: u16 = 12;

const SRC: &str =
    "# Title\n\nSome **bold** text and a [link](b.md) here.\n\n- item one\n- item\ttwo\n";

fn opts(dir: &Path, target: StartTarget) -> StartOptions {
    let mut config = Config::default();
    config.sidebar.default = SidebarMode::Off;
    config.lsp.server = vec![];
    StartOptions {
        target,
        tree_root: dir.to_path_buf(),
        config,
    }
}

fn app() -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.md");
    std::fs::write(&path, SRC).unwrap();
    std::fs::write(dir.path().join("b.md"), "# B\n\nother page\n").unwrap();
    let app = App::new(opts(dir.path(), StartTarget::File(path)), (COLS, ROWS))
        .unwrap()
        .with_env(|_| None);
    (dir, app)
}

fn keys(app: &mut App, s: &str) {
    for c in s.chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
}

fn row_text(app: &App, row: usize) -> String {
    app.page().unwrap().rendered.lines[row]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect()
}

fn screen(app: &App) -> String {
    let mut term = Terminal::new(TestBackend::new(COLS, ROWS)).unwrap();
    term.draw(|f| ramble::ui::draw(f, app)).unwrap();
    term.backend().to_string()
}

#[test]
fn g_shift_r_toggles_and_status_shows_raw() {
    let (_d, mut app) = app();
    assert!(!app.raw());
    assert!(!screen(&app).contains("RAW"));
    keys(&mut app, "gR");
    assert!(app.raw());
    assert_eq!(row_text(&app, 0), "# Title");
    assert_eq!(row_text(&app, 1), "");
    assert_eq!(row_text(&app, 2), SRC.lines().nth(2).unwrap());
    assert_eq!(row_text(&app, 5), "- item    two", "tab is 4 spaces");
    assert!(screen(&app).lines().last().unwrap().contains("RAW"));
    let spans = &app.page().unwrap().rendered.lines[2].spans;
    assert!(spans.len() > 2, "markdown is highlighted: {spans:?}");
    app.execute("Raw");
    assert!(!app.raw());
    assert_eq!(row_text(&app, 0), "Title");
}

#[test]
fn long_lines_soft_wrap_and_keep_their_source_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.md");
    let long = "x".repeat(130);
    std::fs::write(&path, format!("a\n{long}\nb\n")).unwrap();
    let mut app = App::new(opts(dir.path(), StartTarget::File(path)), (COLS, ROWS)).unwrap();
    keys(&mut app, "gR");
    let p = app.page().unwrap();
    assert_eq!(p.rendered.lines.len(), 5);
    assert_eq!(p.rendered.source_lines, vec![1, 2, 2, 2, 3]);
    assert_eq!(row_text(&app, 3), "x".repeat(10));
}

#[test]
fn toggle_keeps_cursor_on_the_same_source_byte() {
    let (_d, mut app) = app();
    // Rendered: row 3 is "Some bold text ..."; `w` puts the cursor on "bold".
    keys(&mut app, "3jw");
    assert_eq!(row_text(&app, 3).get(..9), Some("Some bold"));
    assert_eq!((app.cursor().row, app.cursor().col), (3, 5));
    keys(&mut app, "gR");
    // Raw: line 3 on row 2; "bold" starts at col 7, after "Some **".
    assert_eq!((app.cursor().row, app.cursor().col), (2, 7));
    keys(&mut app, "gR");
    assert_eq!((app.cursor().row, app.cursor().col), (3, 5));
}

#[test]
fn search_finds_raw_markup() {
    let (_d, mut app) = app();
    keys(&mut app, "/**");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let rendered_hits = app.search_highlights().len();
    keys(&mut app, "gR");
    assert!(app.search_highlights().len() >= 2, "both ** drawn raw");
    assert!(rendered_hits < app.search_highlights().len());
    keys(&mut app, "gg");
    keys(&mut app, "n");
    assert_eq!((app.cursor().row, app.cursor().col), (2, 5));
}

#[test]
fn leader_o_opens_editor_at_the_raw_cursor_line() {
    let (dir, app) = app();
    let calls: Rc<RefCell<Vec<LaunchCommand>>> = Rc::default();
    let rec = calls.clone();
    let mut app = app.with_runner(move |c| {
        rec.borrow_mut().push(c.clone());
        Ok(Exit::Code(0))
    });
    keys(&mut app, "gR");
    keys(&mut app, "4j");
    keys(&mut app, " o");
    let Some(Effect::Launch(cmd)) = app.pending_effect().cloned() else {
        panic!("no launch: {}", app.status());
    };
    let file = std::path::absolute(dir.path().join("a.md")).unwrap();
    assert_eq!(
        cmd.argv,
        vec!["vi".to_string(), "+5".into(), file.display().to_string()]
    );
}

#[test]
fn links_work_raw_and_back_restores_raw() {
    let (_d, mut app) = app();
    keys(&mut app, "gR");
    keys(&mut app, "]l");
    assert_eq!(app.cursor().row, 2);
    assert!(app.link_under_cursor().is_some());
    keys(&mut app, "gd");
    assert!(app.title().ends_with("b.md"), "{}", app.title());
    assert!(!app.raw(), "a followed link opens rendered");
    app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
    assert!(app.title().ends_with("a.md"));
    assert!(app.raw(), "back restores raw");
    assert_eq!(app.cursor().row, 2);
    assert_eq!(row_text(&app, 0), "# Title");
}

#[test]
fn reload_and_resize_keep_raw() {
    let (dir, mut app) = app();
    keys(&mut app, "gR");
    std::fs::write(dir.path().join("a.md"), "# New\n\nchanged\n").unwrap();
    app.reload();
    assert!(app.raw());
    assert_eq!(row_text(&app, 0), "# New");
    app.resize(30, ROWS);
    assert!(app.raw());
    assert_eq!(row_text(&app, 2), "changed");
}

#[test]
fn snapshot_raw_view() {
    let (_d, mut app) = app();
    keys(&mut app, "gR");
    insta::assert_snapshot!(screen(&app));
}

#[test]
fn crlf_line_endings_are_not_drawn() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.md");
    std::fs::write(&path, "# CR\r\n\r\nline one\r\n").unwrap();
    let mut app = App::new(opts(dir.path(), StartTarget::File(path)), (COLS, ROWS)).unwrap();
    keys(&mut app, "gR");
    let p = app.page().unwrap();
    assert_eq!(p.rendered.lines.len(), 3, "blank CRLF line keeps its row");
    assert_eq!(p.rendered.source_lines, vec![1, 2, 3]);
    assert_eq!(row_text(&app, 0), "# CR");
    assert_eq!(row_text(&app, 1), "");
    assert_eq!(row_text(&app, 2), "line one");
    assert!(!screen(&app).contains('?'));
}
