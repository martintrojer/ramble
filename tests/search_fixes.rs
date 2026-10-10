//! Regression tests for `*` / `#` on multi-byte and wide text, also after
//! whitespace collapsed in prose, `?` then Esc keeping the search
//! direction, and `#` from inside a word.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ramble::app::{App, Cursor, StartOptions, StartTarget};
use ramble::config::{Config, SidebarShow};
use tempfile::TempDir;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

fn app_with(content: &str) -> (TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("doc.md");
    std::fs::write(&path, content).unwrap();
    let mut config = Config::default();
    config.sidebar.show = SidebarShow::Never;
    config.lsp.server = vec![];
    let opts = StartOptions {
        target: StartTarget::File(path),
        tree_root: dir.path().to_path_buf(),
        config,
        review_cache: None,
        mdroots: ramble::app::MdrootsOptions::memory(),
    };
    (dir, App::new(opts, (40, 12)).unwrap())
}

fn keys(app: &mut App, s: &str) {
    for c in s.chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
}

fn esc(app: &mut App) {
    app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
}

fn at(row: usize, col: usize) -> Cursor {
    Cursor { row, col }
}

fn row_text(app: &App, row: usize) -> String {
    app.page().unwrap().rendered.lines[row]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect()
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// (column, word drawn under it) for each grapheme of `text`; the word is
/// `None` on non-word characters.
fn words_by_col(text: &str) -> Vec<(usize, Option<String>)> {
    let gs: Vec<&str> = text.graphemes(true).collect();
    let word = |g: &str| g.chars().next().is_some_and(is_word);
    let mut out = Vec::new();
    let mut col = 0;
    for (i, g) in gs.iter().enumerate() {
        let w = word(g).then(|| {
            let a = gs[..i].iter().rposition(|g| !word(g)).map_or(0, |j| j + 1);
            let b = gs[i..]
                .iter()
                .position(|g| !word(g))
                .map_or(gs.len(), |j| i + j);
            gs[a..b].concat()
        });
        out.push((col, w));
        col += g.width();
    }
    out
}

/// `*` and `#` on every column of the first row of `line`, drawn as
/// `drawn`, find the word drawn under the cursor and land on its start.
fn assert_star_and_hash_find_drawn_words(line: &str, drawn: &str) {
    let (_d, mut app) = app_with(&format!("{line}\n"));
    assert_eq!(row_text(&app, 0), drawn, "{line:?} drawn");
    for (col, want) in words_by_col(drawn) {
        for key in ["*", "#"] {
            esc(&mut app);
            keys(&mut app, "0");
            while app.cursor().col < col {
                keys(&mut app, "l");
            }
            assert_eq!(app.cursor(), at(0, col), "{line:?}");
            keys(&mut app, key);
            let src = &app.page().unwrap().doc.source;
            let found: Vec<&str> = app.search_hits().iter().map(|h| &src[h.clone()]).collect();
            match &want {
                Some(w) => {
                    assert_eq!(found, vec![w.as_str()], "{line:?} col {col} {key}");
                    let start = drawn.find(w.as_str()).unwrap();
                    let start_col = drawn[..start].width();
                    assert_eq!(app.cursor(), at(0, start_col), "{line:?} col {col} {key}");
                }
                None => {
                    assert_eq!(
                        app.status(),
                        "No word under cursor",
                        "{line:?} col {col} {key}"
                    );
                }
            }
        }
    }
}

#[test]
fn star_and_hash_on_multibyte_and_wide_text_find_the_word_under_the_cursor() {
    for line in ["éa", "日本 abc", "&amp;éééé"] {
        assert_star_and_hash_find_drawn_words(line, line);
    }
}

#[test]
fn star_and_hash_after_collapsed_whitespace_find_the_drawn_word() {
    // Prose collapses a tab (or a wide space) to one drawn space, so the
    // source widths do not add up to the segment as they do verbatim.
    for (line, drawn) in [
        ("é\tabc", "é abc"),
        ("日本\tabc x", "日本 abc x"),
        ("é\u{3000}abc", "é abc"),
    ] {
        assert_star_and_hash_find_drawn_words(line, drawn);
    }
}

#[test]
fn question_then_esc_keeps_the_search_direction() {
    let (_d, mut app) = app_with("foo one\n\nfoo two\n\nfoo three\n");
    keys(&mut app, "/foo");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.cursor(), at(2, 0));
    keys(&mut app, "?");
    assert_eq!(app.search_prompt().as_deref(), Some("?"));
    esc(&mut app);
    keys(&mut app, "n");
    assert_eq!(app.cursor(), at(4, 0), "n still searches forward");
    keys(&mut app, "/");
    assert_eq!(app.search_prompt().as_deref(), Some("/"));
    esc(&mut app);
    keys(&mut app, "?foo");
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.cursor(), at(2, 0));
    keys(&mut app, "/");
    esc(&mut app);
    keys(&mut app, "n");
    assert_eq!(app.cursor(), at(0, 0), "n still searches backward");
}

#[test]
fn hash_inside_a_word_skips_that_word_like_vim() {
    let (_d, mut app) = app_with("foo x foo\n");
    keys(&mut app, "$");
    assert_eq!(app.cursor(), at(0, 8));
    keys(&mut app, "#");
    assert_eq!(app.cursor(), at(0, 0), "previous occurrence, not this one");
    // `*` from inside a word is unchanged: the next occurrence.
    keys(&mut app, "0l*");
    assert_eq!(app.cursor(), at(0, 6));
    // Three occurrences: `#` from inside the last goes to the middle one.
    let (_d, mut app) = app_with("foo foo foo\n");
    keys(&mut app, "$#");
    assert_eq!(app.cursor(), at(0, 4));
}
