//! Front matter in the normal view (docs/specs/2026-10-08-front-matter.md):
//! the folded marker, the expanded table, `za` / `Enter`, reload, yank.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ramble::app::{App, Clipboard, StartOptions, StartTarget};
use ramble::config::Config;
use ramble::doc::parse;
use ramble::frontmatter::FmKind;
use ramble::render::{Theme, palette, render, render_page, to_ansi};
use tempfile::TempDir;

const SRC: &str = "---\ntitle: \"Hello\"\ndate: 2024-01-05\ntags: [rust, tui]\n\
                   authors:\n  - Ann\n  - Bob\nmeta:\n  a: 1\nempty:\n\
                   description: a rather long description that will not fit in the row\n\
                   ---\n\n# Body\n\nSee [b](b.md).\n";

fn rows(src: &str, width: u16, expanded: bool) -> Vec<String> {
    let page = render_page(
        &parse(src.into()),
        width,
        &Theme::catppuccin_mocha(),
        expanded,
    );
    page.lines
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect()
}

#[test]
fn folded_marker_with_key_count() {
    let r = rows(SRC, 40, false);
    assert_eq!(r[0], "▸ front matter · 7 keys");
    assert_eq!(r[1], "");
    assert_eq!(r[2], "Body");
    let one = rows("---\na: 1\n---\n\ntext\n", 40, false);
    assert_eq!(one[0], "▸ front matter · 1 key");
    let page = render_page(&parse(SRC.into()), 40, &Theme::catppuccin_mocha(), false);
    let dim = page.lines[0].spans[0].style.fg;
    assert_eq!(dim, Some(palette::OVERLAY), "the marker is dim");
}

#[test]
fn toml_front_matter_is_recognised() {
    let src = "+++\ntitle = \"T\"\ntags = [\"a\", \"b\"]\n+++\n\n# Body\n";
    let doc = parse(src.into());
    assert_eq!(doc.front_matter_kind, FmKind::Toml);
    assert!(doc.headings.iter().all(|h| h.text == "Body"));
    let r = rows(src, 40, true);
    assert_eq!(r[..4], ["▾ front matter", "title  T", "tags   a, b", ""]);
    assert_eq!(r[4], "Body");
}

#[test]
fn expanded_rows_in_source_order_padded_and_clipped() {
    let r = rows(SRC, 40, true);
    assert_eq!(
        r[..10],
        [
            "▾ front matter",
            "title        Hello",
            "date         2024-01-05",
            "tags         rust, tui",
            "authors      Ann, Bob",
            "meta         {…}",
            "empty        ",
            "description  a rather long description…",
            "",
            "Body",
        ]
    );
    assert!(unicode_width::UnicodeWidthStr::width(r[7].as_str()) <= 40);
    let page = render_page(&parse(SRC.into()), 40, &Theme::catppuccin_mocha(), true);
    let row = &page.lines[1].spans;
    assert_eq!(row[0].style.fg, Some(palette::OVERLAY), "keys are dim");
    assert_eq!(row.last().unwrap().style.fg, Some(palette::TEXT));
}

#[test]
fn key_column_is_capped_at_20() {
    let key = "k".repeat(30);
    let src = format!("---\n{key}: v\nb: w\n---\n\nx\n");
    let r = rows(&src, 60, true);
    assert_eq!(r[1], format!("{}…  v", "k".repeat(19)));
    assert_eq!(r[2], format!("b{}  w", " ".repeat(19)));
}

#[test]
fn unparsed_front_matter_shows_the_raw_lines_dimmed() {
    let src = "---\n# only a comment\nstray text\n---\n\nbody\n";
    assert_eq!(rows(src, 40, false)[0], "▸ front matter · unparsed");
    let r = rows(src, 40, true);
    assert_eq!(
        r[..5],
        [
            "▾ front matter",
            "# only a comment",
            "stray text",
            "",
            "body"
        ]
    );
}

#[test]
fn broken_yaml_still_shows_entries_and_the_body() {
    let src = "---\ntitle: \"unclosed\n\tbad: tab\nauthor: Ann\n---\n\n# Body\n";
    let r = rows(src, 40, true);
    assert!(
        r.iter()
            .any(|l| l.starts_with("author") && l.ends_with("Ann")),
        "{r:?}"
    );
    assert!(r.contains(&"Body".to_string()));
}

#[test]
fn page_without_front_matter_has_no_marker() {
    let r = rows("# Body\n\ntext\n", 40, false);
    assert_eq!(r[0], "Body");
    assert!(r.iter().all(|l| !l.contains("front matter")));
}

#[test]
fn print_mode_leaves_front_matter_out() {
    let page = render(&parse(SRC.into()), 40, &Theme::catppuccin_mocha());
    let out = to_ansi(&page);
    assert!(!out.contains("front matter"));
    assert!(!out.contains("title"));
    assert!(out.contains("Body"));
}

#[test]
fn raw_render_is_the_source() {
    let page = ramble::render::render_raw(&parse(SRC.into()), 80, &Theme::catppuccin_mocha());
    let first: String = page.lines[0]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert_eq!(first, "---");
}

// ---------------------------------------------------------------------------
// App

