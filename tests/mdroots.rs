//! The in-process mdroots backend: pages no `[[lsp.server]]` serves get
//! their link targets, broken links, `gd` and status label from mdroots.
//! Every app uses a temp cache dir outside the browsed tree. Expected
//! values are written here, not taken from zk.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ramble::app::{App, AppEvent, MdrootsOptions, StartOptions, StartTarget};
use ramble::config::{Config, SidebarShow};
use tempfile::TempDir;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/zk");

fn opts(tree: &Path, target: StartTarget, cache: &Path) -> StartOptions {
    let mut config = Config::default();
    config.sidebar.show = SidebarShow::Never;
    config.lsp.server = vec![];
    StartOptions {
        target,
        tree_root: tree.to_path_buf(),
        config,
        review_cache: None,
        mdroots: MdrootsOptions::cache_dir(cache.to_path_buf()),
    }
}

/// A scratch copy of the zk fixture (markers included) plus synthetic
/// wiki, title and anchor cases. Returns (tree, cache) tempdirs: the
/// cache is outside the tree.
fn notebook() -> (TempDir, TempDir) {
    let tree = tempfile::tempdir().unwrap();
    copy_dir(Path::new(FIXTURE), tree.path());
    let root = tree.path();
    write(&root.join("c.md"), &c_source());
    write(
        &root.join("wiki.md"),
        "# Wiki\n\n[[b]] then [[Note A]] then [[c#deep-part]] then [x](c.md#deep%2Dpart)\n",
    );
    (tree, tempfile::tempdir().unwrap())
}

/// `c.md`: the `Deep Part` heading is far enough down to need a jump.
fn c_source() -> String {
    let mut s = String::from("# C\n\n");
    for i in 0..30 {
        s.push_str(&format!("line {i}\n\n"));
    }
    s.push_str("## Deep Part\n\nbody\n");
    s
}

fn copy_dir(from: &Path, to: &Path) {
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dst = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&dst).unwrap();
            copy_dir(&e.path(), &dst);
        } else {
            std::fs::copy(e.path(), dst).unwrap();
        }
    }
}

fn write(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
}

fn open(tree: &Path, cache: &Path, file: &str) -> App {
    let path = tree.join(file);
    App::new(opts(tree, StartTarget::File(path), cache), (60, 20)).unwrap()
}

fn canon(p: &Path) -> PathBuf {
    p.canonicalize().unwrap()
}

/// Pump mdroots replies until `pred` holds (10 s limit).
fn pump_until(app: &mut App, what: &str, pred: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !pred(app) {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}; label {:?}, status {:?}",
            app.lsp_label(),
            app.status()
        );
        app.pump_mdroots(Duration::from_millis(20));
    }
}

fn ready(app: &mut App) {
    pump_until(app, "the root workspace", |a| a.lsp_label() == "mdroots ●");
}

fn keys(app: &mut App, s: &str) {
    for c in s.chars() {
        app.event(AppEvent::Key(KeyEvent::new(
            KeyCode::Char(c),
            KeyModifiers::NONE,
        )));
    }
}

/// Cursor onto link `i` (0-based, in screen order) with `]l`.
fn to_link(app: &mut App, i: usize) {
    keys(app, "gg");
    keys(app, &"]l".repeat(i + 1));
    assert!(app.link_under_cursor().is_some(), "no link {i}");
}

fn page_path(app: &App) -> PathBuf {
    app.page().unwrap().path.clone().unwrap()
}

fn target(app: &App, i: usize) -> Option<PathBuf> {
    app.link_target(i).map(Path::to_path_buf)
}

fn broken(app: &App) -> Vec<usize> {
    app.broken_links().iter().copied().collect()
}

/// Every file under `dir` with its mtime.
fn tree_state(dir: &Path) -> BTreeMap<PathBuf, SystemTime> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let e = e.unwrap();
            let m = e.metadata().unwrap();
            if m.is_dir() {
                stack.push(e.path());
            }
            out.insert(e.path(), m.modified().unwrap());
        }
    }
    out
}

