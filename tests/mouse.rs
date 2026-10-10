//! Mouse (docs/specs/2026-10-08-mouse.md): clicks, double and triple
//! clicks, drag-to-copy, the wheel, popups. Every test draws once into a
//! TestBackend so the recorded layout is real, then injects synthetic
//! `MouseEvent`s with their `Instant`s. Temp dirs only.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ramble::app::{
    App, AppEvent, Clipboard, Effect, Focus, Hit, Layout, Mode, StartOptions, StartTarget,
    VisualKind,
};
use ramble::config::{Config, SidebarMode, SidebarShow, SidebarWidth};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tempfile::TempDir;

const SIZE: (u16, u16) = (90, 16);

#[derive(Clone, Default)]
struct RecClip(Rc<RefCell<Vec<String>>>);

impl Clipboard for RecClip {
    fn copy(&mut self, text: &str) -> anyhow::Result<()> {
        self.0.borrow_mut().push(text.to_string());
        Ok(())
    }
}

impl RecClip {
    fn all(&self) -> Vec<String> {
        self.0.borrow().clone()
    }
}

fn page_src() -> String {
    let mut s = String::from(
        "# Title\n\nAlpha beta [link](b.md) gamma.\n\n日本語 wide\n\nSee [other](c.md) too.\n\n## Second\n\n",
    );
    for i in 0..40 {
        s.push_str(&format!("line {i}\n\n"));
    }
    s
}

fn write(root: &Path, rel: &str, body: &str) -> PathBuf {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, body).unwrap();
    p
}

struct T {
    _dir: TempDir,
    root: PathBuf,
    app: App,
    clip: RecClip,
    t0: Instant,
}

fn setup_with(mode: SidebarMode, edit: impl FnOnce(&mut Config)) -> T {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let a = write(&root, "a.md", &page_src());
    write(&root, "b.md", "# B\n");
    write(&root, "c.md", "# C\n");
    write(&root, "d.md", "# D\n");
    write(&root, "e.md", "# E\n");
    write(&root, "docs/guide.md", "# Guide\n");
    let mut config = Config::default();
    config.lsp.server = vec![];
    config.review.enabled = false;
    config.sidebar.default = mode;
    edit(&mut config);
    let clip = RecClip::default();
    let app = App::new(
        StartOptions {
            target: StartTarget::File(a),
            tree_root: root.clone(),
            config,
            review_cache: None,
            mdroots: ramble::app::MdrootsOptions::memory(),
        },
        SIZE,
    )
    .unwrap()
    .with_clipboard(clip.clone())
    .with_effects(|_| {}, |_| Ok(()));
    let mut t = T {
        _dir: dir,
        root,
        app,
        clip,
        t0: Instant::now(),
    };
    t.draw();
    t
}

fn setup() -> T {
    setup_with(SidebarMode::Split, |_| {})
}

impl T {
    /// Draw a frame (records the layout); returns the screen rows.
    fn draw(&mut self) -> Vec<String> {
        let mut term = Terminal::new(TestBackend::new(SIZE.0, SIZE.1)).unwrap();
        term.draw(|f| ramble::ui::draw(f, &self.app)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..SIZE.1)
            .map(|y| {
                (0..SIZE.0)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect()
            })
            .collect()
    }

    /// Screen cell of the first `text` drawn inside `area` (one cell per
    /// column; ASCII `text`).
    fn find_in(&mut self, area: ratatui::layout::Rect, text: &str) -> (u16, u16) {
        let mut term = Terminal::new(TestBackend::new(SIZE.0, SIZE.1)).unwrap();
        term.draw(|f| ramble::ui::draw(f, &self.app)).unwrap();
        let buf = term.backend().buffer();
        for y in area.top()..area.bottom() {
            let row: Vec<&str> = (area.left()..area.right())
                .map(|x| buf[(x, y)].symbol())
                .collect();
            let n = text.chars().count();
            for i in 0..row.len().saturating_sub(n - 1) {
                if row[i..i + n].concat() == text {
                    return (area.x + i as u16, y);
                }
            }
        }
        panic!("{text:?} not drawn in {area:?}");
    }

    /// Screen cell of `text` on a files row of a fresh frame. The pane's
    /// title row shows the clipped temp root (`.tmpXXXXXX`), which can
    /// spell a short name like `n4`, so only the item rows are searched.
    fn files_cell(&mut self, text: &str) -> (u16, u16) {
        self.draw();
        let items = self.app.layout().files.expect("files drawn").items;
        self.find_in(items, text)
    }

    fn text_cell(&mut self, text: &str) -> (u16, u16) {
        let area = self.app.layout().text.expect("text drawn");
        self.find_in(area, text)
    }

    fn mouse(&mut self, kind: MouseEventKind, (col, row): (u16, u16), at_ms: u64) {
        let ev = MouseEvent {
            kind,
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        };
        let now = self.t0 + Duration::from_millis(at_ms);
        self.app.event(AppEvent::Mouse(ev, now));
    }

