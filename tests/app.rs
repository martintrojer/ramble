//! App state-machine tests: keys go into `App` without a terminal.

use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ramble::app::{App, Cursor, StartOptions, StartTarget};
use ramble::config::Config;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tempfile::TempDir;

const COLS: u16 = 40;
const ROWS: u16 = 12;
/// Content rows: ROWS minus the status line.
const VH: usize = 11;

/// Title, then 20 one-row paragraphs. Rendered rows at width 40:
/// 0 "Title", 1 rule, 2 blank, then paragraph i (1-based) on row 1 + 2i
/// followed by a blank row.
fn source() -> String {
    let mut s = String::from("# Title\n\n");
    for i in 1..=20 {
        s.push_str(&format!("Para {i} has foo.bar words\n\n"));
    }
    s
}

fn para_row(i: usize) -> usize {
    1 + 2 * i
}

fn opts(dir: &Path, target: StartTarget) -> StartOptions {
    StartOptions {
        target,
        tree_root: dir.to_path_buf(),
        config: Config::default(),
    }
}

fn app_with(content: &[u8]) -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("doc.md");
    std::fs::write(&path, content).unwrap();
    let app = App::new(opts(dir.path(), StartTarget::File(path)), (COLS, ROWS)).unwrap();
    (dir, app)
}

fn app() -> (TempDir, App) {
    app_with(source().as_bytes())
}

fn total(app: &App) -> usize {
    app.page().map_or(0, |p| p.rendered.lines.len())
}

