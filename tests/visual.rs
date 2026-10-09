//! Visual mode (`v V C-v gv o`) and the yank operator (`y{motion} yy Y yf
//! yF yu`). Yanks copy source text.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ramble::app::{App, Clipboard, Cursor, Mode, StartOptions, StartTarget, VisualKind};
use ramble::config::{Config, SidebarShow};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tempfile::TempDir;

const COLS: u16 = 30;
const ROWS: u16 = 12;

/// Row 0 "Title", 1 rule, 2 blank, 3-4 the reflowed paragraph, 5 blank,
/// 6 "Second para.", 7 blank, 8 "- one", 9 "- two".
const SRC: &str = "# Title\n\nAlpha **bold** beta gamma\ndelta epsilon zeta eta theta.\n\nSecond para.\n\n- one\n- two\n";

#[derive(Clone, Default)]
struct RecClip(Rc<RefCell<Vec<String>>>);

impl Clipboard for RecClip {
    fn copy(&mut self, text: &str) -> anyhow::Result<()> {
        self.0.borrow_mut().push(text.to_string());
        Ok(())
    }
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
    }
}

fn app_src(src: &str) -> (TempDir, App, RecClip) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    let path = dir.path().join("sub/a.md");
    std::fs::write(&path, src).unwrap();
    let clip = RecClip::default();
    let app = App::new(opts(dir.path(), StartTarget::File(path)), (COLS, ROWS))
        .unwrap()
        .with_clipboard(clip.clone());
    (dir, app, clip)
}

fn app() -> (TempDir, App, RecClip) {
    app_src(SRC)
}

fn keys(app: &mut App, s: &str) {
    for c in s.chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
}

fn send(app: &mut App, code: KeyCode, m: KeyModifiers) {
    app.handle_key(KeyEvent::new(code, m));
}

fn row_text(app: &App, row: usize) -> String {
    app.page().unwrap().rendered.lines[row]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect()
}

fn last(clip: &RecClip) -> String {
    clip.0.borrow().last().cloned().unwrap_or_default()
}

fn at(row: usize, col: usize) -> Cursor {
    Cursor { row, col }
}

#[test]
fn layout_assumptions_hold() {
    let (_d, app, _c) = app();
    assert_eq!(row_text(&app, 3), "Alpha bold beta gamma delta");
    assert_eq!(row_text(&app, 4), "epsilon zeta eta theta.");
    assert_eq!(row_text(&app, 6), "Second para.");
}

#[test]
fn charwise_across_wrapped_rows_copies_exact_source_bytes() {
    let (_d, mut app, clip) = app();
    // From "bold" (row 3 col 6) to the end of "zeta" (row 4).
    keys(&mut app, "3jwv");
    assert_eq!(app.mode(), Mode::Visual(VisualKind::Char));
    keys(&mut app, "jwe");
    keys(&mut app, "y");
    // `**` before "bold" is hidden markup at the edge: left out; the
    // closing `**` and the source newline are between selected chars.
    assert_eq!(last(&clip), "bold** beta gamma\ndelta epsilon zeta");
    assert_eq!(app.mode(), Mode::Normal);
    assert_eq!(app.cursor(), at(3, 6), "cursor to the selection start");
    assert_eq!(app.status(), "Copied 2 lines");
}

#[test]
fn charwise_one_line_status_shows_text() {
    let (_d, mut app, clip) = app();
    keys(&mut app, "6jve");
    keys(&mut app, "y");
    assert_eq!(last(&clip), "Second");
    assert_eq!(app.status(), "Copied Second");
}

#[test]
fn linewise_over_reflowed_paragraph_copies_whole_source_lines() {
    let (_d, mut app, clip) = app();
    // A row of the paragraph copies the source lines it was drawn from.
    keys(&mut app, "4jwwV");
    keys(&mut app, "y");
    assert_eq!(last(&clip), "delta epsilon zeta eta theta.\n");
    keys(&mut app, "kVj");
    keys(&mut app, "y");
    assert_eq!(
        last(&clip),
        "Alpha **bold** beta gamma\ndelta epsilon zeta eta theta.\n"
    );
    assert_eq!(app.status(), "Copied 2 lines");
    // A selection over blank rows skips them.
    keys(&mut app, "ggVG");
    keys(&mut app, "y");
    assert_eq!(last(&clip), SRC);
}

#[test]
fn charwise_capital_y_yanks_whole_lines() {
    let (_d, mut app, clip) = app();
    keys(&mut app, "6jvY");
    assert_eq!(last(&clip), "Second para.\n");
}

#[test]
fn blockwise_copies_a_rendered_rectangle() {
    let (_d, mut app, clip) = app();
    keys(&mut app, "3j");
    send(&mut app, KeyCode::Char('v'), KeyModifiers::CONTROL);
    assert_eq!(app.mode(), Mode::Visual(VisualKind::Block));
    keys(&mut app, "jllll");
    keys(&mut app, "y");
    assert_eq!(last(&clip), "Alpha\nepsil");
}