    fn press(&mut self, at: (u16, u16), ms: u64) {
        self.mouse(MouseEventKind::Down(MouseButton::Left), at, ms);
    }

    fn release(&mut self, at: (u16, u16), ms: u64) {
        self.mouse(MouseEventKind::Up(MouseButton::Left), at, ms);
    }

    fn click(&mut self, at: (u16, u16), ms: u64) {
        self.press(at, ms);
        self.release(at, ms);
    }

    fn drag(&mut self, at: (u16, u16), ms: u64) {
        self.mouse(MouseEventKind::Drag(MouseButton::Left), at, ms);
    }

    fn keys(&mut self, s: &str) {
        for c in s.chars() {
            self.app
                .handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }

    fn selected_file(&self) -> Option<PathBuf> {
        self.app.tree()?.selected().map(Path::to_path_buf)
    }
}

#[test]
fn no_layout_means_no_effect() {
    let mut t = setup();
    t.app.set_layout(Layout::default());
    let before = (t.app.cursor(), t.app.focus(), t.app.scroll());
    assert_eq!(t.app.hit(40, 5), Hit::None);
    for at in [(40, 5), (2, 2), (45, 12)] {
        t.click(at, 0);
        t.mouse(MouseEventKind::ScrollDown, at, 0);
    }
    assert_eq!((t.app.cursor(), t.app.focus(), t.app.scroll()), before);
}

#[test]
fn click_puts_the_cursor_on_the_cell_and_cancels_visual() {
    let mut t = setup();
    t.keys("vj");
    assert_eq!(t.app.mode(), Mode::Visual(VisualKind::Char));
    let (x, y) = t.text_cell("gamma");
    t.click((x + 2, y), 0);
    assert_eq!(t.app.mode(), Mode::Normal);
    let row = t.app.scroll() + (y - t.app.layout().text.unwrap().y) as usize;
    let col = (x + 2 - t.app.layout().text.unwrap().x) as usize;
    assert_eq!(t.app.cursor().row, row);
    assert_eq!(t.app.cursor().col, col);
    assert_eq!(t.app.focus(), Focus::Content);
}

#[test]
fn click_past_the_line_end_clamps_and_wide_chars_land_on_their_start() {
    let mut t = setup();
    let text = t.app.layout().text.unwrap();
    let (x, y) = t.text_cell("wide");
    // "日本語 wide": 日 at cols 0-1, 本 at 2-3. The right half of 本 → col 2.
    let start = x - 7;
    t.click((start + 3, y), 0);
    assert_eq!(t.app.cursor().col, 2);
    // Far right of the row: the last grapheme ("e" of "wide").
    t.click((text.right() - 1, y), 1000);
    assert_eq!(t.app.cursor().col, 10);
}

#[test]
fn click_a_files_row_focuses_and_selects_it_and_the_arrow_expands() {
    let mut t = setup();
    let at = t.files_cell("c.md");
    t.click(at, 0);
    assert_eq!(t.app.focus(), Focus::Files);
    assert_eq!(t.selected_file(), Some(t.root.join("c.md")));
    assert!(
        t.app
            .page()
            .unwrap()
            .path
            .as_ref()
            .unwrap()
            .ends_with("a.md")
    );

    let docs = t.files_cell("docs");
    assert!(!t.app.tree().unwrap().is_expanded(&t.root.join("docs")));
    // A click on the name selects without toggling.
    t.click(docs, 2000);
    assert!(!t.app.tree().unwrap().is_expanded(&t.root.join("docs")));
    // The `▸ ` cell, two columns left of the name.
    t.click((docs.0 - 2, docs.1), 4000);
    assert!(t.app.tree().unwrap().is_expanded(&t.root.join("docs")));
    t.draw();
    t.click((docs.0 - 1, docs.1), 6000);
    assert!(!t.app.tree().unwrap().is_expanded(&t.root.join("docs")));
}

/// Screen cell of the folder arrow (`▸`/`▾`) on the drawn files row `y`.
fn arrow_on_row(t: &mut T, y: u16) -> u16 {
    let files = t.app.layout().files.unwrap().pane;
    let rows = t.draw();
    let row: Vec<char> = rows[y as usize].chars().collect();
    (files.left()..files.right())
        .find(|&x| matches!(row[x as usize], '▸' | '▾'))
        .unwrap_or_else(|| panic!("no arrow on row {y}: {}", rows[y as usize]))
}

#[test]
fn arrow_clicks_follow_the_marker_gutter_and_the_indent_cap() {
    let mut t = setup_with(SidebarMode::Files, |c| {
        c.sidebar.width = SidebarWidth::Fixed(20)
    });
    write(&t.root, "n1/n2/n3/n4/n5/n6/deep.md", "# Deep\n");
    t.app
        .handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
    t.draw();
    let files = t.app.layout().files.unwrap().pane;
    let tree = |t: &T, rel: &str| t.app.tree().unwrap().is_expanded(&t.root.join(rel));

    // Depth 0: the arrow sits right after the 1-column marker gutter.
    let (_, y) = t.files_cell("n1");
    let x = arrow_on_row(&mut t, y);
    assert_eq!(x, files.x + 1, "depth-0 arrow after the gutter");
    t.click((x - 1, y), 0); // the gutter: selects, no toggle
    assert!(!tree(&t, "n1"));
    t.click((x, y), 1000);
    assert!(tree(&t, "n1"));

    // Open down to depth 5 by clicking each arrow's second column.
    let mut path = String::from("n1");
    for d in 2..=6 {
        path.push_str(&format!("/n{d}"));
        let (_, y) = t.files_cell(&format!("n{d}"));
        let x = arrow_on_row(&mut t, y);
        t.click((x + 1, y), 2000 + 1000 * d);
        assert!(tree(&t, &path), "depth {} arrow at {x}", d - 1);
    }
    // n5 (depth 4) and n6 (depth 5) are past the cap: same arrow column.
    let (_, y5) = t.files_cell("n5");
    let (_, y6) = t.files_cell("n6");
    let (x5, x6) = (arrow_on_row(&mut t, y5), arrow_on_row(&mut t, y6));
    assert_eq!(x5, x6, "capped indent");
    // The column right after the arrow cell is the name: no toggle.
    t.click((x6 + 2, y6), 20_000);
    assert!(tree(&t, &path));
    t.click((x6, y6), 22_000);
    assert!(!tree(&t, &path), "deep arrow toggles");
}

#[test]
fn double_click_a_file_opens_it_but_not_after_the_timeout() {
    let mut t = setup();
    let at = t.files_cell("b.md");
    t.click(at, 0);
    t.click(at, 401);
    assert!(
        t.app
            .page()
            .unwrap()
            .path
            .as_ref()
            .unwrap()
            .ends_with("a.md")
    );
    t.click(at, 700);
    assert!(
        t.app
            .page()
            .unwrap()
            .path
            .as_ref()
            .unwrap()
            .ends_with("b.md")
    );
    assert_eq!(t.app.history_depth(), 1);
}

#[test]
fn double_click_a_non_markdown_file_edits_it() {
    let mut t = setup_with(SidebarMode::Files, |c| c.sidebar.show_all = true);
    write(&t.root, "z.txt", "plain\n");
    t.app
        .handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
    t.draw();
    let at = t.files_cell("z.txt");
    t.click(at, 0);
    t.click(at, 100);
    assert_eq!(
        t.app.pending_effect(),
        Some(&Effect::Edit {
            path: t.root.join("z.txt"),
            line: None
        })
    );
}

#[test]
fn outline_click_selects_and_double_click_jumps() {
    let mut t = setup();
    let outline = t.app.layout().outline.unwrap().pane;
    let at = t.find_in(outline, "Second");
    t.click(at, 0);
    assert_eq!(t.app.focus(), Focus::Outline);
    assert_eq!(t.app.outline_selected(), Some(1));
    t.draw();
    let at = t.find_in(outline, "Second");
    t.click(at, 1000);
    t.click(at, 1100);
    assert_eq!(t.app.focus(), Focus::Content);
    let second = t.app.outline()[1].row;
    assert_eq!(t.app.cursor().row, second);
}

#[test]
fn double_click_a_link_follows_it() {
    let mut t = setup();
    let at = t.text_cell("link");
    t.click(at, 0);
    assert_eq!(t.app.history_depth(), 0);
    t.click(at, 100);
    assert!(
        t.app
            .page()
            .unwrap()
            .path
            .as_ref()
            .unwrap()
            .ends_with("b.md")
    );
    assert_eq!(t.app.history_depth(), 1);
}

#[test]
fn double_click_yanks_the_word_and_triple_click_the_line() {
    let mut t = setup();
    let at = t.text_cell("beta");
    t.click((at.0 + 1, at.1), 0);
    t.click((at.0 + 1, at.1), 100);
    assert_eq!(t.clip.all(), ["beta"]);
    assert_eq!(t.app.mode(), Mode::Visual(VisualKind::Char));
    assert_eq!(t.app.status(), "Copied beta");
    t.click((at.0 + 1, at.1), 200);
    assert_eq!(t.app.mode(), Mode::Visual(VisualKind::Line));
    assert_eq!(
        t.clip.all().last().unwrap(),
        "Alpha beta [link](b.md) gamma.\n"
    );
    // A fourth press starts over as a single click.
    t.click((at.0 + 1, at.1), 300);
    assert_eq!(t.app.mode(), Mode::Normal);
    assert_eq!(t.clip.all().len(), 2);
}

#[test]
fn drag_selects_and_copies_on_release_keeping_visual() {
    let mut t = setup();
    let a = t.text_cell("Alpha");
    let g = t.text_cell("gamma");
    t.press(a, 0);
    t.drag((a.0 + 3, a.1), 10);
    assert_eq!(t.app.mode(), Mode::Visual(VisualKind::Char));
    t.drag((g.0 + 4, g.1), 20);
    t.release((g.0 + 4, g.1), 30);
    assert_eq!(t.clip.all(), ["Alpha beta [link](b.md) gamma"]);
    assert_eq!(t.app.mode(), Mode::Visual(VisualKind::Char));
    let text = t.app.layout().text.unwrap();
    assert_eq!(t.app.cursor().col, (g.0 + 4 - text.x) as usize);
}

#[test]
fn press_and_release_without_moving_copies_nothing() {
    let mut t = setup();
    let a = t.text_cell("Alpha");
    t.press(a, 0);
    t.drag(a, 10); // same cell: no selection yet
    t.release(a, 20);
    assert!(t.clip.all().is_empty());
    assert_eq!(t.app.mode(), Mode::Normal);
}

#[test]
fn dragging_below_the_text_scrolls() {
    let mut t = setup();
    let a = t.text_cell("Alpha");
    let below = t.app.layout().status.unwrap();
    t.press(a, 0);
    for i in 0..5 {
        t.drag((a.0, below.y), 10 + i);
    }
    assert_eq!(t.app.scroll(), 5);
    assert_eq!(t.app.mode(), Mode::Visual(VisualKind::Char));
    let vh = t.app.viewport_height();
    assert_eq!(t.app.cursor().row, 5 + vh - 1);
}

#[test]
fn drag_from_the_sidebar_is_ignored() {
    let mut t = setup();
    let at = t.files_cell("b.md");
    let g = t.text_cell("gamma");
    t.press(at, 0);
    t.drag(g, 10);
    t.release(g, 20);
    assert_eq!(t.app.mode(), Mode::Normal);
    assert!(t.clip.all().is_empty());
}

#[test]
fn wheel_scrolls_the_pane_under_the_pointer_without_focusing_it() {
    let mut t = setup();
    let b = t.files_cell("b.md");
    t.click(b, 0);
    assert_eq!(t.app.focus(), Focus::Files);
    let a = t.text_cell("Alpha");
    t.mouse(MouseEventKind::ScrollDown, a, 1000);
    assert_eq!(t.app.scroll(), 3);
    assert_eq!(t.app.focus(), Focus::Files);
    t.mouse(MouseEventKind::ScrollUp, a, 1100);
    assert_eq!(t.app.scroll(), 0);

    // Over the files pane: its selection moves 3 rows (b.md → e.md).
    t.mouse(MouseEventKind::ScrollDown, b, 1200);
    assert_eq!(t.selected_file(), Some(t.root.join("e.md")));
    // Over the outline while files has focus: focus stays.
    let outline = t.app.layout().outline.unwrap().pane;
    t.mouse(
        MouseEventKind::ScrollDown,
        (outline.x + 1, outline.y + 1),
        1300,
    );
    assert_eq!(t.app.focus(), Focus::Files);
    // Horizontal wheel does nothing.
    t.mouse(MouseEventKind::ScrollRight, a, 1400);
    assert_eq!(t.app.scroll(), 0);
}

#[test]
fn picker_click_selects_double_click_opens_and_wheel_moves() {
    let mut t = setup();
    t.keys(" zl");
    assert_eq!(t.app.mode(), Mode::Picker);
    t.draw();
    let picker = t.app.layout().picker.unwrap();
    let other = t.find_in(picker.pane, "other");
    t.click(other, 0);
    assert_eq!(t.app.picker().unwrap().selected, 1);
    t.mouse(MouseEventKind::ScrollDown, other, 1000);
    assert_eq!(t.app.picker().unwrap().selected, 0);
    t.mouse(MouseEventKind::ScrollUp, other, 1100);
    assert_eq!(t.app.picker().unwrap().selected, 1);
    t.click(other, 2000);
    t.click(other, 2100);
    assert!(t.app.picker().is_none());
    assert!(
        t.app
            .page()
            .unwrap()
            .path
            .as_ref()
            .unwrap()
            .ends_with("c.md")
    );
}

#[test]
fn a_click_outside_the_picker_or_help_closes_it_and_does_nothing_else() {
    let mut t = setup();
    t.keys(" zl");
    t.draw();
    let before = t.app.cursor();
    t.click((0, 0), 0);
    assert!(t.app.picker().is_none());
    assert_eq!(t.app.mode(), Mode::Normal);
    assert_eq!(t.app.focus(), Focus::Content);
    assert_eq!(t.app.cursor(), before);

    t.keys("g?");
    assert_eq!(t.app.mode(), Mode::Help);
    t.draw();
    let help = t.app.layout().help.unwrap();
    t.click((help.x + 2, help.y + 3), 1000);
    assert_eq!(t.app.mode(), Mode::Help, "a click inside keeps it");
    t.mouse(MouseEventKind::ScrollDown, (help.x + 2, help.y + 3), 1100);
    assert_eq!(t.app.help_view().unwrap().scroll, 3);
    t.click((0, SIZE.1 - 1), 2000);
    assert_eq!(t.app.mode(), Mode::Normal);
}

#[test]
fn status_line_clicks_and_other_buttons_do_nothing() {
    let mut t = setup();
    let before = (t.app.cursor(), t.app.focus(), t.app.mode());
    t.click((40, SIZE.1 - 1), 0);
    let a = t.text_cell("gamma");
    t.mouse(MouseEventKind::Down(MouseButton::Right), a, 1000);
    t.mouse(MouseEventKind::Down(MouseButton::Middle), a, 1100);
    t.mouse(MouseEventKind::Moved, a, 1200);
    assert_eq!((t.app.cursor(), t.app.focus(), t.app.mode()), before);
}

#[test]
fn typing_modes_ignore_clicks() {
    let mut t = setup();
    t.keys("/al");
    assert_eq!(t.app.mode(), Mode::Search);
    let before = t.app.cursor();
    let g = t.text_cell("gamma");
    t.click(g, 0);
    assert_eq!(t.app.mode(), Mode::Search);
    assert_eq!(t.app.cursor(), before);
}

#[test]
fn disabled_mouse_ignores_events() {
    let mut t = setup_with(SidebarMode::Split, |c| c.mouse.enabled = false);
    let before = (t.app.cursor(), t.app.focus(), t.app.scroll());
    let g = t.text_cell("gamma");
    t.click(g, 0);
    t.mouse(MouseEventKind::ScrollDown, g, 10);
    assert_eq!((t.app.cursor(), t.app.focus(), t.app.scroll()), before);
}

// --- mouse_test_gaps: one test per surviving mutant -----------------------

#[test]
fn a_click_outside_the_hover_popup_closes_it_and_does_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    write(&root, ".fake", "");
    let a = write(&root, "a.md", "# A\n\nsee [b](b.md) here\n\nmore text\n");
    write(&root, "b.md", "# B\n");
    let script = root.join("script.json");
    let steps = serde_json::json!([
        {"expect": "initialize", "reply": {"capabilities": {"hoverProvider": true}}},
        {"expect": "textDocument/hover", "reply": {
            "contents": {"kind": "markdown", "value": "hovered text"}}}
    ]);
    std::fs::write(&script, steps.to_string()).unwrap();
    let mut config = Config::default();
    config.review.enabled = false;
    config.sidebar.show = SidebarShow::Never;
    config.lsp.server = vec![ramble::config::ServerConfig {
        kind: ramble::config::ServerKind::Generic,
        command: vec![
            env!("CARGO_BIN_EXE_fake-lsp").into(),
            script.display().to_string(),
        ],
        root_markers: vec![".fake".into()],
        position_encoding: None,
    }];
    let app = App::new(
        StartOptions {
            target: StartTarget::File(a),
            tree_root: root.clone(),
            config,
            review_cache: None,
            mdroots: ramble::app::MdrootsOptions::memory(),
        },
        SIZE,
    )
    .unwrap();
    let mut t = T {
        _dir: dir,
        root,
        app,
        clip: RecClip::default(),
        t0: Instant::now(),
    };
    let until = |t: &mut T, what: &str, f: &dyn Fn(&App) -> bool| {
        let end = Instant::now() + Duration::from_secs(5);
        while !f(&t.app) {
            assert!(Instant::now() < end, "timed out waiting for {what}");
            t.app.pump_lsp(Duration::from_millis(20));
        }
    };
    until(&mut t, "server", &|a| a.lsp_label().ends_with('●'));
    t.draw();
    let b = t.text_cell("b here");
    t.click(b, 0);
    t.keys("K");
    until(&mut t, "hover", &|a| a.hover_popup().is_some());
    t.draw();
    let hover = t.app.layout().hover.expect("hover drawn");
    t.click((hover.x + 1, hover.y + 1), 1000);
    assert!(t.app.hover_popup().is_some(), "a click inside keeps it");
    let before = t.app.cursor();
    let more = t.text_cell("more");
    t.click(more, 2000);
    assert_eq!(t.app.hover_popup(), None);
    assert_eq!(
        t.app.cursor(),
        before,
        "the closing click does nothing else"
    );
}