fn row_text(app: &App, row: usize) -> String {
    app.page().unwrap().rendered.lines[row]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect()
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

/// Send plain characters, asserting the cursor stays visible after each.
fn keys(app: &mut App, s: &str) {
    for c in s.chars() {
        app.handle_key(key(KeyCode::Char(c)));
        assert_visible(app);
    }
}

fn send(app: &mut App, k: KeyEvent) {
    app.handle_key(k);
    assert_visible(app);
}

fn assert_visible(app: &App) {
    let c = app.cursor();
    let s = app.scroll();
    assert!(
        c.row >= s && c.row < s + VH,
        "cursor row {} outside viewport {s}..{}",
        c.row,
        s + VH
    );
    assert!(s + VH <= total(app).max(VH), "over-scrolled: {s}");
}

fn at(row: usize, col: usize) -> Cursor {
    Cursor { row, col }
}

#[test]
fn layout_assumptions_hold() {
    let (_d, app) = app();
    assert_eq!(row_text(&app, 0), "Title");
    assert_eq!(row_text(&app, 2), "");
    assert_eq!(row_text(&app, para_row(1)), "Para 1 has foo.bar words");
    assert_eq!(row_text(&app, para_row(20)), "Para 20 has foo.bar words");
    assert_eq!(total(&app), para_row(20) + 1, "trailing blank row");
    assert_eq!(app.cursor(), at(0, 0));
    assert_eq!(app.scroll(), 0);
}

#[test]
fn j_k_and_counts() {
    let (_d, mut app) = app();
    keys(&mut app, "j");
    assert_eq!(app.cursor(), at(1, 0));
    keys(&mut app, "5j");
    assert_eq!(app.cursor(), at(6, 0));
    keys(&mut app, "2k");
    assert_eq!(app.cursor(), at(4, 0));
    keys(&mut app, "k");
    assert_eq!(app.cursor(), at(3, 0));
    keys(&mut app, "12j");
    assert_eq!(app.cursor().row, 15);
    assert_eq!(app.scroll(), 15 + 1 - VH);
    keys(&mut app, "100k");
    assert_eq!(app.cursor(), at(0, 0));
    assert_eq!(app.scroll(), 0);
}

#[test]
fn g_and_gg() {
    let (_d, mut app) = app();
    keys(&mut app, "G");
    let last = total(&app) - 1;
    assert_eq!(app.cursor(), at(last, 0));
    assert_eq!(app.scroll(), last + 1 - VH);
    keys(&mut app, "gg");
    assert_eq!(app.cursor(), at(0, 0));
    assert_eq!(app.scroll(), 0);
    keys(&mut app, "4G");
    assert_eq!(app.cursor().row, 3);
    keys(&mut app, "2gg");
    assert_eq!(app.cursor().row, 1);
}

#[test]
fn h_l_0_dollar() {
    let (_d, mut app) = app();
    keys(&mut app, "3j");
    keys(&mut app, "l");
    assert_eq!(app.cursor(), at(3, 1));
    keys(&mut app, "4l");
    assert_eq!(app.cursor(), at(3, 5));
    keys(&mut app, "h");
    assert_eq!(app.cursor(), at(3, 4));
    keys(&mut app, "$");
    let w = "Para 1 has foo.bar words".len();
    assert_eq!(app.cursor(), at(3, w - 1));
    keys(&mut app, "100l");
    assert_eq!(app.cursor(), at(3, w - 1));
    keys(&mut app, "0");
    assert_eq!(app.cursor(), at(3, 0));
    keys(&mut app, "100h");
    assert_eq!(app.cursor(), at(3, 0));
    // `$` sticks to the end of line on vertical moves.
    keys(&mut app, "$2j");
    assert_eq!(app.cursor(), at(5, w - 1));
    keys(&mut app, "j");
    assert_eq!(app.cursor(), at(6, 0), "blank row clamps to column 0");
}

#[test]
fn w_b_e_within_and_across_rows() {
    let (_d, mut app) = app();
    keys(&mut app, "3j");
    // "Para 1 has foo.bar words"
    //  0    5 7   11 14 18
    keys(&mut app, "w");
    assert_eq!(app.cursor(), at(3, 5));
    keys(&mut app, "w");
    assert_eq!(app.cursor(), at(3, 7));
    keys(&mut app, "w");
    assert_eq!(app.cursor(), at(3, 11));
    keys(&mut app, "w");
    assert_eq!(app.cursor(), at(3, 14), "punctuation is its own word");
    keys(&mut app, "w");
    assert_eq!(app.cursor(), at(3, 15));
    keys(&mut app, "2w");
    assert_eq!(app.cursor(), at(4, 0), "empty row is a word stop");
    keys(&mut app, "w");
    assert_eq!(app.cursor(), at(5, 0));
    keys(&mut app, "b");
    assert_eq!(app.cursor(), at(4, 0));
    keys(&mut app, "b");
    assert_eq!(app.cursor(), at(3, 19));
    keys(&mut app, "3b");
    assert_eq!(app.cursor(), at(3, 11));
    keys(&mut app, "e");
    assert_eq!(app.cursor(), at(3, 13));
    keys(&mut app, "e");
    assert_eq!(app.cursor(), at(3, 14));
    keys(&mut app, "2e");
    assert_eq!(app.cursor(), at(3, 23));
    keys(&mut app, "e");
    assert_eq!(app.cursor(), at(5, 3), "e skips empty rows");
    keys(&mut app, "0");
    keys(&mut app, "3e");
    assert_eq!(app.cursor(), at(5, 9));
}

#[test]
fn paragraph_motions() {
    let (_d, mut app) = app();
    keys(&mut app, "}");
    assert_eq!(app.cursor(), at(2, 0));
    keys(&mut app, "}");
    assert_eq!(app.cursor(), at(4, 0));
    keys(&mut app, "3}");
    assert_eq!(app.cursor(), at(10, 0));
    keys(&mut app, "{");
    assert_eq!(app.cursor(), at(8, 0));
    keys(&mut app, "2{");
    assert_eq!(app.cursor(), at(4, 0));
    keys(&mut app, "100{");
    assert_eq!(app.cursor(), at(0, 0));
    keys(&mut app, "100}");
    assert_eq!(app.cursor(), at(total(&app) - 1, 0));
}

#[test]
fn h_m_l_viewport() {
    let (_d, mut app) = app();
    keys(&mut app, "L");
    assert_eq!(app.cursor().row, VH - 1);
    keys(&mut app, "M");
    assert_eq!(app.cursor().row, (VH - 1) / 2);
    keys(&mut app, "H");
    assert_eq!(app.cursor().row, 0);
    // Scroll down, then H/L follow the viewport.
    keys(&mut app, "20j");
    let s = app.scroll();
    assert_eq!(s, 20 + 1 - VH);
    keys(&mut app, "H");
    assert_eq!(app.cursor().row, s);
    keys(&mut app, "L");
    assert_eq!(app.cursor().row, s + VH - 1);
    keys(&mut app, "3H");
    assert_eq!(app.cursor().row, s + 2);
    assert_eq!(app.scroll(), s, "H/M/L never scroll");
}

#[test]
fn half_and_full_page_scrolling() {
    let (_d, mut app) = app();
    let half = VH / 2;
    let max = total(&app) - VH;
    send(&mut app, ctrl('d'));
    assert_eq!((app.scroll(), app.cursor().row), (half, half));
    send(&mut app, ctrl('d'));
    assert_eq!((app.scroll(), app.cursor().row), (2 * half, 2 * half));
    send(&mut app, ctrl('u'));
    assert_eq!((app.scroll(), app.cursor().row), (half, half));
    send(&mut app, ctrl('u'));
    assert_eq!((app.scroll(), app.cursor().row), (0, 0));
    send(&mut app, ctrl('u'));
    assert_eq!((app.scroll(), app.cursor().row), (0, 0));

    send(&mut app, ctrl('f'));
    assert_eq!(app.scroll(), VH);
    assert_eq!(app.cursor().row, VH);
    send(&mut app, ctrl('f'));
    send(&mut app, ctrl('f'));
    assert_eq!(app.scroll(), max);
    send(&mut app, ctrl('f'));
    assert_eq!(
        app.cursor().row,
        total(&app) - 1,
        "C-f at the end moves to the last row"
    );
    send(&mut app, ctrl('b'));
    assert_eq!(app.scroll(), max - VH);
    assert_eq!(app.cursor().row, max - 1);
    send(&mut app, ctrl('b'));
    send(&mut app, ctrl('b'));
    assert_eq!(app.scroll(), 0);
    send(&mut app, ctrl('b'));
    assert_eq!(app.cursor().row, 0);
}

#[test]
fn line_scrolling_keeps_cursor_on_screen() {
    let (_d, mut app) = app();
    keys(&mut app, "3j");
    send(&mut app, ctrl('e'));
    assert_eq!((app.scroll(), app.cursor().row), (1, 3));
    send(&mut app, ctrl('e'));
    send(&mut app, ctrl('e'));
    send(&mut app, ctrl('e'));
    assert_eq!(
        (app.scroll(), app.cursor().row),
        (4, 4),
        "cursor dragged down"
    );
    send(&mut app, ctrl('y'));
    assert_eq!((app.scroll(), app.cursor().row), (3, 4));
    keys(&mut app, "L");
    assert_eq!(app.cursor().row, 3 + VH - 1);
    send(&mut app, ctrl('y'));
    assert_eq!(
        (app.scroll(), app.cursor().row),
        (2, 2 + VH - 1),
        "cursor dragged up"
    );
    for _ in 0..5 {
        send(&mut app, ctrl('y'));
    }
    assert_eq!(app.scroll(), 0);
    keys(&mut app, "G");
    let s = app.scroll();
    send(&mut app, ctrl('e'));
    assert_eq!(app.scroll(), s, "C-e stops at the end");
}

#[test]
fn zz_zt_zb() {
    let (_d, mut app) = app();
    keys(&mut app, "20j");
    keys(&mut app, "zt");
    assert_eq!((app.scroll(), app.cursor().row), (20, 20));
    keys(&mut app, "zz");
    assert_eq!(app.scroll(), 20 - (VH - 1) / 2);
    keys(&mut app, "zb");
    assert_eq!(app.scroll(), 20 + 1 - VH);
    keys(&mut app, "gg");
    keys(&mut app, "zb");
    assert_eq!(app.scroll(), 0);
    keys(&mut app, "zz");
    assert_eq!(app.scroll(), 0);
    keys(&mut app, "G");
    keys(&mut app, "zt");
    assert_eq!(app.scroll(), total(&app) - VH, "zt clamps at the end");
    assert_eq!(app.cursor().row, total(&app) - 1);
}

#[test]
fn resize_keeps_cursor_on_same_source_byte() {
    let mut src = String::from("# Title\n\n");
    let long = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu";
    src.push_str(long);
    src.push_str("\n\nTail paragraph here\n");
    let (_d, mut app) = app_with(src.as_bytes());
    // At width 40 the long paragraph wraps onto rows 3 and 4.
    assert_eq!(row_text(&app, 4).split(' ').next(), Some("theta"));
    keys(&mut app, "4jw");
    assert_eq!(app.cursor(), at(4, 6));
    let page = app.page().unwrap();
    let byte = page.rendered.srcmap.source_at(4, 6).unwrap();
    assert_eq!(&page.doc.source[byte..byte + 4], "iota");

    app.resize(80, ROWS);
    let page = app.page().unwrap();
    let c = app.cursor();
    assert_eq!(c.row, 3, "paragraph fits one row at width 80");
    assert_eq!(page.rendered.srcmap.source_at(c.row, c.col), Some(byte));

    app.resize(20, ROWS);
    let page = app.page().unwrap();
    let c = app.cursor();
    assert_eq!(page.rendered.srcmap.source_at(c.row, c.col), Some(byte));
    assert!(row_text(&app, c.row)[c.col..].starts_with("iota"));
    assert_visible(&app);
}

#[test]
fn resize_from_blank_row_uses_row_above() {
    let (_d, mut app) = app();
    keys(&mut app, "10j");
    assert_eq!(row_text(&app, 10), "");
    app.resize(30, ROWS);
    // Row 9 is "Para 4 ..."; the cursor lands on its row at column 0.
    assert_eq!(app.cursor(), at(9, 0));
    assert!(row_text(&app, 9).starts_with("Para 4"));
}

#[test]
fn resize_shrinking_height_keeps_cursor_visible() {
    let (_d, mut app) = app();
    keys(&mut app, "L");
    app.resize(COLS, 5);
    let c = app.cursor();
    assert!(c.row >= app.scroll() && c.row < app.scroll() + 4);
}

#[test]
fn dir_target_has_no_page() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::new(
        opts(dir.path(), StartTarget::Dir(dir.path().into())),
        (COLS, ROWS),
    )
    .unwrap();
    assert!(app.page().is_none());
    assert!(app.placeholder().unwrap().starts_with("No file loaded."));
    keys(&mut app, "jGwb}{");
    send(&mut app, ctrl('d'));
    assert_eq!(app.cursor(), at(0, 0));
}

