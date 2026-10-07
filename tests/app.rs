//! App state-machine tests: keys go into `App` without a terminal.

use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ramble::app::{App, Cursor, StartOptions, StartTarget};
use ramble::config::{Config, SidebarMode};
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

/// Options with no language servers configured, so no test starts a real
/// one, and the sidebar off, so the content gets the full width these tests
/// assume.
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
fn title_text_names_the_page() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("notes")).unwrap();
    let a = dir.path().join("notes/a.md");
    std::fs::write(&a, "# A\n").unwrap();
    let mut app = App::new(opts(dir.path(), StartTarget::File(a)), (COLS, ROWS)).unwrap();
    assert_eq!(app.title_text(), "ramble — notes/a.md");

    let other = tempfile::tempdir().unwrap();
    let out = other.path().join("out.md");
    std::fs::write(&out, "# Out\n").unwrap();
    app.open_file(&out).unwrap();
    assert_eq!(app.title_text(), format!("ramble — {}", out.display()));

    let evil = dir.path().join("x\x07\x1b]0;pwn.md");
    std::fs::write(&evil, "# E\n").unwrap();
    app.open_file(&evil).unwrap();
    assert_eq!(app.title_text(), "ramble — x]0;pwn.md");

    let stdin = App::new(
        opts(dir.path(), StartTarget::Stdin("# Hi\n".into())),
        (COLS, ROWS),
    )
    .unwrap();
    assert_eq!(stdin.title_text(), "ramble — [stdin]");

    let none = App::new(
        opts(dir.path(), StartTarget::Dir(dir.path().into())),
        (COLS, ROWS),
    )
    .unwrap();
    assert_eq!(none.title_text(), "ramble");
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
    assert_eq!(app.title_text(), "ramble — b.md");

    send(&mut app, ctrl('o'));
    assert_eq!(page_path(&app), dir.path().join("a.md"));
    assert_eq!(app.title_text(), "ramble — a.md");
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
    pump_until(&mut app, "on b.md", |a| a.history_depth() == 1);
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

// -------------------------------------------------------------------------
// Search, marks, yank, live reload (step 7)

use ramble::app::{AppEvent, Clipboard, DELETED_BANNER, FsEvent, Mode};

fn type_search(app: &mut App, prefix: char, pat: &str) {
    keys(app, &prefix.to_string());
    assert_eq!(app.mode(), Mode::Search);
    for c in pat.chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
}

#[test]
fn slash_search_highlights_incrementally_and_n_wraps() {
    let (_d, mut app) = app();
    type_search(&mut app, '/', "para 2");
    assert_eq!(app.search_prompt().as_deref(), Some("/para 2"));
    // Smartcase: lowercase matches "Para 2" and "Para 20".
    assert_eq!(app.search_hits().len(), 2);
    assert!(screen(&app).contains("/para 2"));
    send(&mut app, key(KeyCode::Enter));
    assert_eq!(app.mode(), Mode::Normal);
    assert_eq!(app.cursor(), at(para_row(2), 0));
    keys(&mut app, "n");
    assert_eq!(app.cursor(), at(para_row(20), 0));
    keys(&mut app, "n");
    assert_eq!(app.cursor(), at(para_row(2), 0));
    assert_eq!(app.status(), "search hit BOTTOM, continuing at TOP");
    keys(&mut app, "N");
    assert_eq!(app.cursor(), at(para_row(20), 0));
    assert_eq!(app.status(), "search hit TOP, continuing at BOTTOM");
}

#[test]
fn n_and_shift_n_from_a_blank_row_search_onward_from_the_cursor() {
    // Blank row between Para 3 and Para 4.
    let (_d, mut app) = app();
    type_search(&mut app, '/', "has");
    send(&mut app, key(KeyCode::Enter));
    keys(&mut app, "gg");
    keys(&mut app, &"j".repeat(para_row(3) + 1));
    assert_eq!(row_text(&app, app.cursor().row), "");
    keys(&mut app, "n");
    assert_eq!(app.cursor(), at(para_row(4), 7));
    assert_eq!(app.status(), "");
    keys(&mut app, "kN");
    assert_eq!(app.cursor(), at(para_row(3), 7));
    assert_eq!(app.status(), "");
}

#[test]
fn n_and_shift_n_from_a_heading_rule_row_search_onward() {
    // The rule row under a heading has no source byte either.
    let (_d, mut app) = app_with(b"one foo\n\n## Section\n\nfoo two\n\nlast\n");
    type_search(&mut app, '/', "foo");
    send(&mut app, key(KeyCode::Enter));
    let rule = (0..total(&app))
        .find(|&r| row_text(&app, r).starts_with('\u{2500}'))
        .expect("heading rule row");
    let two = (0..total(&app))
        .find(|&r| row_text(&app, r) == "foo two")
        .unwrap();
    keys(&mut app, "gg");
    keys(&mut app, &"j".repeat(rule));
    keys(&mut app, "n");
    assert_eq!(app.cursor(), at(two, 0));
    assert_eq!(app.status(), "");
    keys(&mut app, "G");
    keys(&mut app, "N");
    assert_eq!(app.cursor(), at(two, 0));
    assert_eq!(app.status(), "");
}

#[test]
fn uppercase_pattern_is_case_sensitive_and_question_searches_back() {
    let (_d, mut app) = app();
    type_search(&mut app, '/', "PARA");
    assert!(app.search_hits().is_empty());
    send(&mut app, key(KeyCode::Enter));
    assert_eq!(app.status(), "Pattern not found: PARA");
    keys(&mut app, "G");
    type_search(&mut app, '?', "Para 1");
    send(&mut app, key(KeyCode::Enter));
    // Backwards from the bottom: Para 19 is the last "Para 1" prefix.
    assert_eq!(app.cursor(), at(para_row(19), 0));
    keys(&mut app, "n");
    assert_eq!(app.cursor(), at(para_row(18), 0));
}

#[test]
fn esc_cancels_prompt_then_clears_highlights() {
    let (_d, mut app) = app();
    type_search(&mut app, '/', "foo");
    send(&mut app, key(KeyCode::Enter));
    assert_eq!(app.search_hits().len(), 20);
    type_search(&mut app, '/', "words");
    send(&mut app, key(KeyCode::Esc));
    assert_eq!(app.mode(), Mode::Normal);
    assert_eq!(app.search_hits().len(), 20, "previous pattern restored");
    send(&mut app, key(KeyCode::Esc));
    assert!(app.search_hits().is_empty());
    assert!(app.search_highlights().is_empty());
}

#[test]
fn star_and_hash_search_whole_word_under_cursor() {
    let (_d, mut app) = app();
    goto_row_text(&mut app, "Para 3 has foo.bar words");
    keys(&mut app, "ww"); // on "has"
    keys(&mut app, "*");
    assert_eq!(app.search_hits().len(), 20);
    assert_eq!(app.cursor().row, para_row(4));
    keys(&mut app, "#");
    assert_eq!(app.cursor().row, para_row(3));
    // Whole word, ignoring case (as vim's `*`): not "Paragraph".
    let (_d, mut app) = app_with(b"Para Paragraph para\n");
    keys(&mut app, "*");
    assert_eq!(app.search_hits().len(), 2);
    assert_eq!(app.cursor(), at(0, 15));
}

fn goto_row_text(app: &mut App, text: &str) {
    let row = (0..total(app)).find(|&r| row_text(app, r) == text).unwrap();
    keys(app, "gg");
    for _ in 0..row {
        keys(app, "j");
    }
}

#[test]
fn hit_across_a_wrap_highlights_both_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("w.md");
    std::fs::write(&path, "aaaa bbbb cccc dddd eeee ffff gggg hhhh iiii\n").unwrap();
    let app = App::new(opts(dir.path(), StartTarget::File(path)), (20, ROWS)).unwrap();
    let mut app = app;
    type_search(&mut app, '/', "dddd eeee");
    // Rows: "aaaa bbbb cccc dddd" / "eeee ffff gggg hhhh" / "iiii".
    assert_eq!(row_text(&app, 0), "aaaa bbbb cccc dddd");
    let spans: Vec<(usize, usize, usize)> = app
        .search_highlights()
        .iter()
        .map(|s| (s.row, s.col_start, s.col_end))
        .collect();
    assert_eq!(spans, vec![(0, 15, 19), (1, 0, 4)]);
}

#[test]
fn hits_stay_visible_on_the_cursor_row_and_the_current_hit_stands_out() {
    use ramble::render::palette;
    let (_d, mut app) = app();
    // "Para 1 has ...": hits at cols 1 and 3 of every paragraph row.
    type_search(&mut app, '/', "a");
    send(&mut app, key(KeyCode::Enter));
    let row = para_row(1);
    assert_eq!(app.cursor(), at(row, 1));
    let mut term = Terminal::new(TestBackend::new(COLS, ROWS)).unwrap();
    term.draw(|f| ramble::ui::draw(f, &app)).unwrap();
    let buf = term.backend().buffer();
    let y = row as u16;
    let other_row = buf[(3, y + 2)].clone();
    let current = buf[(1, y)].clone();
    let other_on_cursor_row = buf[(3, y)].clone();
    let plain_on_cursor_row = buf[(2, y)].clone();
    assert_eq!(other_row.bg, palette::YELLOW, "hit off the cursor row");
    assert_eq!(
        other_on_cursor_row.bg,
        palette::YELLOW,
        "hit on the cursor row"
    );
    assert_ne!(other_on_cursor_row.fg, other_on_cursor_row.bg);
    assert_ne!(
        plain_on_cursor_row.bg,
        palette::YELLOW,
        "cursorline between hits"
    );
    // The current hit: its own colour, drawn reversed under the cursor.
    assert_eq!(current.bg, palette::PEACH, "current hit");
    assert_ne!(current.fg, current.bg);
    assert!(
        current
            .modifier
            .contains(ratatui::style::Modifier::REVERSED)
    );
}