#[test]
fn the_deleted_file_banner_row_is_not_text() {
    let mut t = setup();
    let path = t.root.join("a.md");
    t.app
        .event(AppEvent::FsWatch(path, ramble::app::FsEvent::Removed));
    t.draw();
    assert!(t.app.layout().banner);
    t.keys("5j");
    let before = t.app.cursor();
    let text = t.app.layout().text.unwrap();
    assert_eq!(t.app.hit(text.x + 2, text.y), Hit::None);
    t.click((text.x + 2, text.y), 0);
    assert_eq!(t.app.cursor(), before);
}

#[test]
fn dragging_onto_the_top_row_scrolls_up() {
    let mut t = setup();
    let text = t.app.layout().text.unwrap();
    t.mouse(MouseEventKind::ScrollDown, (text.x + 2, text.y + 5), 0);
    t.mouse(MouseEventKind::ScrollDown, (text.x + 2, text.y + 5), 0);
    assert_eq!(t.app.scroll(), 6);
    t.draw();
    t.press((text.x + 2, text.y + 8), 1000);
    t.drag((text.x + 2, text.y + 4), 1010);
    for i in 0..3 {
        t.drag((text.x + 2, text.y), 1020 + i);
    }
    assert_eq!(t.app.scroll(), 3);
    assert_eq!(t.app.cursor().row, 3);
    assert_eq!(t.app.mode(), Mode::Visual(VisualKind::Char));
}