#[test]
fn zk_fixture_pages_get_targets_broken_links_and_the_label() {
    let (tree, cache) = notebook();
    let root = canon(tree.path());
    let mut app = open(tree.path(), cache.path(), "a.md");
    // No answer applied yet: the same dash as with no backend.
    assert_eq!(app.lsp_label(), "—");
    ready(&mut app);
    assert_eq!(target(&app, 0), Some(root.join("b.md")));
    assert!(broken(&app).is_empty());

    // A second page of the ready root is answered by the root directly.
    app.open_file(&tree.path().join("broken.md")).unwrap();
    pump_until(&mut app, "broken.md answered", |a| a.lsp_label() != "—");
    assert_eq!(app.lsp_label(), "mdroots ●", "never the single-file label");
    assert_eq!(target(&app, 0), None);
    assert_eq!(broken(&app), [0]);

    for (file, want) in [
        ("b.md", "a.md"),
        ("emoji.md", "b.md"),
        ("tagged.md", "a.md"),
    ] {
        app.open_file(&tree.path().join(file)).unwrap();
        pump_until(&mut app, file, |a| a.link_target(0).is_some());
        assert_eq!(target(&app, 0), Some(root.join(want)), "{file}");
        assert!(broken(&app).is_empty(), "{file}");
    }

    // Wiki, title and anchor links: ranges match ramble's links exactly.
    app.open_file(&tree.path().join("wiki.md")).unwrap();
    pump_until(&mut app, "wiki.md", |a| a.link_target(3).is_some());
    assert_eq!(target(&app, 0), Some(root.join("b.md")));
    assert_eq!(
        target(&app, 1),
        Some(root.join("a.md")),
        "[[Note A]] by title"
    );
    assert_eq!(target(&app, 2), Some(root.join("c.md")));
    assert_eq!(target(&app, 3), Some(root.join("c.md")));
    assert!(broken(&app).is_empty());
}

#[test]
fn gd_follows_mdroots_targets_and_lands_on_the_anchor() {
    let (tree, cache) = notebook();
    let root = canon(tree.path());
    let mut app = open(tree.path(), cache.path(), "wiki.md");
    ready(&mut app);

    // `[[Note A]]`: only mdroots knows the title; the local parse would
    // look for a missing `Note A.md`.
    to_link(&mut app, 1);
    keys(&mut app, "gd");
    assert_eq!(canon(&page_path(&app)), root.join("a.md"));
    assert_eq!(app.history_depth(), 1);

    let deep_row = |app: &App| {
        let p = app.page().unwrap();
        let h = p
            .doc
            .headings
            .iter()
            .find(|h| h.slug == "deep-part")
            .unwrap();
        p.rendered.srcmap.row_for(h.range.start).unwrap()
    };
    for i in [2, 3] {
        app.open_file(&tree.path().join("wiki.md")).unwrap();
        pump_until(&mut app, "wiki.md", |a| a.link_target(3).is_some());
        to_link(&mut app, i);
        keys(&mut app, "gd");
        assert_eq!(canon(&page_path(&app)), root.join("c.md"), "link {i}");
        // `#deep%2Dpart` is percent-decoded like the local parse does.
        assert_eq!(app.cursor().row, deep_row(&app), "link {i} anchor");
        assert!(app.status().is_empty(), "{}", app.status());
    }
}

#[test]
fn a_link_mdroots_does_not_report_is_followed_locally() {
    // An unfenced header: ramble parses its link, mdroots' workspace takes
    // the line as front matter. That link gets no target; `gd` resolves it
    // locally.
    let (tree, cache) = notebook();
    let root = canon(tree.path());
    write(
        &tree.path().join("hdr.md"),
        "Title: x [l](a)\n\n# H\n\nSee [b](b).\n",
    );
    let mut app = open(tree.path(), cache.path(), "hdr.md");
    ready(&mut app);
    pump_until(&mut app, "[b](b)", |a| a.link_target(1).is_some());
    assert_eq!(target(&app, 0), None);
    assert_eq!(target(&app, 1), Some(root.join("b.md")));
    to_link(&mut app, 0);
    keys(&mut app, "gd");
    assert_eq!(canon(&page_path(&app)), root.join("a.md"));
}

