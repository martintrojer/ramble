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
