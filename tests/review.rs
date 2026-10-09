//! Comment mode and review markers (debrief spec §2): capture with `cc` and
//! visual `c`, the prompt and `C-e`, and the gutter, tree, status count and
//! `]r` / `[r` driven by the debrief-review batch. Every test uses a temp
//! repo (`.git` dir) and a temp cache dir; nothing sleeps.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use debrief_review::{Comment, Rev, Review, Side};
use ramble::app::{
    App, AppEvent, EMPTY_COMMENT, Effect, Exit, LaunchCommand, Mode, NO_MORE_REVIEW, NO_REVIEW,
    NO_SOURCE_LINE, REVIEW_POLL, StartOptions, StartTarget,
};
use ramble::config::{Config, SidebarMode, SidebarShow};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tempfile::TempDir;

const COLS: u16 = 60;
const ROWS: u16 = 24;

/// A temp repo root (canonical, with `.git`) and a temp cache dir.
struct Repo {
    _dir: TempDir,
    root: PathBuf,
    cache: PathBuf,
}

impl Repo {
    fn new() -> Repo {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let root = base.join("repo");
        std::fs::create_dir_all(root.join(".git")).unwrap();
        let cache = base.join("cache");
        Repo {
            _dir: dir,
            root,
            cache,
        }
    }

    fn write(&self, rel: &str, body: &str) -> PathBuf {
        let p = self.root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, body).unwrap();
        p
    }

    /// A second handle on the batch, as another process would have.
    fn review(&self) -> Review {
        Review::open(&self.cache, &self.root).unwrap()
    }

    fn comments(&self) -> Vec<Comment> {
        self.review().comments().to_vec()
    }

    fn add(&self, path: &str, a: u32, b: u32) {
        self.review()
            .add(Comment::on_file(path, a..=b, "x", "note"))
            .unwrap();
    }

    fn app(&self, target: StartTarget, config: Config) -> App {
        App::new(
            StartOptions {
                target,
                tree_root: self.root.clone(),
                config,
                review_cache: Some(self.cache.clone()),
            },
            (COLS, ROWS),
        )
        .unwrap()
    }

    fn open(&self, rel: &str, body: &str) -> App {
        let path = self.write(rel, body);
        self.app(StartTarget::File(path), config(None))
    }
}

fn config(sidebar: Option<SidebarMode>) -> Config {
    let mut c = Config::default();
    c.lsp.server = vec![];
    match sidebar {
        Some(m) => {
            c.sidebar.default = m;
            c.sidebar.auto_hide_below = 0;
        }
        None => c.sidebar.show = SidebarShow::Never,
    }
    c
}