#[derive(Clone, Default)]
struct RecClip(Rc<RefCell<Vec<String>>>);

impl Clipboard for RecClip {
    fn copy(&mut self, text: &str) -> anyhow::Result<()> {
        self.0.borrow_mut().push(text.to_string());
        Ok(())
    }
}

fn opts(dir: &Path, path: &Path) -> StartOptions {
    let mut config = Config::default();
    config.sidebar.show = false;
    config.lsp.server = vec![];
    StartOptions {
        target: StartTarget::File(path.to_path_buf()),
        tree_root: dir.to_path_buf(),
        config,
    }
}

fn app_src(src: &str) -> (TempDir, App, RecClip) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.md");
    std::fs::write(&path, src).unwrap();
    std::fs::write(dir.path().join("b.md"), "# B\n").unwrap();
    let clip = RecClip::default();
    let app = App::new(opts(dir.path(), &path), (40, 20))
        .unwrap()
        .with_clipboard(clip.clone());
    (dir, app, clip)
}

fn keys(app: &mut App, s: &str) {
    for c in s.chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
}

fn enter(app: &mut App) {
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
}

fn row(app: &App, r: usize) -> String {
    app.page().unwrap().rendered.lines[r]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect()
}

#[test]
fn za_toggles() {
    let (_d, mut app, _) = app_src(SRC);
    assert!(row(&app, 0).starts_with('▸'));
    keys(&mut app, "za");
    assert!(app.front_matter_expanded());
    assert_eq!(row(&app, 0), "▾ front matter");
    assert!(row(&app, 1).starts_with("title"));
    keys(&mut app, "za");
    assert!(!app.front_matter_expanded());
    assert!(row(&app, 0).starts_with('▸'));
    assert_eq!(row(&app, 2), "Body");
}

#[test]
fn za_in_raw_view_or_without_front_matter_does_nothing() {
    let (_d, mut app, _) = app_src("# Body\n");
    keys(&mut app, "za");
    assert!(!app.front_matter_expanded());
    let (_d, mut app, _) = app_src(SRC);
    keys(&mut app, "gRza");
    assert!(!app.front_matter_expanded());
    assert_eq!(row(&app, 0), "---");
}

#[test]
fn enter_on_the_marker_toggles_and_on_a_link_follows() {
    let (_d, mut app, _) = app_src(SRC);
    enter(&mut app);
    assert!(app.front_matter_expanded());
    assert_eq!(app.cursor().row, 0, "cursor stays on the marker");
    enter(&mut app);
    assert!(!app.front_matter_expanded());
    // Row 5 is "See b." (marker, blank, Body, rule, blank).
    keys(&mut app, "6G");
    assert!(row(&app, 5).starts_with("See"), "{:?}", row(&app, 5));
    keys(&mut app, "0w"); // onto the link text
    enter(&mut app);
    assert!(
        app.title().ends_with("b.md"),
        "Enter on a link follows it: {}",
        app.title()
    );
}

#[test]
fn state_survives_a_reload_and_resets_on_a_new_page() {
    let (d, mut app, _) = app_src(SRC);
    keys(&mut app, "za");
    std::fs::write(d.path().join("a.md"), SRC.replace("Ann", "Anna")).unwrap();
    app.reload();
    assert!(app.front_matter_expanded(), "kept across a reload");
    assert_eq!(row(&app, 4), "authors      Anna, Bob");
    app.open_file(&d.path().join("b.md")).unwrap();
    assert!(!app.front_matter_expanded());
    app.open_file(&d.path().join("a.md")).unwrap();
    assert!(
        !app.front_matter_expanded(),
        "folded on every newly opened page"
    );
    assert!(row(&app, 0).starts_with('▸'));
}

#[test]
fn yank_on_an_entry_row_copies_source_text() {
    let (_d, mut app, clip) = app_src(SRC);
    keys(&mut app, "za");
    keys(&mut app, "5G"); // row 4: authors
    assert!(row(&app, 4).starts_with("authors"));
    keys(&mut app, "yy");
    assert_eq!(
        clip.0.borrow().last().unwrap(),
        "authors:\n  - Ann\n  - Bob\n"
    );
    keys(&mut app, "2G"); // row 1: title
    keys(&mut app, "w"); // onto the value
    keys(&mut app, "vey");
    assert_eq!(clip.0.borrow().last().unwrap(), "Hello");
}

#[test]
fn folded_marker_yank_copies_the_block() {
    let (_d, mut app, clip) = app_src("---\na: 1\n---\n\nbody\n");
    keys(&mut app, "yy");
    assert_eq!(clip.0.borrow().last().unwrap(), "---\na: 1\n---\n");
}

#[test]
fn resize_keeps_the_cursor_on_an_entry_row_and_fold_moves_it_to_the_marker() {
    let (_d, mut app, _) = app_src(SRC);
    keys(&mut app, "za5G");
    assert!(row(&app, 4).starts_with("authors"));
    app.event(ramble::app::AppEvent::Resize(50, 20));
    assert_eq!(app.cursor().row, 4, "{:?}", row(&app, app.cursor().row));
    keys(&mut app, "za");
    assert_eq!(
        app.cursor().row,
        0,
        "folding moves the cursor to the marker"
    );
}

