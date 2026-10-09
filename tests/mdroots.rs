//! The in-process mdroots backend: pages no `[[lsp.server]]` serves get
//! their link targets, broken links, `gd` and status label from mdroots.
//! Every app uses a temp cache dir outside the browsed tree. Expected
//! values are written here, not taken from zk.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ramble::app::{App, AppEvent, HelpLine, MdrootsOptions, StartOptions, StartTarget};
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

// Pickers: notes, search, tags and backlinks from mdroots.

/// Set `path`'s modification time to `secs` after the epoch.
fn set_mtime(path: &Path, secs: u64) {
    let t = SystemTime::UNIX_EPOCH + Duration::from_secs(secs);
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(t)
        .unwrap();
}

fn leader(app: &mut App, s: &str) {
    keys(app, " ");
    keys(app, s);
}

fn enter(app: &mut App) {
    app.event(AppEvent::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )));
}

fn esc(app: &mut App) {
    app.event(AppEvent::Key(KeyEvent::new(
        KeyCode::Esc,
        KeyModifiers::NONE,
    )));
}

/// Pump until the open picker has its items.
fn loaded(app: &mut App) {
    pump_until(app, "the picker's items", |a| {
        a.picker().is_some_and(|p| !p.loading)
    });
}

/// The open picker's title and (label, detail) rows.
fn rows(app: &App) -> (String, Vec<(String, String)>) {
    let p = app.picker().expect("a picker");
    let items = p
        .items
        .iter()
        .map(|i| (i.label.clone(), i.detail.clone()))
        .collect();
    (p.title.to_string(), items)
}

fn row(label: &str, detail: &str) -> (String, String) {
    (label.into(), detail.into())
}

/// Source line (1-based) of the cursor's row.
fn cursor_line(app: &App) -> usize {
    app.page().unwrap().rendered.source_lines[app.cursor().row]
}

#[test]
fn the_notes_picker_lists_titles_newest_first() {
    let (tree, cache) = notebook();
    let root = tree.path();
    for (i, f) in [
        "wiki.md",
        "a.md",
        "c.md",
        "tagged.md",
        "emoji.md",
        "broken.md",
        "b.md",
    ]
    .iter()
    .enumerate()
    {
        set_mtime(&root.join(f), 1_000 + 10 * i as u64);
    }
    let mut app = open(root, cache.path(), "a.md");
    leader(&mut app, "zf");
    loaded(&mut app);
    let (title, items) = rows(&app);
    assert_eq!(title, "Notes");
    assert_eq!(
        items,
        [
            row("Note B", "b.md"),
            row("Broken", "broken.md"),
            row("Emoji", "emoji.md"),
            row("Tagged", "tagged.md"),
            row("C", "c.md"),
            row("Note A", "a.md"),
            row("Wiki", "wiki.md"),
        ]
    );
    keys(&mut app, "emoj");
    enter(&mut app);
    assert_eq!(canon(&page_path(&app)), canon(&root.join("emoji.md")));
}

#[test]
fn a_search_hit_opens_its_note_at_the_line() {
    let (tree, cache) = notebook();
    let root = tree.path();
    let mut app = open(root, cache.path(), "a.md");
    // The prompt first, then the query.
    leader(&mut app, "zs");
    assert!(app.picker().unwrap().prompting);
    keys(&mut app, "back to");
    enter(&mut app);
    loaded(&mut app);
    let (title, items) = rows(&app);
    assert_eq!(title, "Search: back to");
    // "Back to [Note A](a)." is on line 3 of b.md.
    assert_eq!(items, [row("Note B", "b.md:3")]);
    enter(&mut app);
    assert_eq!(canon(&page_path(&app)), canon(&root.join("b.md")));
    assert_eq!(cursor_line(&app), 3);

    // `:Search q` skips the prompt; a deep line is jumped to.
    app.open_file(&root.join("a.md")).unwrap();
    keys(&mut app, ":Search body");
    enter(&mut app);
    loaded(&mut app);
    // `body` is the last line of c.md: 2 + 30 * 2 + 3.
    assert_eq!(rows(&app).1, [row("C", "c.md:65")]);
    enter(&mut app);
    assert_eq!(canon(&page_path(&app)), canon(&root.join("c.md")));
    assert_eq!(cursor_line(&app), 65);
}