#[test]
fn yy_y_and_count_yank_source_lines() {
    let (_d, mut app, clip) = app();
    keys(&mut app, "3jyy");
    assert_eq!(
        last(&clip),
        "Alpha **bold** beta gamma\ndelta epsilon zeta eta theta.\n"
    );
    keys(&mut app, "5jY");
    assert_eq!(last(&clip), "- one\n");
    keys(&mut app, "2yy");
    assert_eq!(last(&clip), "- one\n- two\n");
    keys(&mut app, "gg3j4yy");
    assert_eq!(
        last(&clip),
        "Alpha **bold** beta gamma\ndelta epsilon zeta eta theta.\n\nSecond para.\n"
    );
}

#[test]
fn yy_on_a_blank_row_yanks_nothing() {
    let (_d, mut app, clip) = app();
    keys(&mut app, "2jyy");
    assert!(clip.0.borrow().is_empty());
    assert_eq!(app.status(), "Nothing to yank");
}

#[test]
fn y_motions() {
    let (_d, mut app, clip) = app();
    keys(&mut app, "6jyw");
    assert_eq!(last(&clip), "Second ");
    assert_eq!(app.cursor(), at(6, 0));
    keys(&mut app, "y$");
    assert_eq!(last(&clip), "Second para.");
    // Exclusive, ends in column 0, starts at the first non-blank: linewise
    // (vim `:help exclusive-linewise`).
    keys(&mut app, "y}");
    assert_eq!(last(&clip), "Second para.\n");
    assert_eq!(app.cursor(), at(6, 0));
    // Past the first non-blank it stays charwise, ending before column 0.
    keys(&mut app, "wy}");
    assert_eq!(last(&clip), "para.");
    assert_eq!(app.cursor(), at(6, 7));
    // `yw` over the last word of a line stops at its end.
    keys(&mut app, "$yw");
    assert_eq!(last(&clip), ".");
    keys(&mut app, "0");
    keys(&mut app, "yG");
    assert_eq!(last(&clip), "Second para.\n\n- one\n- two\n");
    assert_eq!(app.cursor().row, 6);
    keys(&mut app, "8jygg");
    assert_eq!(last(&clip), SRC);
    assert_eq!(app.cursor().row, 0);
    keys(&mut app, "8jyj");
    assert_eq!(last(&clip), "- one\n- two\n");
    keys(&mut app, "kyk");
    assert_eq!(last(&clip), "Second para.\n");
    assert_eq!(app.mode(), Mode::Normal);
}

#[test]
fn y_then_non_motion_cancels() {
    let (_d, mut app, clip) = app();
    keys(&mut app, "yq");
    assert!(!app.should_quit());
    assert_eq!(app.mode(), Mode::Normal);
    assert!(clip.0.borrow().is_empty());
}

#[test]
fn o_swaps_anchor_and_cursor() {
    let (_d, mut app, clip) = app();
    keys(&mut app, "6jwv$");
    keys(&mut app, "o");
    assert_eq!(app.cursor(), at(6, 7));
    keys(&mut app, "0y");
    assert_eq!(last(&clip), "Second para.");
}