#[test]
fn stdin_target_titles_stdin() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::new(
        opts(dir.path(), StartTarget::Stdin("# Hi\n".into())),
        (COLS, ROWS),
    )
    .unwrap();
    assert_eq!(app.title(), "[stdin]");
    assert_eq!(row_text(&app, 0), "Hi");
}

#[test]
fn binary_file_sets_status_and_keeps_page() {
    let (dir, mut app) = app_with(b"\x00\x01binary");
    assert!(app.page().is_none());
    assert_eq!(app.status(), "looks binary");

    let good = dir.path().join("good.md");
    std::fs::write(&good, "# Good\n").unwrap();
    app.open_file(&good).unwrap();
    assert_eq!(app.title(), "good.md");
    assert_eq!(app.status(), "");
    let bin = dir.path().join("bin.md");
    std::fs::write(&bin, b"a\x00b").unwrap();
    app.open_file(&bin).unwrap();
    assert_eq!(app.status(), "looks binary");
    assert_eq!(app.title(), "good.md", "page unchanged");
}

#[test]
fn lossy_file_opens_with_status() {
    let (_d, app) = app_with(b"# Bad \xff byte\n");
    assert!(app.page().is_some());
    assert_eq!(app.status(), "not valid UTF-8");
}

#[test]
fn open_missing_file_is_err_and_open_resets_cursor() {
    let (dir, mut app) = app();
    keys(&mut app, "G");
    assert!(app.open_file(&dir.path().join("nope.md")).is_err());
    assert!(app.cursor().row > 0, "failed open leaves state alone");
    app.open_file(&dir.path().join("doc.md")).unwrap();
    assert_eq!((app.cursor(), app.scroll()), (at(0, 0), 0));
}

