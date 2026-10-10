//! Visual selections and history survive a re-layout; a failed operator
//! motion cancels the operator; operator counts are capped.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ramble::app::{App, Clipboard, Mode, StartOptions, StartTarget};
use ramble::config::{Config, SidebarShow};
use tempfile::TempDir;

const ROWS: u16 = 12;

const LONG: &str = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau";

#[derive(Clone, Default)]
struct RecClip(Arc<Mutex<Vec<String>>>);

impl Clipboard for RecClip {
    fn copy(&mut self, text: &str) -> anyhow::Result<()> {
        self.0.lock().unwrap().push(text.to_string());
        Ok(())
    }
}

impl RecClip {
    fn last(&self) -> Option<String> {
        self.0.lock().unwrap().last().cloned()
    }
}

fn opts(dir: &Path, path: &Path) -> StartOptions {
    let mut config = Config::default();
    config.sidebar.show = SidebarShow::Never;
    config.lsp.server = vec![];
    StartOptions {
        target: StartTarget::File(path.to_path_buf()),
        tree_root: dir.to_path_buf(),
        config,
        review_cache: None,
        mdroots: ramble::app::MdrootsOptions::memory(),
    }
}

/// `a.md` holds `src` (and `b.md` holds `b`), opened at `cols`.
fn app_files(src: &str, b: &str, cols: u16) -> (TempDir, App, RecClip) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.md");
    std::fs::write(&path, src).unwrap();
    std::fs::write(dir.path().join("b.md"), b).unwrap();
    let clip = RecClip::default();
    let app = App::new(opts(dir.path(), &path), (cols, ROWS))
        .unwrap()
        .with_clipboard(clip.clone());
    (dir, app, clip)
}

fn app(src: &str, cols: u16) -> (TempDir, App, RecClip) {
    app_files(src, "# B\n", cols)
}

/// Type `s`; `\n` is Enter.
fn keys(app: &mut App, s: &str) {
    for c in s.chars() {
        let code = if c == '\n' {
            KeyCode::Enter
        } else {
            KeyCode::Char(c)
        };
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }
}

fn ctrl(app: &mut App, c: char) {
    app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
}

/// The source byte under the cursor.
fn cursor_byte(app: &App) -> usize {
    let p = app.page().unwrap();
    let c = app.cursor();
    p.rendered.srcmap.source_at(c.row, c.col).unwrap()
}

fn src() -> String {
    format!("# Title\n\n{LONG}\n\nTail.\n")
}

#[test]
fn active_selection_keeps_its_source_text_across_a_resize() {
    for (from, to) in [(30, 60), (60, 30), (30, 100)] {
        let (_d, mut app, clip) = app(&src(), from);
        keys(&mut app, "/iota\n");
        keys(&mut app, "veee");
        assert!(matches!(app.mode(), Mode::Visual(_)));
        app.resize(to, ROWS);
        keys(&mut app, "y");
        assert_eq!(
            clip.last().as_deref(),
            Some("iota kappa lambda"),
            "{from} -> {to}"
        );
    }
}

#[test]
fn backward_selection_keeps_its_source_text_across_a_resize() {
    let (_d, mut app, clip) = app(&src(), 30);
    keys(&mut app, "/lambda\n");
    keys(&mut app, "ev");
    // Anchor at the end of "lambda", cursor back at "iota".
    keys(&mut app, "bbb");
    app.resize(60, ROWS);
    keys(&mut app, "y");
    assert_eq!(clip.last().as_deref(), Some("iota kappa lambda"));
}

#[test]
fn gv_reselects_the_same_source_text_after_a_resize() {
    let (_d, mut app, clip) = app(&src(), 30);
    keys(&mut app, "/iota\n");
    keys(&mut app, "veee");
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.mode(), Mode::Normal);
    app.resize(60, ROWS);
    keys(&mut app, "gg");
    keys(&mut app, "gvy");
    assert_eq!(clip.last().as_deref(), Some("iota kappa lambda"));
    app.resize(25, ROWS);
    keys(&mut app, "gvy");
    assert_eq!(clip.last().as_deref(), Some("iota kappa lambda"));
}

#[test]
fn back_restores_the_same_source_position_after_a_resize() {
    // The link sits late in a paragraph that reflows with the width.
    let a = format!("# A\n\n{LONG} [next](b.md) end\n\nTail.\n");
    let (_d, mut app, _c) = app_files(&a, &format!("# B\n\n{LONG}\n"), 30);
    keys(&mut app, "/next\n");
    let left = cursor_byte(&app);
    assert_eq!(&a[left..left + 4], "next");
    keys(&mut app, "\n");
    assert_eq!(app.title(), "b.md");
    app.resize(80, ROWS);
    ctrl(&mut app, 'o');
    assert_eq!(app.title(), "a.md");
    assert_eq!(cursor_byte(&app), left, "C-o lands on the link");
    let c = app.cursor();
    assert!(c.row >= app.scroll() && c.row < app.scroll() + ROWS as usize);
}