#[test]
fn search_hits_survive_resize() {
    let (_d, mut app) = app();
    type_search(&mut app, '/', "Para 7 ");
    send(&mut app, key(KeyCode::Enter));
    app.resize(20, ROWS);
    let hl = app.search_highlights();
    assert!(!hl.is_empty());
    assert!(row_text(&app, hl[0].row).contains("Para 7"));
}

#[test]
fn marks_set_and_jump_and_are_per_page() {
    let (_d, mut app) = app();
    keys(&mut app, "5j");
    let r = app.cursor().row;
    keys(&mut app, "maG'a");
    assert_eq!(app.cursor(), at(r, 0));
    keys(&mut app, "'b");
    assert_eq!(app.status(), "Mark not set: b");
    app.resize(20, ROWS);
    keys(&mut app, "G'a");
    assert!(row_text(&app, app.cursor().row).starts_with("Para"));
}

#[derive(Clone, Default)]
struct RecClip(Rc<RefCell<Vec<String>>>);

impl Clipboard for RecClip {
    fn copy(&mut self, text: &str) -> anyhow::Result<()> {
        self.0.borrow_mut().push(text.to_string());
        Ok(())
    }
}

#[test]
fn y_copies_link_dest_else_file_path() {
    let (dir, app, _fx) = nav_app();
    let clip = RecClip::default();
    let mut app = app.with_clipboard(clip.clone());
    goto_text(&mut app, "w");
    keys(&mut app, "y");
    assert_eq!(app.status(), "Copied b.md#deep");
    keys(&mut app, "gg");
    keys(&mut app, "y");
    let path = dir.path().join("a.md").display().to_string();
    assert_eq!(
        *clip.0.borrow(),
        vec!["b.md#deep".to_string(), path.clone()]
    );
    assert_eq!(app.status(), format!("Copied {path}"));
}

#[test]
fn default_clipboard_queues_osc52_for_the_terminal() {
    let (_d, mut app, _fx) = nav_app();
    goto_text(&mut app, "u");
    keys(&mut app, "y");
    let out = String::from_utf8(app.take_terminal_output()).unwrap();
    assert_eq!(out, ramble::app::osc52("https://example.com"));
    assert!(app.take_terminal_output().is_empty());
}

#[test]
fn fs_watch_change_reloads_keeping_cursor_on_same_text() {
    let (dir, mut app) = app();
    goto_row_text(&mut app, "Para 5 has foo.bar words");
    keys(&mut app, "w");
    let path = dir.path().join("doc.md");
    let scroll = app.scroll();
    let edited = source().replace("Para 9 has", "Para 9 now has") + "New tail\n";
    std::fs::write(&path, edited).unwrap();
    app.event(AppEvent::FsWatch(path, FsEvent::Changed));
    assert_eq!(row_text(&app, app.cursor().row), "Para 5 has foo.bar words");
    assert_eq!(app.cursor().col, 5);
    assert_eq!(app.scroll(), scroll);
    assert!((0..total(&app)).any(|r| row_text(&app, r) == "New tail"));
    assert!((0..total(&app)).any(|r| row_text(&app, r).starts_with("Para 9 now")));
}

#[test]
fn fs_watch_event_for_other_file_is_ignored() {
    let (dir, mut app) = app();
    std::fs::write(dir.path().join("doc.md"), "# Changed\n").unwrap();
    app.event(AppEvent::FsWatch(
        dir.path().join("other.md"),
        FsEvent::Changed,
    ));
    assert_eq!(row_text(&app, 0), "Title");
}

#[test]
fn fs_watch_delete_shows_banner_and_keeps_content() {
    let (dir, mut app) = app();
    let path = dir.path().join("doc.md");
    std::fs::remove_file(&path).unwrap();
    app.event(AppEvent::FsWatch(path.clone(), FsEvent::Removed));
    assert_eq!(app.banner(), Some(DELETED_BANNER));
    assert_eq!(row_text(&app, 0), "Title");
    assert!(screen(&app).contains(DELETED_BANNER));
    // A Changed for the missing file keeps the banner and content too.
    app.event(AppEvent::FsWatch(path.clone(), FsEvent::Changed));
    assert_eq!(app.banner(), Some(DELETED_BANNER));
    // Recreated: reload clears the banner.
    std::fs::write(&path, "# Back\n").unwrap();
    app.event(AppEvent::FsWatch(path, FsEvent::Changed));
    assert_eq!(app.banner(), None);
    assert_eq!(row_text(&app, 0), "Back");
}

// -------------------------------------------------------------------------
// LSP features (step 11), driven by the scripted fake server.

use ramble::config::{ServerConfig, ServerKind};
use serde_json::{Value, json};
use std::ops::Range;
use std::time::{Duration, Instant};

/// A temp notebook (root marker `.fake`) with `a.md` holding `source` and
/// `b.md`, and an app whose only server is fake-lsp running the script
/// `script(root)` builds. Returns the log of every message fake-lsp read.
fn lsp_app(source: &str, script: impl FnOnce(&Path) -> Value) -> (TempDir, App, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join(".fake"), "").unwrap();
    std::fs::write(root.join("a.md"), source).unwrap();
    std::fs::write(root.join("b.md"), b_source()).unwrap();
    let script_path = root.join("script.json");
    std::fs::write(&script_path, script(&root).to_string()).unwrap();
    let log = root.join("log.jsonl");
    let mut o = opts(&root, StartTarget::File(root.join("a.md")));
    o.config.lsp.server = vec![ServerConfig {
        kind: ServerKind::Generic,
        command: vec![
            env!("CARGO_BIN_EXE_fake-lsp").into(),
            script_path.display().to_string(),
            log.display().to_string(),
        ],
        root_markers: vec![".fake".into()],
        position_encoding: None,
    }];
    let app = App::new(o, (COLS, ROWS)).unwrap();
    (dir, app, log)
}

fn init_step() -> Value {
    json!({"expect": "initialize", "reply": {"capabilities": {
        "definitionProvider": true, "hoverProvider": true, "documentLinkProvider": {},
    }}})
}

/// Pump LSP events until `pred` holds (5 s limit).
fn pump_until(app: &mut App, what: &str, pred: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !pred(app) {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}; status {:?}",
            app.status()
        );
        app.pump_lsp(Duration::from_millis(20));
    }
}

/// Pump for a while, to show that something does not happen.
fn pump_for(app: &mut App, d: Duration) {
    let end = Instant::now() + d;
    while Instant::now() < end {
        app.pump_lsp(Duration::from_millis(10));
    }
}

fn uri(path: &Path) -> String {
    ramble::lsp::canonical_uri(path).as_str().to_string()
}

/// A documentLink entry over source line `line`, columns `cols`.
fn doc_link(line: u32, cols: Range<u32>, target: &Path) -> Value {
    json!({
        "range": {"start": {"line": line, "character": cols.start},
                  "end": {"line": line, "character": cols.end}},
        "target": uri(target),
    })
}

/// The messages fake-lsp logged with `method`. A line fake-lsp is still
/// writing does not parse and is skipped.
fn logged(log: &Path, method: &str) -> Vec<Value> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|m| m["method"] == method)
        .collect()
}

/// Wait until fake-lsp has logged `n` messages with `method`.
fn wait_logged(log: &Path, method: &str, n: usize) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let got = logged(log, method);
        if got.len() >= n {
            return got;
        }
        assert!(Instant::now() < deadline, "fake-lsp never got {n} {method}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn running(app: &mut App) {
    pump_until(app, "server running", |a| a.lsp_label().ends_with('●'));
}

#[test]
fn lsp_starts_off_the_ui_thread_then_shows_running() {
    let (_d, mut app, _log) = lsp_app(
        "# A\n\n[b](b)\n",
        |_| json!([{"sleep_ms": 300}, init_step()]),
    );
    assert_eq!(
        app.lsp_label(),
        "generic ○",
        "starting while the server sleeps"
    );
    running(&mut app);
    assert_eq!(app.lsp_label(), "generic ●");
    assert!(screen(&app).contains("generic ●"), "{}", screen(&app));
}

#[test]
fn no_server_for_the_root_shows_dash() {
    let (_d, app) = app();
    assert_eq!(app.lsp_label(), "—");
    assert!(screen(&app).contains('—'));
}