#[test]
fn a_text_click_moves_focus_from_the_sidebar_to_the_content() {
    let mut t = setup();
    let b = t.files_cell("b.md");
    t.click(b, 0);
    assert_eq!(t.app.focus(), Focus::Files);
    let g = t.text_cell("gamma");
    t.click(g, 1000);
    assert_eq!(t.app.focus(), Focus::Content);
}

#[test]
fn a_files_click_leaves_visual_mode() {
    let mut t = setup();
    t.keys("vl");
    assert_eq!(t.app.mode(), Mode::Visual(VisualKind::Char));
    let b = t.files_cell("b.md");
    t.click(b, 0);
    assert_eq!(t.app.mode(), Mode::Normal);
    assert_eq!(t.app.focus(), Focus::Files);
}

#[test]
fn a_click_with_a_stale_layout_keeps_the_cursor_on_screen() {
    // A resize lands between the last draw and the click: the recorded
    // text area is taller than the new viewport.
    let mut t = setup();
    let text = t.app.layout().text.unwrap();
    t.app.event(AppEvent::Resize(SIZE.0, 8));
    let low = (text.x + 1, text.y + 12);
    t.click(low, 0);
    let (row, scroll, vh) = (t.app.cursor().row, t.app.scroll(), t.app.viewport_height());
    assert_eq!(row, 12);
    assert!(
        (scroll..scroll + vh).contains(&row),
        "row {row} scroll {scroll} vh {vh}"
    );
}

