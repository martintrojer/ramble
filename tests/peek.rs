//! `sidebar.show = "auto"`: the peek (docs/specs/2026-10-07-sidebar-layout.md
//! D11). Hidden while you read, shown while you use it, drawn over the page
//! when there is no spare room. Temp dirs only; no LSP, no review.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ramble::app::{
    App, AppEvent, Effect, Focus, FsEvent, HelpLine, Hit, Layout, StartOptions, StartTarget,
};
use ramble::config::{Config, SidebarMode, SidebarShow, SidebarSide};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use tempfile::TempDir;

const ROWS: u16 = 20;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn keys(app: &mut App, s: &str) {
    for c in s.chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
}

fn esc(app: &mut App) {
    app.handle_key(key(KeyCode::Esc));
}

fn enter(app: &mut App) {
    app.handle_key(key(KeyCode::Enter));
}

/// `C-w` then `c`.
fn win(app: &mut App, c: char) {
    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    app.handle_key(key(KeyCode::Char(c)));
}

fn write(root: &Path, rel: &str, body: &str) -> PathBuf {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, body).unwrap();
    p
}

/// A page with a long paragraph (its layout shows the width) and headings
/// far down (an outline jump moves the cursor).
fn page_src() -> String {
    let mut s = String::from("# Title\n\n");
    s.push_str(&"word ".repeat(80));
    s.push_str("\n\n");
    for i in 0..30 {
        s.push_str(&format!("line {i}\n\n"));
    }
    s.push_str("## Far\n\nfar text\n");
    s
}

struct T {
    _dir: TempDir,
    root: PathBuf,
    app: App,
}

fn auto_config(edit: impl FnOnce(&mut Config)) -> Config {
    let mut c = Config::default();
    c.lsp.server = vec![];
    c.review.enabled = false;
    c.sidebar.show = SidebarShow::Auto;
    edit(&mut c);
    c
}

fn fixture() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    write(&root, "a.md", &page_src());
    write(&root, "b.md", "# B\n\nbee\n");
    write(&root, "docs/c.md", "# C\n");
    write(&root, "x.txt", "plain\n");
    (dir, root)
}

fn start(target: impl FnOnce(&Path) -> StartTarget, cols: u16, config: Config) -> T {
    let (dir, root) = fixture();
    let app = App::new(
        StartOptions {
            target: target(&root),
            tree_root: root.clone(),
            config,
        },
        (cols, ROWS),
    )
    .unwrap();
    T {
        _dir: dir,
        root,
        app,
    }
}

/// `show = "auto"` on `a.md`, `cols` wide.
fn on_file(cols: u16) -> T {
    on_file_with(cols, |_| {})
}

fn on_file_with(cols: u16, edit: impl FnOnce(&mut Config)) -> T {
    start(
        |r| StartTarget::File(r.join("a.md")),
        cols,
        auto_config(edit),
    )
}

/// The widest rendered row: the page's layout width.
fn page_width(app: &App) -> usize {
    app.page()
        .unwrap()
        .rendered
        .lines
        .iter()
        .map(|l| l.width())
        .max()
        .unwrap()
}

fn rendered(app: &App) -> String {
    format!("{:?}", app.page().unwrap().rendered.lines)
}

fn screen(app: &App) -> Vec<String> {
    let (cols, rows) = app.size();
    let mut term = Terminal::new(TestBackend::new(cols, rows)).unwrap();
    term.draw(|f| ramble::ui::draw(f, app)).unwrap();
    let buf = term.backend().buffer().clone();
    (0..rows)
        .map(|y| {
            let mut line = String::new();
            let mut x = 0;
            while x < cols {
                let s = buf[(x, y)].symbol();
                line.push_str(s);
                x += unicode_width::UnicodeWidthStr::width(s).max(1) as u16;
            }
            line
        })
        .collect()
}

fn click(app: &mut App, (col, row): (u16, u16), at: Instant) {
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        app.event(AppEvent::Mouse(
            MouseEvent {
                kind,
                column: col,
                row,
                modifiers: KeyModifiers::NONE,
            },
            at,
        ));
    }
}

fn hidden(app: &App) -> bool {
    !app.sidebar_visible() && app.sidebar_drawn_cols() == 0
}

#[test]
fn a_file_start_in_auto_begins_hidden_and_at_full_width() {
    let t = on_file(80);
    assert!(hidden(&t.app));
    assert_eq!(t.app.focus(), Focus::Content);
    assert_eq!(page_width(&t.app), 80, "no columns kept for the sidebar");
}