#[test]
fn link_under_cursor_uses_srcmap() {
    let (_d, mut app) = app_with(b"See [the link](x.md) here\n");
    assert_eq!(app.link_under_cursor(), None);
    keys(&mut app, "w");
    assert_eq!(app.cursor(), at(0, 4));
    assert_eq!(app.link_under_cursor(), Some(0));
    keys(&mut app, "ww");
    assert_eq!(app.link_under_cursor(), None);
}

#[test]
fn quit_keys() {
    for seq in ["q", "ZZ"] {
        let (_d, mut app) = app();
        keys(&mut app, seq);
        assert!(app.should_quit(), "{seq}");
    }
    let (_d, mut app) = app();
    keys(&mut app, "Z");
    assert!(!app.should_quit());
    send(&mut app, ctrl('c'));
    assert!(app.should_quit());
}

#[test]
fn status_position_and_message() {
    let (_d, mut app) = app();
    assert_eq!(app.position_label(), "Top");
    keys(&mut app, "G");
    assert_eq!(app.position_label(), "100%");
    send(&mut app, ctrl('y'));
    let t = total(&app);
    assert_eq!(
        app.position_label(),
        format!("{}%", (app.scroll() + VH) * 100 / t)
    );
    app.set_status("hello");
    assert_eq!(app.status(), "hello");
    let (_d, small) = app_with(b"# Small\n");
    assert_eq!(small.position_label(), "All");
}