#[test]
fn a_drag_after_switching_to_linewise_visual_is_ignored() {
    let mut t = setup();
    let a = t.text_cell("Alpha");
    let g = t.text_cell("gamma");
    t.press(a, 0);
    t.keys("V");
    assert_eq!(t.app.mode(), Mode::Visual(VisualKind::Line));
    t.drag(g, 10);
    t.release(g, 20);
    assert_eq!(t.app.mode(), Mode::Visual(VisualKind::Line));
    assert!(t.clip.all().is_empty());
}

// --- mouse_click_clears_pending_keys -----------------------------------------

#[test]
fn a_clue_click_keeps_the_pending_leader_sequence() {
    let mut t = setup();
    t.keys(" ");
    t.app.tick(Instant::now() + ramble::app::CLUE_DELAY * 2);
    assert!(t.app.clue_visible());
    t.draw();
    let clue = t.app.layout().clue.expect("clue drawn");
    t.click((clue.x + 2, clue.y + 1), 0);
    assert!(t.app.clue_visible(), "the clue box stays open");
    t.keys("zl");
    assert_eq!(
        t.app.mode(),
        Mode::Picker,
        "<leader>zl opened the links picker"
    );
}

#[test]
fn status_gutter_and_border_clicks_keep_the_count_and_pending_keys() {
    let mut t = setup();
    let border = t.app.layout().sidebar.unwrap();
    let status = t.app.layout().status.unwrap();
    let border = (border.right() - 1, border.y + 2);
    let status = (status.x + 40, status.y);
    // Count, then a status click, then `j`: moves 5 rows.
    t.keys("5");
    t.click(status, 0);
    t.click(border, 1000);
    t.keys("j");
    assert_eq!(t.app.cursor().row, 5);
    // Pending `g`, a border click, `g`: `gg` back to the top.
    t.keys("g");
    t.click(border, 2000);
    t.click(status, 3000);
    t.keys("g");
    assert_eq!(t.app.cursor().row, 0);
}

