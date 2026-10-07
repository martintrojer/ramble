//! Sidebar (spec § Sidebar): tree building, modes, focus, opening files,
//! outline, history, and a split-mode screen snapshot. Temp dirs only.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ramble::app::sidebar::{
    NARROW_MESSAGE, NO_PANE_ABOVE, NO_PANE_BELOW, OFF_MESSAGE, Tree, item_paths,
};
use ramble::app::{App, AppEvent, Effect, Focus, FsEvent, StartOptions, StartTarget};
use ramble::config::{Config, SidebarMode, SidebarReading};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tempfile::TempDir;

const SIZE: (u16, u16) = (80, 16);

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
    let mut off = app_on(
        &root,
        StartTarget::File(path.clone()),
        config(SidebarMode::Off),
    );
    off.set_sidebar_mode(SidebarMode::Outline);
    let want = off.page().unwrap().rendered.lines.len();
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
    assert_eq!(app.status(), NARROW_MESSAGE);
    app.open_file(&root.join("a.md")).unwrap();
    assert_eq!(app.sidebar_mode(), SidebarMode::Outline);
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
    // <leader>e from files: outline, then split, then off.
    keys(&mut app, " e e e");
    assert_eq!(app.sidebar_mode(), SidebarMode::Off);
    app.open_file(&root.join("a.md")).unwrap();
    assert_eq!(app.sidebar_mode(), SidebarMode::Off, "manual mode kept");
    app.open_file(&root.join("docs/guide.md")).unwrap();
    assert_eq!(app.sidebar_mode(), SidebarMode::Off);

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
    app.execute("Sidebar off");
    app.handle_key(ctrl('o'));
    assert_eq!(
        app.page().unwrap().path.as_deref(),
        Some(root.join("a.md").as_path())
    );
    assert_eq!(app.sidebar_mode(), SidebarMode::Split, "entry's mode");
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
    app.set_sidebar_mode(SidebarMode::Off);
    win(&mut app, 'j');
    assert_eq!(app.status(), OFF_MESSAGE);
    let mut narrow = App::new(
        StartOptions {
            target: StartTarget::File(root.join("a.md")),
            tree_root: root.clone(),
            config: config(SidebarMode::Split),
        },
        (40, 10),
    )
    .unwrap();
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

#[test]
fn leader_e_cycles_modes_and_rerenders_width() {
    let (_d, root) = fixture();
    let long = "word ".repeat(40);
    let path = write(&root, "long.md", &format!("{long}\n"));
    let mut app = app_on(&root, StartTarget::File(path), config(SidebarMode::Off));
    let rows_full = app.page().unwrap().rendered.lines.len();
    assert_eq!(app.sidebar_cols(), 0);
    keys(&mut app, " e");
    assert_eq!(app.sidebar_mode(), SidebarMode::Files);
    assert_eq!(app.sidebar_cols(), 31);
    assert!(
        app.page().unwrap().rendered.lines.len() > rows_full,
        "narrower content wraps into more rows"
    );
    keys(&mut app, " e");
    assert_eq!(app.sidebar_mode(), SidebarMode::Outline);
    keys(&mut app, " e");
    assert_eq!(app.sidebar_mode(), SidebarMode::Split);
    keys(&mut app, " e");
    assert_eq!(app.sidebar_mode(), SidebarMode::Off);
    assert_eq!(app.page().unwrap().rendered.lines.len(), rows_full);
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
    // Turning the sidebar off while it has focus gives focus back.
    app.set_sidebar_mode(SidebarMode::Off);
    assert_eq!(app.focus(), Focus::Content);
    win(&mut app, 'h');
    assert_eq!(app.focus(), Focus::Content);
    assert_eq!(app.status(), OFF_MESSAGE);
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
fn narrow_window_drops_the_sidebar() {
    let (_d, root) = fixture();
    let app = App::new(
        StartOptions {
            target: StartTarget::Dir(root.clone()),
            tree_root: root.clone(),
            config: Config::default(),
        },
        (40, 10),
    )
    .unwrap();
    assert_eq!(app.sidebar_mode(), SidebarMode::Files);
    assert_eq!(app.sidebar_cols(), 0);
    assert_eq!(app.status(), NARROW_MESSAGE);
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
    c.sidebar.width = 24;
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