fn screen(app: &App) -> String {
    let mut term = Terminal::new(TestBackend::new(COLS, ROWS)).unwrap();
    term.draw(|f| ramble::ui::draw(f, app)).unwrap();
    term.backend().to_string()
}

#[test]
fn snapshot_full_screen() {
    let (_d, mut app) = app();
    keys(&mut app, "3jw");
    app.set_status("hi");
    insta::assert_snapshot!(screen(&app));
}

#[test]
fn draw_marks_cursor_cell() {
    let (_d, mut app) = app();
    keys(&mut app, "3jw");
    let mut term = Terminal::new(TestBackend::new(COLS, ROWS)).unwrap();
    term.draw(|f| ramble::ui::draw(f, &app)).unwrap();
    let buf = term.backend().buffer();
    let cell = &buf[(5, 3)];
    assert!(cell.modifier.contains(ratatui::style::Modifier::REVERSED));
    assert_ne!(buf[(30, 3)].bg, buf[(30, 4)].bg, "cursorline is shaded");
}

#[test]
fn dir_placeholder_is_drawn() {
    let dir = tempfile::tempdir().unwrap();
    let app = App::new(
        opts(dir.path(), StartTarget::Dir(dir.path().into())),
        (COLS, ROWS),
    )
    .unwrap();
    let s = screen(&app);
    assert!(s.contains("No file loaded"), "{s}");
}

#[test]
fn huge_count_saturates_instead_of_reversing() {
    let (_d, mut app) = app();
    keys(&mut app, "5j");
    keys(&mut app, "99999999999999999999j");
    assert_eq!(app.cursor().row, total(&app) - 1, "huge j goes to the end");
    keys(&mut app, "99999999999999999999k");
    assert_eq!(app.cursor().row, 0, "huge k goes to the top");
    keys(&mut app, "3j99999999999999999999l");
    let w = "Para 1 has foo.bar words".len();
    assert_eq!(app.cursor(), at(3, w - 1), "huge l goes to end of row");
    keys(&mut app, "99999999999999999999h");
    assert_eq!(app.cursor(), at(3, 0), "huge h goes to column 0");
}

fn wrapped_app() -> (TempDir, App) {
    let src = "# Title\n\nalpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu\n";
    app_with(src.as_bytes())
}

#[test]
fn e_stops_at_end_of_wrapped_row() {
    let (_d, mut app) = wrapped_app();
    let row3 = row_text(&app, 3);
    assert!(row3.ends_with("eta"), "{row3:?}");
    assert!(row_text(&app, 4).starts_with("theta"));
    keys(&mut app, "3j$b");
    assert_eq!(app.cursor(), at(3, row3.len() - 3));
    keys(&mut app, "e");
    assert_eq!(
        app.cursor(),
        at(3, row3.len() - 1),
        "e does not run a word across the wrap"
    );
    keys(&mut app, "e");
    assert_eq!(app.cursor(), at(4, 4), "next e ends `theta`");
}

#[test]
fn l_with_count_counts_up_from_bottom() {
    let (_d, mut app) = app();
    keys(&mut app, "3L");
    assert_eq!(app.cursor().row, VH - 3);
    keys(&mut app, "100L");
    assert_eq!(app.cursor().row, 0, "L count clamps to the top row");
}

#[test]
fn render_width_is_capped_by_max_width() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("doc.md");
    let long = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu";
    std::fs::write(&path, format!("# Title\n\n{long}\n")).unwrap();
    let mut o = opts(dir.path(), StartTarget::File(path));
    o.config.render.max_width = 40;
    let app = App::new(o, (200, ROWS)).unwrap();
    let widths: Vec<usize> = (0..total(&app))
        .map(|r| unicode_width::UnicodeWidthStr::width(row_text(&app, r).as_str()))
        .collect();
    assert!(widths.iter().all(|&w| w <= 40), "{widths:?}");
    assert!(total(&app) > 4, "paragraph wraps at 40, not 200");
}