#[test]
fn a_byte_order_mark_does_not_hide_the_front_matter() {
    let doc = ramble::doc::from_bytes(b"\xEF\xBB\xBF---\ntitle: T\n---\n\n# Body\n").unwrap();
    assert!(doc.front_matter.is_some());
    let page = render_page(&doc, 40, &Theme::catppuccin_mocha(), false);
    let first: String = page.lines[0]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert_eq!(first, "▸ front matter · 1 key");
    let out = to_ansi(&render(&doc, 40, &Theme::catppuccin_mocha()));
    assert!(!out.contains("title"), "{out}");
    // Stdin text goes through `parse` directly.
    let doc = parse("\u{feff}---\ntitle: T\n---\n\n# Body\n".into());
    assert!(doc.front_matter.is_some());
    assert!(!doc.source.starts_with('\u{feff}'));
}

#[test]
fn an_empty_block_is_front_matter_not_rules() {
    for src in [
        "---\n---\n\n# Body\n",
        "---\n\n---\n\n# Body\n",
        "+++\n+++\n\n# Body\n",
        "---\r\n---\r\n\r\n# Body\r\n",
        "+++\n+++\nBody\n",
    ] {
        let doc = parse(src.into());
        let fm = doc.front_matter.clone().expect(src);
        assert!(src[fm].ends_with(['-', '+']), "{src:?}");
        assert!(
            doc.blocks
                .iter()
                .all(|b| !matches!(b, ramble::doc::Block::Rule { .. })),
            "{src:?}: {:?}",
            doc.blocks
        );
        let r = rows(src, 40, false);
        assert_eq!(r[..3], ["▸ front matter · 0 keys", "", "Body"], "{src:?}");
        let out = to_ansi(&render(&doc, 40, &Theme::catppuccin_mocha()));
        assert!(!out.contains("+++") && !out.contains('─'), "{src:?}: {out}");
    }
    assert_eq!(parse("+++\n+++\n".into()).front_matter_kind, FmKind::Toml);
    let r = rows("---\n\n---\n\n# Body\n", 40, true);
    assert_eq!(r[..3], ["▾ front matter", "", "Body"]);
    // Inline markup right after the block is parsed as such.
    let r = rows("+++\n+++\n**bold** text\n", 40, false);
    assert_eq!(r[2], "bold text");
    // Not at the start, or not closed: no front matter.
    for src in ["\n---\n---\n", "---\n", "---\ntext\n"] {
        assert!(parse(src.into()).front_matter.is_none(), "{src:?}");
    }
}

#[test]
fn duplicate_and_non_string_keys_show_in_source_order_and_yank_their_line() {
    let src = "---\na: 1\nb: 2\na: 3\n007: z\n---\n\nbody\n";
    let r = rows(src, 40, true);
    assert_eq!(r[1..5], ["a    1", "b    2", "a    3", "007  z"]);
    let (_d, mut app, clip) = app_src(src);
    keys(&mut app, "za5Gyy"); // row 4: 007
    assert_eq!(clip.0.borrow().last().unwrap(), "007: z\n");
}

#[test]
fn toml_multi_line_string_row_shows_the_text() {
    let src = "+++\nmulti = \"\"\"\nline1\nline2\"\"\"\nz = 1\n+++\n\nbody\n";
    let r = rows(src, 40, true);
    assert_eq!(r[1..3], ["multi  line1 line2", "z      1"]);
    let (_d, mut app, clip) = app_src(src);
    keys(&mut app, "za2Gyy");
    assert_eq!(
        clip.0.borrow().last().unwrap(),
        "multi = \"\"\"\nline1\nline2\"\"\"\n"
    );
}

#[test]
fn enter_on_row_0_in_raw_view_is_follow_not_the_fold() {
    let (_d, mut app, _) = app_src(SRC);
    keys(&mut app, "gR");
    assert_eq!(row(&app, 0), "---");
    // Toggling there would be a no-op (raw view), so check the keymap.
    let enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(
        app.keymap(&[enter]),
        ramble::app::KeyResult::Action(ramble::app::Action::Follow)
    );
}

#[test]
fn unparsed_crlf_rows_have_no_carriage_return() {
    let src = "---\r\n# only a comment\r\nstray text\r\n---\r\n\r\nbody\r\n";
    let r = rows(src, 40, true);
    assert_eq!(r[1..3], ["# only a comment", "stray text"]);
}

#[test]
fn narrow_width_keeps_the_value_visible() {
    let src = "---\nauthorname: A\n---\n\nx\n";
    let entry = |w: u16| {
        rows(src, w, true)
            .into_iter()
            .find(|r| r.starts_with("auth"))
            .unwrap()
    };
    // 12 columns: the key column shrinks so the value keeps a column.
    assert_eq!(entry(12), "authorna…  A");
    for w in 4..=12 {
        let r = rows(src, w, true);
        assert!(
            r.iter()
                .all(|l| unicode_width::UnicodeWidthStr::width(l.as_str()) <= w as usize),
            "{w}: {r:?}"
        );
    }
}
