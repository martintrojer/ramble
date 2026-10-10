//! `C-h/j/k/l` (`[keys] pane_nav`): the moves of `C-w h/j/k/l`, and the
//! tmux hand-off at the edge. A recording runner stands in for tmux and an
//! injected environment for `$TMUX`; no real tmux is run.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ramble::app::sidebar::NO_PANE_BELOW;
use ramble::app::{App, Exit, Focus, LaunchCommand, Mode, StartOptions, StartTarget};
use ramble::config::{Config, SidebarMode, SidebarShow, SidebarSide, SidebarWidth};
use tempfile::TempDir;

type Calls = Rc<RefCell<Vec<Vec<String>>>>;

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn keys(app: &mut App, s: &str) {
    for c in s.chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
}

fn write(root: &Path, rel: &str, body: &str) -> PathBuf {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, body).unwrap();
    p
}

fn config(mode: SidebarMode, side: SidebarSide) -> Config {
    let mut c = Config::default();
    c.lsp.server = vec![];
    c.review.enabled = false;
    c.sidebar.default = mode;
    c.sidebar.side = side;
    c.sidebar.width = SidebarWidth::Fixed(20);
    c
}

struct T {
    _dir: TempDir,
    app: App,
    calls: Calls,
}

impl T {
    /// The tmux commands run so far, then forget them.
    fn tmux(&self) -> Vec<String> {
        self.calls
            .borrow_mut()
            .drain(..)
            .map(|a| a.join(" "))
            .collect()
    }
}

/// `a.md` open, 100x20, with `$TMUX` set when `tmux` and a recording pane
/// runner.
fn start(config: Config, tmux: bool) -> T {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let path = write(&root, "a.md", "# A\n\nalpha\n\n## Two\n\nbeta\n");
    write(&root, "b.md", "# B\n");
    let calls: Calls = Rc::default();
    let rec = calls.clone();
    let app = App::new(
        StartOptions {
            target: StartTarget::File(path),
            tree_root: root,
            config,
            review_cache: None,
            mdroots: ramble::app::MdrootsOptions::memory(),
        },
        (100, 20),
    )
    .unwrap()
    .with_env(move |k| (tmux && k == "TMUX").then(|| "/tmp/tmux-1/default,1,0".into()))
    .with_pane_runner(move |cmd: &LaunchCommand| {
        rec.borrow_mut().push(cmd.argv.clone());
        Ok(Exit::Code(0))
    });
    T {
        _dir: dir,
        app,
        calls,
    }
}

#[test]
fn ctrl_h_l_move_between_a_left_sidebar_and_the_content() {
    let mut t = start(config(SidebarMode::Files, SidebarSide::Left), true);
    assert_eq!(t.app.focus(), Focus::Content);
    t.app.handle_key(ctrl('h'));
    assert_eq!(t.app.focus(), Focus::Files, "C-h to the sidebar");
    t.app.handle_key(ctrl('l'));
    assert_eq!(t.app.focus(), Focus::Content, "C-l back to the content");
    assert!(t.tmux().is_empty(), "moves inside ramble don't reach tmux");
}

#[test]
fn ctrl_l_h_move_between_the_content_and_a_right_sidebar() {
    let mut t = start(config(SidebarMode::Split, SidebarSide::Right), true);
    t.app.handle_key(ctrl('l'));
    assert_eq!(
        t.app.focus(),
        Focus::Files,
        "C-l to the sidebar on the right"
    );
    t.app.handle_key(ctrl('j'));
    assert_eq!(t.app.focus(), Focus::Outline);
    t.app.handle_key(ctrl('h'));
    assert_eq!(t.app.focus(), Focus::Content, "C-h back to the content");
    t.app.handle_key(ctrl('l'));
    assert_eq!(t.app.focus(), Focus::Outline, "returns to the last pane");
    assert!(t.tmux().is_empty());
    // The edges are now the other way round.
    t.app.handle_key(ctrl('l'));
    assert_eq!(t.tmux(), ["tmux select-pane -R"], "right of the sidebar");
    t.app.handle_key(ctrl('h'));
    t.app.handle_key(ctrl('h'));
    assert_eq!(t.app.focus(), Focus::Content);
    assert_eq!(t.tmux(), ["tmux select-pane -L"], "left of the content");
}

#[test]
fn ctrl_j_k_move_between_split_panes_from_anywhere() {
    let mut t = start(config(SidebarMode::Split, SidebarSide::Left), true);
    t.app.handle_key(ctrl('j'));
    assert_eq!(t.app.focus(), Focus::Outline, "from the content");
    t.app.handle_key(ctrl('k'));
    assert_eq!(t.app.focus(), Focus::Files);
    t.app.handle_key(ctrl('j'));
    assert_eq!(t.app.focus(), Focus::Outline);
    t.app.handle_key(ctrl('l'));
    t.app.handle_key(ctrl('k'));
    assert_eq!(t.app.focus(), Focus::Files, "from the content");
    assert!(t.tmux().is_empty());
    assert_eq!(t.app.status(), "");
}

#[test]
fn edges_hand_off_to_tmux_with_the_matching_flag() {
    let mut t = start(config(SidebarMode::Split, SidebarSide::Left), true);
    t.app.handle_key(ctrl('l'));
    assert_eq!(t.tmux(), ["tmux select-pane -R"], "right of the content");
    t.app.handle_key(ctrl('h'));
    t.app.handle_key(ctrl('h'));
    assert_eq!(t.app.focus(), Focus::Files);
    assert_eq!(t.tmux(), ["tmux select-pane -L"], "left of the sidebar");
    t.app.handle_key(ctrl('k'));
    assert_eq!(t.tmux(), ["tmux select-pane -U"], "above the files");
    t.app.handle_key(ctrl('j'));
    t.app.handle_key(ctrl('j'));
    assert_eq!(t.app.focus(), Focus::Outline);
    assert_eq!(t.tmux(), ["tmux select-pane -D"], "below the outline");
    assert_eq!(t.app.status(), "", "a hand-off posts no status");
}