#[test]
fn always_and_never_keep_their_meaning() {
    let t = on_file_with(120, |c| c.sidebar.show = SidebarShow::Always);
    assert!(t.app.sidebar_cols() > 0);
    let mut t = on_file_with(120, |c| c.sidebar.show = SidebarShow::Never);
    assert!(hidden(&t.app));
    win(&mut t.app, 'h');
    assert!(hidden(&t.app), "never: C-w h doesn't open a peek");
    assert_eq!(t.app.focus(), Focus::Content);
}

#[test]
fn directory_start_shows_the_tree_and_the_first_page_hides_it() {
    let mut t = start(
        |r| StartTarget::Dir(r.to_path_buf()),
        80,
        auto_config(|_| {}),
    );
    assert!(
        t.app.sidebar_cols() > 0,
        "no page: the tree is drawn beside"
    );
    assert_eq!(t.app.focus(), Focus::Files);
    // Rows: docs, a.md, b.md.
    keys(&mut t.app, "j");
    enter(&mut t.app);
    assert_eq!(
        t.app.page().unwrap().path.as_deref(),
        Some(t.root.join("a.md").as_path())
    );
    assert!(hidden(&t.app));
    assert_eq!(t.app.focus(), Focus::Content);
    assert_eq!(page_width(&t.app), 80, "first render at the reading width");
}

#[test]
fn c_w_h_at_80_cols_peeks_over_the_page_without_re_layout() {
    let mut t = on_file(80);
    let before = rendered(&t.app);
    win(&mut t.app, 'h');
    assert_eq!(t.app.focus(), Focus::Outline);
    assert!(t.app.sidebar_visible());
    assert!(t.app.sidebar_overlay());
    assert_eq!(
        t.app.sidebar_cols(),
        0,
        "an overlay takes no layout columns"
    );
    assert_eq!(t.app.sidebar_drawn_cols(), 17);
    assert_eq!(rendered(&t.app), before, "the text doesn't move");
    let s = screen(&t.app);
    assert!(s[0].starts_with("Outline"), "{s:?}");
    assert!(s[0].contains('│'), "the overlay has its border: {s:?}");
    let side = t.app.layout().sidebar.unwrap();
    assert_eq!((side.x, side.width), (0, 17));
    // Back to the content hides it, still without a re-layout.
    win(&mut t.app, 'l');
    assert!(hidden(&t.app));
    assert_eq!(t.app.focus(), Focus::Content);
    assert_eq!(rendered(&t.app), before);
    assert!(screen(&t.app)[0].starts_with("Title"));
}

#[test]
fn c_w_h_at_120_and_240_cols_peeks_beside_the_page() {
    for cols in [120, 240] {
        let mut t = on_file(cols);
        let before = rendered(&t.app);
        win(&mut t.app, 'h');
        assert_eq!(t.app.focus(), Focus::Outline, "{cols}");
        assert!(!t.app.sidebar_overlay(), "{cols}: spare room");
        assert!(t.app.sidebar_cols() > 0, "{cols}");
        assert_eq!(rendered(&t.app), before, "{cols}: page keeps max_width");
        win(&mut t.app, 'l');
        assert!(hidden(&t.app), "{cols}");
    }
}

#[test]
fn beside_needs_the_width_plus_border_past_max_width_and_the_gutter() {
    // max_width 100, width 16: beside iff 17 <= cols - 100.
    let mut t = on_file(117);
    win(&mut t.app, 'h');
    assert!(!t.app.sidebar_overlay(), "117: exactly fits beside");
    let mut t = on_file(116);
    win(&mut t.app, 'h');
    assert!(t.app.sidebar_overlay(), "116: one column short");
}

#[test]
fn right_side_peeks_with_c_w_l_and_draws_the_overlay_at_the_right_edge() {
    let mut t = on_file_with(80, |c| c.sidebar.side = SidebarSide::Right);
    win(&mut t.app, 'h');
    assert!(hidden(&t.app), "C-w h points at the content on the right");
    win(&mut t.app, 'l');
    assert_eq!(t.app.focus(), Focus::Outline);
    assert!(t.app.sidebar_overlay());
    let s = screen(&t.app);
    assert!(s[0].starts_with("Title"), "{s:?}");
    let tail: String = s[0].chars().skip(63).collect();
    assert_eq!(tail, "│Outline         ", "{s:?}");
    let side = t.app.layout().sidebar.unwrap();
    assert_eq!((side.x, side.width), (63, 17));
    win(&mut t.app, 'h');
    assert!(hidden(&t.app));
}