#[test]
fn forward_restores_the_same_source_position_after_a_resize() {
    let a = "# A\n\n[next](b.md)\n";
    let b = format!("# B\n\n{LONG}\n");
    let (_d, mut app, _c) = app_files(a, &b, 30);
    keys(&mut app, "/next\n\n");
    assert_eq!(app.title(), "b.md");
    keys(&mut app, "/sigma\n");
    let left = cursor_byte(&app);
    assert_eq!(&b[left..left + 5], "sigma");
    ctrl(&mut app, 'o');
    assert_eq!(app.title(), "a.md");
    app.resize(70, ROWS);
    ctrl(&mut app, 'i');
    assert_eq!(app.title(), "b.md");
    assert_eq!(cursor_byte(&app), left, "C-i lands on sigma");
}

#[test]
fn yank_to_an_unset_mark_is_cancelled() {
    let (_d, mut app, clip) = app(&src(), 30);
    keys(&mut app, "3j");
    let before = app.cursor();
    keys(&mut app, "y'a");
    assert_eq!(clip.last(), None, "nothing yanked");
    assert_eq!(app.status(), "Mark not set: a");
    assert_eq!(app.mode(), Mode::Normal);
    assert_eq!(app.cursor(), before);
    // A set mark still yanks linewise.
    keys(&mut app, "majy'a");
    assert_eq!(app.cursor(), before, "cursor to the range start");
    assert_eq!(clip.last().as_deref(), Some(format!("{LONG}\n").as_str()));
}

#[test]
fn huge_operator_count_product_is_capped() {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (_d, mut app, clip) = app(&src(), 30);
        keys(&mut app, "999999y999999w");
        tx.send((app.mode(), clip.last())).unwrap();
    });
    let (mode, yanked) = rx
        .recv_timeout(Duration::from_secs(60))
        .expect("999999y999999w did not finish: count product not capped");
    assert_eq!(mode, Mode::Normal);
    assert!(yanked.is_some_and(|t| t.starts_with("Title")));
}

/// A paragraph mixing two-byte Greek, three-byte wide CJK and ASCII.
const MIXED: &str = "αβγδεζη θικλμνξ 日本語 テキスト alpha beta gamma delta iota kappa lambda tail rest words more words here";

fn mixed_src() -> String {
    format!("# Title\n\n{MIXED}\n\nTail.\n")
}

const WIDTHS: [(u16, u16); 4] = [(30, 60), (60, 30), (30, 100), (100, 30)];

/// Keys putting the cursor on a word start (or inside a word), the word
/// there, and what `veee` from there selects. The search lands on the
/// paragraph start; `w`/`l` move by drawn cell on any layout.
const MIXED_CASES: [(&str, &str, &str); 3] = [
    ("/αβγ\n8w", "iota", "iota kappa lambda"),
    ("/αβγ\nw", "θικλμνξ", "θικλμνξ 日本語 テキスト"),
    ("/αβγ\n2wl", "本語", "本語 テキスト alpha"),
];

#[test]
fn selections_on_multibyte_and_wide_text_keep_their_source_text_across_a_resize() {
    for (to_word, _, sel) in MIXED_CASES {
        for (from, to) in WIDTHS {
            {
                let (_d, mut app, clip) = app(&mixed_src(), from);
                keys(&mut app, to_word);
                keys(&mut app, "veee");
                app.resize(to, ROWS);
                keys(&mut app, "y");
                assert_eq!(clip.last().as_deref(), Some(sel), "active {from} -> {to}");
            }
            let (_d, mut app, clip) = app(&mixed_src(), from);
            keys(&mut app, to_word);
            keys(&mut app, "veee");
            app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            app.resize(to, ROWS);
            keys(&mut app, "gggvy");
            assert_eq!(clip.last().as_deref(), Some(sel), "gv {from} -> {to}");
        }
    }
}

#[test]
fn cursor_on_multibyte_and_wide_text_stays_on_its_grapheme_across_a_resize() {
    for (to_word, word, _) in MIXED_CASES {
        for (from, to) in WIDTHS {
            let (_d, mut app, clip) = app(&mixed_src(), from);
            keys(&mut app, to_word);
            keys(&mut app, "vey");
            assert_eq!(clip.last().as_deref(), Some(word), "before {from}");
            keys(&mut app, to_word);
            app.resize(to, ROWS);
            keys(&mut app, "vey");
            assert_eq!(clip.last().as_deref(), Some(word), "{from} -> {to}");
        }
    }
}

