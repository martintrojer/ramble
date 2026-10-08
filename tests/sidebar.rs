//! Sidebar (spec § Sidebar): tree building, modes, focus, opening files,
//! outline, history, and a split-mode screen snapshot. Temp dirs only.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ramble::app::sidebar::{
    HIDDEN_MESSAGE, NARROW_MESSAGE, NO_PANE_ABOVE, NO_PANE_BELOW, TREE_STAYS_MESSAGE, Tree,
    item_paths,
};
use ramble::app::{App, AppEvent, Effect, Focus, FsEvent, StartOptions, StartTarget};
use ramble::config::{Config, SidebarMode, SidebarReading, SidebarWidth};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tempfile::TempDir;

const SIZE: (u16, u16) = (80, 16);
/// The fixture's rows are short: the auto width is `min_width` (16) plus
/// the border.
const MIN_COLS: u16 = 17;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn keys(app: &mut App, s: &str) {
    for c in s.chars() {
        app.handle_key(key(KeyCode::Char(c)));
    }
}

/// `C-w` then `c`.
fn win(app: &mut App, c: char) {
    app.handle_key(ctrl('w'));
    app.handle_key(key(KeyCode::Char(c)));
}

fn write(root: &Path, rel: &str, body: &str) -> PathBuf {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, body).unwrap();
    p
}

/// root/
///   .gitignore     (ignores build/)
///   .hidden.md
///   a.md, b.txt
///   build/out.md
///   docs/guide.md, docs/deep/x.md
///   img/logo.png
fn fixture() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    write(&root, ".gitignore", "build/\n");
    write(&root, ".hidden.md", "# H\n");
    write(&root, "a.md", "# A\n\nalpha\n\n## A two\n\nmore\n");
    write(&root, "b.txt", "plain\n");
    write(&root, "build/out.md", "# Out\n");
    write(&root, "docs/guide.md", "# Guide\n\nSee [a](../a.md).\n");
    write(&root, "docs/deep/x.md", "# X\n");
    write(&root, "img/logo.png", "png");
    (dir, root)
}

fn names(tree: &Tree) -> Vec<String> {
    tree.visible_items()
        .iter()
        .map(|i| format!("{}{}", "  ".repeat(i.depth), i.name))
        .collect()
}

fn app_on(root: &Path, target: StartTarget, config: Config) -> App {
    App::new(
        StartOptions {
            target,
            tree_root: root.to_path_buf(),
            config,
        },
        SIZE,
    )
    .unwrap()
}

fn config(mode: SidebarMode) -> Config {
    let mut c = Config::default();
    c.sidebar.default = mode;
    c
}

#[test]
fn tree_respects_gitignore_hides_dotfiles_and_non_markdown() {
    let (_d, root) = fixture();
    let mut tree = Tree::new(&root, false);
    // img/ is not walked yet, so it may hold markdown and is shown.
    assert_eq!(names(&tree), ["docs", "img", "a.md"]);
    tree.expand(&root.join("img"));
    assert_eq!(names(&tree), ["docs", "a.md"], "img/ has no markdown");
    tree.expand(&root.join("docs"));
    assert_eq!(names(&tree), ["docs", "  deep", "  guide.md", "a.md"]);
    tree.collapse(&root.join("docs"));
    assert_eq!(names(&tree), ["docs", "a.md"]);
}

#[test]
fn show_all_lists_other_files() {
    let (_d, root) = fixture();
    let tree = Tree::new(&root, true);
    let items = tree.visible_items();
    assert_eq!(names(&tree), ["docs", "img", "a.md", "b.txt"]);
    let txt = items.iter().find(|i| i.name == "b.txt").unwrap();
    assert!(!txt.markdown);
}

#[test]
fn tree_is_walked_lazily() {
    let (_d, root) = fixture();
    let mut tree = Tree::new(&root, true);
    assert!(
        !item_paths(&tree.visible_items()).contains(&root.join("docs/guide.md")),
        "children of a collapsed dir are not listed"
    );
    assert!(!tree.is_expanded(&root.join("docs")));
    // Root entries not hidden or ignored: a.md, b.txt, docs, img.
    assert_eq!(tree.entries_read(), 4, "only the root level is read");
    tree.expand(&root.join("docs"));
    assert_eq!(tree.entries_read(), 6, "docs/ adds deep and guide.md only");
}

#[test]
fn auto_default_per_start_target() {
    let (_d, root) = fixture();
    let auto = || config(SidebarMode::Auto);
    let dir = app_on(&root, StartTarget::Dir(root.clone()), auto());
    assert_eq!(dir.sidebar_mode(), SidebarMode::Files);
    let file = app_on(&root, StartTarget::File(root.join("a.md")), auto());
    assert_eq!(file.sidebar_mode(), SidebarMode::Outline);
    let stdin = app_on(&root, StartTarget::Stdin("# S\n".into()), auto());
    assert_eq!(stdin.sidebar_mode(), SidebarMode::Outline);
    let split = app_on(
        &root,
        StartTarget::Dir(root.clone()),
        config(SidebarMode::Split),
    );
    assert_eq!(split.sidebar_mode(), SidebarMode::Split, "config wins");
    let mut c = auto();
    c.sidebar.reading = SidebarReading::Split;
    let file = app_on(&root, StartTarget::File(root.join("a.md")), c.clone());
    assert_eq!(file.sidebar_mode(), SidebarMode::Split, "reading = split");
    let dir = app_on(&root, StartTarget::Dir(root.clone()), c);
    assert_eq!(dir.sidebar_mode(), SidebarMode::Files);
}

/// Dir start in auto, then open a.md from the tree (selected by `G`).
fn open_a_from_tree(reading: SidebarReading) -> App {
    let (_d, root) = fixture();
    let mut c = config(SidebarMode::Auto);
    c.sidebar.reading = reading;
    let mut app = app_on(&root, StartTarget::Dir(root.clone()), c);
    assert_eq!(app.sidebar_mode(), SidebarMode::Files);
    win(&mut app, 'h');
    keys(&mut app, "G");
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(
        app.page().unwrap().path.as_deref(),
        Some(root.join("a.md").as_path())
    );
    assert_eq!(app.focus(), Focus::Content);
    app
}

#[test]
fn auto_switches_to_reading_when_a_page_opens() {
    let app = open_a_from_tree(SidebarReading::Outline);
    assert_eq!(app.sidebar_mode(), SidebarMode::Outline);
    assert_eq!(app.status(), "");
    let app = open_a_from_tree(SidebarReading::Split);
    assert_eq!(app.sidebar_mode(), SidebarMode::Split);
}