// -------------------------------------------------------------------------
// Link following and history (step 6)

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// Recorded side effects: URLs opened and files edited.
#[derive(Default)]
struct Effects {
    opened: Vec<String>,
    edited: Vec<PathBuf>,
}

fn a_source() -> String {
    let mut s = String::from(
        "# A\n\n[b](b.md)\n\n[h](#section-two)\n\n[x](missing.md)\n\n\
         [u](https://example.com)\n\n[t](notes.txt)\n\n[w](b.md#deep)\n\n\
         [bad](#nope)\n\n[[b]]\n\n",
    );
    for i in 1..=15 {
        s.push_str(&format!("Filler {i}\n\n"));
    }
    s.push_str("## Section two\n\nend\n");
    s
}

fn b_source() -> String {
    let mut s = String::from("# B page\n\nback to [a](a.md)\n\n");
    for i in 1..=20 {
        s.push_str(&format!("B filler {i}\n\n"));
    }
    s.push_str("## Deep\n\nbottom\n");
    s
}

fn nav_app() -> (TempDir, App, Rc<RefCell<Effects>>) {
    let (dir, app, fx) = nav_app_with(&a_source());
    std::fs::write(dir.path().join("b.md"), b_source()).unwrap();
    std::fs::write(dir.path().join("notes.txt"), "plain\n").unwrap();
    (dir, app, fx)
}

/// Row whose text is exactly `text`.
fn find_row(app: &App, text: &str) -> usize {
    (0..total(app))
        .find(|&r| row_text(app, r) == text)
        .unwrap_or_else(|| panic!("no row {text:?}"))
}

/// Move the cursor to the start of the row reading `text` using `gg` + `j`.
fn goto_text(app: &mut App, text: &str) {
    let row = find_row(app, text);
    keys(app, "gg");
    for _ in 0..row {
        keys(app, "j");
    }
    assert_eq!(app.cursor(), at(row, 0));
}

fn page_path(app: &App) -> PathBuf {
    app.page().unwrap().path.clone().unwrap()
}

#[test]
fn gd_opens_markdown_link_and_c_o_returns() {
    let (dir, mut app, _fx) = nav_app();
    goto_text(&mut app, "b");
    let before = (app.cursor(), app.scroll());
    keys(&mut app, "gd");
    assert_eq!(page_path(&app), dir.path().join("b.md"));
    assert_eq!(row_text(&app, 0), "B page");
    assert_eq!((app.cursor(), app.scroll()), (at(0, 0), 0));
    assert_eq!(app.history_depth(), 1);
    assert_eq!(app.title(), "b.md");

    send(&mut app, ctrl('o'));
    assert_eq!(page_path(&app), dir.path().join("a.md"));
    assert_eq!((app.cursor(), app.scroll()), before);
    assert_eq!(app.history_depth(), 0);
}

#[test]
fn enter_and_c_bracket_follow() {
    let (dir, mut app, _fx) = nav_app();
    goto_text(&mut app, "b");
    send(&mut app, key(KeyCode::Enter));
    assert_eq!(page_path(&app), dir.path().join("b.md"));
    send(&mut app, ctrl('t'));
    assert_eq!(page_path(&app), dir.path().join("a.md"));
    // crossterm reports C-] as C-5.
    send(&mut app, ctrl('5'));
    assert_eq!(page_path(&app), dir.path().join("b.md"));
    send(&mut app, ctrl('o'));
    send(&mut app, ctrl(']'));
    assert_eq!(page_path(&app), dir.path().join("b.md"));
}

#[test]
fn back_and_forward_round_trip_restores_cursor_and_scroll() {
    let (dir, mut app, _fx) = nav_app();
    goto_text(&mut app, "b");
    keys(&mut app, "gd");
    // On b.md, move to the bottom and a few rows up, mid-line.
    keys(&mut app, "G3kw");
    let b_bottom = (app.cursor(), app.scroll());
    assert!(b_bottom.1 > 0, "b.md scrolled");

    send(&mut app, ctrl('o'));
    assert_eq!(page_path(&app), dir.path().join("a.md"));
    assert_eq!(app.forward_depth(), 1);
    send(&mut app, key(KeyCode::Tab));
    assert_eq!(page_path(&app), dir.path().join("b.md"));
    assert_eq!((app.cursor(), app.scroll()), b_bottom);
    send(&mut app, ctrl('o'));
    send(&mut app, ctrl('i'));
    assert_eq!(page_path(&app), dir.path().join("b.md"));
    assert_eq!((app.cursor(), app.scroll()), b_bottom);
    // Nothing further forward.
    send(&mut app, key(KeyCode::Tab));
    assert_eq!(page_path(&app), dir.path().join("b.md"));
    assert_eq!(app.history_depth(), 1);
}