#[test]
fn every_focus_key_into_the_sidebar_peeks_and_back_hides() {
    for (to, back) in [('w', 'w'), ('W', 'W'), ('h', 'p')] {
        let mut t = on_file(80);
        win(&mut t.app, to);
        assert_eq!(t.app.focus(), Focus::Outline, "C-w {to}");
        win(&mut t.app, back);
        assert_eq!(t.app.focus(), Focus::Content, "C-w {back}");
        assert!(hidden(&t.app), "C-w {back} hides");
    }
    // C-w p from the content into the sidebar peeks too.
    let mut t = on_file(80);
    win(&mut t.app, 'h');
    win(&mut t.app, 'l');
    win(&mut t.app, 'p');
    assert_eq!(t.app.focus(), Focus::Outline, "C-w p");
    assert!(t.app.sidebar_visible());
}

#[test]
fn c_w_j_and_k_peek_in_split_mode_only() {
    let mut t = on_file_with(80, |c| c.sidebar.default = SidebarMode::Split);
    win(&mut t.app, 'j');
    assert_eq!(t.app.focus(), Focus::Outline);
    assert!(t.app.sidebar_visible());
    win(&mut t.app, 'l');
    assert!(hidden(&t.app));
    win(&mut t.app, 'k');
    assert_eq!(t.app.focus(), Focus::Files);
    let mut t = on_file(80);
    win(&mut t.app, 'j');
    assert!(hidden(&t.app), "no pane below: no peek");
    assert_eq!(t.app.focus(), Focus::Content);
}

#[test]
fn esc_clears_the_filter_first_then_hides_the_peek() {
    let mut t = on_file(80);
    win(&mut t.app, 'h');
    keys(&mut t.app, "/far");
    enter(&mut t.app);
    esc(&mut t.app);
    assert!(t.app.sidebar_visible(), "the first Esc clears the filter");
    assert_eq!(t.app.focus(), Focus::Outline);
    esc(&mut t.app);
    assert!(hidden(&t.app), "the second hides");
    assert_eq!(t.app.focus(), Focus::Content);
}

#[test]
fn esc_on_a_pinned_sidebar_or_with_always_does_nothing_new() {
    let mut t = on_file(80);
    keys(&mut t.app, " e");
    win(&mut t.app, 'h');
    esc(&mut t.app);
    assert_eq!(t.app.focus(), Focus::Outline, "pinned");
    let mut t = on_file_with(120, |c| c.sidebar.show = SidebarShow::Always);
    win(&mut t.app, 'h');
    esc(&mut t.app);
    assert_eq!(t.app.focus(), Focus::Outline, "always");
}

#[test]
fn leader_e_pins_and_unpins() {
    let mut t = on_file(120);
    keys(&mut t.app, " e");
    assert!(t.app.sidebar_visible(), "pinned");
    assert!(t.app.sidebar_cols() > 0, "a pin lays out beside");
    assert_eq!(t.app.focus(), Focus::Content);
    win(&mut t.app, 'h');
    win(&mut t.app, 'l');
    assert!(t.app.sidebar_visible(), "a pin stays when focus returns");
    keys(&mut t.app, " e");
    assert!(hidden(&t.app), "unpinned and hidden");
    win(&mut t.app, 'h');
    win(&mut t.app, 'l');
    assert!(hidden(&t.app), "back to peeking");
}

#[test]
fn leader_e_on_an_unpinned_peek_hides_it() {
    let mut t = on_file(80);
    win(&mut t.app, 'h');
    keys(&mut t.app, " e");
    assert!(hidden(&t.app));
    assert_eq!(t.app.focus(), Focus::Content);
    win(&mut t.app, 'h');
    win(&mut t.app, 'l');
    assert!(hidden(&t.app), "still unpinned");
}

#[test]
fn leader_shift_e_peeks_and_focuses_the_first_pane() {
    let mut t = on_file(80);
    keys(&mut t.app, " E");
    assert_eq!(t.app.sidebar_mode(), SidebarMode::Files);
    assert_eq!(t.app.focus(), Focus::Files);
    assert!(t.app.sidebar_overlay(), "a peek, not a pin");
    keys(&mut t.app, " E");
    assert_eq!(t.app.sidebar_mode(), SidebarMode::Split);
    assert_eq!(t.app.focus(), Focus::Files);
    assert!(t.app.sidebar_visible(), "cycling keeps the peek");
    win(&mut t.app, 'l');
    assert!(hidden(&t.app), "unpinned: focus back hides");
}