#[test]
fn auto_switch_uses_the_new_width_for_the_first_layout() {
    let (_d, root) = fixture();
    let long = "word ".repeat(40);
    let path = write(&root, "long.md", &format!("{long}\n"));
    let outline = app_on(
        &root,
        StartTarget::File(path.clone()),
        config(SidebarMode::Outline),
    );
    let want = outline.page().unwrap().rendered.lines.len();
    let mut app = app_on(
        &root,
        StartTarget::Dir(root.clone()),
        config(SidebarMode::Auto),
    );
    app.open_file(&path).unwrap();
    assert_eq!(app.sidebar_mode(), SidebarMode::Outline);
    assert_eq!(app.page().unwrap().rendered.lines.len(), want);
}

#[test]
fn auto_does_not_post_narrow_status_on_each_page() {
    let (_d, root) = fixture();
    let mut app = App::new(
        StartOptions {
            target: StartTarget::Dir(root.clone()),
            tree_root: root.clone(),
            config: Config::default(),
        },
        (40, 10),
    )
    .unwrap();
    assert_eq!(app.status(), "");
    app.open_file(&root.join("a.md")).unwrap();
    assert_eq!(app.sidebar_mode(), SidebarMode::Outline);
    assert!(!app.sidebar_visible(), "auto-hidden below 80 columns");
    assert_eq!(app.status(), "");
    app.open_file(&root.join("docs/guide.md")).unwrap();
    assert_eq!(app.status(), "");
}

#[test]
fn manual_pick_stops_auto_switching() {
    let (_d, root) = fixture();
    let mut app = app_on(
        &root,
        StartTarget::Dir(root.clone()),
        config(SidebarMode::Auto),
    );
    // <leader>E from files: split.
    keys(&mut app, " E");
    assert_eq!(app.sidebar_mode(), SidebarMode::Split);
    app.open_file(&root.join("a.md")).unwrap();
    assert_eq!(app.sidebar_mode(), SidebarMode::Split, "manual mode kept");
    app.open_file(&root.join("docs/guide.md")).unwrap();
    assert_eq!(app.sidebar_mode(), SidebarMode::Split);

    let mut app = app_on(
        &root,
        StartTarget::Dir(root.clone()),
        config(SidebarMode::Auto),
    );
    app.execute("Sidebar files");
    app.open_file(&root.join("a.md")).unwrap();
    assert_eq!(app.sidebar_mode(), SidebarMode::Files, ":Sidebar is manual");
}

#[test]
fn history_restores_entry_mode_and_manual_survives_back() {
    let (_d, root) = fixture();
    let mut c = config(SidebarMode::Auto);
    c.sidebar.reading = SidebarReading::Split;
    let mut app = app_on(&root, StartTarget::Dir(root.clone()), c);
    app.open_file(&root.join("a.md")).unwrap();
    assert_eq!(app.sidebar_mode(), SidebarMode::Split, "auto");
    // From the tree, so a.md's entry (split) is pushed before the pick.
    win(&mut app, 'k');
    keys(&mut app, "gg");
    app.handle_key(key(KeyCode::Enter));
    keys(&mut app, "jj");
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(
        app.page().unwrap().path.as_deref(),
        Some(root.join("docs/guide.md").as_path())
    );
    app.execute("Sidebar files");
    app.execute("Sidebar off");
    assert!(!app.sidebar_visible());
    app.handle_key(ctrl('o'));
    assert_eq!(
        app.page().unwrap().path.as_deref(),
        Some(root.join("a.md").as_path())
    );
    assert_eq!(app.sidebar_mode(), SidebarMode::Split, "entry's mode");
    assert!(!app.sidebar_visible(), "history never shows the sidebar");
    // Still manual after C-o: a new page keeps the current mode.
    app.set_sidebar_mode(SidebarMode::Files);
    app.open_file(&root.join("docs/guide.md")).unwrap();
    assert_eq!(app.sidebar_mode(), SidebarMode::Files, "still manual");
}

#[test]
fn ctrl_w_j_k_move_between_split_panes() {
    let (_d, root) = fixture();
    let mut app = app_on(
        &root,
        StartTarget::File(root.join("a.md")),
        config(SidebarMode::Split),
    );
    win(&mut app, 'j');
    assert_eq!(app.focus(), Focus::Outline, "from content");
    win(&mut app, 'j');
    assert_eq!(app.focus(), Focus::Outline, "nothing below the outline");
    win(&mut app, 'k');
    assert_eq!(app.focus(), Focus::Files);
    win(&mut app, 'k');
    assert_eq!(app.focus(), Focus::Files, "nothing above the files");
    win(&mut app, 'j');
    assert_eq!(app.focus(), Focus::Outline, "from files");
    win(&mut app, 'l');
    win(&mut app, 'k');
    assert_eq!(app.focus(), Focus::Files, "from content");
    assert_eq!(app.status(), "");
}

#[test]
fn ctrl_w_j_k_in_single_pane_and_hidden_sidebar() {
    let (_d, root) = fixture();
    let mut app = app_on(
        &root,
        StartTarget::File(root.join("a.md")),
        config(SidebarMode::Outline),
    );
    win(&mut app, 'j');
    assert_eq!(app.status(), NO_PANE_BELOW);
    win(&mut app, 'h');
    win(&mut app, 'k');
    assert_eq!(app.status(), NO_PANE_ABOVE);
    assert_eq!(app.focus(), Focus::Outline);
    app.execute("Sidebar hide");
    win(&mut app, 'j');
    assert_eq!(app.status(), HIDDEN_MESSAGE);
    let narrow_app = |auto_hide_below, cols| {
        let mut c = config(SidebarMode::Split);
        c.sidebar.auto_hide_below = auto_hide_below;
        App::new(
            StartOptions {
                target: StartTarget::File(root.join("a.md")),
                tree_root: root.clone(),
                config: c,
            },
            (cols, 10),
        )
        .unwrap()
    };
    let mut narrow = narrow_app(80, 40);
    win(&mut narrow, 'k');
    assert_eq!(narrow.status(), HIDDEN_MESSAGE, "auto-hidden");
    assert_eq!(narrow.focus(), Focus::Content);
    // Auto-hide off: the MIN_CONTENT guard drops it instead (16 + 1 + 10
    // columns don't fit in 24).
    let mut narrow = narrow_app(0, 24);
    win(&mut narrow, 'k');
    assert_eq!(narrow.status(), NARROW_MESSAGE);
    assert_eq!(narrow.focus(), Focus::Content);
}