#[test]
fn tags_merge_case_insensitively_and_open_their_notes() {
    let (tree, cache) = notebook();
    let root = tree.path();
    write(
        &root.join("t2.md"),
        "---\ntags: [Project, x]\n---\n# Second tagged\n",
    );
    set_mtime(&root.join("tagged.md"), 1_000);
    set_mtime(&root.join("t2.md"), 2_000);
    let mut app = open(root, cache.path(), "a.md");
    leader(&mut app, "zz");
    loaded(&mut app);
    let (title, items) = rows(&app);
    assert_eq!(title, "Tags");
    // `Project` sorts before `project`: its spelling names the merged row.
    assert_eq!(items, [row("Project", "2"), row("x", "1")]);
    enter(&mut app);
    assert!(app.picker().unwrap().loading, "a new request");
    loaded(&mut app);
    let (title, items) = rows(&app);
    assert_eq!(title, "Tag: Project");
    assert_eq!(
        items,
        [row("Second tagged", "t2.md"), row("Tagged", "tagged.md")]
    );
    enter(&mut app);
    assert_eq!(canon(&page_path(&app)), canon(&root.join("t2.md")));
}

#[test]
fn backlinks_list_linking_notes_and_land_on_the_link_back() {
    let (tree, cache) = notebook();
    let root = tree.path();
    let mut app = open(root, cache.path(), "a.md");
    keys(&mut app, "grr");
    loaded(&mut app);
    let (title, items) = rows(&app);
    assert_eq!(title, "Backlinks");
    // Sorted by source path; lines are 1-based.
    assert_eq!(
        items,
        [
            row("Note B", "b.md:3"),
            row("Tagged", "tagged.md:6"),
            row("Wiki", "wiki.md:3"),
        ]
    );
    // wiki.md links to a.md by title only (`[[Note A]]`): landing needs
    // mdroots' target for it.
    keys(&mut app, "wiki");
    enter(&mut app);
    assert_eq!(canon(&page_path(&app)), canon(&root.join("wiki.md")));
    pump_until(&mut app, "the link back", |a| {
        a.link_under_cursor() == Some(1)
    });
    assert_eq!(target(&app, 1), Some(canon(&root.join("a.md"))));
}

#[test]
fn a_reply_for_an_earlier_picker_is_dropped() {
    let (tree, cache) = notebook();
    let mut app = open(tree.path(), cache.path(), "a.md");
    // Two pickers in a row: the worker answers both, in order; the first
    // one's answer arrives while the second is loading and must not fill it.
    leader(&mut app, "zf");
    assert!(app.picker().unwrap().loading);
    esc(&mut app);
    leader(&mut app, "zz");
    assert!(app.picker().unwrap().loading);
    loaded(&mut app);
    // Let any late answer arrive too.
    app.pump_mdroots(Duration::from_millis(200));
    let (title, items) = rows(&app);
    assert_eq!(title, "Tags");
    assert_eq!(items, [row("project", "1")]);
}