#[test]
fn without_stacked_panes_j_and_k_hand_off_from_the_content() {
    let mut t = start(config(SidebarMode::Outline, SidebarSide::Left), true);
    t.app.handle_key(ctrl('j'));
    t.app.handle_key(ctrl('k'));
    assert_eq!(t.tmux(), ["tmux select-pane -D", "tmux select-pane -U"]);
    assert_eq!(t.app.focus(), Focus::Content);
}

#[test]
fn a_hidden_sidebar_is_an_edge() {
    let mut c = config(SidebarMode::Files, SidebarSide::Left);
    c.sidebar.show = SidebarShow::Never;
    let mut t = start(c, true);
    t.app.handle_key(ctrl('h'));
    assert_eq!(t.app.focus(), Focus::Content);
    assert_eq!(t.tmux(), ["tmux select-pane -L"]);
}

#[test]
fn an_auto_sidebar_peeks_instead_of_handing_off() {
    let mut c = config(SidebarMode::Files, SidebarSide::Left);
    c.sidebar.show = SidebarShow::Auto;
    let mut t = start(c, true);
    assert!(!t.app.sidebar_visible());
    t.app.handle_key(ctrl('h'));
    assert_eq!(t.app.focus(), Focus::Files, "peek opened");
    t.app.handle_key(ctrl('l'));
    assert_eq!(t.app.focus(), Focus::Content);
    assert!(!t.app.sidebar_visible(), "back to the content closes it");
    assert!(t.tmux().is_empty());
}

#[test]
fn outside_tmux_edges_do_what_ctrl_w_does() {
    let mut t = start(config(SidebarMode::Outline, SidebarSide::Left), false);
    t.app.handle_key(ctrl('l'));
    assert_eq!(t.app.focus(), Focus::Content);
    t.app.handle_key(ctrl('j'));
    assert_eq!(t.app.status(), NO_PANE_BELOW, "as C-w j");
    t.app.handle_key(ctrl('h'));
    t.app.handle_key(ctrl('h'));
    assert_eq!(t.app.focus(), Focus::Outline);
    assert!(t.tmux().is_empty(), "no TMUX: tmux is never run");
}

#[test]
fn an_empty_tmux_variable_counts_as_unset() {
    let mut t = start(config(SidebarMode::Files, SidebarSide::Left), false);
    t.app = t.app.with_env(|k| (k == "TMUX").then(String::new));
    t.app.handle_key(ctrl('l'));
    assert!(t.tmux().is_empty());
}

#[test]
fn pane_nav_false_unbinds_ctrl_hjkl_only() {
    let mut c = config(SidebarMode::Split, SidebarSide::Left);
    c.keys.pane_nav = false;
    let mut t = start(c, true);
    for k in ['h', 'j', 'k', 'l'] {
        t.app.handle_key(ctrl(k));
        assert_eq!(t.app.focus(), Focus::Content, "C-{k} unbound");
    }
    assert!(t.tmux().is_empty(), "no hand-off either");
    assert!(
        !t.app
            .help_lines()
            .iter()
            .any(|l| format!("{l:?}").contains("C-h")),
        "no C-h row in help"
    );
    // The C-w forms stay.
    t.app.handle_key(ctrl('w'));
    keys(&mut t.app, "h");
    assert_eq!(t.app.focus(), Focus::Files);
    t.app.handle_key(ctrl('w'));
    keys(&mut t.app, "j");
    assert_eq!(t.app.focus(), Focus::Outline);
}

#[test]
fn help_lists_the_pane_keys() {
    let t = start(config(SidebarMode::Files, SidebarSide::Left), false);
    let lines = format!("{:?}", t.app.help_lines());
    for k in ["C-h", "C-j", "C-k", "C-l"] {
        assert!(lines.contains(&format!("\"{k}\"")), "no {k} row: {lines}");
    }
}

#[test]
fn the_comment_box_keeps_ctrl_j_newline_and_ctrl_h() {
    let mut c = config(SidebarMode::Files, SidebarSide::Left);
    c.review.enabled = true;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir(root.join(".git")).unwrap();
    let path = write(&root, "a.md", "# A\n\nalpha\n");
    let calls: Calls = Rc::default();
    let rec = calls.clone();
    let mut app = App::new(
        StartOptions {
            target: StartTarget::File(path),
            tree_root: root.clone(),
            config: c,
            review_cache: Some(root.join(".cache")),
            mdroots: ramble::app::MdrootsOptions::memory(),
        },
        (100, 20),
    )
    .unwrap()
    .with_env(|k| (k == "TMUX").then(|| "x".into()))
    .with_pane_runner(move |cmd: &LaunchCommand| {
        rec.borrow_mut().push(cmd.argv.clone());
        Ok(Exit::Code(0))
    });
    keys(&mut app, "ccone");
    assert_eq!(app.mode(), Mode::Comment, "{}", app.status());
    app.handle_key(ctrl('j'));
    keys(&mut app, "two");
    assert_eq!(app.comment_text(), Some("one\ntwo"), "C-j is a newline");
    // C-h does nothing in the box, as before: no edit, no pane move.
    app.handle_key(ctrl('h'));
    app.handle_key(ctrl('k'));
    app.handle_key(ctrl('l'));
    assert_eq!(app.comment_text(), Some("one\ntwo"));
    assert_eq!(app.mode(), Mode::Comment);
    assert_eq!(app.focus(), Focus::Content);
    assert!(calls.borrow().is_empty(), "no tmux from the comment box");
}