#[test]
fn ctrl_w_shift_w_reverses_and_p_returns() {
    let (_d, root) = fixture();
    let mut app = app_on(
        &root,
        StartTarget::File(root.join("a.md")),
        config(SidebarMode::Split),
    );
    app.handle_key(ctrl('w'));
    app.handle_key(KeyEvent::new(KeyCode::Char('W'), KeyModifiers::SHIFT));
    assert_eq!(app.focus(), Focus::Outline);
    win(&mut app, 'W');
    assert_eq!(app.focus(), Focus::Files);
    win(&mut app, 'W');
    assert_eq!(app.focus(), Focus::Content);

    win(&mut app, 'j');
    win(&mut app, 'k');
    assert_eq!(app.focus(), Focus::Files);
    win(&mut app, 'p');
    assert_eq!(app.focus(), Focus::Outline);
    win(&mut app, 'p');
    assert_eq!(app.focus(), Focus::Files);
    win(&mut app, 'l');
    win(&mut app, 'p');
    assert_eq!(app.focus(), Focus::Files, "back from content");
    // The previous pane is gone: fall back to the content.
    win(&mut app, 'j');
    app.set_sidebar_mode(SidebarMode::Outline);
    win(&mut app, 'h');
    assert_eq!(app.focus(), Focus::Outline);
    win(&mut app, 'p');
    assert_eq!(app.focus(), Focus::Content);
}

fn hidden() -> Config {
    let mut c = Config::default();
    c.sidebar.show = false;
    c
}

#[test]
fn leader_e_shows_and_hides_and_rerenders_width() {
    let (_d, root) = fixture();
    let long = "word ".repeat(40);
    let path = write(&root, "long.md", &format!("{long}\n"));
    let mut app = app_on(&root, StartTarget::File(path), hidden());
    let rows_full = app.page().unwrap().rendered.lines.len();
    assert!(!app.sidebar_visible());
    assert_eq!(app.sidebar_cols(), 0);
    keys(&mut app, " e");
    assert!(app.sidebar_visible());
    assert_eq!(app.sidebar_mode(), SidebarMode::Outline, "mode unchanged");
    assert_eq!(app.sidebar_cols(), MIN_COLS);
    assert!(
        app.page().unwrap().rendered.lines.len() > rows_full,
        "narrower content wraps into more rows"
    );
    keys(&mut app, " e");
    assert!(!app.sidebar_visible());
    assert_eq!(app.sidebar_mode(), SidebarMode::Outline);
    assert_eq!(app.page().unwrap().rendered.lines.len(), rows_full);
}

#[test]
fn leader_shift_e_cycles_outline_files_split() {
    let (_d, root) = fixture();
    let mut app = app_on(
        &root,
        StartTarget::File(root.join("a.md")),
        Config::default(),
    );
    assert_eq!(app.sidebar_mode(), SidebarMode::Outline);
    keys(&mut app, " E");
    assert_eq!(app.sidebar_mode(), SidebarMode::Files);
    assert!(app.tree().is_some(), "the tree is built for the files pane");
    keys(&mut app, " E");
    assert_eq!(app.sidebar_mode(), SidebarMode::Split);
    keys(&mut app, " E");
    assert_eq!(app.sidebar_mode(), SidebarMode::Outline, "never off");
    assert!(app.sidebar_visible());
}

#[test]
fn leader_shift_e_while_hidden_changes_mode_and_shows() {
    let (_d, root) = fixture();
    let mut app = app_on(&root, StartTarget::File(root.join("a.md")), hidden());
    assert!(!app.sidebar_visible());
    keys(&mut app, " E");
    assert_eq!(app.sidebar_mode(), SidebarMode::Files);
    assert!(app.sidebar_visible());
    assert!(app.sidebar_cols() > 0);
}

#[test]
fn sidebar_commands_set_mode_and_visibility() {
    let (_d, root) = fixture();
    let mut app = app_on(
        &root,
        StartTarget::File(root.join("a.md")),
        Config::default(),
    );
    app.execute("Sidebar hide");
    assert!(!app.sidebar_visible());
    app.execute("Sidebar show");
    assert!(app.sidebar_visible());
    app.execute("Sidebar toggle");
    assert!(!app.sidebar_visible());
    app.execute("Sidebar toggle");
    assert!(app.sidebar_visible());
    app.execute("Sidebar off");
    assert!(!app.sidebar_visible(), "off is hide");
    assert_eq!(
        app.sidebar_mode(),
        SidebarMode::Outline,
        "hide keeps the mode"
    );
    app.execute("Sidebar split");
    assert!(app.sidebar_visible(), "a mode shows it");
    assert_eq!(app.sidebar_mode(), SidebarMode::Split);
    app.execute("Sidebar left");
    assert!(app.status().starts_with(":Sidebar files|outline|split"));
}

#[test]
fn no_page_draws_the_tree_with_focus_despite_hidden_and_narrow() {
    let (_d, root) = fixture();
    let mut app = App::new(
        StartOptions {
            target: StartTarget::Dir(root.clone()),
            tree_root: root.clone(),
            config: hidden(),
        },
        (60, 10),
    )
    .unwrap();
    assert!(app.sidebar_visible());
    assert_eq!(app.sidebar_cols(), MIN_COLS);
    assert_eq!(app.focus(), Focus::Files);
    assert_eq!(
        app.tree().unwrap().selected(),
        Some(root.join("docs").as_path()),
        "first row selected"
    );
    assert_eq!(app.status(), "");
    let mut term = Terminal::new(TestBackend::new(60, 10)).unwrap();
    term.draw(|f| ramble::ui::draw(f, &app)).unwrap();
    assert!(term.backend().to_string().contains("a.md"));
    // j/k/Enter work right away.
    keys(&mut app, "jj");
    assert_eq!(
        app.tree().unwrap().selected(),
        Some(root.join("a.md").as_path())
    );
    // <leader>e cannot hide the only thing on screen.
    keys(&mut app, " e");
    assert!(app.sidebar_visible());
    assert_eq!(app.status(), TREE_STAYS_MESSAGE);
    // The first page applies steps 2-4: hidden (show = false), focus moves.
    app.handle_key(key(KeyCode::Enter));
    assert!(app.page().is_some());
    assert!(!app.sidebar_visible());
    assert_eq!(app.focus(), Focus::Content);
    // Laid out at full width: no sidebar was counted.
    assert_eq!(app.sidebar_cols(), 0);
}

#[test]
fn first_page_from_the_command_line_moves_focus_to_the_content() {
    let (_d, root) = fixture();
    let mut app = App::new(
        StartOptions {
            target: StartTarget::Dir(root.clone()),
            tree_root: root.clone(),
            config: config(SidebarMode::Files),
        },
        (100, 16),
    )
    .unwrap();
    assert_eq!(app.focus(), Focus::Files);
    app.execute(&format!("e {}", root.join("a.md").display()));
    assert!(app.page().is_some());
    assert!(app.sidebar_visible(), "files pane stays");
    assert_eq!(app.focus(), Focus::Content);
}