#[test]
fn document_link_targets_are_applied_and_gd_follows_them() {
    // `[b](b)` has no `.md`: the local parse alone resolves to a missing file.
    let (dir, mut app, log) = lsp_app("# A\n\n[b](b)\n", |root| {
        json!([init_step(), {"expect": "textDocument/documentLink",
                             "reply": [doc_link(2, 0..6, &root.join("b.md"))]}])
    });
    let root = dir.path().canonicalize().unwrap();
    pump_until(&mut app, "documentLink applied", |a| {
        a.link_target(0).is_some()
    });
    assert_eq!(app.link_target(0), Some(root.join("b.md").as_path()));
    let opens = logged(&log, "textDocument/didOpen");
    assert_eq!(opens.len(), 1);
    assert_eq!(opens[0]["params"]["textDocument"]["version"], 1);
    assert_eq!(opens[0]["params"]["textDocument"]["languageId"], "markdown");
    goto_text(&mut app, "b");
    keys(&mut app, "gd");
    assert_eq!(page_path(&app), root.join("b.md"));
    assert_eq!(app.history_depth(), 1);
}

#[test]
fn diagnostics_mark_covered_links_broken_and_muted() {
    let (dir, mut app, _log) = lsp_app("# A\n\n[gone](nowhere) and [b](b.md)\n", |root| {
        json!([init_step(), {"expect": "textDocument/didOpen"}, {"send": {
            "jsonrpc": "2.0", "method": "textDocument/publishDiagnostics",
            "params": {"uri": uri(&root.join("a.md")), "version": 1, "diagnostics": [{
                "range": {"start": {"line": 2, "character": 0},
                          "end": {"line": 2, "character": 15}},
                "message": "dead link",
            }]},
        }}])
    });
    let _ = dir;
    pump_until(&mut app, "diagnostics", |a| !a.broken_links().is_empty());
    assert_eq!(
        app.broken_links().iter().copied().collect::<Vec<_>>(),
        vec![0]
    );
    // Drawn muted: the broken link's cells differ from the healthy link's.
    let mut term = Terminal::new(TestBackend::new(COLS, ROWS)).unwrap();
    term.draw(|f| ramble::ui::draw(f, &app)).unwrap();
    let buf = term.backend().buffer();
    let row = find_row(&app, "gone and b") as u16;
    let gone = &buf[(0, row)];
    let b = &buf[(9, row)];
    assert_eq!(gone.symbol(), "g");
    assert_eq!(b.symbol(), "b");
    assert_eq!(gone.fg, ramble::render::palette::OVERLAY);
    assert!(gone.modifier.contains(ratatui::style::Modifier::DIM));
    assert_ne!(b.fg, ramble::render::palette::OVERLAY);
}

#[test]
fn diagnostics_for_an_older_version_are_ignored() {
    let (_dir, mut app, _log) = lsp_app("# A\n\n[gone](nowhere)\n", |root| {
        json!([init_step(), {"expect": "textDocument/didOpen"}, {"send": {
            "jsonrpc": "2.0", "method": "textDocument/publishDiagnostics",
            "params": {"uri": uri(&root.join("a.md")), "version": 7, "diagnostics": [{
                "range": {"start": {"line": 2, "character": 0},
                          "end": {"line": 2, "character": 15}},
                "message": "dead link",
            }]},
        }}])
    });
    running(&mut app);
    pump_for(&mut app, Duration::from_millis(300));
    assert!(app.broken_links().is_empty());
}

#[test]
fn diagnostics_for_another_file_are_ignored() {
    // N3: b.md's diagnostics cover a range that holds a.md's link.
    let (_dir, mut app, _log) = lsp_app("# A\n\n[gone](nowhere)\n", |root| {
        json!([init_step(), {"expect": "textDocument/didOpen"}, {"send": {
            "jsonrpc": "2.0", "method": "textDocument/publishDiagnostics",
            "params": {"uri": uri(&root.join("b.md")), "diagnostics": [{
                "range": {"start": {"line": 2, "character": 0},
                          "end": {"line": 2, "character": 15}},
                "message": "dead link",
            }]},
        }}])
    });
    running(&mut app);
    pump_for(&mut app, Duration::from_millis(300));
    assert!(app.broken_links().is_empty());
}

#[test]
fn reply_for_a_page_the_user_left_is_dropped() {
    let (dir, mut app, _log) = lsp_app("# A\n\n[b](b.md)\n", |root| {
        json!([init_step(),
               {"expect": "textDocument/documentLink", "as": "a"},
               {"expect": "textDocument/documentLink", "as": "b"},
               {"respond": "a", "result": [doc_link(2, 0..9, &root.join("a.md"))]}])
    });
    let root = dir.path().canonicalize().unwrap();
    running(&mut app);
    let first_page = app.page_id();
    goto_text(&mut app, "b");
    keys(&mut app, "gd");
    // No documentLink target yet: definition (fake answers null), then local.
    pump_until(&mut app, "on b.md", |a| {
        a.page().unwrap().path.as_deref() == Some(root.join("b.md").as_path())
    });
    assert!(app.page_id() > first_page);
    // b.md's own link `back to [a](a.md)` is link 0 on the new page; a.md's
    // late reply (also for link 0) must not land there.
    pump_for(&mut app, Duration::from_millis(300));
    assert_eq!(app.link_target(0), None);
}

#[test]
fn reply_for_an_older_version_is_dropped_after_reload() {
    let (dir, mut app, log) = lsp_app("# A\n\n[b](b)\n", |root| {
        json!([init_step(),
               {"expect": "textDocument/documentLink", "as": "v1"},
               {"expect": "textDocument/documentLink", "as": "v2"},
               {"respond": "v1", "result": [doc_link(2, 0..6, &root.join("a.md"))]},
               // v2 is answered only after the test sends a hover (`K`).
               {"expect": "textDocument/hover", "reply": null},
               {"respond": "v2", "result": [doc_link(2, 0..6, &root.join("b.md"))]}])
    });
    let root = dir.path().canonicalize().unwrap();
    running(&mut app);
    let path = root.join("a.md");
    std::fs::write(&path, "# A\n\n[b](b)\n\nmore\n").unwrap();
    app.event(AppEvent::FsWatch(path, FsEvent::Changed));
    // The fs-watch reload re-sent didOpen with a new version.
    let opens = wait_logged(&log, "textDocument/didOpen", 2);
    assert_eq!(opens[1]["params"]["textDocument"]["version"], 2);
    assert!(
        opens[1]["params"]["textDocument"]["text"]
            .as_str()
            .unwrap()
            .contains("more")
    );
    assert_eq!(app.lsp_version(), Some(2));
    pump_for(&mut app, Duration::from_millis(300));
    assert_eq!(app.link_target(0), None, "v1 reply dropped");
    goto_text(&mut app, "b");
    keys(&mut app, "K");
    pump_until(&mut app, "v2 reply", |a| a.link_target(0).is_some());
    assert_eq!(app.link_target(0), Some(root.join("b.md").as_path()));
}

#[test]
fn versions_keep_increasing_when_a_page_is_revisited() {
    let (_dir, mut app, log) = lsp_app("# A\n\n[b](b.md)\n", |_| json!([init_step()]));
    running(&mut app);
    goto_text(&mut app, "b");
    keys(&mut app, "gd");
    pump_until(&mut app, "on b.md", |a| a.history_depth() == 1);
    send(&mut app, ctrl('o'));
    let opens = wait_logged(&log, "textDocument/didOpen", 3);
    let versions: Vec<(String, i64)> = opens
        .iter()
        .map(|m| {
            let td = &m["params"]["textDocument"];
            (
                td["uri"]
                    .as_str()
                    .unwrap()
                    .rsplit('/')
                    .next()
                    .unwrap()
                    .to_string(),
                td["version"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        versions,
        vec![("a.md".into(), 1), ("b.md".into(), 1), ("a.md".into(), 2)]
    );
}

#[test]
fn spinner_shows_after_two_seconds_without_a_reply() {
    let (_dir, mut app, log) = lsp_app("# A\n\n[b](b.md)\n", |_| {
        // Hold the documentLink request and never answer it.
        json!([init_step(), {"expect": "textDocument/documentLink", "as": "held"}])
    });
    let t0 = Instant::now();
    app.tick(t0);
    running(&mut app);
    wait_logged(&log, "textDocument/documentLink", 1);
    assert!(!app.lsp_spinning());
    app.tick(t0 + Duration::from_millis(1900));
    assert!(!app.lsp_spinning());
    assert_eq!(app.lsp_label(), "generic ●");
    app.tick(t0 + Duration::from_millis(2500));
    assert!(app.lsp_spinning());
    assert!(
        app.lsp_label().len() > "generic ●".len(),
        "{}",
        app.lsp_label()
    );
}

#[test]
fn server_crash_shows_status_and_falls_back_to_no_lsp() {
    let (_dir, mut app, _log) = lsp_app("# A\n\n[b](b.md)\n", |_| {
        json!([init_step(), {"expect": "textDocument/didOpen"},
               {"stderr": "boom"}, {"exit": 3}])
    });
    pump_until(&mut app, "crash noticed", |a| {
        a.status().contains("LSP off")
    });
    assert_eq!(app.lsp_label(), "—");
    // gd still works from the local parse.
    goto_text(&mut app, "b");
    keys(&mut app, "gd");
    assert!(page_path(&app).ends_with("b.md"));
    assert_eq!(app.lsp_label(), "—", "not respawned for this root");
}

#[test]
fn start_failure_is_no_lsp_mode() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join(".fake"), "").unwrap();
    std::fs::write(root.join("a.md"), "# A\n").unwrap();
    let mut o = opts(&root, StartTarget::File(root.join("a.md")));
    o.config.lsp.server = vec![ServerConfig {
        kind: ServerKind::Zk,
        command: vec![root.join("no-such-server").display().to_string()],
        root_markers: vec![".fake".into()],
        position_encoding: None,
    }];
    let mut app = App::new(o, (COLS, ROWS)).unwrap();
    assert_eq!(app.lsp_label(), "zk ○");
    pump_until(&mut app, "start failure", |a| a.lsp_label() == "—");
    // A missing command is quiet: the kind only, no instance name or path.
    assert_eq!(app.status(), "zk not installed; no LSP");
    // N1: the failure is remembered; visiting another page under the same
    // root, or coming back, must not respawn the server.
    std::fs::write(root.join("b.md"), "# B\n").unwrap();
    app.open_file(&root.join("b.md")).unwrap();
    assert_eq!(app.lsp_label(), "—", "not respawned on another page");
    app.open_file(&root.join("a.md")).unwrap();
    assert_eq!(app.lsp_label(), "—", "not respawned on revisit");
    pump_for(&mut app, Duration::from_millis(200));
    assert_eq!(app.lsp_label(), "—");
}

/// An app whose only server, of `kind`, runs `command` for notebooks
/// marked `.fake`, opened on `a.md` in a fresh notebook.
fn failing_server_app(kind: ServerKind, command: Vec<String>) -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join(".fake"), "").unwrap();
    std::fs::write(root.join("a.md"), "# A\n").unwrap();
    let mut o = opts(&root, StartTarget::File(root.join("a.md")));
    o.config.lsp.server = vec![ServerConfig {
        kind,
        command,
        root_markers: vec![".fake".into()],
        position_encoding: None,
    }];
    let app = App::new(o, (COLS, ROWS)).unwrap();
    (dir, app)
}