#[test]
fn a_text_click_drops_the_count() {
    let mut t = setup();
    let g = t.text_cell("gamma");
    t.keys("5");
    t.click(g, 0);
    let row = t.app.cursor().row;
    t.keys("j");
    assert_eq!(t.app.cursor().row, row + 1);
}

// --- mouse_scrolled_list_double_click ---------------------------------------

/// A tree of 30 files f00.md..f29.md plus a page with 30 headings and 30
/// links, sidebar in split mode.
fn many() -> T {
    let mut t = setup_with(SidebarMode::Files, |_| {});
    let mut page = String::from("# Many\n\n");
    for i in 0..30 {
        write(&t.root, &format!("f{i:02}.md"), &format!("# F{i}\n"));
        page.push_str(&format!("## H{i:02}\n\n[l{i:02}](f{i:02}.md)\n\n"));
    }
    std::fs::write(t.root.join("a.md"), page).unwrap();
    t.app
        .handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
    t.draw();
    t
}

fn opened(t: &T) -> String {
    let p = t.app.page().unwrap().path.clone().unwrap();
    p.file_name().unwrap().to_string_lossy().into_owned()
}

#[test]
fn clicks_in_a_scrolled_files_list_hit_the_drawn_row_and_it_stays_put() {
    let mut t = many();
    let files = t.app.layout().files.unwrap();
    t.click((files.pane.x + 3, files.pane.y + 1), 0);
    t.keys("G");
    t.draw();
    assert!(t.app.layout().files.unwrap().skip > 0, "list scrolled");
    let at = t.files_cell("f20.md");
    t.click(at, 1000);
    assert_eq!(t.selected_file(), Some(t.root.join("f20.md")));
    t.draw();
    assert_eq!(t.files_cell("f20.md"), at, "the list did not move");
    t.click(at, 1100);
    assert_eq!(opened(&t), "f20.md");
}