#[test]
fn a_plain_folder_works_and_caches_only_in_the_cache_dir() {
    let tree = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let root = canon(tree.path());
    write(&root.join("a.md"), "# A\n\n[b](b) and [gone](gone.md)\n");
    write(&root.join("b.md"), "# B\n");
    let before = tree_state(tree.path());
    let mut app = open(tree.path(), cache.path(), "a.md");
    ready(&mut app);
    assert_eq!(target(&app, 0), Some(root.join("b.md")));
    assert_eq!(broken(&app), [1]);
    drop(app);
    // Whatever mdroots cached (its policy decides) is in the cache dir:
    // the browsed tree is untouched.
    assert_eq!(tree_state(tree.path()), before);
    // The registry (opened for any non-memory session) shows the cache
    // went to the dir we gave, not the user's.
    assert!(cache.path().join("roots.v1.db").exists());
}

#[test]
fn a_session_never_writes_inside_the_browsed_tree() {
    let (tree, cache) = notebook();
    let before = tree_state(tree.path());
    let mut app = open(tree.path(), cache.path(), "wiki.md");
    ready(&mut app);
    to_link(&mut app, 0);
    keys(&mut app, "gd");
    pump_until(&mut app, "b.md", |a| a.link_target(0).is_some());
    app.reload();
    app.refresh();
    pump_until(&mut app, "b.md again", |a| a.lsp_label() == "mdroots ●");
    drop(app);
    assert_eq!(tree_state(tree.path()), before, "files or mtimes changed");
}

#[test]
fn a_new_note_resolves_a_broken_title_link_through_the_mdroots_watcher() {
    let (tree, cache) = notebook();
    let root = canon(tree.path());
    write(&root.join("p.md"), "# P\n\nSee [[Later Title]].\n");
    let mut app = open(tree.path(), cache.path(), "p.md");
    ready(&mut app);
    assert!(app.mdroots_watching(), "mdroots must watch the root here");
    assert_eq!(target(&app, 0), None);
    assert_eq!(broken(&app), [0]);

    // No ramble event: mdroots' watcher notices the new note on its own.
    let later = root.join("later.md");
    write(&later, "# Later Title\n");
    pump_until(&mut app, "[[Later Title]] resolved", |a| {
        a.link_target(0).is_some()
    });
    assert_eq!(target(&app, 0), Some(later));
    assert!(broken(&app).is_empty());
}

#[test]
fn reloading_the_page_refreshes_it_in_mdroots() {
    let (tree, cache) = notebook();
    let root = canon(tree.path());
    let page = root.join("p.md");
    write(&page, "# P\n\nnothing yet\n");
    let mut app = open(tree.path(), cache.path(), "p.md");
    ready(&mut app);
    write(&page, "# P\n\n[[Note B]] and [x](nowhere.md)\n");
    app.reload();
    pump_until(&mut app, "the reloaded page", |a| {
        a.lsp_label() == "mdroots ●" && a.link_target(0).is_some()
    });
    assert_eq!(target(&app, 0), Some(root.join("b.md")));
    assert_eq!(broken(&app), [1]);
}

#[test]
fn stdin_pages_get_no_backend() {
    let tree = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let mut app = App::new(
        opts(
            tree.path(),
            StartTarget::Stdin("# S\n\n[b](b)\n".into()),
            cache.path(),
        ),
        (60, 20),
    )
    .unwrap();
    app.pump_mdroots(Duration::from_millis(200));
    assert_eq!(app.lsp_label(), "—");
    assert_eq!(target(&app, 0), None);
}
