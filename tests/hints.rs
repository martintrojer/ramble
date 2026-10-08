//! Hint labels (`s`) on links that are only partly on screen.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ramble::app::{App, Mode, StartOptions, StartTarget};
use ramble::config::Config;
use ramble::render::ScreenSpan;
use tempfile::TempDir;

const COLS: u16 = 40;
const ROWS: u16 = 12;

/// An app on `dir/a.md` holding `source`, sidebar off and no language
/// servers, so the content gets the full width.
fn app_with(source: &str) -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.md");
    std::fs::write(&path, source).unwrap();
    let mut config = Config::default();
    config.sidebar.show = false;
    config.lsp.server = vec![];
    let opts = StartOptions {
        target: StartTarget::File(path),
        tree_root: dir.path().to_path_buf(),
        config,
    };
    let app = App::new(opts, (COLS, ROWS)).unwrap();
    (dir, app)
}

fn press(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    app.handle_key(KeyEvent::new(code, mods));
}

/// Drawn spans of the page's only link, in screen order.
fn link_spans(app: &App) -> Vec<ScreenSpan> {
    let p = app.page().unwrap();
    assert_eq!(p.doc.links.len(), 1);
    let mut v = p
        .rendered
        .srcmap
        .spans_for(p.doc.links[0].text_range.clone());
    v.sort();
    v
}

#[test]
fn s_labels_a_wrapped_link_at_its_first_visible_row() {
    // A link wrapping over rows 0 and 1, then enough filler to scroll.
    let mut s = String::from("[alpha bravo charlie delta echo foxtrot golf hotel india](#t)\n\n");
    for i in 1..=30 {
        s.push_str(&format!("Filler paragraph {i}\n\n"));
    }
    s.push_str("# T\n");
    let (_d, mut app) = app_with(&s);
    let spans = link_spans(&app);
    let (first, second) = (spans[0], spans[spans.len() - 1]);
    assert_eq!((first.row, second.row), (0, 1), "link wraps: {spans:?}");

    // Scroll one line: the link's first row is above the viewport.
    press(&mut app, KeyCode::Char('e'), KeyModifiers::CONTROL);
    assert_eq!(app.scroll(), 1);

    press(&mut app, KeyCode::Char('s'), KeyModifiers::NONE);
    assert_eq!(app.mode(), Mode::Hint);
    let hints: Vec<_> = app
        .hints()
        .iter()
        .map(|(l, sp)| (l.to_string(), *sp))
        .collect();
    assert_eq!(hints, [("a".to_string(), second)], "label on visible row");
    assert_eq!(second.col_start, 0, "continuation starts the row");

    // Typing the label follows the link.
    press(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
    assert_eq!(app.mode(), Mode::Normal);
    assert_eq!(app.history_depth(), 1, "followed");
}