#[test]
fn clicks_in_a_scrolled_outline_hit_the_drawn_row() {
    let mut t = many();
    t.keys(" E"); // split: files over outline
    t.draw();
    let outline = t.app.layout().outline.expect("outline drawn").pane;
    t.click((outline.x + 3, outline.y + 1), 0);
    t.keys("G");
    t.draw();
    t.draw();
    assert!(t.app.layout().outline.unwrap().skip > 0, "outline scrolled");
    let at = t.find_in(outline, "H25");
    t.click(at, 1000);
    t.draw();
    assert_eq!(t.find_in(outline, "H25"), at, "the outline did not move");
    t.click(at, 1100);
    let row = t
        .app
        .outline()
        .iter()
        .find(|o| o.text == "H25")
        .unwrap()
        .row;
    assert_eq!(t.app.cursor().row, row);
}

#[test]
fn clicks_in_a_scrolled_picker_hit_the_drawn_row() {
    let mut t = many();
    t.keys(" zl");
    for _ in 0..29 {
        t.app
            .handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL));
    }
    t.draw();
    let picker = t.app.layout().picker.unwrap();
    assert!(picker.skip > 0, "picker scrolled");
    let at = t.find_in(picker.pane, "l20");
    t.click(at, 0);
    t.draw();
    assert_eq!(t.find_in(picker.pane, "l20"), at, "the picker did not move");
    t.click(at, 100);
    assert_eq!(opened(&t), "f20.md");
}

// --- mouse_outline_wheel_unfocused -------------------------------------------

/// A page with 80 headings; the sidebar shows the outline only (auto).
fn long_outline() -> T {
    let mut t = setup_with(SidebarMode::Auto, |_| {});
    let mut page = String::from("# Long\n\n");
    for i in 1..=80 {
        page.push_str(&format!("## H{i}\n\ntext {i}\n\n"));
    }
    std::fs::write(t.root.join("a.md"), page).unwrap();
    t.app
        .handle_key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL));
    t.draw();
    t
}

/// The outline rows as drawn (title excluded), trimmed.
fn outline_rows(t: &mut T) -> Vec<String> {
    let pane = t.app.layout().outline.expect("outline drawn").pane;
    let rows = t.draw();
    (pane.y + 1..pane.bottom())
        .map(|y| {
            let r: String = rows[y as usize]
                .chars()
                .skip(pane.x as usize)
                .take(pane.width as usize)
                .collect();
            r.trim_matches(|c: char| c == ' ' || c == '▎').to_string()
        })
        .collect()
}

#[test]
fn wheel_over_the_unfocused_outline_scrolls_it_and_focus_keeps_the_view() {
    let mut t = long_outline();
    assert_eq!(t.app.focus(), Focus::Content);
    let pane = t.app.layout().outline.expect("outline drawn").pane;
    let before = outline_rows(&mut t);
    assert_eq!(before[0], "Long");
    let at = (pane.x + 3, pane.y + 4);
    for i in 0..8 {
        t.mouse(MouseEventKind::ScrollDown, at, i);
    }
    let after = outline_rows(&mut t);
    assert_eq!(after[0], "H24", "8 notches x 3 rows");
    assert_eq!(t.app.focus(), Focus::Content);
    t.mouse(MouseEventKind::ScrollUp, at, 100);
    assert_eq!(outline_rows(&mut t)[0], "H21");

    // Focusing the outline selects the current heading (Long, scrolled
    // off the top), and the list scrolls the minimum to show it.
    t.app
        .handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    t.keys("h");
    assert_eq!(t.app.focus(), Focus::Outline);
    assert_eq!(t.app.outline_selected(), Some(0));
    assert_eq!(outline_rows(&mut t)[0], "Long");
}

#[test]
fn focusing_the_outline_on_a_deep_page_selects_and_shows_the_current_heading() {
    let mut t = long_outline();
    t.keys("200j");
    t.draw();
    let cur = t.app.outline().iter().position(|o| o.current).unwrap();
    assert_eq!(t.app.outline()[cur].text, "H40");
    assert!(
        !outline_rows(&mut t).contains(&"H40".to_string()),
        "not drawn yet"
    );
    t.app
        .handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    t.keys("h");
    assert_eq!(t.app.outline_selected(), Some(cur));
    let rows = outline_rows(&mut t);
    assert_eq!(
        rows.last().unwrap(),
        "H40",
        "scrolled the minimum: {rows:?}"
    );
    // Enter goes to the selected heading, not back up the page.
    let row = t.app.cursor().row;
    t.app
        .handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(t.app.cursor().row, t.app.outline()[cur].row);
    assert!(t.app.cursor().row <= row);
}