#[test]
fn no_page_with_outline_mode_shows_the_files_pane() {
    let (_d, root) = fixture();
    let app = app_on(
        &root,
        StartTarget::Dir(root.clone()),
        config(SidebarMode::Outline),
    );
    assert_eq!(app.sidebar_panes(), [Focus::Files]);
    assert!(app.tree().is_some());
    assert_eq!(app.focus(), Focus::Files);
}

#[test]
fn no_page_tree_fits_a_tiny_terminal() {
    let (_d, root) = fixture();
    let mut c = Config::default();
    c.sidebar.width = SidebarWidth::Fixed(30);
    let app = App::new(
        StartOptions {
            target: StartTarget::Dir(root.clone()),
            tree_root: root.clone(),
            config: c,
        },
        (20, 10),
    )
    .unwrap();
    assert_eq!(app.sidebar_cols(), 20, "clamped to the terminal");
    assert_eq!(app.focus(), Focus::Files);
    assert_eq!(app.status(), "", "no narrow message with no page");
    let mut term = Terminal::new(TestBackend::new(20, 10)).unwrap();
    term.draw(|f| ramble::ui::draw(f, &app)).unwrap();
    assert!(term.backend().to_string().contains("a.md"));
}

fn resize(app: &mut App, cols: u16) {
    app.event(AppEvent::Resize(cols, SIZE.1));
}

#[test]
fn first_page_on_a_narrow_terminal_is_laid_out_hidden() {
    let (_d, root) = fixture();
    let long = "word ".repeat(40);
    let path = write(&root, "long.md", &format!("{long}\n"));
    let mut c = Config::default();
    c.sidebar.auto_hide_below = 0;
    c.sidebar.show = false;
    let full = App::new(
        StartOptions {
            target: StartTarget::File(path.clone()),
            tree_root: root.clone(),
            config: c,
        },
        (60, 16),
    )
    .unwrap();
    let want = full.page().unwrap().rendered.lines.len();
    let mut app = App::new(
        StartOptions {
            target: StartTarget::Dir(root.clone()),
            tree_root: root.clone(),
            config: Config::default(),
        },
        (60, 16),
    )
    .unwrap();
    assert_eq!(app.sidebar_cols(), MIN_COLS, "tree drawn with no page");
    app.open_file(&path).unwrap();
    assert!(!app.sidebar_visible());
    assert_eq!(app.page().unwrap().rendered.lines.len(), want);
    assert_eq!(app.focus(), Focus::Content);
}

#[test]
fn narrow_terminal_hides_and_widening_shows_again() {
    let (_d, root) = fixture();
    let mut app = app_on(
        &root,
        StartTarget::File(root.join("a.md")),
        Config::default(),
    );
    assert!(app.sidebar_visible());
    win(&mut app, 'h');
    assert_eq!(app.focus(), Focus::Outline);
    resize(&mut app, 60);
    assert!(!app.sidebar_visible());
    assert_eq!(app.sidebar_cols(), 0);
    assert_eq!(app.focus(), Focus::Content, "focus leaves a hidden pane");
    assert_eq!(app.status(), "", "auto-hide is silent");
    resize(&mut app, 100);
    assert!(app.sidebar_visible());
    assert_eq!(app.sidebar_cols(), MIN_COLS);
}

#[test]
fn narrow_start_hides_the_sidebar() {
    let (_d, root) = fixture();
    let app = App::new(
        StartOptions {
            target: StartTarget::File(root.join("a.md")),
            tree_root: root.clone(),
            config: Config::default(),
        },
        (79, 16),
    )
    .unwrap();
    assert!(!app.sidebar_visible());
    assert_eq!(app.status(), "");
    let mut c = Config::default();
    c.sidebar.auto_hide_below = 0;
    let app = App::new(
        StartOptions {
            target: StartTarget::File(root.join("a.md")),
            tree_root: root.clone(),
            config: c,
        },
        (79, 16),
    )
    .unwrap();
    assert!(app.sidebar_visible(), "0 disables auto-hide");
}

#[test]
fn user_hide_survives_widening() {
    let (_d, root) = fixture();
    let mut app = app_on(
        &root,
        StartTarget::File(root.join("a.md")),
        Config::default(),
    );
    keys(&mut app, " e");
    assert!(!app.sidebar_visible());
    resize(&mut app, 60);
    resize(&mut app, 120);
    assert!(!app.sidebar_visible(), "the user hid it");
    // Shown, then hidden while narrow, then widened: hidden is the flag.
    keys(&mut app, " e");
    assert!(app.sidebar_visible());
    resize(&mut app, 60);
    keys(&mut app, " e");
    assert!(app.sidebar_visible(), "shown while narrow");
    keys(&mut app, " e");
    assert!(!app.sidebar_visible(), "hidden while narrow");
    resize(&mut app, 120);
    assert!(!app.sidebar_visible(), "a hide while narrow sticks");
}

#[test]
fn leader_e_while_narrow_shows_until_the_next_resize() {
    let (_d, root) = fixture();
    let mut app = app_on(
        &root,
        StartTarget::File(root.join("a.md")),
        Config::default(),
    );
    resize(&mut app, 60);
    assert!(!app.sidebar_visible());
    keys(&mut app, " e");
    assert!(app.sidebar_visible(), "the user's choice wins while narrow");
    assert_eq!(app.sidebar_cols(), MIN_COLS);
    // Internal re-layouts (a mode change) keep the override.
    app.set_sidebar_mode(SidebarMode::Split);
    assert!(app.sidebar_visible());
    // The same width again is not a resize.
    resize(&mut app, 60);
    assert!(app.sidebar_visible());
    resize(&mut app, 70);
    assert!(!app.sidebar_visible(), "a resize ends the override");
    // <leader>E while narrow shows it too.
    keys(&mut app, " E");
    assert!(app.sidebar_visible());
    resize(&mut app, 65);
    assert!(!app.sidebar_visible());
}

#[test]
fn history_restore_never_changes_visibility() {
    let (_d, root) = fixture();
    let mut app = app_on(
        &root,
        StartTarget::File(root.join("a.md")),
        Config::default(),
    );
    app.execute("Sidebar split");
    app.open_file(&root.join("docs/guide.md")).unwrap();
    app.handle_key(ctrl('o'));
    assert!(app.sidebar_visible());
    // Hidden, then back and forward: still hidden, the mode is restored.
    app.execute("Sidebar files");
    keys(&mut app, " e");
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(
        app.page().unwrap().path.as_deref(),
        Some(root.join("docs/guide.md").as_path())
    );
    assert!(!app.sidebar_visible());
    app.handle_key(ctrl('o'));
    assert_eq!(app.sidebar_mode(), SidebarMode::Files);
    assert!(!app.sidebar_visible());
}