#[test]
fn missing_server_is_reported_once_per_kind() {
    let (dir, mut app) = failing_server_app(
        ServerKind::Marksman,
        vec!["ramble-no-such-lsp-binary".into()],
    );
    pump_until(&mut app, "start failure", |a| a.lsp_label() == "—");
    assert_eq!(app.status(), "marksman not installed; no LSP");
    // Another root of the same kind: a new instance fails too, quietly.
    let other = dir.path().canonicalize().unwrap().join("other");
    std::fs::create_dir(&other).unwrap();
    std::fs::write(other.join(".fake"), "").unwrap();
    std::fs::write(other.join("c.md"), "# C\n").unwrap();
    app.open_file(&other.join("c.md")).unwrap();
    assert_eq!(
        app.lsp_label(),
        "marksman ○",
        "a new root starts its own server"
    );
    app.set_status("");
    pump_until(&mut app, "second start failure", |a| a.lsp_label() == "—");
    pump_for(&mut app, Duration::from_millis(100));
    assert_eq!(app.status(), "", "shown once per kind per session");
}

#[test]
fn failing_server_shows_a_short_reason() {
    let long = "x".repeat(200);
    let (_dir, mut app) = failing_server_app(
        ServerKind::Generic,
        vec![
            "/bin/sh".into(),
            "-c".into(),
            format!("echo {long} >&2; exit 3"),
        ],
    );
    pump_until(&mut app, "start failure", |a| {
        a.lsp_label() == "—" && !a.status().is_empty()
    });
    let status = app.status().to_string();
    assert!(status.starts_with("generic failed to start: "), "{status}");
    let reason = status.trim_start_matches("generic failed to start: ");
    assert!(!reason.is_empty(), "{status}");
    assert!(reason.chars().count() <= 60, "reason too long: {reason:?}");
    assert!(!status.contains('@'), "no instance name: {status}");
    assert!(!status.contains("/bin/sh"), "no command: {status}");
}

#[test]
fn gd_without_a_document_link_uses_definition() {
    // `[x](x)` resolves locally to a missing file; definition says b.md.
    let (dir, mut app, log) = lsp_app("# A\n\n[x](x)\n", |root| {
        json!([init_step(),
               {"expect": "textDocument/documentLink", "reply": []},
               {"expect": "textDocument/definition",
                "reply": {"uri": uri(&root.join("b.md")), "range": {
                    "start": {"line": 0, "character": 0},
                    "end": {"line": 0, "character": 0}}}}])
    });
    let root = dir.path().canonicalize().unwrap();
    running(&mut app);
    wait_logged(&log, "textDocument/documentLink", 1);
    goto_text(&mut app, "x");
    keys(&mut app, "gd");
    pump_until(&mut app, "definition followed", |a| {
        a.page().unwrap().path.as_deref() == Some(root.join("b.md").as_path())
    });
    let defs = logged(&log, "textDocument/definition");
    assert_eq!(
        defs[0]["params"]["position"],
        json!({"line": 2, "character": 1})
    );
}

#[test]
fn null_definition_falls_back_to_the_local_parse() {
    let (_dir, mut app, log) = lsp_app("# A\n\n[b](b.md)\n", |_| {
        json!([init_step(), {"expect": "textDocument/documentLink", "reply": []},
               {"expect": "textDocument/definition", "reply": null}])
    });
    running(&mut app);
    wait_logged(&log, "textDocument/documentLink", 1);
    goto_text(&mut app, "b");
    keys(&mut app, "gd");
    pump_until(&mut app, "local follow", |a| {
        a.page()
            .unwrap()
            .path
            .as_ref()
            .is_some_and(|p| p.ends_with("b.md"))
    });
}

#[test]
fn definition_reply_after_the_cursor_moved_is_dropped() {
    let (_dir, mut app, log) = lsp_app("# A\n\n[b](b.md)\n\nplain\n", |root| {
        json!([init_step(), {"expect": "textDocument/documentLink", "reply": []},
               {"expect": "textDocument/definition", "as": "d"},
               {"sleep_ms": 200},
               {"respond": "d", "result": {"uri": uri(&root.join("b.md")), "range": {
                   "start": {"line": 0, "character": 0},
                   "end": {"line": 0, "character": 0}}}}])
    });
    running(&mut app);
    wait_logged(&log, "textDocument/documentLink", 1);
    goto_text(&mut app, "b");
    keys(&mut app, "gd");
    keys(&mut app, "j");
    pump_for(&mut app, Duration::from_millis(500));
    assert!(page_path(&app).ends_with("a.md"));
}

#[test]
fn anchor_link_with_a_fragment_less_target_lands_on_the_heading() {
    let (dir, mut app, _log) = lsp_app("# A\n\n[deep](b#deep)\n", |root| {
        // zk drops the fragment: the target is plain b.md.
        json!([init_step(), {"expect": "textDocument/documentLink",
                             "reply": [doc_link(2, 0..14, &root.join("b.md"))]}])
    });
    let root = dir.path().canonicalize().unwrap();
    pump_until(&mut app, "documentLink", |a| a.link_target(0).is_some());
    goto_text(&mut app, "deep");
    keys(&mut app, "gd");
    assert_eq!(page_path(&app), root.join("b.md"));
    assert_eq!(row_text(&app, app.cursor().row), "Deep");
}

#[test]
fn k_shows_hover_popup_and_esc_closes_it() {
    let (_dir, mut app, log) = lsp_app("# A\n\n[b](b.md)\n", |_| {
        json!([init_step(), {"expect": "textDocument/hover", "reply": {
            "contents": {"kind": "markdown", "value": "# B page\n\nhovered text"}}}])
    });
    running(&mut app);
    goto_text(&mut app, "b");
    keys(&mut app, "K");
    pump_until(&mut app, "hover", |a| a.hover_popup().is_some());
    assert_eq!(app.hover_popup(), Some("# B page\n\nhovered text"));
    assert!(screen(&app).contains("hovered text"), "{}", screen(&app));
    let hovers = logged(&log, "textDocument/hover");
    assert_eq!(
        hovers[0]["params"]["position"],
        json!({"line": 2, "character": 1})
    );
    send(&mut app, key(KeyCode::Esc));
    assert_eq!(app.hover_popup(), None);
    assert!(!screen(&app).contains("hovered text"));
}

#[test]
fn k_without_a_server_is_a_status() {
    let (_d, mut app, _fx) = nav_app();
    goto_text(&mut app, "b");
    keys(&mut app, "K");
    assert!(app.status().contains("No hover"), "{}", app.status());
}

#[test]
fn stdin_page_gets_no_lsp() {
    let (_dir, mut app, log) = lsp_app("# A\n", |_| json!([init_step()]));
    running(&mut app);
    let dir = tempfile::tempdir().unwrap();
    let mut o = opts(dir.path(), StartTarget::Stdin("# S\n".into()));
    o.config = Config::default();
    o.config.lsp.server = vec![];
    let s = App::new(o, (COLS, ROWS)).unwrap();
    assert_eq!(s.lsp_label(), "—");
    assert_eq!(wait_logged(&log, "textDocument/didOpen", 1).len(), 1);
}

#[test]
fn injected_lsp_event_for_another_server_is_ignored() {
    let (_dir, mut app, _log) = lsp_app("# A\n\n[gone](x)\n", |_| json!([init_step()]));
    running(&mut app);
    let path = app.page().unwrap().path.clone().unwrap();
    app.event(AppEvent::Lsp(ramble::lsp::LspEvent::Diagnostics {
        server: "other@/".into(),
        uri: ramble::lsp::canonical_uri(&path),
        version: None,
        diagnostics: vec![lsp_types::Diagnostic {
            range: lsp_types::Range::new(
                lsp_types::Position::new(2, 0),
                lsp_types::Position::new(2, 9),
            ),
            message: "dead".into(),
            ..Default::default()
        }],
    }));
    assert!(app.broken_links().is_empty());
}