#[test]
fn a_lazy_root_walks_files_for_notes_and_marks_other_pickers_partial() {
    // A monorepo marker makes mdroots index a working set only.
    let tree = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let root = tree.path();
    write(&root.join(".buckconfig"), "");
    std::fs::create_dir_all(root.join("d1")).unwrap();
    std::fs::create_dir_all(root.join("d2")).unwrap();
    write(&root.join("d1/a.md"), "# Alpha\n");
    write(&root.join("d1/b.md"), "# Beta\n\n[a](a.md)\n");
    write(&root.join("d2/c.md"), "# Gamma\n");
    let mut app = open(root, cache.path(), "d1/a.md");
    leader(&mut app, "zf");
    loaded(&mut app);
    let (title, items) = rows(&app);
    assert_eq!(title, "Notes");
    // The file walk of the tree root: stems, sorted by path.
    assert_eq!(
        items,
        [
            row("a", "d1/a.md"),
            row("b", "d1/b.md"),
            row("c", "d2/c.md")
        ]
    );
    esc(&mut app);
    keys(&mut app, "grr");
    loaded(&mut app);
    let (title, items) = rows(&app);
    assert_eq!(title, "Backlinks (partial)");
    assert_eq!(items, [row("Beta", "d1/b.md:3")]);
}

#[test]
fn a_plain_folder_root_walks_files_for_notes_and_marks_other_pickers_partial() {
    // No markers: mdroots may root the page at its own folder (loose), so
    // its lists can miss notes in sibling folders.
    let tree = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let root = tree.path();
    std::fs::create_dir_all(root.join("d1")).unwrap();
    std::fs::create_dir_all(root.join("d2")).unwrap();
    write(
        &root.join("d1/a.md"),
        "---\ntags: [x]\n---\n# Alpha\n\nneedle\n",
    );
    write(&root.join("d2/b.md"), "# Beta\n\nneedle\n");
    let mut app = open(root, cache.path(), "d1/a.md");
    leader(&mut app, "zf");
    loaded(&mut app);
    let (title, items) = rows(&app);
    assert_eq!(title, "Notes");
    assert_eq!(items, [row("a", "d1/a.md"), row("b", "d2/b.md")]);
    esc(&mut app);
    leader(&mut app, "zz");
    loaded(&mut app);
    assert_eq!(rows(&app).0, "Tags (partial)");
    esc(&mut app);
    keys(&mut app, ":Search needle");
    enter(&mut app);
    loaded(&mut app);
    assert_eq!(rows(&app).0, "Search: needle (partial)");
}

// `K`: the link target's preview from mdroots.

/// Pump until the hover popup opens.
fn hovered(app: &mut App) -> String {
    pump_until(app, "the hover popup", |a| a.hover_popup().is_some());
    app.hover_popup().unwrap().to_string()
}

/// Whether help lists a `K` row now.
fn help_has_k(app: &App) -> bool {
    app.help_lines()
        .iter()
        .any(|l| matches!(l, HelpLine::Item { keys, .. } if keys == "K"))
}

#[test]
fn k_previews_the_link_target_note() {
    let (tree, cache) = notebook();
    let mut app = open(tree.path(), cache.path(), "a.md");
    to_link(&mut app, 0);
    // Before the page's links arrive there is nothing to preview yet.
    assert!(!help_has_k(&app));
    keys(&mut app, "K");
    assert_eq!(app.status(), "No hover information");
    assert_eq!(app.hover_popup(), None);
    ready(&mut app);
    assert!(help_has_k(&app), "K row once the link has a note target");
    keys(&mut app, "K");
    // The excerpt starts with the H1: no second title line.
    assert_eq!(hovered(&mut app), "# Note B\n\nBack to [Note A](a).");
}

#[test]
fn k_shows_front_matter_but_not_its_title() {
    let (tree, cache) = notebook();
    write(
        &tree.path().join("fm.md"),
        "---\ntitle: FM Title\ntags: [project, x]\n---\nline1\nline2\n",
    );
    write(
        &tree.path().join("p.md"),
        "# P\n\n[[fm]] and [t](tagged.md)\n",
    );
    let mut app = open(tree.path(), cache.path(), "p.md");
    ready(&mut app);
    to_link(&mut app, 0);
    keys(&mut app, "K");
    assert_eq!(
        hovered(&mut app),
        "# FM Title\n\ntags: project, x\n\nline1\nline2"
    );
    esc(&mut app);
    assert_eq!(app.hover_popup(), None);
    to_link(&mut app, 1);
    keys(&mut app, "K");
    assert_eq!(
        hovered(&mut app),
        "tags: project\n\n# Tagged\n\nLinks to [Note A](a)."
    );
}