#[test]
fn focus_moves_with_ctrl_w() {
    let (_d, root) = fixture();
    let mut app = app_on(
        &root,
        StartTarget::File(root.join("a.md")),
        config(SidebarMode::Split),
    );
    assert_eq!(app.focus(), Focus::Content);
    win(&mut app, 'w');
    assert_eq!(app.focus(), Focus::Files);
    win(&mut app, 'w');
    assert_eq!(app.focus(), Focus::Outline);
    win(&mut app, 'w');
    assert_eq!(app.focus(), Focus::Content);
    win(&mut app, 'w');
    win(&mut app, 'w');
    win(&mut app, 'l');
    assert_eq!(app.focus(), Focus::Content);
    win(&mut app, 'h');
    assert_eq!(
        app.focus(),
        Focus::Outline,
        "C-w h returns to the last pane"
    );
    // Hiding the sidebar while it has focus gives focus back.
    keys(&mut app, " e");
    assert_eq!(app.focus(), Focus::Content);
    win(&mut app, 'h');
    assert_eq!(app.focus(), Focus::Content);
    assert_eq!(app.status(), HIDDEN_MESSAGE);
}

#[test]
fn focus_returns_to_content_when_its_pane_goes() {
    let (_d, root) = fixture();
    let mut app = app_on(
        &root,
        StartTarget::File(root.join("a.md")),
        config(SidebarMode::Split),
    );
    win(&mut app, 'w');
    win(&mut app, 'w');
    assert_eq!(app.focus(), Focus::Outline);
    // The sidebar stays shown, but the outline pane is gone.
    app.set_sidebar_mode(SidebarMode::Files);
    assert!(app.sidebar_cols() > 0);
    assert_eq!(app.focus(), Focus::Content);
}

#[test]
fn min_content_guard_drops_the_sidebar_and_says_so() {
    let (_d, root) = fixture();
    let start = |width, cols| {
        let mut c = Config::default();
        c.sidebar.auto_hide_below = 0;
        c.sidebar.width = width;
        App::new(
            StartOptions {
                target: StartTarget::File(root.join("a.md")),
                tree_root: root.clone(),
                config: c,
            },
            (cols, 10),
        )
        .unwrap()
    };
    // The auto width fits: 16 + border, the page keeps 23.
    let app = start(SidebarWidth::Auto, 40);
    assert_eq!(app.sidebar_cols(), MIN_COLS);
    assert_eq!(app.status(), "");
    // A fixed width is cut so the page keeps MIN_CONTENT (10).
    let app = start(SidebarWidth::Fixed(30), 40);
    assert_eq!(app.sidebar_cols(), 30, "29 columns + border");
    // Too narrow for min_width plus MIN_CONTENT: dropped, said so.
    let app = start(SidebarWidth::Auto, 24);
    assert!(app.sidebar_visible());
    assert_eq!(app.sidebar_cols(), 0);
    assert_eq!(app.status(), NARROW_MESSAGE);
    let app = start(SidebarWidth::Fixed(30), 24);
    assert_eq!(app.sidebar_cols(), 0);
}

#[test]
fn enter_opens_file_and_pushes_history_with_sidebar_mode() {
    let (_d, root) = fixture();
    let mut app = app_on(
        &root,
        StartTarget::File(root.join("a.md")),
        config(SidebarMode::Files),
    );
    win(&mut app, 'h');
    assert_eq!(app.focus(), Focus::Files);
    assert_eq!(
        app.tree().unwrap().selected(),
        Some(root.join("a.md").as_path())
    );
    keys(&mut app, "gg");
    assert_eq!(
        app.tree().unwrap().selected(),
        Some(root.join("docs").as_path())
    );
    app.handle_key(key(KeyCode::Enter));
    assert!(app.tree().unwrap().is_expanded(&root.join("docs")));
    keys(&mut app, "jj");
    assert_eq!(
        app.tree().unwrap().selected(),
        Some(root.join("docs/guide.md").as_path())
    );
    app.handle_key(key(KeyCode::Enter));
    let page = app.page().unwrap();
    assert_eq!(
        page.path.as_deref(),
        Some(root.join("docs/guide.md").as_path())
    );
    assert_eq!(app.history_depth(), 1);
    assert_eq!(app.focus(), Focus::Content);

    // History restores the sidebar mode the page was left in.
    app.set_sidebar_mode(SidebarMode::Outline);
    app.handle_key(ctrl('o'));
    assert_eq!(
        app.page().unwrap().path.as_deref(),
        Some(root.join("a.md").as_path())
    );
    assert_eq!(app.sidebar_mode(), SidebarMode::Files);
    app.handle_key(key(KeyCode::Tab));
    assert_eq!(app.sidebar_mode(), SidebarMode::Outline);
}

#[test]
fn non_markdown_from_tree_goes_to_editor() {
    let (_d, root) = fixture();
    let mut c = config(SidebarMode::Files);
    c.sidebar.show_all = true;
    let mut app = app_on(&root, StartTarget::Dir(root.clone()), c);
    win(&mut app, 'h');
    keys(&mut app, "G");
    assert_eq!(
        app.tree().unwrap().selected(),
        Some(root.join("b.txt").as_path())
    );
    keys(&mut app, "o");
    assert_eq!(
        app.pending_effect(),
        Some(&Effect::Edit {
            path: root.join("b.txt"),
            line: None
        })
    );
    assert!(app.page().is_none());
}

#[test]
fn h_l_collapse_and_expand() {
    let (_d, root) = fixture();
    let mut app = app_on(
        &root,
        StartTarget::Dir(root.clone()),
        config(SidebarMode::Files),
    );
    win(&mut app, 'h');
    keys(&mut app, "l");
    assert!(app.tree().unwrap().is_expanded(&root.join("docs")));
    keys(&mut app, "jh");
    // On a child: h collapses the parent and selects it.
    assert!(!app.tree().unwrap().is_expanded(&root.join("docs")));
    assert_eq!(
        app.tree().unwrap().selected(),
        Some(root.join("docs").as_path())
    );
}

#[test]
fn tree_follows_current_file() {
    let (_d, root) = fixture();
    let mut app = app_on(
        &root,
        StartTarget::File(root.join("docs/deep/x.md")),
        config(SidebarMode::Files),
    );
    let tree = app.tree().unwrap();
    assert!(tree.is_expanded(&root.join("docs")));
    assert!(tree.is_expanded(&root.join("docs/deep")));
    assert!(
        !tree.is_expanded(&root.join("img")),
        "only ancestors walked"
    );
    assert_eq!(tree.selected(), Some(root.join("docs/deep/x.md").as_path()));
    app.open_file(&root.join("a.md")).unwrap();
    assert_eq!(
        app.tree().unwrap().selected(),
        Some(root.join("a.md").as_path())
    );
}