#[test]
fn dir_placeholder_points_at_the_file_tree() {
    assert!(ramble::app::NO_FILE_MESSAGE.contains("file tree"));
}

// -------------------------------------------------------------------------
// Notebook pickers and the `:` command line (step 13).

/// Like [`lsp_app`], with a chosen server kind and `extra` files written
/// into the notebook first.
fn kind_app(
    kind: ServerKind,
    source: &str,
    extra: &[(&str, &str)],
    script: impl FnOnce(&Path) -> Value,
) -> (TempDir, App, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join(".fake"), "").unwrap();
    std::fs::write(root.join("a.md"), source).unwrap();
    std::fs::write(root.join("b.md"), b_source()).unwrap();
    for (name, text) in extra {
        std::fs::write(root.join(name), text).unwrap();
    }
    let script_path = root.join("script.json");
    std::fs::write(&script_path, script(&root).to_string()).unwrap();
    let log = root.join("log.jsonl");
    let mut o = opts(&root, StartTarget::File(root.join("a.md")));
    o.config.lsp.server = vec![ServerConfig {
        kind,
        command: vec![
            env!("CARGO_BIN_EXE_fake-lsp").into(),
            script_path.display().to_string(),
            log.display().to_string(),
        ],
        root_markers: vec![".fake".into()],
        position_encoding: None,
    }];
    (dir, App::new(o, (COLS, ROWS)).unwrap(), log)
}

fn zk_init() -> Value {
    json!({"expect": "initialize", "reply": {"capabilities": {
        "definitionProvider": true, "hoverProvider": true, "documentLinkProvider": {},
        "referencesProvider": true,
    }}})
}

fn zk_notes(root: &Path, names: &[(&str, &str)]) -> Value {
    Value::Array(
        names
            .iter()
            .map(|(title, f)| json!({"title": title, "absPath": root.join(f)}))
            .collect(),
    )
}

fn leader(app: &mut App, s: &str) {
    send(app, key(KeyCode::Char(' ')));
    keys(app, s);
}

fn picker_labels(app: &App) -> Vec<String> {
    app.picker()
        .map(|p| p.items.iter().map(|i| i.label.clone()).collect())
        .unwrap_or_default()
}

fn command(app: &mut App, s: &str) {
    keys(app, ":");
    assert_eq!(app.mode(), Mode::Command);
    keys(app, s);
    send(app, key(KeyCode::Enter));
}