#[test]
fn focusing_the_outline_keeps_the_offset_when_the_current_heading_is_drawn() {
    let mut t = long_outline();
    let pane = t.app.layout().outline.unwrap().pane;
    for i in 0..3 {
        t.mouse(MouseEventKind::ScrollDown, (pane.x + 3, pane.y + 4), i);
    }
    assert_eq!(outline_rows(&mut t)[0], "H9");
    // Move the content into H12's section: drawn in the scrolled outline.
    let h12 = t
        .app
        .outline()
        .iter()
        .find(|o| o.text == "H12")
        .unwrap()
        .row;
    t.keys(&format!("{}G", h12 + 2));
    t.draw();
    t.app
        .handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    t.keys("h");
    assert_eq!(t.app.outline_selected(), Some(12));
    assert_eq!(outline_rows(&mut t)[0], "H9", "no jump");
}

#[test]
fn focusing_the_outline_selects_the_current_heading_when_it_is_drawn() {
    let mut t = long_outline();
    t.keys("12j"); // into H3's section
    t.draw();
    t.app
        .handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    t.keys("h");
    let cur = t.app.outline().iter().position(|o| o.current).unwrap();
    assert!(cur > 0);
    assert_eq!(t.app.outline_selected(), Some(cur));
}

/// The sidebar on the right (D9): split mode, fixed width.
fn right() -> T {
    setup_with(SidebarMode::Split, |c| {
        c.sidebar.side = ramble::config::SidebarSide::Right;
        c.sidebar.width = SidebarWidth::Fixed(20);
    })
}

#[test]
fn right_side_layout_puts_the_sidebar_at_the_right_edge() {
    let t = right();
    let l = t.app.layout();
    let side = l.sidebar.unwrap();
    assert_eq!(side.right(), SIZE.0, "flush with the right edge");
    assert_eq!(side.width, 21);
    let files = l.files.unwrap().pane;
    assert_eq!(files.x, side.x + 1, "border column on the left");
    assert_eq!(files.right(), SIZE.0);
    assert_eq!(l.gutter.or(l.text).unwrap().x, 0, "content from column 0");
    assert!(l.text.unwrap().right() <= side.x);
    // The border column hits the border; the content's last column does not.
    assert_eq!(t.app.hit(side.x, side.y + 2), Hit::Border);
    assert_ne!(t.app.hit(side.x - 1, side.y + 2), Hit::Border);
}

#[test]
fn right_side_click_a_files_row_selects_it_and_the_arrow_toggles() {
    let mut t = right();
    let files = t.app.layout().files.unwrap().pane;
    let at = t.files_cell("c.md");
    t.click(at, 0);
    assert_eq!(t.app.focus(), Focus::Files);
    assert_eq!(t.selected_file(), Some(t.root.join("c.md")));

    let docs = t.files_cell("docs");
    assert_eq!(docs.0, files.x + 3, "gutter and arrow before the name");
    t.click(docs, 2000);
    assert!(!t.app.tree().unwrap().is_expanded(&t.root.join("docs")));
    t.click((docs.0 - 2, docs.1), 4000);
    assert!(t.app.tree().unwrap().is_expanded(&t.root.join("docs")));
    t.draw();
    t.click((docs.0 - 1, docs.1), 6000);
    assert!(!t.app.tree().unwrap().is_expanded(&t.root.join("docs")));

    // A text click goes back to the content.
    let g = t.text_cell("gamma");
    t.click(g, 8000);
    assert_eq!(t.app.focus(), Focus::Content);
}

#[test]
fn right_side_wheel_scrolls_the_pane_under_the_pointer() {
    let mut t = right();
    let b = t.files_cell("b.md");
    t.click(b, 0);
    t.mouse(MouseEventKind::ScrollDown, b, 1000);
    assert_eq!(t.selected_file(), Some(t.root.join("e.md")));
    assert_eq!(t.app.scroll(), 0, "the page did not scroll");
    let a = t.text_cell("Alpha");
    t.mouse(MouseEventKind::ScrollDown, a, 1100);
    assert_eq!(t.app.scroll(), 3);
    assert_eq!(t.app.focus(), Focus::Files);
    let outline = t.app.layout().outline.unwrap().pane;
    t.mouse(
        MouseEventKind::ScrollDown,
        (outline.x + 1, outline.y + 1),
        1200,
    );
    assert_eq!(t.app.focus(), Focus::Files);
    assert_eq!(t.app.scroll(), 3);
}

#[test]
fn right_side_outline_click_selects_and_double_click_jumps() {
    let mut t = right();
    let outline = t.app.layout().outline.unwrap().pane;
    let at = t.find_in(outline, "Second");
    t.click(at, 0);
    assert_eq!(t.app.focus(), Focus::Outline);
    t.click(at, 100);
    assert_eq!(t.app.focus(), Focus::Content);
    assert_eq!(t.app.cursor().row, t.app.outline()[1].row);
}