#[test]
fn file_outside_root_has_no_selection_and_titles_path() {
    let (_d, root) = fixture();
    let other = tempfile::tempdir().unwrap();
    let out = write(&other.path().canonicalize().unwrap(), "o.md", "# O\n");
    let app = app_on(
        &root.join("docs"),
        StartTarget::File(out.clone()),
        config(SidebarMode::Files),
    );
    let tree = app.tree().unwrap();
    assert_eq!(tree.root(), root.join("docs"));
    assert_eq!(tree.selected(), None);
    assert_eq!(app.sidebar_title(), out.display().to_string());
}

#[test]
fn outline_marker_follows_cursor_and_enter_jumps() {
    let (_d, root) = fixture();
    let mut app = app_on(
        &root,
        StartTarget::File(root.join("a.md")),
        config(SidebarMode::Outline),
    );
    let current = |app: &App| {
        app.outline()
            .iter()
            .position(|o| o.current)
            .map(|i| app.outline()[i].text.clone())
    };
    assert_eq!(current(&app).as_deref(), Some("A"));
    keys(&mut app, "G");
    assert_eq!(current(&app).as_deref(), Some("A two"));
    keys(&mut app, "gg");
    assert_eq!(current(&app).as_deref(), Some("A"));

    win(&mut app, 'h');
    assert_eq!(app.focus(), Focus::Outline);
    keys(&mut app, "j");
    app.handle_key(key(KeyCode::Enter));
    let two = app.outline()[1].row;
    assert_eq!(app.cursor().row, two);
    assert_eq!(app.focus(), Focus::Content);
    assert_eq!(current(&app).as_deref(), Some("A two"));
}

#[test]
fn slash_filters_focused_pane_and_esc_clears() {
    let (_d, root) = fixture();
    let mut app = app_on(
        &root,
        StartTarget::Dir(root.clone()),
        config(SidebarMode::Files),
    );
    win(&mut app, 'h');
    keys(&mut app, "/GUI");
    assert_eq!(app.filter_prompt().as_deref(), Some("/GUI"));
    // Only walked dirs are searched: guide.md is not visible yet.
    assert!(app.tree().unwrap().visible_items().is_empty());
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.filter_prompt(), None);
    assert_eq!(names(app.tree().unwrap()), ["docs", "img", "a.md"]);
    keys(&mut app, "l/a.");
    app.handle_key(key(KeyCode::Enter));
    assert_eq!(app.filter_prompt(), None);
    assert_eq!(app.tree().unwrap().filter(), Some("a."));
    assert_eq!(names(app.tree().unwrap()), ["a.md"]);
    assert_eq!(
        app.tree().unwrap().selected(),
        Some(root.join("a.md").as_path())
    );
    app.handle_key(key(KeyCode::Esc));
    assert_eq!(app.tree().unwrap().filter(), None);

    // Content focus: `/` is still page search.
    win(&mut app, 'l');
    keys(&mut app, "/");
    assert!(app.search_prompt().is_some());
}

#[test]
fn snapshot_split_mode() {
    let (_d, root) = fixture();
    let mut c = config(SidebarMode::Split);
    c.sidebar.width = SidebarWidth::Fixed(24);
    let mut app = app_on(&root, StartTarget::File(root.join("docs/guide.md")), c);
    win(&mut app, 'h');
    let mut term = Terminal::new(TestBackend::new(SIZE.0, SIZE.1)).unwrap();
    term.draw(|f| ramble::ui::draw(f, &app)).unwrap();
    insta::assert_snapshot!(term.backend().to_string());
}

#[test]
fn ctrl_l_refreshes_the_tree_keeping_expansion_and_selection() {
    let (_d, root) = fixture();
    let mut app = app_on(
        &root,
        StartTarget::File(root.join("a.md")),
        config(SidebarMode::Files),
    );
    win(&mut app, 'h');
    keys(&mut app, "ggl"); // expand docs
    keys(&mut app, "j"); // docs/deep
    let deep = root.join("docs/deep");
    assert_eq!(app.tree().unwrap().selected(), Some(deep.as_path()));
    write(&root, "docs/new.md", "# N\n");
    std::fs::remove_file(root.join("docs/guide.md")).unwrap();
    app.handle_key(ctrl('l'));
    let tree = app.tree().unwrap();
    assert_eq!(names(tree), ["docs", "  deep", "  new.md", "img", "a.md"]);
    assert!(tree.is_expanded(&root.join("docs")));
    assert_eq!(tree.selected(), Some(deep.as_path()), "selection kept");
    assert_eq!(app.focus(), Focus::Files);
    assert_eq!(app.status(), "Refreshed");
    // A vanished selection moves to its nearest surviving ancestor.
    std::fs::remove_dir_all(&deep).unwrap();
    app.execute("Refresh");
    let tree = app.tree().unwrap();
    assert_eq!(tree.selected(), Some(root.join("docs").as_path()));
    assert!(!tree.is_expanded(&deep));
}

#[test]
fn refresh_rewalks_collapsed_but_walked_dirs() {
    let (_d, root) = fixture();
    write(&root, "notes/n.md", "# N\n");
    let mut tree = Tree::new(&root, false);
    tree.expand(&root.join("notes"));
    tree.collapse(&root.join("notes"));
    assert_eq!(names(&tree), ["docs", "img", "notes", "a.md"]);
    // An unwalked dir counts as holding markdown, so only a re-walk of the
    // collapsed dir can find it empty and hide it.
    std::fs::remove_file(root.join("notes/n.md")).unwrap();
    tree.refresh();
    assert_eq!(names(&tree), ["docs", "img", "a.md"]);
    assert!(!tree.is_expanded(&root.join("notes")));
}

#[test]
fn ctrl_l_keeps_a_collapsed_ancestor_of_the_open_file_collapsed() {
    let (_d, root) = fixture();
    let mut app = app_on(
        &root,
        StartTarget::File(root.join("docs/deep/x.md")),
        config(SidebarMode::Files),
    );
    win(&mut app, 'h');
    keys(&mut app, "ggh");
    let docs = root.join("docs");
    assert!(!app.tree().unwrap().is_expanded(&docs));
    app.handle_key(ctrl('l'));
    let tree = app.tree().unwrap();
    assert!(!tree.is_expanded(&docs), "refresh re-expanded docs/");
    assert_eq!(tree.selected(), Some(docs.as_path()));
}