#[test]
fn sidebar_mode_commands_peek_show_pins_hide_unpins() {
    let mut t = on_file(80);
    t.app.execute("Sidebar files");
    assert_eq!(t.app.focus(), Focus::Files);
    assert!(t.app.sidebar_overlay());
    win(&mut t.app, 'l');
    assert!(hidden(&t.app));
    t.app.execute("Sidebar show");
    assert!(t.app.sidebar_visible());
    assert_eq!(t.app.focus(), Focus::Content);
    win(&mut t.app, 'h');
    win(&mut t.app, 'l');
    assert!(t.app.sidebar_visible(), "pinned");
    t.app.execute("Sidebar hide");
    assert!(hidden(&t.app));
    t.app.execute("Sidebar off");
    assert!(
        t.app.status().starts_with(":Sidebar "),
        "{}",
        t.app.status()
    );
}

#[test]
fn sidebar_hide_closes_a_peek_and_show_pins_it_beside() {
    let mut t = on_file(120);
    win(&mut t.app, 'h');
    t.app.execute("Sidebar hide");
    assert!(hidden(&t.app));
    assert_eq!(t.app.focus(), Focus::Content);
    let mut t = on_file(100);
    win(&mut t.app, 'h');
    assert!(t.app.sidebar_overlay());
    t.app.execute("Sidebar show");
    assert!(!t.app.sidebar_overlay(), "a pin lays out beside");
    assert!(t.app.sidebar_cols() > 0);
    win(&mut t.app, 'l');
    assert!(t.app.sidebar_visible(), "pinned");
}

#[test]
fn an_outline_jump_hides_the_peek() {
    let mut t = on_file(80);
    win(&mut t.app, 'h');
    keys(&mut t.app, "G");
    enter(&mut t.app);
    assert!(hidden(&t.app));
    assert_eq!(t.app.focus(), Focus::Content);
    let row = t.app.cursor().row;
    assert!(row > 30, "jumped to Far: {row}");
}

#[test]
fn opening_from_the_tree_hides_but_folders_and_other_files_keep_the_peek() {
    let mut t = on_file_with(80, |c| {
        c.sidebar.default = SidebarMode::Files;
        c.sidebar.show_all = true;
    });
    win(&mut t.app, 'h');
    assert_eq!(t.app.focus(), Focus::Files);
    // Rows: docs, a.md, b.md, x.txt.
    keys(&mut t.app, "gg");
    enter(&mut t.app);
    assert!(t.app.sidebar_visible(), "a folder toggles; the peek stays");
    keys(&mut t.app, "G");
    enter(&mut t.app);
    assert!(
        matches!(t.app.pending_effect(), Some(Effect::Edit { .. })),
        "x.txt goes to the editor"
    );
    assert!(t.app.sidebar_visible(), "an edit keeps the peek");
    assert_eq!(t.app.focus(), Focus::Files);
    keys(&mut t.app, "k");
    enter(&mut t.app);
    assert_eq!(
        t.app.page().unwrap().path.as_deref(),
        Some(t.root.join("b.md").as_path())
    );
    assert!(hidden(&t.app));
    assert_eq!(t.app.focus(), Focus::Content);
    assert_eq!(page_width(&t.app), 80, "laid out at the full width");
}

#[test]
fn a_binary_file_from_the_tree_keeps_the_peek() {
    let mut t = on_file_with(80, |c| c.sidebar.default = SidebarMode::Files);
    std::fs::write(t.root.join("b.md"), b"\0\0\0binary").unwrap();
    win(&mut t.app, 'h');
    keys(&mut t.app, "ggjj");
    enter(&mut t.app);
    assert_eq!(t.app.status(), "looks binary");
    assert!(t.app.sidebar_visible());
    assert_eq!(t.app.focus(), Focus::Files);
}

#[test]
fn live_reload_leaves_the_peek_alone() {
    let mut t = on_file(80);
    win(&mut t.app, 'h');
    let file = t.root.join("a.md");
    write(&t.root, "a.md", &format!("{}\nmore\n", page_src()));
    t.app.event(AppEvent::FsWatch(file, FsEvent::Changed));
    assert!(t.app.sidebar_visible(), "reload");
    assert_eq!(t.app.focus(), Focus::Outline);
}