#[test]
fn selection_ending_on_a_blank_row_keeps_its_source_text_across_a_resize() {
    for (from, to) in WIDTHS {
        // Anchor on the blank separator above "Tail.".
        {
            let (_d, mut app, clip) = app(&src(), from);
            keys(&mut app, "/Tail\nkvj");
            app.resize(to, ROWS);
            keys(&mut app, "y");
            assert_eq!(clip.last().as_deref(), Some("T"), "active {from} -> {to}");
        }

        // Cursor on the blank separator, anchor on "lambda".
        let (_d, mut app, clip) = app(&src(), from);
        keys(&mut app, "/tau\nvj");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        app.resize(to, ROWS);
        keys(&mut app, "gggvy");
        assert_eq!(clip.last().as_deref(), Some("tau"), "gv {from} -> {to}");
    }
}

#[test]
fn gv_over_greek_and_ascii_keeps_its_source_text_when_widened() {
    // The review probe: the selection is on an ASCII-only row at 30
    // columns, and shares a segment with the Greek text at 60.
    let src = "# Title\n\nαβγδεζη θικλμνξ οπρστ φχψω alpha beta gamma delta iota kappa lambda tail rest words more words here\n\nTail.\n";
    let (_d, mut app, clip) = app(src, 30);
    keys(&mut app, "/iota\nveeey");
    assert_eq!(clip.last().as_deref(), Some("iota kappa lambda"));
    app.resize(60, ROWS);
    keys(&mut app, "gvy");
    assert_eq!(clip.last().as_deref(), Some("iota kappa lambda"));
}

#[test]
fn active_selection_with_its_cursor_on_a_blank_row_keeps_its_source_text_across_a_resize() {
    // (keys, what `y` copies): cursor on the blank separator below "tau"
    // (anchor on "tau"), and above "Tail." (anchor on "T", after `o`).
    for (k, sel) in [("/tau\nvj", "tau"), ("/Tail\nkvjo", "T")] {
        {
            let (_d, mut app, clip) = app(&src(), 30);
            keys(&mut app, k);
            keys(&mut app, "y");
            assert_eq!(clip.last().as_deref(), Some(sel), "control {k:?}");
        }
        for (from, to) in WIDTHS.into_iter().chain([(30, 30)]) {
            let (_d, mut app, clip) = app(&src(), from);
            keys(&mut app, k);
            app.resize(to, ROWS);
            keys(&mut app, "y");
            assert_eq!(clip.last().as_deref(), Some(sel), "{k:?} {from} -> {to}");
        }
    }
}

/// [`MIXED`] with a tab, which a paragraph draws as one space.
fn mixed_tab_src() -> String {
    mixed_src().replace("テキスト alpha", "テキスト\talpha")
}

#[test]
fn selections_after_a_collapsed_tab_keep_their_source_text_across_a_resize() {
    for (to_word, _, sel) in MIXED_CASES {
        let sel = sel.replace("テキスト alpha", "テキスト\talpha");
        for (from, to) in WIDTHS {
            {
                let (_d, mut app, clip) = app(&mixed_tab_src(), from);
                keys(&mut app, to_word);
                keys(&mut app, "veee");
                app.resize(to, ROWS);
                keys(&mut app, "y");
                assert_eq!(clip.last(), Some(sel.clone()), "active {from} -> {to}");
            }
            let (_d, mut app, clip) = app(&mixed_tab_src(), from);
            keys(&mut app, to_word);
            keys(&mut app, "veee");
            app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
            app.resize(to, ROWS);
            keys(&mut app, "gggvy");
            assert_eq!(clip.last(), Some(sel.clone()), "gv {from} -> {to}");
        }
    }
}

#[test]
fn cursor_after_a_collapsed_tab_stays_on_its_grapheme_across_a_resize() {
    for (to_word, word, _) in MIXED_CASES {
        for (from, to) in WIDTHS {
            let (_d, mut app, clip) = app(&mixed_tab_src(), from);
            keys(&mut app, to_word);
            keys(&mut app, "vey");
            assert_eq!(clip.last().as_deref(), Some(word), "before {from}");
            keys(&mut app, to_word);
            app.resize(to, ROWS);
            keys(&mut app, "vey");
            assert_eq!(clip.last().as_deref(), Some(word), "{from} -> {to}");
        }
    }
}