#[test]
fn watcher_reload_keeps_a_collapsed_ancestor_of_the_open_file_collapsed() {
    let (_d, root) = fixture();
    let file = root.join("docs/deep/x.md");
    let mut app = app_on(
        &root,
        StartTarget::File(file.clone()),
        config(SidebarMode::Files),
    );
    win(&mut app, 'h');
    keys(&mut app, "ggjh");
    let deep = root.join("docs/deep");
    assert!(!app.tree().unwrap().is_expanded(&deep));
    write(&root, "docs/deep/x.md", "# X changed\n");
    app.event(AppEvent::FsWatch(file, FsEvent::Changed));
    let tree = app.tree().unwrap();
    assert!(!tree.is_expanded(&deep), "watcher reload re-expanded deep/");
    assert_eq!(tree.selected(), Some(deep.as_path()));
}

#[test]
fn ctrl_l_with_no_page_refreshes_the_tree_only() {
    let (_d, root) = fixture();
    let mut app = app_on(
        &root,
        StartTarget::Dir(root.clone()),
        config(SidebarMode::Files),
    );
    write(&root, "c.md", "# C\n");
    app.handle_key(ctrl('l'));
    assert_eq!(names(app.tree().unwrap()), ["docs", "img", "a.md", "c.md"]);
    assert_eq!(app.status(), "Refreshed tree");
    assert!(app.take_clear_request());
}