#[test]
fn new_follow_clears_forward() {
    let (dir, mut app, _fx) = nav_app();
    goto_text(&mut app, "b");
    keys(&mut app, "gd");
    send(&mut app, ctrl('o'));
    assert_eq!(app.forward_depth(), 1);
    goto_text(&mut app, "h");
    keys(&mut app, "gd");
    assert_eq!(app.forward_depth(), 0);
    send(&mut app, key(KeyCode::Tab));
    assert_eq!(page_path(&app), dir.path().join("a.md"));
    assert_eq!(app.history_depth(), 1);
}

#[test]
fn anchor_jump_lands_on_heading_and_is_undoable() {
    let (_d, mut app, _fx) = nav_app();
    goto_text(&mut app, "h");
    let before = (app.cursor(), app.scroll());
    keys(&mut app, "gd");
    let heading = find_row(&app, "Section two");
    assert_eq!(app.cursor(), at(heading, 0));
    assert!(app.scroll() <= heading && heading < app.scroll() + VH);
    assert_eq!(app.history_depth(), 1);
    send(&mut app, ctrl('o'));
    assert_eq!((app.cursor(), app.scroll()), before);
}

#[test]
fn path_with_anchor_opens_then_jumps() {
    let (dir, mut app, _fx) = nav_app();
    goto_text(&mut app, "w");
    keys(&mut app, "gd");
    assert_eq!(page_path(&app), dir.path().join("b.md"));
    assert_eq!(app.cursor(), at(find_row(&app, "Deep"), 0));
}

#[test]
fn wikilink_adds_md() {
    let (dir, mut app, _fx) = nav_app();
    goto_text(&mut app, "b");
    let wiki = (0..total(&app))
        .filter(|&r| row_text(&app, r) == "b")
        .nth(1)
        .expect("wikilink row");
    for _ in app.cursor().row..wiki {
        keys(&mut app, "j");
    }
    keys(&mut app, "gd");
    assert_eq!(page_path(&app), dir.path().join("b.md"));
}

#[test]
fn unknown_anchor_and_missing_file_set_status_only() {
    let (dir, mut app, fx) = nav_app();
    goto_text(&mut app, "bad");
    let pos = app.cursor();
    keys(&mut app, "gd");
    assert_eq!(app.status(), "No heading #nope");
    assert_eq!((app.cursor(), app.history_depth()), (pos, 0));

    goto_text(&mut app, "x");
    keys(&mut app, "gd");
    assert_eq!(app.status(), "No such file: missing.md");
    assert_eq!(page_path(&app), dir.path().join("a.md"));
    assert_eq!(app.history_depth(), 0);
    assert!(fx.borrow().opened.is_empty() && fx.borrow().edited.is_empty());
}

#[test]
fn external_link_uses_injected_opener() {
    let (dir, mut app, fx) = nav_app();
    goto_text(&mut app, "u");
    keys(&mut app, "gd");
    assert_eq!(fx.borrow().opened, ["https://example.com"]);
    assert_eq!(app.status(), "Opened https://example.com");
    assert_eq!(page_path(&app), dir.path().join("a.md"));
    assert_eq!(app.history_depth(), 0);

    keys(&mut app, "gx");
    assert_eq!(fx.borrow().opened.len(), 2);
    goto_text(&mut app, "b");
    keys(&mut app, "gx");
    assert_eq!(app.status(), "Not a URL");
    assert_eq!(fx.borrow().opened.len(), 2);
    assert_eq!(page_path(&app), dir.path().join("a.md"));
}

#[test]
fn non_markdown_link_goes_to_injected_editor() {
    let (dir, mut app, fx) = nav_app();
    goto_text(&mut app, "t");
    keys(&mut app, "gd");
    let txt = dir.path().join("notes.txt");
    assert_eq!(app.pending_editor(), Some(txt.as_path()));
    assert!(fx.borrow().edited.is_empty(), "app never runs it itself");
    app.run_pending_editor();
    assert_eq!(fx.borrow().edited, [txt]);
    assert_eq!(app.pending_editor(), None);
    assert_eq!(page_path(&app), dir.path().join("a.md"));
    assert_eq!(app.history_depth(), 0);
}