#[test]
fn gv_reselects_the_last_selection() {
    let (_d, mut app, clip) = app();
    keys(&mut app, "6jve");
    send(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    keys(&mut app, "gggv");
    assert_eq!(app.mode(), Mode::Visual(VisualKind::Char));
    assert_eq!(app.cursor(), at(6, 5));
    keys(&mut app, "y");
    assert_eq!(last(&clip), "Second");
}

#[test]
fn esc_and_ctrl_c_cancel_without_copying() {
    let (_d, mut app, clip) = app();
    keys(&mut app, "vj");
    send(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert_eq!(app.mode(), Mode::Normal);
    keys(&mut app, "V");
    send(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(app.mode(), Mode::Normal);
    assert!(!app.should_quit(), "C-c in visual cancels, not quits");
    keys(&mut app, "y");
    send(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(!app.should_quit());
    assert_eq!(app.mode(), Mode::Normal);
    assert!(clip.0.borrow().is_empty());
    send(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(app.should_quit());
}

#[test]
fn yf_yf_and_yu_yank_paths_and_link_target() {
    let (dir, mut app, clip) = app_src("# A\n\nsee [b](b.md#x) here\n");
    keys(&mut app, "yf");
    let abs = std::path::absolute(dir.path().join("sub/a.md")).unwrap();
    assert_eq!(last(&clip), abs.display().to_string());
    keys(&mut app, "yF");
    assert_eq!(last(&clip), "sub/a.md");
    assert_eq!(app.status(), "Copied sub/a.md");
    keys(&mut app, "3jwyu");
    assert_eq!(last(&clip), "b.md#x");
    keys(&mut app, "ggyu");
    assert_eq!(last(&clip), abs.display().to_string());
}

#[test]
fn stdin_yf_reports_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let clip = RecClip::default();
    let mut app = App::new(
        opts(dir.path(), StartTarget::Stdin("# S\n".into())),
        (COLS, ROWS),
    )
    .unwrap()
    .with_clipboard(clip.clone());
    keys(&mut app, "yf");
    assert_eq!(app.status(), "No file (stdin)");
    assert!(clip.0.borrow().is_empty());
    keys(&mut app, "yy");
    assert_eq!(last(&clip), "# S\n");
}

#[test]
fn raw_view_yanks_raw_rows() {
    let (_d, mut app, clip) = app();
    keys(&mut app, "gR");
    assert!(app.raw());
    keys(&mut app, "2jyy");
    assert_eq!(last(&clip), "Alpha **bold** beta gamma\n");
    keys(&mut app, "wv3l");
    keys(&mut app, "y");
    assert_eq!(last(&clip), "**bo");
}

#[test]
fn multibyte_selection_stays_on_char_boundaries() {
    let (_d, mut app, clip) = app_src("# T\n\nnaïve café über\n");
    keys(&mut app, "3jwvl");
    keys(&mut app, "y");
    assert_eq!(last(&clip), "ca");
    keys(&mut app, "v$");
    keys(&mut app, "y");
    assert_eq!(last(&clip), "café über");
}

#[test]
fn selection_is_drawn_and_wins_on_the_cursor_row() {
    let (_d, mut app, _c) = app();
    keys(&mut app, "6jv3l");
    let mut term = Terminal::new(TestBackend::new(COLS, ROWS)).unwrap();
    term.draw(|f| ramble::ui::draw(f, &app)).unwrap();
    let buf = term.backend().buffer();
    let sel = buf[(1, 6)].bg;
    assert_eq!(sel, buf[(0, 6)].bg, "anchor cell selected");
    assert_ne!(sel, buf[(8, 6)].bg, "past the selection: cursorline only");
    assert!(
        buf[(3, 6)]
            .modifier
            .contains(ratatui::style::Modifier::REVERSED),
        "cursor stays reversed on top"
    );
    // The cursor is drawn over the cursorline, not the selection.
    assert_eq!(
        buf[(3, 6)].bg,
        buf[(8, 6)].bg,
        "cursor cell keeps cursorline bg"
    );
    assert_ne!(buf[(3, 6)].bg, sel);
    insta::assert_snapshot!(term.backend().to_string());
}

#[test]
fn selection_wins_over_search_hits() {
    let (_d, mut app, _c) = app();
    keys(&mut app, "/para");
    send(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    let hit_row = app.cursor().row;
    keys(&mut app, "0V");
    let mut term = Terminal::new(TestBackend::new(COLS, ROWS)).unwrap();
    term.draw(|f| ramble::ui::draw(f, &app)).unwrap();
    let buf = term.backend().buffer();
    assert_eq!(buf[(8, hit_row as u16)].bg, buf[(1, hit_row as u16)].bg);
}

#[test]
fn reload_in_visual_mode_drops_the_selection() {
    let src = "aaa\n\nbbb ccc\n\nddd eee\n";
    let (dir, mut app, clip) = app_src(src);
    keys(&mut app, "4jwv");
    assert_eq!(app.mode(), Mode::Visual(VisualKind::Char));
    std::fs::write(dir.path().join("sub/a.md"), format!("{src}!\n")).unwrap();
    app.reload();
    assert_eq!(app.mode(), Mode::Normal, "reload leaves visual mode");
    assert!(app.selection_spans().is_empty());
    keys(&mut app, "y");
    assert_eq!(app.mode(), Mode::OpPending, "y starts the operator");
    send(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(clip.0.borrow().is_empty());
}

#[test]
fn reload_while_y_is_pending_cancels_it() {
    let (dir, mut app, clip) = app_src("aaa\n\nbbb\n");
    keys(&mut app, "y");
    std::fs::write(dir.path().join("sub/a.md"), "aaa\n\nbbb!\n").unwrap();
    app.reload();
    assert_eq!(app.mode(), Mode::Normal);
    keys(&mut app, "j");
    assert!(
        clip.0.borrow().is_empty(),
        "j after reload is a motion, not yj"
    );
}

/// vim `:help exclusive-linewise` (nvim -u NONE: `gg0y}` -> V register).
#[test]
fn exclusive_motion_ending_in_column_0_is_linewise_from_the_first_non_blank() {
    let (_d, mut app, clip) = app_src("one two\nthree\n\nfour\n");
    keys(&mut app, "y}");
    assert_eq!(last(&clip), "one two\nthree\n");
    // Past the first non-blank: charwise, end moved to the previous row.
    keys(&mut app, "wy}");
    assert_eq!(last(&clip), "two\nthree");
}