fn screen(app: &App, cols: u16, rows: u16) -> Vec<String> {
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

/// The sidebar part of a screen row: up to the border.
fn side(row: &str) -> &str {
    row.split('│').next().unwrap()
}

fn sized(root: &Path, target: StartTarget, config: Config, size: (u16, u16)) -> App {
    App::new(
        StartOptions {
            target,
            tree_root: root.to_path_buf(),
            config,
        },
        size,
    )
    .unwrap()
}

#[test]
fn short_outline_gives_min_width_and_long_rows_widen_it() {
    let (_d, root) = fixture();
    let app = app_on(
        &root,
        StartTarget::File(root.join("a.md")),
        Config::default(),
    );
    assert_eq!(app.sidebar_cols(), MIN_COLS);
    // A 26-column heading needs 1 (marker gutter) + 26 + 1 padding = 28.
    let path = write(&root, "w.md", "# A heading of 26 columns!!!\n");
    let app = sized(&root, StartTarget::File(path), Config::default(), (120, 16));
    assert_eq!(app.sidebar_cols(), 28 + 1);
}

#[test]
fn wide_terminal_grows_the_sidebar_into_room_the_page_cannot_use() {
    let (_d, root) = fixture();
    let long = format!("# {}\n", "x".repeat(60));
    let path = write(&root, "w.md", &long);
    let app = sized(
        &root,
        StartTarget::File(path.clone()),
        Config::default(),
        (110, 16),
    );
    assert_eq!(app.sidebar_cols(), 38 + 1, "35% of 110");
    let app = sized(&root, StartTarget::File(path), Config::default(), (160, 16));
    assert_eq!(
        app.sidebar_cols(),
        48 + 1,
        "max_width: 160 - 49 leaves 111 > 100"
    );
    assert_eq!(app.page().unwrap().rendered.lines[0].width(), 60);
}

#[test]
fn long_heading_ends_in_an_ellipsis_with_the_current_marker_in_the_gutter() {
    let (_d, root) = fixture();
    let path = write(
        &root,
        "h.md",
        "# Installing on macOS with Homebrew and friends\n\ntext\n",
    );
    let app = sized(&root, StartTarget::File(path), Config::default(), (80, 10));
    let w = app.sidebar_cols() - 1;
    assert_eq!(w, 28, "35% of 80");
    let rows = screen(&app, 80, 10);
    let row = side(&rows[1]);
    assert!(row.starts_with("▎Installing on macOS with "), "{row:?}");
    assert!(row.ends_with('…'), "{row:?}");
    assert_eq!(unicode_width::UnicodeWidthStr::width(row), w as usize);
}

#[test]
fn long_file_name_keeps_its_extension_and_deep_trees_cap_the_indent() {
    let (_d, root) = fixture();
    let deep = "d1/d2/d3/d4/d5/d6/d7/d8/d9";
    write(&root, &format!("{deep}/n.md"), "# N\n");
    let long = write(
        &root,
        "2026-10-07-a-really-long-ramble-design-spec.md",
        "# L\n",
    );
    let mut c = config(SidebarMode::Files);
    c.sidebar.width = SidebarWidth::Fixed(20);
    let mut app = sized(&root, StartTarget::File(long), c, (80, 30));
    app.open_file(&root.join(deep).join("n.md")).unwrap();
    let rows: Vec<String> = screen(&app, 80, 30)
        .iter()
        .map(|r| side(r).trim_end().to_string())
        .collect();
    let name = rows.iter().find(|r| r.contains("2026")).unwrap();
    assert!(name.ends_with("spec.md"), "{name:?}");
    assert!(name.contains('…'), "{name:?}");
    // Depth 9 would indent 18 columns; the name keeps 8 instead:
    // 20 - 1 (gutter) - 2 (icon) - 8 = 9, so 8 columns of indent at most.
    // The open file is marked in the gutter (D8).
    let n = rows.iter().find(|r| r.ends_with("n.md")).unwrap();
    assert_eq!(n.as_str(), format!("▎{}  n.md", " ".repeat(8)));
    let col = |r: &str| r.chars().position(|c| c == '▾');
    let d9 = rows.iter().find(|r| r.trim_start() == "▾ d9").unwrap();
    assert_eq!(col(d9), Some(1 + 8), "{d9:?}");
    let d3 = rows.iter().find(|r| r.trim_start() == "▾ d3").unwrap();
    assert_eq!(col(d3), Some(1 + 4), "shallow rows indent as before");
    assert!(!name.starts_with('▎'), "only the open file is marked");
}

#[test]
fn review_mark_survives_a_long_name() {
    let (_d, root) = fixture();
    let long = write(&root, "a-very-long-file-name-for-review.md", "# L\n");
    let mut c = config(SidebarMode::Files);
    c.sidebar.width = SidebarWidth::Fixed(20);
    let mut app = sized(&root, StartTarget::File(root.join("a.md")), c, (80, 16));
    let mut m = ramble::review::Markers::default();
    m.files.insert(
        ramble::review::canonical(&long),
        ramble::review::FileMarks {
            count: 12,
            lines: vec![(1, 1)],
        },
    );
    m.files.insert(
        ramble::review::canonical(&root.join("a.md")),
        ramble::review::FileMarks {
            count: 3,
            lines: vec![(1, 1)],
        },
    );
    app.event(AppEvent::Review(m));
    let rows: Vec<String> = screen(&app, 80, 16)[..15]
        .iter()
        .map(|r| side(r).to_string())
        .collect();
    let row = rows.iter().find(|r| r.contains("a-v")).unwrap();
    assert!(row.trim_end().ends_with(".md ● 12"), "{row:?}");
    assert!(row.contains('…'), "{row:?}");
    let a = rows.iter().find(|r| r.contains(" a.md")).unwrap();
    assert!(a.trim_end().ends_with("a.md ● 3"), "{a:?}");
    assert!(a.starts_with('▎'), "the open file is marked: {a:?}");
}

#[test]
fn expanding_widens_at_once_and_narrows_at_the_next_page() {
    let (_d, root) = fixture();
    write(
        &root,
        "docs/a-rather-long-name-inside-docs-folder.md",
        "# R\n",
    );
    let mut app = sized(
        &root,
        StartTarget::File(root.join("a.md")),
        config(SidebarMode::Files),
        (160, 16),
    );
    assert_eq!(app.sidebar_cols(), MIN_COLS);
    win(&mut app, 'h');
    keys(&mut app, "ggl"); // expand docs
    // 1 gutter + 2 indent + 2 icon + 40 name + 1 padding, + border.
    assert_eq!(app.sidebar_cols(), 46 + 1, "widened right away");
    keys(&mut app, "h"); // collapse
    assert_eq!(app.sidebar_cols(), 47, "never narrows within a page");
    app.open_file(&root.join("docs/guide.md")).unwrap();
    // guide.md is revealed: docs/ stays expanded, so the long row counts.
    assert_eq!(app.sidebar_cols(), 47);
    win(&mut app, 'h');
    keys(&mut app, "ggh"); // collapse docs again
    app.open_file(&root.join("a.md")).unwrap();
    assert_eq!(app.sidebar_cols(), MIN_COLS, "narrows at the page change");
}

#[test]
fn page_layout_follows_width_changes() {
    let (_d, root) = fixture();
    let words = "word ".repeat(60);
    let path = write(&root, "p.md", &format!("# T\n\n{words}\n"));
    write(
        &root,
        "docs/a-rather-long-name-inside-docs-folder.md",
        "# R\n",
    );
    let mut app = sized(
        &root,
        StartTarget::File(path),
        config(SidebarMode::Files),
        (100, 16),
    );
    let before = app.page().unwrap().rendered.lines.len();
    win(&mut app, 'h');
    keys(&mut app, "ggl");
    assert!(app.sidebar_cols() > MIN_COLS);
    let after = app.page().unwrap().rendered.lines.len();
    assert!(after > before, "re-laid out narrower: {before} -> {after}");
}

#[test]
fn colon_opens_the_command_line_from_the_tree() {
    let (_d, root) = fixture();
    let mut app = app_on(&root, StartTarget::Dir(root.clone()), Config::default());
    assert_eq!(app.focus(), Focus::Files);
    keys(&mut app, ":");
    assert_eq!(app.mode(), ramble::app::Mode::Command);
}

// Each width hook (D5) has a test: delete the hook and its test fails.

const LONG_NAME: &str = "2026-10-07-a-really-long-ramble-design-spec.md";

#[test]
fn terminal_resize_refits_the_width() {
    let (_d, root) = fixture();
    let path = write(&root, "w.md", &format!("# {}\n", "x".repeat(60)));
    let mut app = sized(&root, StartTarget::File(path), Config::default(), (80, 16));
    assert_eq!(app.sidebar_cols(), 28 + 1, "35% of 80");
    app.event(AppEvent::Resize(200, 16));
    assert_eq!(app.sidebar_cols(), 48 + 1, "max_width");
}

#[test]
fn mode_change_refits_the_width() {
    let (_d, root) = fixture();
    write(&root, LONG_NAME, "# L\n");
    let mut app = sized(
        &root,
        StartTarget::File(root.join("a.md")),
        Config::default(),
        (160, 16),
    );
    assert_eq!(app.sidebar_cols(), MIN_COLS);
    app.set_sidebar_mode(SidebarMode::Files);
    assert!(app.sidebar_cols() > MIN_COLS, "{}", app.sidebar_cols());
}

#[test]
fn show_refits_the_width() {
    let (_d, root) = fixture();
    write(&root, LONG_NAME, "# L\n");
    let mut c = hidden();
    c.sidebar.auto_hide_below = 0;
    let mut app = sized(&root, StartTarget::File(root.join("a.md")), c, (160, 16));
    assert_eq!(app.sidebar_cols(), 0);
    // `<leader>E` sets the mode while hidden, then shows: only the show
    // refits for the files pane.
    keys(&mut app, " E");
    assert_eq!(app.sidebar_mode(), SidebarMode::Files);
    assert!(app.sidebar_cols() > MIN_COLS, "{}", app.sidebar_cols());
}

#[test]
fn ctrl_l_refits_the_width() {
    let (_d, root) = fixture();
    let mut app = sized(
        &root,
        StartTarget::Dir(root.clone()),
        config(SidebarMode::Files),
        (160, 16),
    );
    assert_eq!(app.sidebar_cols(), MIN_COLS);
    write(&root, LONG_NAME, "# L\n");
    app.handle_key(ctrl('l'));
    assert!(app.sidebar_cols() > MIN_COLS, "{}", app.sidebar_cols());
}

#[test]
fn review_marks_refit_the_width() {
    let (_d, root) = fixture();
    let mut app = sized(
        &root,
        StartTarget::File(root.join("a.md")),
        config(SidebarMode::Files),
        (160, 16),
    );
    assert_eq!(app.sidebar_cols(), MIN_COLS);
    let mut m = ramble::review::Markers::default();
    m.files.insert(
        ramble::review::canonical(&root.join("a.md")),
        ramble::review::FileMarks {
            count: 123_456_789,
            lines: vec![(1, 1)],
        },
    );
    app.event(AppEvent::Review(m));
    assert!(app.sidebar_cols() > MIN_COLS, "{}", app.sidebar_cols());
}

#[test]
fn filter_refits_the_width() {
    let (_d, root) = fixture();
    let mut app = sized(
        &root,
        StartTarget::File(root.join("a.md")),
        config(SidebarMode::Files),
        (160, 16),
    );
    assert_eq!(app.sidebar_cols(), MIN_COLS);
    win(&mut app, 'h');
    // The title shows the filter: `Files /a-really-long-filter`.
    keys(&mut app, "/a-really-long-filter");
    app.handle_key(key(KeyCode::Enter));
    assert!(app.sidebar_cols() > MIN_COLS, "{}", app.sidebar_cols());
}