#[test]
fn gd_off_a_link_is_a_status() {
    let (_d, mut app, _fx) = nav_app();
    keys(&mut app, "gd");
    assert_eq!(app.status(), "No link under cursor");
    assert_eq!(app.history_depth(), 0);
}

#[test]
fn c_o_returns_to_stdin_page() {
    let dir = tempfile::tempdir().unwrap();
    let b = dir.path().join("b.md");
    std::fs::write(&b, b_source()).unwrap();
    let text = format!("# From stdin\n\n[b]({})\n", b.display());
    let mut app = App::new(opts(dir.path(), StartTarget::Stdin(text)), (COLS, ROWS))
        .unwrap()
        .with_effects(|_| panic!("no opener"), |_| panic!("no editor"));
    goto_text(&mut app, "b");
    keys(&mut app, "gd");
    assert_eq!(app.page().unwrap().path.as_deref(), Some(b.as_path()));
    send(&mut app, ctrl('o'));
    assert_eq!(app.title(), "[stdin]");
    assert_eq!(row_text(&app, 0), "From stdin");
    assert_eq!(app.cursor(), at(find_row(&app, "b"), 0));
    send(&mut app, key(KeyCode::Tab));
    assert_eq!(app.page().unwrap().path.as_deref(), Some(b.as_path()));
}

#[test]
fn status_line_shows_back_depth() {
    let (_d, mut app, _fx) = nav_app();
    assert!(screen(&app).contains("← 0"), "{}", screen(&app));
    goto_text(&mut app, "b");
    keys(&mut app, "gd");
    assert!(screen(&app).contains("← 1"), "{}", screen(&app));
}

/// An app on `dir/a.md` holding `source`, with recording effects.
fn nav_app_with(source: &str) -> (TempDir, App, Rc<RefCell<Effects>>) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.md"), source).unwrap();
    let fx = Rc::new(RefCell::new(Effects::default()));
    let (o, e) = (fx.clone(), fx.clone());
    let app = App::new(
        opts(dir.path(), StartTarget::File(dir.path().join("a.md"))),
        (COLS, ROWS),
    )
    .unwrap()
    .with_effects(
        move |url| o.borrow_mut().opened.push(url.to_string()),
        move |path| {
            e.borrow_mut().edited.push(path.to_path_buf());
            Ok(())
        },
    );
    (dir, app, fx)
}

#[test]
fn missing_non_markdown_link_is_a_status_not_the_editor() {
    let (dir, mut app, fx) = nav_app_with("# A\n\n[m](gone.txt)\n");
    goto_text(&mut app, "m");
    keys(&mut app, "gd");
    assert_eq!(app.status(), "No such file: gone.txt");
    assert_eq!(app.pending_editor(), None);
    app.run_pending_editor();
    assert!(fx.borrow().edited.is_empty());
    assert_eq!(page_path(&app), dir.path().join("a.md"));
    assert_eq!(app.history_depth(), 0);
}

#[test]
fn missing_file_with_anchor_status_omits_the_anchor() {
    let (dir, mut app, _fx) = nav_app_with("# A\n\n[g](gone.md#sec)\n");
    goto_text(&mut app, "g");
    keys(&mut app, "gd");
    assert_eq!(app.status(), "No such file: gone.md");
    assert_eq!(page_path(&app), dir.path().join("a.md"));
    assert_eq!(app.history_depth(), 0);
}

#[test]
fn stdin_relative_link_resolves_against_cwd() {
    // `cargo test` runs with the crate root as cwd; never change it here
    // (tests share the process).
    let rel = "tests/fixtures/doc/headings.md";
    let cwd = std::env::current_dir().unwrap();
    assert!(cwd.join(rel).is_file(), "fixture missing from {cwd:?}");
    let dir = tempfile::tempdir().unwrap();
    let text = format!("# From stdin\n\n[r]({rel})\n");
    let mut app = App::new(opts(dir.path(), StartTarget::Stdin(text)), (COLS, ROWS))
        .unwrap()
        .with_effects(|_| panic!("no opener"), |_| panic!("no editor"));
    goto_text(&mut app, "r");
    keys(&mut app, "gd");
    assert_eq!(page_path(&app), cwd.join(rel), "status: {}", app.status());
    assert_eq!(app.history_depth(), 1);
}