fn send(app: &mut App, k: KeyEvent) {
    app.event(AppEvent::Key(k));
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn keys(app: &mut App, s: &str) {
    for c in s.chars() {
        send(app, key(KeyCode::Char(c)));
    }
}

/// Type `body` into the open prompt and press Enter.
fn type_and_save(app: &mut App, body: &str) {
    assert_eq!(app.mode(), Mode::Comment, "prompt open: {}", app.status());
    keys(app, body);
    send(app, key(KeyCode::Enter));
    assert_eq!(app.mode(), Mode::Normal);
}

fn row_text(app: &App, row: usize) -> String {
    app.page().unwrap().rendered.lines[row]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect()
}

fn row_of(app: &App, text: &str) -> usize {
    let n = app.page().unwrap().rendered.lines.len();
    (0..n)
        .find(|&r| row_text(app, r).contains(text))
        .unwrap_or_else(|| panic!("no row with {text:?}"))
}

/// Put the cursor on rendered `row`.
fn goto(app: &mut App, row: usize) {
    keys(app, &format!("{}G", row + 1));
    assert_eq!(app.cursor().row, row);
}

fn goto_text(app: &mut App, text: &str) {
    let row = row_of(app, text);
    goto(app, row);
}

fn screen(app: &App) -> Vec<String> {
    let mut term = Terminal::new(TestBackend::new(COLS, ROWS)).unwrap();
    term.draw(|f| ramble::ui::draw(f, app)).unwrap();
    let buf = term.backend().buffer().clone();
    (0..ROWS)
        .map(|y| {
            (0..COLS)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect()
        })
        .collect()
}

fn gutter_rows(app: &App) -> Vec<usize> {
    let s = screen(app);
    (0..ROWS as usize - 1)
        .filter(|&y| s[y].starts_with('●'))
        .map(|y| app.scroll() + y)
        .collect()
}

/// Lines and excerpt of the only comment in the batch.
fn only(repo: &Repo) -> ((u32, u32), String, String, String) {
    let cs = repo.comments();
    assert_eq!(cs.len(), 1, "{cs:?}");
    let c = &cs[0];
    assert!(c.rev.is_none());
    (c.lines, c.excerpt.clone(), c.body.clone(), c.path.clone())
}

const DOC: &str = "# Title\n\nalpha\n\nbeta\n\ngamma\n\ndelta\n";

#[test]
fn cc_on_a_paragraph_line_saves_one_comment() {
    let repo = Repo::new();
    let mut app = repo.open("notes/doc.md", DOC);
    goto_text(&mut app, "beta");
    keys(&mut app, "cc");
    assert_eq!(app.mode(), Mode::Comment);
    assert!(
        screen(&app)[ROWS as usize - 1].starts_with("comment: "),
        "prompt drawn"
    );
    keys(&mut app, "too vaguex");
    send(&mut app, key(KeyCode::Backspace));
    assert_eq!(app.comment_prompt().unwrap(), "comment: too vague");
    send(&mut app, key(KeyCode::Enter));
    assert_eq!(app.status(), "Comment added (1 in batch)");
    let (lines, excerpt, body, path) = only(&repo);
    assert_eq!(lines, (5, 5));
    assert_eq!(excerpt, "beta");
    assert_eq!(body, "too vague");
    assert_eq!(path, "notes/doc.md");
    assert_eq!(app.review_comments_here().len(), 1);
    assert_eq!(app.review_root(), Some(repo.root.as_path()));
}

#[test]
fn visual_lines_over_three_list_items_cover_their_source_lines() {
    let repo = Repo::new();
    let src = "# L\n\n- one\n- two\n- three\n- four\n";
    let mut app = repo.open("l.md", src);
    goto_text(&mut app, "one");
    keys(&mut app, "Vjjc");
    type_and_save(&mut app, "list");
    let (lines, excerpt, ..) = only(&repo);
    assert_eq!(lines, (3, 5));
    assert_eq!(excerpt, "- one\n- two\n- three");
}

#[test]
fn charwise_and_block_selections_take_whole_lines() {
    let repo = Repo::new();
    let mut app = repo.open("doc.md", DOC);
    goto_text(&mut app, "alpha");
    keys(&mut app, "lvjjc");
    type_and_save(&mut app, "v");
    goto_text(&mut app, "gamma");
    send(&mut app, ctrl('v'));
    keys(&mut app, "jjc");
    type_and_save(&mut app, "block");
    let cs = repo.comments();
    let got: Vec<_> = cs.iter().map(|c| (c.lines, c.excerpt.as_str())).collect();
    assert_eq!(
        got,
        [((3, 5), "alpha\n\nbeta"), ((7, 9), "gamma\n\ndelta")],
        "rows map to whole source lines"
    );
}

#[test]
fn a_table_row_maps_to_its_source_line() {
    let repo = Repo::new();
    let src = "# T\n\n| a | b |\n|---|---|\n| one | two |\n| three | four |\n";
    let mut app = repo.open("t.md", src);
    goto_text(&mut app, "three");
    keys(&mut app, "cc");
    type_and_save(&mut app, "row");
    let (lines, excerpt, ..) = only(&repo);
    assert_eq!(lines, (6, 6));
    assert_eq!(excerpt, "| three | four |");
}

#[test]
fn an_empty_table_row_maps_to_its_own_source_line() {
    let repo = Repo::new();
    let src = "# T\n\n| a | b |\n|---|---|\n|   |   |\n| three | four |\n";
    let mut app = repo.open("t.md", src);
    let empty = row_of(&app, "three") - 1;
    goto(&mut app, empty);
    keys(&mut app, "cc");
    type_and_save(&mut app, "empty row");
    let (lines, excerpt, ..) = only(&repo);
    assert_eq!(lines, (5, 5));
    assert_eq!(excerpt, "|   |   |");
}

#[test]
fn a_code_block_line_and_a_blank_line_inside_it() {
    let repo = Repo::new();
    let src = "# C\n\n```rust\nfn a() {}\n\nfn b() {}\n```\n";
    let mut app = repo.open("c.md", src);
    let b = row_of(&app, "fn b");
    goto(&mut app, b);
    keys(&mut app, "cc");
    type_and_save(&mut app, "code");
    // The blank code line is the row above `fn b`; it draws nothing.
    goto(&mut app, b - 1);
    assert_eq!(row_text(&app, b - 1).trim(), "");
    keys(&mut app, "cc");
    type_and_save(&mut app, "blank");
    let cs = repo.comments();
    let got: Vec<_> = cs.iter().map(|c| (c.lines, c.excerpt.as_str())).collect();
    assert_eq!(got, [((5, 5), ""), ((6, 6), "fn b() {}")]);
    // Both rows are marked and are `]r` targets.
    assert_eq!(gutter_rows(&app), vec![b - 1, b]);
    keys(&mut app, "gg]r");
    assert_eq!(app.cursor().row, b - 1);
    keys(&mut app, "]r");
    assert_eq!(app.cursor().row, b);
}

#[test]
fn the_front_matter_marker_row_maps_to_the_front_matter_lines() {
    let repo = Repo::new();
    let src = "---\ntitle: T\ntags: [a]\n---\n\n# Body\n";
    let mut app = repo.open("fm.md", src);
    assert!(
        row_text(&app, 0).contains("front matter"),
        "{}",
        row_text(&app, 0)
    );
    keys(&mut app, "cc");
    type_and_save(&mut app, "fm");
    let (lines, excerpt, ..) = only(&repo);
    assert_eq!(lines, (1, 4));
    assert_eq!(excerpt, "---\ntitle: T\ntags: [a]\n---");
}

#[test]
fn a_separator_row_has_no_source_line() {
    let repo = Repo::new();
    let mut app = repo.open("doc.md", DOC);
    let blank = row_of(&app, "alpha") + 1;
    assert_eq!(row_text(&app, blank), "");
    goto(&mut app, blank);
    keys(&mut app, "cc");
    assert_eq!(app.mode(), Mode::Normal);
    assert_eq!(app.status(), NO_SOURCE_LINE);
}

#[test]
fn esc_cancels_and_empty_enter_saves_nothing() {
    let repo = Repo::new();
    let mut app = repo.open("doc.md", DOC);
    keys(&mut app, "ccdraft");
    send(&mut app, key(KeyCode::Esc));
    assert_eq!(app.mode(), Mode::Normal);
    assert!(app.comment_prompt().is_none());
    assert!(!repo.cache.join("reviews").exists(), "nothing written");

    keys(&mut app, "cc   ");
    send(&mut app, key(KeyCode::Enter));
    assert_eq!(app.status(), EMPTY_COMMENT);
    assert!(!repo.cache.join("reviews").exists(), "nothing written");
    assert!(repo.comments().is_empty());
}

#[test]
fn gutter_status_count_and_jumps_follow_the_batch() {
    let repo = Repo::new();
    let mut app = repo.open("doc.md", DOC);
    keys(&mut app, "]r");
    assert_eq!(app.status(), NO_REVIEW);
    assert_eq!(app.review_gutter(), 0);
    let (alpha, beta, delta) = (
        row_of(&app, "alpha"),
        row_of(&app, "beta"),
        row_of(&app, "delta"),
    );
    goto(&mut app, beta);
    keys(&mut app, "Vjjc");
    type_and_save(&mut app, "two lines");
    goto(&mut app, delta);
    keys(&mut app, "cc");
    type_and_save(&mut app, "last");
    assert_eq!(app.review_gutter(), 1);
    assert_eq!(app.review_count(), 2);
    let s = screen(&app);
    let gamma = row_of(&app, "gamma");
    for (row, text) in [(beta, "beta"), (gamma, "gamma"), (delta, "delta")] {
        assert_eq!(s[row].trim_end(), format!("●{text}"), "row {row}");
    }
    assert!(
        s[alpha].starts_with(" alpha"),
        "content moved: {:?}",
        s[alpha]
    );
    assert!(s[ROWS as usize - 1].contains("review 2"), "{s:?}");

    keys(&mut app, "gg]r");
    assert_eq!(app.cursor().row, beta);
    keys(&mut app, "]r");
    assert_eq!(app.cursor().row, delta);
    keys(&mut app, "]r");
    assert_eq!(app.status(), NO_MORE_REVIEW);
    keys(&mut app, "[r");
    assert_eq!(app.cursor().row, beta);
}

/// A reflowed paragraph: lines 2..n reach their rows through the srcmap.
#[test]
fn comments_inside_a_reflowed_paragraph_mark_their_rows() {
    let repo = Repo::new();
    let src = "# P\n\nfirst line\nsecond line\nthird line\n\nlast para\nmore\n";
    repo.add("p.md", 4, 4);
    repo.add("p.md", 8, 8);
    repo.add("p.md", 40, 40);
    let app = repo.open("p.md", src);
    let more = row_of(&app, "more");
    let second = row_of(&app, "second");
    assert_eq!(gutter_rows(&app), vec![second, more]);
}

#[test]
fn another_handle_s_comment_appears_at_the_next_poll() {
    let repo = Repo::new();
    let mut app = repo.open("doc.md", DOC);
    let start = Instant::now();
    repo.add("doc.md", 5, 5);
    app.tick(start);
    assert_eq!(app.review_count(), 0, "not polled within the second");
    app.event(AppEvent::Tick(
        start + REVIEW_POLL + Duration::from_millis(1),
    ));
    assert_eq!(app.review_count(), 1);
    assert_eq!(gutter_rows(&app), vec![row_of(&app, "beta")]);
}

#[test]
fn a_diff_comment_counts_nowhere() {
    let repo = Repo::new();
    let rev = Rev {
        commit: "abc".into(),
        change: None,
        summary: None,
    };
    repo.review()
        .add(Comment::on_diff(
            "doc.md",
            rev,
            Side::New,
            5..=5,
            "beta",
            "b",
        ))
        .unwrap();
    let app = repo.open("doc.md", DOC);
    assert_eq!(app.review_gutter(), 0);
    assert_eq!(app.review_count(), 0);
    assert!(gutter_rows(&app).is_empty());
    assert!(app.review_comments_here().is_empty());
}

#[test]
fn tree_marks_files_with_counts_and_folders() {
    let repo = Repo::new();
    let doc = repo.write("doc.md", DOC);
    repo.write("sub/b.md", "# B\n");
    repo.add("sub/b.md", 1, 1);
    repo.add("sub/b.md", 1, 1);
    let app = repo.app(StartTarget::File(doc), config(Some(SidebarMode::Files)));
    assert_eq!(app.file_marker(&repo.root.join("sub")), Some(('●', None)));
    assert_eq!(
        app.file_marker(&repo.root.join("sub/b.md")),
        Some(('●', Some(2)))
    );
    assert_eq!(app.file_marker(&repo.root.join("doc.md")), None);
    let s = screen(&app);
    assert!(s.iter().any(|l| l.contains("sub ●")), "{s:?}");
}

#[test]
fn stdin_pages_refuse() {
    let repo = Repo::new();
    let mut app = repo.app(StartTarget::Stdin(DOC.into()), config(None));
    keys(&mut app, "cc");
    assert_eq!(app.mode(), Mode::Normal);
    assert_eq!(app.status(), "Can't comment on stdin");
    keys(&mut app, "Vc");
    assert_eq!(app.status(), "Can't comment on stdin");
    assert_eq!(app.mode(), Mode::Normal);
}

#[test]
fn review_disabled_leaves_c_unbound() {
    let repo = Repo::new();
    repo.add("doc.md", 5, 5);
    let path = repo.write("doc.md", DOC);
    let mut c = config(None);
    c.review.enabled = false;
    let mut app = repo.app(StartTarget::File(path), c);
    assert_eq!(app.review_count(), 0, "no markers");
    assert_eq!(app.review_gutter(), 0);
    keys(&mut app, "cc");
    assert_eq!(app.mode(), Mode::Normal);
    keys(&mut app, "Vc");
    assert!(matches!(app.mode(), Mode::Visual(_)), "c does nothing");
}

#[test]
fn raw_view_rows_are_source_lines_blank_ones_too() {
    let repo = Repo::new();
    let mut app = repo.open("doc.md", DOC);
    keys(&mut app, "gR");
    assert!(app.raw());
    goto(&mut app, 4);
    keys(&mut app, "cc");
    type_and_save(&mut app, "raw beta");
    goto(&mut app, 3);
    assert_eq!(row_text(&app, 3), "");
    keys(&mut app, "cc");
    type_and_save(&mut app, "raw blank");
    let cs = repo.comments();
    let got: Vec<_> = cs.iter().map(|c| (c.lines, c.excerpt.as_str())).collect();
    assert_eq!(got, [((4, 4), ""), ((5, 5), "beta")]);
    assert_eq!(gutter_rows(&app), vec![3, 4]);
}

#[test]
fn a_symlinked_directory_resolves_to_the_real_path() {
    let repo = Repo::new();
    repo.write("notes/doc.md", DOC);
    let link = repo.root.parent().unwrap().join("link");
    std::os::unix::fs::symlink(repo.root.join("notes"), &link).unwrap();
    let mut app = repo.app(StartTarget::File(link.join("doc.md")), config(None));
    goto_text(&mut app, "beta");
    keys(&mut app, "cc");
    type_and_save(&mut app, "via link");
    let (.., path) = only(&repo);
    assert_eq!(path, "notes/doc.md");
    assert_eq!(app.review_count(), 1, "marker lookup uses the same path");
    assert_eq!(app.review_gutter(), 1);
}

#[test]
fn a_tmp_path_resolves_through_its_canonical_form() {
    // On macOS /tmp is a symlink to /private/tmp.
    let dir = tempfile::Builder::new().tempdir_in("/tmp").unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::write(root.join("doc.md"), DOC).unwrap();
    let cache = dir.path().join("cache");
    let mut app = App::new(
        StartOptions {
            target: StartTarget::File(root.join("doc.md")),
            tree_root: root.clone(),
            config: config(None),
            review_cache: Some(cache.clone()),
        },
        (COLS, ROWS),
    )
    .unwrap();
    keys(&mut app, "cc");
    type_and_save(&mut app, "tmp");
    let r = Review::open(&cache, &root).unwrap();
    assert_eq!(r.comments().len(), 1);
    assert_eq!(r.comments()[0].path, "doc.md");
    assert_eq!(
        app.review_root(),
        Some(root.canonicalize().unwrap().as_path())
    );
    assert_eq!(app.review_count(), 1);
}

/// What the fake editor does with its file.
#[derive(Clone, Copy)]
enum Fake {
    Write(&'static str),
    Fail,
}

fn with_fake_editor(app: App, fake: Fake) -> (App, Rc<RefCell<Vec<LaunchCommand>>>) {
    let calls: Rc<RefCell<Vec<LaunchCommand>>> = Rc::default();
    let rec = calls.clone();
    let app = app
        .with_env(|k| (k == "EDITOR").then(|| "myed -w".to_string()))
        .with_runner(move |cmd| {
            rec.borrow_mut().push(cmd.clone());
            let file = Path::new(cmd.argv.last().unwrap());
            match fake {
                Fake::Write(s) => {
                    std::fs::write(file, s).unwrap();
                    Ok(Exit::Code(0))
                }
                Fake::Fail => Ok(Exit::Code(3)),
            }
        });
    (app, calls)
}

#[test]
fn ctrl_e_edits_the_comment_in_the_editor_and_saves_it() {
    let repo = Repo::new();
    let app = repo.open("doc.md", DOC);
    let (mut app, calls) = with_fake_editor(app, Fake::Write("longer\nbody\n"));
    goto_text(&mut app, "gamma");
    keys(&mut app, "ccstart");
    send(&mut app, ctrl('e'));
    assert_eq!(app.mode(), Mode::Normal);
    assert!(matches!(
        app.pending_effect(),
        Some(Effect::EditComment { initial, lines: (7, 7), .. }) if initial == "start"
    ));
    app.run_pending_effect();
    let argv = &calls.borrow()[0].argv;
    assert_eq!(&argv[..2], ["myed", "-w"]);
    assert!(!Path::new(&argv[2]).exists(), "temp file removed");
    let (lines, excerpt, body, _) = only(&repo);
    assert_eq!((lines, excerpt.as_str()), ((7, 7), "gamma"));
    assert_eq!(body, "longer\nbody");
    assert_eq!(app.status(), "Comment added (1 in batch)");
    assert_eq!(app.review_count(), 1);
}

#[test]
fn ctrl_e_seeds_the_file_with_the_typed_text() {
    let repo = Repo::new();
    let app = repo.open("doc.md", DOC);
    let seen: Rc<RefCell<String>> = Rc::default();
    let s = seen.clone();
    let mut app = app.with_runner(move |cmd| {
        *s.borrow_mut() = std::fs::read_to_string(cmd.argv.last().unwrap()).unwrap();
        Ok(Exit::Code(0))
    });
    keys(&mut app, "cckeep me");
    send(&mut app, ctrl('e'));
    app.run_pending_effect();
    assert_eq!(*seen.borrow(), "keep me");
    assert_eq!(only(&repo).2, "keep me");
}

#[test]
fn ctrl_e_with_an_emptied_file_saves_nothing() {
    let repo = Repo::new();
    let app = repo.open("doc.md", DOC);
    let (mut app, _c) = with_fake_editor(app, Fake::Write("  \n"));
    keys(&mut app, "ccdraft");
    send(&mut app, ctrl('e'));
    app.run_pending_effect();
    assert_eq!(app.status(), EMPTY_COMMENT);
    assert!(repo.comments().is_empty());
}

#[test]
fn ctrl_e_with_a_failing_editor_saves_nothing() {
    let repo = Repo::new();
    let app = repo.open("doc.md", DOC);
    let (mut app, _c) = with_fake_editor(app, Fake::Fail);
    keys(&mut app, "ccdraft");
    send(&mut app, ctrl('e'));
    app.run_pending_effect();
    assert_eq!(
        app.status(),
        "editor: exited with status 3; comment not saved"
    );
    assert!(repo.comments().is_empty());
}

#[test]
fn a_new_page_in_another_repo_switches_batches() {
    let a = Repo::new();
    let b = Repo::new();
    a.add("doc.md", 5, 5);
    let mut app = a.open("doc.md", DOC);
    assert_eq!(app.review_count(), 1);
    let other = b.write("doc.md", DOC);
    app.open_file(&other).unwrap();
    assert_eq!(app.review_count(), 0);
    assert_eq!(app.review_gutter(), 0);
    keys(&mut app, "cc");
    type_and_save(&mut app, "b");
    // b's batch lives in a's cache dir (one dir for every root).
    assert_eq!(Review::open(&a.cache, &b.root).unwrap().comments().len(), 1);
    assert_eq!(a.comments().len(), 1);
}