#[test]
fn k_on_a_link_with_no_note_is_a_status() {
    let (tree, cache) = notebook();
    write(&tree.path().join("t.txt"), "plain\n");
    write(
        &tree.path().join("p.md"),
        "# P\n\n[gone](missing) [u](https://example.org) [h](#p) [t](t.txt) [self](p.md)\n",
    );
    let mut app = open(tree.path(), cache.path(), "p.md");
    ready(&mut app);
    // mdroots gives the same-page anchor and t.txt a target.
    pump_until(&mut app, "the t.txt target", |a| a.link_target(3).is_some());
    for i in 0..5 {
        to_link(&mut app, i);
        assert!(!help_has_k(&app), "link {i}: no K row");
        app.set_status("");
        keys(&mut app, "K");
        assert_eq!(app.status(), "No hover information", "link {i}");
        app.pump_mdroots(Duration::from_millis(100));
        assert_eq!(app.hover_popup(), None, "link {i}");
    }
}

#[test]
fn k_on_a_link_with_no_note_closes_an_open_preview() {
    let (tree, cache) = notebook();
    write(
        &tree.path().join("p.md"),
        "# P\n\n[[b]] and [gone](missing)\n",
    );
    let mut app = open(tree.path(), cache.path(), "p.md");
    ready(&mut app);
    to_link(&mut app, 0);
    keys(&mut app, "K");
    hovered(&mut app);
    // The popup is open; `K` on a link with no note must not leave it.
    to_link(&mut app, 1);
    keys(&mut app, "K");
    assert_eq!(app.status(), "No hover information");
    assert_eq!(app.hover_popup(), None);
}

#[test]
fn a_preview_reply_is_dropped_only_when_the_page_changed() {
    let (tree, cache) = notebook();
    let mut app = open(tree.path(), cache.path(), "wiki.md");
    ready(&mut app);
    // The cursor moved before the reply: kept (parity with the LSP hover).
    to_link(&mut app, 0);
    keys(&mut app, "K");
    keys(&mut app, "gg");
    assert_eq!(hovered(&mut app), "# Note B\n\nBack to [Note A](a).");
    esc(&mut app);
    // The page changed before the reply: dropped.
    to_link(&mut app, 1);
    keys(&mut app, "K");
    app.open_file(&tree.path().join("b.md")).unwrap();
    pump_until(&mut app, "b.md answered", |a| a.link_target(0).is_some());
    app.pump_mdroots(Duration::from_millis(300));
    assert_eq!(app.hover_popup(), None);
    // The page reply was not made stale by the preview.
    assert_eq!(app.lsp_label(), "mdroots ●");
}

#[test]
fn k_on_a_comment_is_not_replaced_by_a_late_preview() {
    let (tree, cache) = notebook();
    let root = tree.path();
    write(&root.join("p.md"), "# P\n\n[b](b)\n\nbeta\n");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    let review = cache.path().join("review");
    let c = debrief_review::Comment::on_file("p.md", 5..=5, "beta", "COMMENT MUST WIN");
    debrief_review::Review::open(&review, root)
        .unwrap()
        .add(c)
        .unwrap();
    let mut o = opts(root, StartTarget::File(root.join("p.md")), cache.path());
    o.review_cache = Some(review);
    let mut app = App::new(o, (60, 20)).unwrap();
    ready(&mut app);
    to_link(&mut app, 0);
    keys(&mut app, "K");
    keys(&mut app, "G");
    keys(&mut app, "K");
    let comment = Some("● line 5\nCOMMENT MUST WIN");
    assert_eq!(app.hover_popup(), comment);
    app.pump_mdroots(Duration::from_millis(500));
    assert_eq!(app.hover_popup(), comment, "late preview dropped");
}