/// The executeCommand requests fake-lsp got after zk.index.
fn executed(log: &Path, n: usize) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let got: Vec<Value> = logged(log, "workspace/executeCommand")
            .into_iter()
            .filter(|m| m["params"]["command"] != "zk.index")
            .collect();
        if got.len() >= n {
            return got;
        }
        assert!(Instant::now() < deadline, "never got {n} executeCommand");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn notes_without_a_server_walk_files_filter_and_open() {
    let (dir, mut app, _fx) = nav_app();
    std::fs::create_dir_all(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub/deep.md"), "# Deep note\n").unwrap();
    leader(&mut app, "zf");
    assert_eq!(app.mode(), Mode::Picker);
    assert_eq!(picker_labels(&app), ["a", "b", "deep"]);
    keys(&mut app, "dee");
    assert_eq!(picker_labels(&app), ["deep"]);
    send(&mut app, key(KeyCode::Backspace));
    send(&mut app, key(KeyCode::Backspace));
    send(&mut app, key(KeyCode::Backspace));
    send(&mut app, ctrl('n'));
    send(&mut app, ctrl('n'));
    send(&mut app, ctrl('p'));
    assert_eq!(app.picker().unwrap().selected, 1);
    send(&mut app, key(KeyCode::Enter));
    assert_eq!(app.mode(), Mode::Normal);
    assert!(app.picker().is_none());
    assert_eq!(page_path(&app), dir.path().join("b.md"));
    assert_eq!(app.history_depth(), 1, "Enter pushes history");
    keys(&mut app, ":Notes");
    send(&mut app, key(KeyCode::Enter));
    assert!(app.picker().is_some());
    send(&mut app, key(KeyCode::Esc));
    assert!(app.picker().is_none());
    assert_eq!(app.mode(), Mode::Normal);
}

#[test]
fn zk_ops_are_unavailable_without_a_zk_server() {
    let (_d, mut app, _fx) = nav_app();
    leader(&mut app, "zs");
    assert_eq!(app.status(), "Search needs a zk notebook");
    leader(&mut app, "zz");
    assert_eq!(app.status(), "Tags needs a zk notebook");
    keys(&mut app, "grr");
    assert_eq!(app.status(), "Backlinks need a language server");
    command(&mut app, "Backlinks");
    assert_eq!(app.status(), "Backlinks need a language server");
    assert!(app.picker().is_none());
    use ramble::notebook::Op;
    assert_eq!(app.available_ops(), [Op::Notes, Op::Links]);
}

#[test]
fn zk_notes_go_through_zk_list_with_the_server_root() {
    let (dir, mut app, log) = kind_app(ServerKind::Zk, "# A\n", &[], |root| {
        json!([zk_init(), {"expect": "workspace/executeCommand", "reply": {}},
               {"expect": "workspace/executeCommand",
                "reply": zk_notes(root, &[("Note B", "b.md"), ("Note A", "a.md")])}])
    });
    let root = dir.path().canonicalize().unwrap();
    running(&mut app);
    use ramble::notebook::Op;
    assert_eq!(app.available_ops(), Op::ALL);
    leader(&mut app, "zf");
    assert!(app.picker().unwrap().loading);
    assert!(screen(&app).contains("Loading…"), "{}", screen(&app));
    pump_until(&mut app, "notes", |a| !picker_labels(a).is_empty());
    assert_eq!(picker_labels(&app), ["Note B", "Note A"]);
    assert_eq!(app.picker().unwrap().items[0].detail, "b.md");
    let sent = &executed(&log, 1)[0]["params"];
    assert_eq!(sent["command"], "zk.list");
    assert_eq!(sent["arguments"][0], json!(root));
    assert_eq!(
        sent["arguments"][1],
        json!({"select": ["title", "absPath"], "sort": ["modified"]})
    );
    send(&mut app, key(KeyCode::Enter));
    assert_eq!(page_path(&app), root.join("b.md"));
}

#[test]
fn search_prompts_once_then_filters_results() {
    let (_d, mut app, log) = kind_app(ServerKind::Zk, "# A\n", &[], |root| {
        json!([zk_init(), {"expect": "workspace/executeCommand", "reply": {}},
               {"expect": "workspace/executeCommand",
                "reply": zk_notes(root, &[("Alpha one", "a.md"), ("Beta", "b.md")])}])
    });
    running(&mut app);
    leader(&mut app, "zs");
    assert!(app.picker().unwrap().prompting);
    keys(&mut app, "-foo bar*");
    assert!(picker_labels(&app).is_empty(), "nothing sent while typing");
    send(&mut app, key(KeyCode::Enter));
    pump_until(&mut app, "results", |a| picker_labels(a).len() == 2);
    let sent = &executed(&log, 1)[0]["params"];
    assert_eq!(sent["arguments"][1]["match"], json!(["-foo bar*"]));
    assert!(!app.picker().unwrap().prompting);
    keys(&mut app, "bet");
    assert_eq!(picker_labels(&app), ["Beta"]);
    assert_eq!(executed(&log, 1).len(), 1, "the filter is local");
}

#[test]
fn tags_open_a_second_picker_of_notes_by_tag() {
    let (dir, mut app, log) = kind_app(ServerKind::Zk, "# A\n", &[], |root| {
        json!([zk_init(), {"expect": "workspace/executeCommand", "reply": {}},
               {"expect": "workspace/executeCommand",
                "reply": [{"id": 1, "kind": "tag", "name": "project", "note_count": 1}]},
               {"expect": "workspace/executeCommand",
                "reply": zk_notes(root, &[("Note B", "b.md")])}])
    });
    let root = dir.path().canonicalize().unwrap();
    running(&mut app);
    command(&mut app, "Tags");
    pump_until(&mut app, "tags", |a| !picker_labels(a).is_empty());
    assert_eq!(picker_labels(&app), ["project"]);
    assert_eq!(app.picker().unwrap().items[0].detail, "1");
    send(&mut app, key(KeyCode::Enter));
    pump_until(&mut app, "notes by tag", |a| picker_labels(a) == ["Note B"]);
    let sent = executed(&log, 2);
    assert_eq!(sent[0]["params"]["command"], "zk.tag.list");
    assert_eq!(
        sent[1]["params"]["arguments"][1]["tags"],
        json!(["project"])
    );
    send(&mut app, key(KeyCode::Enter));
    assert_eq!(page_path(&app), root.join("b.md"));
}

#[test]
fn a_stale_picker_reply_is_dropped() {
    // Both requests are held, and "first" is answered while the second
    // picker is still loading: only the seq check can drop it.
    let (_d, mut app, log) = kind_app(ServerKind::Zk, "# A\n", &[], |root| {
        json!([zk_init(), {"expect": "workspace/executeCommand", "reply": {}},
               {"expect": "workspace/executeCommand", "as": "first"},
               {"expect": "workspace/executeCommand", "as": "second"},
               {"respond": "first", "result": zk_notes(root, &[("First", "a.md")])},
               {"respond": "second", "result": zk_notes(root, &[("Second", "b.md")])}])
    });
    running(&mut app);
    leader(&mut app, "zf");
    send(&mut app, key(KeyCode::Esc));
    leader(&mut app, "zf");
    executed(&log, 2);
    pump_until(&mut app, "second", |a| !picker_labels(a).is_empty());
    pump_for(&mut app, Duration::from_millis(300));
    assert_eq!(picker_labels(&app), ["Second"]);
}

#[test]
fn a_picker_reply_from_another_server_instance_is_dropped() {
    let (dir, mut app, log) = kind_app(ServerKind::Zk, "# A\n", &[], |_| {
        json!([zk_init(), {"expect": "workspace/executeCommand", "reply": {}},
               {"expect": "workspace/executeCommand", "as": "held"}])
    });
    let root = dir.path().canonicalize().unwrap();
    running(&mut app);
    leader(&mut app, "zf");
    executed(&log, 1);
    app.event(AppEvent::Lsp(ramble::lsp::LspEvent::Response {
        server: "zk@/elsewhere".into(),
        id: 2,
        tag: ramble::app::PICKER_TAG_BASE | 1,
        result: Ok(zk_notes(&root, &[("Foreign", "b.md")])),
    }));
    assert!(app.picker().unwrap().loading, "foreign reply dropped");
    assert!(picker_labels(&app).is_empty());
}

/// A note of `n` paragraphs, none of which links anywhere.
fn filler(name: &str, n: usize) -> String {
    let mut s = format!("# {name}\n\n");
    for i in 1..=n {
        s.push_str(&format!("{name} filler {i}\n\n"));
    }
    s
}

#[test]
fn zk_backlinks_send_references_at_0_0_and_open_at_the_line() {
    // c.md has no link back to a.md, so zk's line is used.
    let mut c = filler("C page", 20);
    c.push_str("## Deep\n\nbottom\n");
    let (dir, mut app, log) = kind_app(ServerKind::Zk, "# A\n", &[("c.md", &c)], |root| {
        json!([zk_init(), {"expect": "workspace/executeCommand", "reply": {}},
               {"expect": "textDocument/references", "reply": [
                   {"uri": uri(&root.join("c.md")), "range": {
                       "start": {"line": 42, "character": 0},
                       "end": {"line": 42, "character": 0}}}]}])
    });
    let root = dir.path().canonicalize().unwrap();
    running(&mut app);
    keys(&mut app, "grr");
    pump_until(&mut app, "backlinks", |a| !picker_labels(a).is_empty());
    let req = &logged(&log, "textDocument/references")[0]["params"];
    assert_eq!(req["position"], json!({"line": 0, "character": 0}));
    assert_eq!(req["context"]["includeDeclaration"], json!(false));
    let item = app.picker().unwrap().items[0].clone();
    assert_eq!(item.label, "C page", "first heading of c.md");
    assert_eq!(item.detail, "c.md:43");
    send(&mut app, key(KeyCode::Enter));
    assert_eq!(page_path(&app), root.join("c.md"));
    assert_eq!(row_text(&app, app.cursor().row), "Deep", "c.md line 43");
    assert_eq!(app.history_depth(), 1);
}

#[test]
fn zk_backlinks_land_on_the_first_link_back_not_zks_line() {
    // zk reports the first substring hit of "a" (line 3, "para 0"); the
    // wikilink to a.md is on line 83.
    let mut long = String::from("# Long\n\npara 0\n");
    for i in 1..40 {
        long.push_str(&format!("\npara {i}\n"));
    }
    long.push_str("\n[[a]] link\n");
    assert_eq!(long.lines().position(|l| l == "[[a]] link"), Some(82));
    let (dir, mut app, _log) = kind_app(ServerKind::Zk, "# A\n", &[("long.md", &long)], |root| {
        json!([zk_init(), {"expect": "workspace/executeCommand", "reply": {}},
                   {"expect": "textDocument/references", "reply": [
                       {"uri": uri(&root.join("long.md")), "range": {
                           "start": {"line": 2, "character": 0},
                           "end": {"line": 2, "character": 0}}}]}])
    });
    let root = dir.path().canonicalize().unwrap();
    running(&mut app);
    keys(&mut app, "grr");
    pump_until(&mut app, "backlinks", |a| !picker_labels(a).is_empty());
    assert_eq!(app.picker().unwrap().items[0].detail, "long.md:3");
    send(&mut app, key(KeyCode::Enter));
    assert_eq!(page_path(&app), root.join("long.md"));
    assert_eq!(
        row_text(&app, app.cursor().row),
        "a link",
        "long.md line 83"
    );
    assert_eq!(app.link_under_cursor(), Some(0));
}

/// A note whose only link, `link` (on its own line 83), comes after a line
/// 3 reading "para 0" (where zk's substring hit lands).
fn long_note(link: &str) -> String {
    let mut long = String::from("# Long\n\npara 0\n");
    for i in 1..40 {
        long.push_str(&format!("\npara {i}\n"));
    }
    long.push_str(&format!("\n{link} link\n"));
    assert_eq!(long.lines().position(|l| l.starts_with(link)), Some(82));
    long
}

/// zk's backlinks reply: long.md at line 3.
fn long_md_at_line_3(root: &Path) -> Value {
    json!({"expect": "textDocument/references", "reply": [
        {"uri": uri(&root.join("long.md")), "range": {
            "start": {"line": 2, "character": 0},
            "end": {"line": 2, "character": 0}}}]})
}

#[test]
fn zk_backlinks_land_on_an_extensionless_markdown_link_back() {
    // `[x](a)` is zk's default link style; long.md's documentLink gets the
    // fake's null, so only local resolution (a + ".md") finds the link.
    let long = long_note("[x](a)");
    let (dir, mut app, _log) = kind_app(ServerKind::Zk, "# A\n", &[("long.md", &long)], |root| {
        json!([zk_init(), {"expect": "workspace/executeCommand", "reply": {}},
               long_md_at_line_3(root)])
    });
    let root = dir.path().canonicalize().unwrap();
    running(&mut app);
    keys(&mut app, "grr");
    pump_until(&mut app, "backlinks", |a| !picker_labels(a).is_empty());
    send(&mut app, key(KeyCode::Enter));
    assert_eq!(page_path(&app), root.join("long.md"));
    assert_eq!(
        row_text(&app, app.cursor().row),
        "x link",
        "long.md line 83"
    );
    assert_eq!(app.link_under_cursor(), Some(0));
}

#[test]
fn zk_backlinks_land_once_the_document_link_reply_arrives() {
    // `[x](alias)` does not resolve locally; only long.md's documentLink
    // reply, which arrives after the page opened, says it targets a.md.
    let long = long_note("[x](alias)");
    let (dir, mut app, _log) = kind_app(ServerKind::Zk, "# A\n", &[("long.md", &long)], |root| {
        json!([zk_init(), {"expect": "workspace/executeCommand", "reply": {}},
               long_md_at_line_3(root),
               {"expect": "textDocument/documentLink",
                "reply": [doc_link(82, 0..10, &root.join("a.md"))]}])
    });
    let root = dir.path().canonicalize().unwrap();
    running(&mut app);
    keys(&mut app, "grr");
    pump_until(&mut app, "backlinks", |a| !picker_labels(a).is_empty());
    send(&mut app, key(KeyCode::Enter));
    assert_eq!(page_path(&app), root.join("long.md"));
    assert_eq!(
        row_text(&app, app.cursor().row),
        "para 0",
        "zk's line first"
    );
    pump_until(&mut app, "documentLink applied", |a| {
        a.link_target(0).is_some()
    });
    assert_eq!(
        row_text(&app, app.cursor().row),
        "x link",
        "long.md line 83"
    );
    assert_eq!(app.link_under_cursor(), Some(0));
}

#[test]
fn zk_backlinks_do_not_jump_once_the_cursor_moved() {
    // As above, but the user moves before the documentLink reply (held
    // 300 ms) arrives: the cursor stays put.
    let long = long_note("[x](alias)");
    let (dir, mut app, _log) = kind_app(ServerKind::Zk, "# A\n", &[("long.md", &long)], |root| {
        json!([zk_init(), {"expect": "workspace/executeCommand", "reply": {}},
               long_md_at_line_3(root),
               {"expect": "textDocument/documentLink", "as": "dl"},
               {"sleep_ms": 300},
               {"respond": "dl", "result": [doc_link(82, 0..10, &root.join("a.md"))]}])
    });
    let root = dir.path().canonicalize().unwrap();
    running(&mut app);
    keys(&mut app, "grr");
    pump_until(&mut app, "backlinks", |a| !picker_labels(a).is_empty());
    send(&mut app, key(KeyCode::Enter));
    assert_eq!(page_path(&app), root.join("long.md"));
    keys(&mut app, "j");
    let moved = app.cursor();
    pump_until(&mut app, "documentLink applied", |a| {
        a.link_target(0).is_some()
    });
    assert_eq!(app.cursor(), moved);
}

#[test]
fn gd_on_an_extensionless_markdown_link_without_lsp_opens_the_md_file() {
    let (dir, mut app, _fx) = nav_app_with("# A\n\n[x](b)\n");
    std::fs::write(dir.path().join("b.md"), b_source()).unwrap();
    goto_text(&mut app, "x");
    keys(&mut app, "gd");
    assert_eq!(page_path(&app), dir.path().join("b.md"));
    assert_eq!(app.history_depth(), 1);
}

#[test]
fn marksman_backlinks_use_the_first_heading_or_are_hidden() {
    let src = "Intro text\n\n## Topic\n";
    let (_d, mut app, log) = kind_app(
        ServerKind::Marksman,
        src,
        &[("n.md", "plain\n")],
        |_| json!([zk_init(), {"expect": "textDocument/references", "reply": []}]),
    );
    running(&mut app);
    leader(&mut app, "zb");
    pump_until(&mut app, "loaded", |a| {
        a.picker().is_some_and(|p| !p.loading)
    });
    let req = &logged(&log, "textDocument/references")[0]["params"];
    assert_eq!(req["position"], json!({"line": 2, "character": 0}));
    send(&mut app, key(KeyCode::Esc));
    command(&mut app, "e n.md");
    assert!(page_path(&app).ends_with("n.md"));
    running(&mut app);
    keys(&mut app, "grr");
    assert_eq!(app.status(), "Backlinks need a language server");
    assert!(app.picker().is_none());
}

#[test]
fn links_picker_follows_through_follow() {
    let (dir, mut app, fx) = nav_app();
    leader(&mut app, "zl");
    let p = app.picker().unwrap();
    assert_eq!(p.items[0].label, "b");
    assert_eq!(p.items[0].detail, "b.md");
    keys(&mut app, "#section");
    assert_eq!(picker_labels(&app), ["h"]);
    send(&mut app, key(KeyCode::Enter));
    assert_eq!(row_text(&app, app.cursor().row), "Section two");
    command(&mut app, "Links");
    keys(&mut app, "example");
    send(&mut app, key(KeyCode::Enter));
    assert_eq!(fx.borrow().opened, ["https://example.com"]);
    command(&mut app, "Links");
    keys(&mut app, "missing");
    send(&mut app, key(KeyCode::Enter));
    assert_eq!(app.status(), "No such file: missing.md");
    assert!(page_path(&app).starts_with(dir.path()));
}

#[test]
fn command_line_commands() {
    let (dir, mut app, _fx) = nav_app();
    std::fs::create_dir_all(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub/c.md"), "# C\n\n[up](../b.md)\n").unwrap();
    keys(&mut app, ":e sub/c.m");
    assert_eq!(app.cmdline_prompt().as_deref(), Some(":e sub/c.m"));
    assert!(screen(&app).contains(":e sub/c.m"));
    keys(&mut app, "d");
    send(&mut app, key(KeyCode::Enter));
    assert_eq!(page_path(&app), dir.path().join("sub/c.md"));
    assert_eq!(app.history_depth(), 1);
    command(&mut app, "e ../b.md");
    assert_eq!(page_path(&app), dir.path().join("sub/../b.md"));
    command(&mut app, "e nope.md");
    assert!(app.status().contains("No such file"), "{}", app.status());
    command(&mut app, "Sidebar files");
    assert_eq!(app.sidebar_mode(), SidebarMode::Files);
    command(&mut app, "Sidebar sideways");
    assert!(app.status().starts_with(":Sidebar"));
    command(&mut app, "Launch nope");
    assert_eq!(app.status(), "No launcher named nope");
    command(&mut app, "Bogus");
    assert_eq!(app.status(), "Not a command: Bogus");
    keys(&mut app, ":x");
    send(&mut app, key(KeyCode::Esc));
    assert_eq!(app.mode(), Mode::Normal);
    assert_eq!(app.cmdline_prompt(), None);
    command(&mut app, "q");
    assert!(app.should_quit());
}

#[test]
fn colon_e_from_stdin_resolves_against_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::new(
        opts(dir.path(), StartTarget::Stdin("# S\n".into())),
        (COLS, ROWS),
    )
    .unwrap();
    command(&mut app, "e Cargo.toml.md");
    assert!(app.status().contains("No such file"), "{}", app.status());
    command(&mut app, "e README.md");
    let cwd = std::env::current_dir().unwrap();
    assert_eq!(page_path(&app), cwd.join("README.md"));
}

// Link and heading motions, hints (`]l [l ; , ]] [[ s`).

/// Links on rows 2 and 4, a heading below, a link after it.
fn links_source() -> String {
    "# Top\n\nsee [one](#end) and [two](#end)\n\n[three](#end) here\n\n## Mid\n\ntext\n\n## End\n\n[four](#top)\n".into()
}

/// Screen (row, col) of the first cell of the row text `word` in `app`.
fn word_at(app: &App, word: &str) -> Cursor {
    (0..total(app))
        .find_map(|r| row_text(app, r).find(word).map(|c| at(r, c)))
        .unwrap_or_else(|| panic!("no {word:?}"))
}

#[test]
fn link_motions_visit_links_in_screen_order_and_stop_at_ends() {
    let (_d, mut app, _fx) = nav_app_with(&links_source());
    let (one, two, three, four) = (
        word_at(&app, "one"),
        word_at(&app, "two"),
        word_at(&app, "three"),
        word_at(&app, "four"),
    );
    keys(&mut app, "]l");
    assert_eq!(app.cursor(), one);
    keys(&mut app, "]l");
    assert_eq!(app.cursor(), two);
    // From inside a link, ]l goes to the following one.
    keys(&mut app, "l]l");
    assert_eq!(app.cursor(), three);
    keys(&mut app, "]l");
    assert_eq!(app.cursor(), four);
    keys(&mut app, "]l");
    assert_eq!(app.cursor(), four, "no wrap-around");
    assert_eq!(app.status(), "No more links");
    keys(&mut app, "[l");
    assert_eq!(app.cursor(), three);
    keys(&mut app, "2[l");
    assert_eq!(app.cursor(), one);
    keys(&mut app, "5]l");
    assert_eq!(app.cursor(), four, "a count stops early at the end");
    keys(&mut app, "gg");
    keys(&mut app, "[l");
    assert_eq!(app.status(), "No more links");
    assert_eq!(app.cursor(), at(0, 0));
}

#[test]
fn semicolon_and_comma_repeat_the_last_link_motion() {
    let (_d, mut app, _fx) = nav_app_with(&links_source());
    keys(&mut app, ";");
    assert_eq!(app.status(), "No previous link motion");
    assert_eq!(app.cursor(), at(0, 0));
    keys(&mut app, ",");
    assert_eq!(app.status(), "No previous link motion");
    let (one, two, three) = (
        word_at(&app, "one"),
        word_at(&app, "two"),
        word_at(&app, "three"),
    );
    keys(&mut app, "]l;");
    assert_eq!(app.cursor(), two);
    keys(&mut app, ",");
    assert_eq!(app.cursor(), one);
    // After [l, ; goes backward and , forward.
    keys(&mut app, "G[l;");
    assert_eq!(app.cursor(), two);
    keys(&mut app, ",");
    assert_eq!(app.cursor(), three);
    // Heading motions do not change the repeat direction.
    keys(&mut app, "gg]];");
    assert_eq!(app.cursor(), three, "; still goes backward");
}

#[test]
fn heading_motions() {
    let (_d, mut app, _fx) = nav_app_with(&links_source());
    let (mid, end) = (find_row(&app, "Mid"), find_row(&app, "End"));
    keys(&mut app, "]]");
    assert_eq!(app.cursor(), at(mid, 0));
    keys(&mut app, "]]");
    assert_eq!(app.cursor(), at(end, 0));
    keys(&mut app, "]]");
    assert_eq!(app.status(), "No more headings");
    assert_eq!(app.cursor(), at(end, 0));
    keys(&mut app, "2[[");
    assert_eq!(app.cursor(), at(0, 0));
    keys(&mut app, "[[");
    assert_eq!(app.status(), "No more headings");
    keys(&mut app, "9]]");
    assert_eq!(app.cursor(), at(end, 0));
}

#[test]
fn heading_motion_only_keeps_the_heading_visible() {
    let (_d, mut app) = app_with(format!("{}## Last\n\nx\n", source()).as_bytes());
    let last = find_row_plain(&app, "Last");
    keys(&mut app, "]]");
    assert_eq!(app.cursor(), at(last, 0));
    assert_eq!(app.scroll(), last + 1 - VH, "scrolled just enough");
}

fn find_row_plain(app: &App, text: &str) -> usize {
    (0..total(app)).find(|&r| row_text(app, r) == text).unwrap()
}

#[test]
fn hint_labels_are_single_then_all_double() {
    use ramble::app::{HINT_ALPHABET, hint_labels};
    assert_eq!(hint_labels(3), ["a", "s", "d"]);
    let one = hint_labels(26);
    assert_eq!(one.concat(), HINT_ALPHABET);
    let two = hint_labels(27);
    assert_eq!(two.len(), 27);
    assert!(two.iter().all(|l| l.len() == 2));
    assert_eq!(&two[..2], ["aa", "as"]);
    assert_eq!(two[26], "sa");
    let set: std::collections::HashSet<_> = two.iter().collect();
    assert_eq!(set.len(), 27, "labels are unique");
}

#[test]
fn s_label_follows_the_link_and_esc_cancels() {
    let (_d, mut app, _fx) = nav_app_with(&links_source());
    keys(&mut app, "s");
    assert_eq!(app.mode(), ramble::app::Mode::Hint);
    let labels: Vec<_> = app.hints().iter().map(|(l, _)| l.to_string()).collect();
    // [four] is below the viewport.
    assert!(word_at(&app, "four").row >= VH);
    assert_eq!(labels, ["a", "s", "d"]);
    let shown = screen(&app);
    send(&mut app, key(KeyCode::Esc));
    assert_eq!(app.mode(), ramble::app::Mode::Normal);
    assert!(app.hints().is_empty());
    assert_eq!(app.cursor(), at(0, 0));
    assert_ne!(shown, screen(&app), "labels were drawn");

    // "d" is the third link, [three](#end): follows to the End heading.
    keys(&mut app, "sd");
    assert_eq!(app.mode(), ramble::app::Mode::Normal);
    assert_eq!(app.cursor(), at(find_row(&app, "End"), 0));
    assert_eq!(app.history_depth(), 1);
    send(&mut app, ctrl('o'));
    assert_eq!(
        app.cursor(),
        word_at(&app, "three"),
        "history holds the link"
    );

    keys(&mut app, "sx");
    assert_eq!(app.status(), "No such hint");
    assert_eq!(app.mode(), ramble::app::Mode::Normal);
}

#[test]
fn s_with_no_visible_links() {
    let (_d, mut app) = app();
    keys(&mut app, "s");
    assert_eq!(app.status(), "No links");
    assert_eq!(app.mode(), ramble::app::Mode::Normal);
}

#[test]
fn more_than_26_visible_links_get_two_letter_labels() {
    // 30 one-letter links, ten per paragraph, each to #t.
    let mut s = String::from("# T\n\n");
    for p in 0..3 {
        for i in 0..10 {
            s.push_str(&format!("[{}](#t) ", (b'a' + (p * 10 + i) % 26) as char));
        }
        s.push_str("\n\n");
    }
    let (_d, mut app, _fx) = nav_app_with(&s);
    keys(&mut app, "s");
    let hints: Vec<_> = app
        .hints()
        .iter()
        .map(|(l, sp)| (l.to_string(), *sp))
        .collect();
    assert_eq!(hints.len(), 30);
    assert!(hints.iter().all(|(l, _)| l.len() == 2));
    // The 29th link (index 28) is "sd"; typing "s" narrows, "d" follows.
    assert_eq!(hints[28].0, "sd");
    let target = hints[28].1;
    keys(&mut app, "s");
    assert_eq!(app.mode(), ramble::app::Mode::Hint);
    assert!(
        app.hints().iter().all(|(l, _)| l.len() == 1),
        "prefix typed"
    );
    keys(&mut app, "d");
    assert_eq!(app.mode(), ramble::app::Mode::Normal);
    assert_eq!(app.history_depth(), 1, "followed");
    assert_eq!(app.cursor(), at(0, 0), "jumped to #t");
    send(&mut app, ctrl('o'));
    assert_eq!(app.cursor(), at(target.row, target.col_start));
}

#[test]
fn review_jump_and_gutter_follow_markers() {
    let (dir, mut app) = app();
    keys(&mut app, "]r");
    assert_eq!(app.status(), ramble::app::NO_REVIEW);
    let mut m = ramble::review::Markers::default();
    m.files.insert(
        ramble::review::canonical(&dir.path().join("doc.md")),
        ramble::review::FileMarks {
            count: 1,
            lines: vec![(5, 5)],
        },
    );
    app.event(AppEvent::Review(m));
    assert_eq!(app.review_gutter(), 1);
    keys(&mut app, "]r");
    // Source line 5 is "Para 2".
    assert_eq!(app.cursor().row, para_row(2));
    assert!(screen(&app).contains("review 1"));
}

// -------------------------------------------------------------------------
// The `g?` help overlay with a language server (step 16).

fn help_keys(app: &App) -> Vec<String> {
    app.help_lines()
        .into_iter()
        .filter_map(|l| match l {
            ramble::app::HelpLine::Item { keys, .. } => Some(keys),
            ramble::app::HelpLine::Group(_) => None,
        })
        .collect()
}

#[test]
fn help_lists_hover_and_backlinks_once_a_zk_server_runs() {
    let (_dir, mut app, _log) = kind_app(ServerKind::Zk, "# A\n\n[b](b.md)\n", &[], |_| {
        json!([zk_init()])
    });
    running(&mut app);
    let k = help_keys(&app);
    for row in [
        "K",
        "Space zs",
        "Space zz",
        "Space zb",
        "grr",
        ":Search <query>",
    ] {
        assert!(k.iter().any(|x| x == row), "missing {row}: {k:?}");
    }
}

#[test]
fn esc_in_help_closes_the_hover_first() {
    let (_dir, mut app, _log) = lsp_app("# A\n\n[b](b.md)\n", |_| {
        json!([init_step(), {"expect": "textDocument/hover", "reply": {
            "contents": {"kind": "markdown", "value": "hovered"}}}])
    });
    running(&mut app);
    goto_text(&mut app, "b");
    keys(&mut app, "K");
    pump_until(&mut app, "hover", |a| a.hover_popup().is_some());
    keys(&mut app, "g?");
    assert_eq!(app.mode(), Mode::Help);
    send(&mut app, key(KeyCode::Esc));
    assert_eq!(app.hover_popup(), None);
    assert_eq!(app.mode(), Mode::Help, "first Esc only closed the hover");
    send(&mut app, key(KeyCode::Esc));
    assert_eq!(app.mode(), Mode::Normal);
}

/// `[render] math` reaches the interactive renderer.
#[test]
fn render_math_config_is_applied() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("doc.md");
    std::fs::write(&path, "A $x^2$ b\n").unwrap();
    let mut o = opts(dir.path(), StartTarget::File(path.clone()));
    let app = App::new(o.clone(), (COLS, ROWS)).unwrap();
    assert_eq!(row_text(&app, 0), "A x² b");
    o.config.render.math = false;
    let app = App::new(o, (COLS, ROWS)).unwrap();
    assert_eq!(row_text(&app, 0), "A $x^2$ b");
}