#[test]
fn overlay_clicks_hit_the_sidebar_and_a_text_click_hides_it() {
    let mut t = on_file(80);
    let t0 = Instant::now();
    win(&mut t.app, 'h');
    screen(&t.app);
    // Row 0 is the title; the items start below: Title, Far.
    assert_eq!(t.app.hit(2, 2), Hit::Outline(1));
    assert!(matches!(t.app.hit(40, 2), Hit::Text { .. }));
    click(&mut t.app, (2, 2), t0);
    assert_eq!(t.app.outline_selected(), Some(1));
    assert!(t.app.sidebar_visible(), "a sidebar click keeps the peek");
    click(&mut t.app, (2, 2), t0 + Duration::from_millis(100));
    assert!(hidden(&t.app), "a double click jumps and hides");
    assert!(t.app.cursor().row > 30);
    // A peek, then a click in the text beside the overlay.
    win(&mut t.app, 'h');
    screen(&t.app);
    click(&mut t.app, (40, 3), t0 + Duration::from_secs(5));
    assert!(hidden(&t.app));
    assert_eq!(t.app.focus(), Focus::Content);
}

#[test]
fn right_side_overlay_clicks_hit_the_sidebar() {
    let mut t = on_file_with(80, |c| c.sidebar.side = SidebarSide::Right);
    win(&mut t.app, 'l');
    screen(&t.app);
    assert_eq!(t.app.hit(66, 2), Hit::Outline(1));
    assert_eq!(t.app.hit(63, 2), Hit::Border);
    assert!(matches!(t.app.hit(10, 2), Hit::Text { .. }));
}

#[test]
fn the_sidebar_is_hit_tested_above_the_hover_popup() {
    let t = on_file(80);
    let r = Rect::new(0, 0, 20, 10);
    t.app.set_layout(Layout {
        sidebar: Some(r),
        hover: Some(Rect::new(5, 0, 30, 5)),
        text: Some(Rect::new(0, 0, 80, 19)),
        ..Layout::default()
    });
    assert_eq!(t.app.hit(6, 1), Hit::Border, "the overlay is on top");
    assert_eq!(t.app.hit(25, 1), Hit::Hover);
}

#[test]
fn narrow_terminals_overlay_and_a_pin_survives_resizes() {
    let mut t = on_file(60);
    win(&mut t.app, 'h');
    assert!(t.app.sidebar_overlay(), "narrow peek");
    win(&mut t.app, 'l');
    keys(&mut t.app, " e");
    assert!(t.app.sidebar_overlay(), "narrow pin: an overlay too");
    assert_eq!(t.app.sidebar_cols(), 0);
    t.app.event(AppEvent::Resize(70, ROWS));
    assert!(t.app.sidebar_visible(), "no narrow override to clear");
    t.app.event(AppEvent::Resize(120, ROWS));
    assert!(!t.app.sidebar_overlay(), "wide: the pin goes beside");
    assert!(t.app.sidebar_cols() > 0);
}

#[test]
fn a_tiny_overlay_is_clamped_to_the_terminal() {
    let mut t = on_file(12);
    win(&mut t.app, 'h');
    assert_eq!(t.app.focus(), Focus::Outline);
    assert_eq!(t.app.sidebar_drawn_cols(), 12);
    screen(&t.app);
}

#[test]
fn a_resize_re_decides_beside_or_overlay() {
    let mut t = on_file(120);
    win(&mut t.app, 'h');
    assert!(!t.app.sidebar_overlay());
    t.app.event(AppEvent::Resize(100, ROWS));
    assert!(t.app.sidebar_overlay());
    assert_eq!(page_width(&t.app), 100, "the overlay gives the page back");
    t.app.event(AppEvent::Resize(240, ROWS));
    assert!(!t.app.sidebar_overlay());
    assert_eq!(t.app.focus(), Focus::Outline);
}

fn help_items(app: &App) -> Vec<(String, String)> {
    app.help_lines()
        .into_iter()
        .filter_map(|l| match l {
            HelpLine::Item { keys, desc, .. } => Some((keys, desc)),
            HelpLine::Group(_) => None,
        })
        .collect()
}

#[test]
fn help_lists_the_focus_keys_while_peek_hidden_and_the_pin_text() {
    let t = on_file(80);
    let items = help_items(&t.app);
    for k in ["C-w h", "C-w w", "C-w W", "C-w p"] {
        assert!(
            items.iter().any(|(keys, _)| keys == k),
            "{k} row while a peek could open: {items:?}"
        );
    }
    let e: Vec<_> = items.iter().filter(|(k, _)| k == "Space e").collect();
    assert_eq!(e.len(), 1, "{e:?}");
    assert_eq!(e[0].1, "show or hide the sidebar (pin in auto)");
    let t = on_file_with(120, |c| c.sidebar.show = SidebarShow::Always);
    let items = help_items(&t.app);
    let e: Vec<_> = items.iter().filter(|(k, _)| k == "Space e").collect();
    assert_eq!(e.len(), 1, "{e:?}");
    assert_eq!(e[0].1, "show or hide the sidebar");
}