fn page_text(app: &App) -> String {
    (0..total(app))
        .map(|r| row_text(app, r))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn ctrl_l_rereads_the_file_without_a_watcher_event() {
    let (dir, mut app) = app();
    goto_row_text(&mut app, "Para 5 has foo.bar words");
    keys(&mut app, "w");
    let cursor = app.cursor();
    let edited = source() + "New tail\n";
    std::fs::write(dir.path().join("doc.md"), edited).unwrap();
    assert!(
        !page_text(&app).contains("New tail"),
        "no reload without C-l"
    );
    app.handle_key(ctrl('l'));
    assert!(page_text(&app).contains("New tail"));
    assert_eq!(app.cursor(), cursor, "cursor kept");
    assert_eq!(app.status(), "Refreshed");
    assert!(app.take_clear_request(), "terminal clear requested");
    assert!(!app.take_clear_request(), "request taken once");
}

#[test]
fn colon_e_without_a_path_and_refresh_reread_the_file() {
    let (dir, mut app) = app_with(b"# One\n");
    for (i, cmd) in ["e", "Refresh", "edit"].into_iter().enumerate() {
        let body = format!("# Body{i}\n");
        std::fs::write(dir.path().join("doc.md"), &body).unwrap();
        app.handle_key(key(KeyCode::Char(':')));
        keys(&mut app, cmd);
        app.handle_key(key(KeyCode::Enter));
        assert!(page_text(&app).contains(&format!("Body{i}")), ":{cmd}");
        assert_eq!(app.status(), "Refreshed");
        assert!(app.take_clear_request());
    }
}

#[test]
fn ctrl_l_on_a_deleted_file_keeps_the_banner_and_content() {
    let (dir, mut app) = app_with(b"# Kept\n");
    std::fs::remove_file(dir.path().join("doc.md")).unwrap();
    app.handle_key(ctrl('l'));
    assert!(page_text(&app).contains("Kept"));
    assert_eq!(app.banner(), Some(ramble::app::DELETED_BANNER));
    assert_ne!(app.status(), "Refreshed");
}

#[test]
fn ctrl_l_keeps_binary_and_lossy_statuses() {
    let (dir, mut app) = app_with(b"# One\n");
    std::fs::write(dir.path().join("doc.md"), b"# a\0b\n").unwrap();
    app.handle_key(ctrl('l'));
    assert_eq!(app.status(), "looks binary");
    std::fs::write(dir.path().join("doc.md"), b"# a\xffb\n").unwrap();
    app.handle_key(ctrl('l'));
    assert_eq!(app.status(), "not valid UTF-8");
}

#[test]
fn ctrl_l_on_stdin_does_not_reread() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::new(
        opts(dir.path(), StartTarget::Stdin("# Hi\n".into())),
        (COLS, ROWS),
    )
    .unwrap();
    app.handle_key(ctrl('l'));
    assert!(page_text(&app).contains("Hi"));
    assert_eq!(app.status(), "Nothing to refresh (stdin)");
    assert!(app.take_clear_request());
    let mut o = opts(dir.path(), StartTarget::Stdin("# Hi\n".into()));
    o.config.sidebar.default = SidebarMode::Files;
    let mut app = App::new(o, (COLS * 2, ROWS)).unwrap();
    app.handle_key(ctrl('l'));
    assert_eq!(app.status(), "Refreshed tree; stdin page unchanged");
}
