//! Comment mode and review markers (debrief spec §2): capture with `cc` and
//! visual `c`, the prompt and `C-e`, and the gutter, tree, status count and
//! `]r` / `[r` driven by the debrief-review batch. Every test uses a temp
//! repo (`.git` dir) and a temp cache dir; nothing sleeps.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use debrief_review::{Comment, Rev, Review, Side};
use ramble::app::{
    App, AppEvent, CommentBox, EMPTY_COMMENT, Effect, Exit, LaunchCommand, Mode, NO_MORE_REVIEW,
    NO_REVIEW, NO_SOURCE_LINE, REVIEW_POLL, StartOptions, StartTarget,
};
use ramble::config::{Config, SidebarMode, SidebarShow};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
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
                mdroots: ramble::app::MdrootsOptions::memory(),
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

/// The comment box's title and text as one line: `comment [ISSUE] L5: why`.
fn prompt(app: &App) -> String {
    format!(
        "{}: {}",
        app.comment_title().unwrap().trim(),
        app.comment_text().unwrap()
    )
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
        screen(&app).iter().any(|l| l.contains("╭ comment L5 ")),
        "box drawn"
    );
    keys(&mut app, "too vaguex");
    send(&mut app, key(KeyCode::Backspace));
    assert_eq!(prompt(&app), "comment L5: too vague");
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
fn a_multi_line_paste_keeps_its_newlines_in_the_box_and_runs_no_keys() {
    let repo = Repo::new();
    let mut app = repo.open("doc.md", DOC);
    goto_text(&mut app, "beta");
    keys(&mut app, "cc");
    let cursor = app.cursor();
    app.event(AppEvent::Paste("first\r\nsecond\tthird\rjj".into()));
    assert_eq!(app.mode(), Mode::Comment, "the newline was not Enter");
    assert_eq!(app.comment_text(), Some("first\nsecond third\njj"));
    assert_eq!(app.cursor(), cursor, "no keys replayed");
    assert!(repo.comments().is_empty(), "nothing saved yet");
    send(&mut app, key(KeyCode::Enter));
    assert_eq!(only(&repo).2, "first\nsecond third\njj");
}

fn alt(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT)
}

#[test]
fn the_prompt_edits_at_the_cursor() {
    let repo = Repo::new();
    let mut app = repo.open("doc.md", DOC);
    goto_text(&mut app, "beta");
    keys(&mut app, "cc");
    keys(&mut app, "why this here");
    send(&mut app, alt('b'));
    send(&mut app, alt('b'));
    keys(&mut app, "is ");
    assert_eq!(prompt(&app), "comment L5: why is this here");
    send(&mut app, ctrl('a'));
    send(&mut app, key(KeyCode::Delete));
    keys(&mut app, "W");
    send(&mut app, key(KeyCode::End));
    send(&mut app, ctrl('w'));
    keys(&mut app, "there?");
    assert_eq!(prompt(&app), "comment L5: Why is this there?");
    send(&mut app, key(KeyCode::Home));
    send(&mut app, alt('f'));
    send(&mut app, key(KeyCode::Right));
    send(&mut app, key(KeyCode::Right));
    send(&mut app, ctrl('u'));
    send(&mut app, key(KeyCode::Left));
    send(&mut app, key(KeyCode::Backspace));
    assert_eq!(prompt(&app), "comment L5:  this there?");
    send(&mut app, key(KeyCode::Delete));
    // Pasted text goes in at the cursor too.
    app.event(AppEvent::Paste("is ".into()));
    assert_eq!(prompt(&app), "comment L5: is this there?");
    send(&mut app, key(KeyCode::Enter));
    assert_eq!(only(&repo).2, "is this there?");
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
    assert!(app.comment_text().is_none());
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
fn comments_here_match_a_symlinked_file_alias_both_ways() {
    let repo = Repo::new();
    let real = repo.write("real.md", DOC);
    let alias = repo.root.join("alias.md");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    repo.add("alias.md", 5, 5);
    let app = repo.app(StartTarget::File(real), config(None));
    assert_eq!(app.review_count(), 1);
    assert_eq!(app.review_comments_here().len(), 1);

    let repo = Repo::new();
    let real = repo.write("real.md", DOC);
    let alias = repo.root.join("alias.md");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    repo.add("real.md", 5, 5);
    repo.add("other.md", 5, 5);
    let app = repo.app(StartTarget::File(alias), config(None));
    assert_eq!(app.review_count(), 1);
    assert_eq!(app.review_comments_here().len(), 1);
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
            mdroots: ramble::app::MdrootsOptions::memory(),
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
fn tab_cycles_the_kind_through_untyped_and_saves_it() {
    let repo = Repo::new();
    let mut app = repo.open("doc.md", DOC);
    goto_text(&mut app, "beta");
    keys(&mut app, "ccwhy");
    assert_eq!(prompt(&app), "comment L5: why", "starts untyped");
    let tab = key(KeyCode::Tab);
    let back = KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT);
    let mut labels = Vec::new();
    for _ in 0..5 {
        send(&mut app, tab);
        labels.push(prompt(&app));
    }
    assert_eq!(
        labels,
        [
            "comment [ISSUE] L5: why",
            "comment [SUGGESTION] L5: why",
            "comment [QUESTION] L5: why",
            "comment [NIT] L5: why",
            "comment L5: why",
        ]
    );
    send(&mut app, back);
    assert_eq!(prompt(&app), "comment [NIT] L5: why");
    send(&mut app, back);
    send(&mut app, key(KeyCode::Char('\t')));
    assert_eq!(prompt(&app), "comment [NIT] L5: why");
    send(&mut app, back);
    assert!(
        screen(&app)
            .iter()
            .any(|l| l.contains("╭ comment [QUESTION] L5 ")),
        "the title shows the kind"
    );
    send(&mut app, key(KeyCode::Enter));
    let cs = repo.comments();
    assert_eq!(cs[0].kind.as_deref(), Some("question"));
    assert_eq!(cs[0].body, "why");
    keys(&mut app, "ccplain");
    assert_eq!(prompt(&app), "comment L5: plain", "next one untyped");
    send(&mut app, key(KeyCode::Enter));
    assert_eq!(repo.comments()[1].kind, None);
}

#[test]
fn configured_kinds_replace_the_builtins_and_none_leaves_tab_inert() {
    let repo = Repo::new();
    let path = repo.write("doc.md", DOC);
    let mut c = config(None);
    c.review.kinds = vec![debrief_review::Kind {
        id: "praise".into(),
        definition: None,
    }];
    let mut app = repo.app(StartTarget::File(path.clone()), c);
    keys(&mut app, "ccx");
    send(&mut app, key(KeyCode::Tab));
    assert_eq!(prompt(&app), "comment [PRAISE] L1: x");
    send(&mut app, key(KeyCode::Tab));
    assert_eq!(prompt(&app), "comment L1: x");
    send(&mut app, key(KeyCode::Esc));
    let mut c = config(None);
    c.review.kinds = vec![];
    let mut app = repo.app(StartTarget::File(path), c);
    keys(&mut app, "ccx");
    send(&mut app, key(KeyCode::Tab));
    send(
        &mut app,
        KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
    );
    assert_eq!(prompt(&app), "comment L1: x");
    assert_eq!(app.mode(), Mode::Comment);
    send(&mut app, key(KeyCode::Enter));
    assert_eq!(repo.comments()[0].kind, None);
}

#[test]
fn ctrl_e_keeps_the_kind_picked_in_the_prompt() {
    let repo = Repo::new();
    let app = repo.open("doc.md", DOC);
    let (mut app, _calls) = with_fake_editor(app, Fake::Write("edited\n"));
    keys(&mut app, "ccx");
    send(&mut app, key(KeyCode::Tab));
    send(&mut app, ctrl('e'));
    app.run_pending_effect();
    let cs = repo.comments();
    assert_eq!(
        (cs[0].body.as_str(), cs[0].kind.as_deref()),
        ("edited", Some("issue"))
    );
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

fn status_line(app: &App) -> String {
    screen(app)[ROWS as usize - 1].clone()
}

#[test]
fn the_status_line_shows_the_comment_on_the_cursor_line() {
    let repo = Repo::new();
    // A two-line body comes from the editor; the prompt is one line.
    let mut app = repo.open("doc.md", DOC).with_runner(|cmd| {
        std::fs::write(cmd.argv.last().unwrap(), "first line\nsecond").unwrap();
        Ok(Exit::Code(0))
    });
    goto_text(&mut app, "beta");
    keys(&mut app, "cc");
    send(&mut app, ctrl('e'));
    app.run_pending_effect();
    // The action's own message wins until the cursor moves.
    assert_eq!(app.status(), "Comment added (1 in batch)");
    assert!(status_line(&app).contains("Comment added"));
    keys(&mut app, "j");
    assert_eq!(app.status(), "", "moving clears the message");
    assert_eq!(app.review_preview(), None);
    assert!(!status_line(&app).contains('●'), "{}", status_line(&app));
    keys(&mut app, "k");
    assert_eq!(app.review_preview().as_deref(), Some("● first line"));
    assert!(
        status_line(&app).contains("  ● first line "),
        "{}",
        status_line(&app)
    );
}

#[test]
fn a_message_set_by_a_motion_stays_and_hides_the_preview() {
    let repo = Repo::new();
    repo.add("doc.md", 9, 9);
    let mut app = repo.open("doc.md", DOC);
    goto_text(&mut app, "delta");
    keys(&mut app, "]r");
    assert_eq!(app.status(), NO_MORE_REVIEW);
    assert!(status_line(&app).contains(NO_MORE_REVIEW));
    assert!(!status_line(&app).contains("● note"));
}

#[test]
fn a_long_comment_preview_is_cut_with_an_ellipsis() {
    let repo = Repo::new();
    let body = "word ".repeat(30);
    repo.review()
        .add(Comment::on_file("doc.md", 5..=5, "beta", body.trim()))
        .unwrap();
    let mut app = repo.open("doc.md", DOC);
    goto_text(&mut app, "beta");
    let row = status_line(&app);
    assert!(row.contains("● word word"), "{row}");
    assert!(row.contains('…'), "{row}");
    assert!(row.trim_end().ends_with("← 0"), "right group kept: {row}");
}

#[test]
fn k_on_a_commented_line_shows_its_comments() {
    let repo = Repo::new();
    let mut r = repo.review();
    r.add(Comment::on_file("doc.md", 5..=7, "x", "spans three\nlines"))
        .unwrap();
    r.add(Comment::on_file("doc.md", 5..=5, "beta", "just beta"))
        .unwrap();
    let mut app = repo.open("doc.md", DOC);
    goto_text(&mut app, "beta");
    keys(&mut app, "K");
    assert_eq!(
        app.hover_popup(),
        Some("● lines 5-7\nspans three\nlines\n\n● line 5\njust beta")
    );
    assert!(screen(&app).iter().any(|l| l.contains("● lines 5-7")));
    send(&mut app, key(KeyCode::Esc));
    assert_eq!(app.hover_popup(), None);
    goto_text(&mut app, "gamma");
    keys(&mut app, "K");
    assert_eq!(app.hover_popup(), Some("● lines 5-7\nspans three\nlines"));
    send(&mut app, key(KeyCode::Esc));
    // No comment: today's link hover (no link here).
    goto_text(&mut app, "alpha");
    keys(&mut app, "K");
    assert_eq!(app.hover_popup(), None);
    assert_eq!(app.status(), "No link under cursor");
}

fn leader(app: &mut App, s: &str) {
    keys(app, &format!(" {s}"));
}

fn picker_labels(app: &App) -> Vec<String> {
    let p = app.picker().expect("picker open");
    p.items
        .iter()
        .map(|i| format!("{}  {}", i.label, i.detail))
        .collect()
}

fn diff_comment(path: &str, commit: &str, body: &str) -> Comment {
    let rev = Rev {
        commit: commit.into(),
        change: None,
        summary: None,
    };
    Comment::on_diff(path, rev, Side::New, 2..=3, "x", body)
}

#[test]
fn leader_rl_lists_the_batch_sorted_and_enter_jumps() {
    let repo = Repo::new();
    repo.write("b.md", DOC);
    let mut r = repo.review();
    r.add(Comment::on_file("doc.md", 9..=9, "delta", "on delta"))
        .unwrap();
    r.add(Comment::on_file("b.md", 5..=7, "beta", "in b\nmore"))
        .unwrap();
    r.add(Comment::on_file("doc.md", 3..=3, "alpha", "on alpha"))
        .unwrap();
    r.add(diff_comment("doc.md", "0123456789abcdef", "on a diff"))
        .unwrap();
    let mut app = repo.open("doc.md", DOC);
    leader(&mut app, "rl");
    assert_eq!(app.mode(), Mode::Picker);
    assert_eq!(app.picker().unwrap().title, "Review comments");
    assert_eq!(
        picker_labels(&app),
        [
            "b.md:5-7  in b",
            "doc.md:2-3 @01234567  on a diff",
            "doc.md:3  on alpha",
            "doc.md:9  on delta",
        ]
    );
    assert!(
        screen(&app)
            .iter()
            .any(|l| l.contains("C-d remove  C-e edit")),
        "footer drawn"
    );
    // Enter on doc.md:9 (same page): the cursor lands on delta's row.
    for _ in 0..3 {
        send(&mut app, ctrl('n'));
    }
    send(&mut app, key(KeyCode::Enter));
    assert_eq!(app.mode(), Mode::Normal);
    assert_eq!(app.cursor().row, row_of(&app, "delta"));
    // A diff comment only says where it lives.
    leader(&mut app, "rl");
    send(&mut app, ctrl('n'));
    send(&mut app, key(KeyCode::Enter));
    assert_eq!(app.status(), "Comment on 01234567 — open it in debrief");
    assert!(
        app.page()
            .unwrap()
            .path
            .as_ref()
            .unwrap()
            .ends_with("doc.md")
    );
    // Another file: opened, cursor on the first row of line 5.
    leader(&mut app, "rl");
    send(&mut app, key(KeyCode::Enter));
    assert!(app.page().unwrap().path.as_ref().unwrap().ends_with("b.md"));
    assert_eq!(app.cursor().row, row_of(&app, "beta"));
    assert_eq!(app.history_depth(), 1, "opened like a followed link");
}

#[test]
fn the_list_filters_on_typed_text_and_c_d_removes() {
    let repo = Repo::new();
    let mut r = repo.review();
    r.add(Comment::on_file("doc.md", 3..=3, "alpha", "first"))
        .unwrap();
    r.add(Comment::on_file("doc.md", 5..=5, "beta", "second"))
        .unwrap();
    let mut app = repo.open("doc.md", DOC);
    assert_eq!(app.review_count(), 2);
    leader(&mut app, "rl");
    // `d` and `e` are typed text, not commands.
    keys(&mut app, "sec");
    assert_eq!(picker_labels(&app), ["doc.md:5  second"]);
    send(&mut app, ctrl('d'));
    assert_eq!(app.status(), "Comment removed (1 in batch)");
    assert_eq!(app.mode(), Mode::Picker, "stays open");
    assert!(picker_labels(&app).is_empty());
    for _ in 0..3 {
        send(&mut app, key(KeyCode::Backspace));
    }
    assert_eq!(picker_labels(&app), ["doc.md:3  first"]);
    assert_eq!(repo.comments().len(), 1);
    assert_eq!(repo.comments()[0].body, "first");
    assert_eq!(app.review_count(), 1, "markers follow");
    send(&mut app, key(KeyCode::Esc));
    assert_eq!(app.mode(), Mode::Normal);
}

#[test]
fn c_e_in_the_list_edits_the_body_in_the_editor() {
    let repo = Repo::new();
    repo.add("doc.md", 5, 5);
    let app = repo.open("doc.md", DOC);
    let (mut app, calls) = with_fake_editor(app, Fake::Write("better\n"));
    leader(&mut app, "rl");
    send(&mut app, ctrl('e'));
    assert_eq!(
        app.pending_effect(),
        Some(&Effect::EditReviewComment {
            id: repo.comments()[0].id,
            initial: "note".into(),
        })
    );
    app.run_pending_effect();
    assert_eq!(calls.borrow()[0].argv[..2], ["myed", "-w"]);
    assert_eq!(repo.comments()[0].body, "better");
    assert_eq!(app.status(), "Comment edited");
    assert_eq!(picker_labels(&app), ["doc.md:5  better"]);

    let (mut app, _) = with_fake_editor(app, Fake::Fail);
    send(&mut app, ctrl('e'));
    app.run_pending_effect();
    assert_eq!(
        app.status(),
        "editor: exited with status 3; comment not changed"
    );
    assert_eq!(repo.comments()[0].body, "better");
}

#[test]
fn leader_rr_with_an_empty_batch_says_so() {
    let repo = Repo::new();
    let mut app = repo.open("doc.md", DOC);
    leader(&mut app, "rr");
    assert_eq!(app.status(), "No comments to send");
    assert!(app.pending_effect().is_none());
}

fn send_config(command: &[&str]) -> Config {
    let mut c = config(None);
    c.send.command = command.iter().map(|s| s.to_string()).collect();
    c
}

#[test]
fn leader_rr_pipes_the_markdown_to_the_command_and_clears() {
    let repo = Repo::new();
    let out = repo.root.parent().unwrap().join("out.md");
    repo.add("doc.md", 5, 5);
    repo.add("doc.md", 9, 9);
    let path = repo.write("doc.md", DOC);
    let script = format!("cat > '{}'", out.display());
    let mut config = send_config(&["sh", "-c", &script]);
    config.send.preamble = Some("Fix these.".into());
    let mut app = repo.app(StartTarget::File(path), config);
    leader(&mut app, "rr");
    assert_eq!(app.pending_effect(), Some(&Effect::SendReview));
    app.run_pending_effect();
    assert_eq!(app.status(), "Review sent (2 comments)");
    let md = std::fs::read_to_string(&out).unwrap();
    assert!(md.starts_with("# Review feedback\n\nFix these.\n"), "{md}");
    assert!(md.contains("## Item 1"), "{md}");
    assert!(md.contains("## Item 2"), "{md}");
    assert!(md.contains("note"), "{md}");
    assert!(repo.comments().is_empty(), "batch cleared");
    assert_eq!(app.review_count(), 0);
    assert!(app.take_terminal_output().is_empty(), "no clipboard");
}

#[test]
fn leader_rr_falls_back_to_the_clipboard_when_the_command_fails() {
    let repo = Repo::new();
    repo.add("doc.md", 5, 5);
    let path = repo.write("doc.md", DOC);
    let mut app = repo.app(StartTarget::File(path), send_config(&["false"]));
    let md = repo
        .review()
        .to_markdown(None, &debrief_review::builtin_kinds());
    leader(&mut app, "rr");
    app.run_pending_effect();
    assert_eq!(app.status(), "Review copied to clipboard (1 comments)");
    assert_eq!(
        app.take_terminal_output(),
        ramble::app::osc52(&md).into_bytes()
    );
    assert!(repo.comments().is_empty(), "batch cleared");
}

#[test]
fn leader_rr_keeps_the_batch_when_the_clipboard_fails() {
    struct Broken;
    impl ramble::app::Clipboard for Broken {
        fn copy(&mut self, _: &str) -> anyhow::Result<()> {
            anyhow::bail!("no terminal")
        }
    }
    let repo = Repo::new();
    repo.add("doc.md", 5, 5);
    let mut app = repo.open("doc.md", DOC).with_clipboard(Broken);
    leader(&mut app, "rr");
    app.run_pending_effect();
    assert_eq!(app.status(), "Review not sent: no terminal");
    assert_eq!(repo.comments().len(), 1, "batch kept");
    assert_eq!(app.review_count(), 1);
}

#[test]
fn leader_rr_keeps_a_comment_added_during_the_hand_back() {
    // Another handle adds a comment after ramble's last poll, while the
    // review is being handed back: it was not sent, so it stays.
    struct AddsOne(PathBuf, PathBuf, Rc<RefCell<String>>);
    impl ramble::app::Clipboard for AddsOne {
        fn copy(&mut self, text: &str) -> anyhow::Result<()> {
            *self.2.borrow_mut() = text.to_string();
            Review::open(&self.0, &self.1)?.add(Comment::on_file("doc.md", 9..=9, "x", "late"))?;
            Ok(())
        }
    }
    let repo = Repo::new();
    repo.add("doc.md", 5, 5);
    let path = repo.write("doc.md", DOC);
    let copied = Rc::new(RefCell::new(String::new()));
    let clip = AddsOne(repo.cache.clone(), repo.root.clone(), copied.clone());
    let mut app = repo
        .app(StartTarget::File(path), send_config(&["false"]))
        .with_clipboard(clip);
    leader(&mut app, "rr");
    app.run_pending_effect();
    assert_eq!(app.status(), "Review copied to clipboard (1 comments)");
    assert!(!copied.borrow().contains("late"), "not sent");
    let left = repo.comments();
    assert_eq!(left.len(), 1, "{left:?}");
    assert_eq!(left[0].body, "late");
    assert_eq!(app.review_count(), 1);
}

#[test]
fn leader_rr_sends_a_comment_added_since_the_last_poll() {
    let repo = Repo::new();
    repo.add("doc.md", 5, 5);
    let path = repo.write("doc.md", DOC);
    let copied = Rc::new(RefCell::new(String::new()));
    struct Rec(Rc<RefCell<String>>);
    impl ramble::app::Clipboard for Rec {
        fn copy(&mut self, text: &str) -> anyhow::Result<()> {
            *self.0.borrow_mut() = text.to_string();
            Ok(())
        }
    }
    let mut app = repo
        .app(StartTarget::File(path), send_config(&["false"]))
        .with_clipboard(Rec(copied.clone()));
    repo.review()
        .add(Comment::on_file("doc.md", 9..=9, "x", "unpolled"))
        .unwrap();
    leader(&mut app, "rr");
    app.run_pending_effect();
    assert_eq!(app.status(), "Review copied to clipboard (2 comments)");
    assert!(copied.borrow().contains("unpolled"));
    assert!(repo.comments().is_empty());
}

#[cfg(unix)]
#[test]
fn leader_rr_says_so_when_the_batch_cannot_be_cleared() {
    use std::os::unix::fs::PermissionsExt;
    let repo = Repo::new();
    repo.add("doc.md", 5, 5);
    let mut app = repo.open("doc.md", DOC);
    let reviews = repo.cache.join("reviews");
    let perm = |mode| std::fs::set_permissions(&reviews, std::fs::Permissions::from_mode(mode));
    perm(0o555).unwrap();
    leader(&mut app, "rr");
    app.run_pending_effect();
    perm(0o755).unwrap();
    let status = app.status().to_string();
    assert!(
        status.starts_with("Review copied, but the batch wasn't cleared: ")
            && status.ends_with(" (sending again repeats it)"),
        "{status}"
    );
    assert_eq!(repo.comments().len(), 1, "batch kept");
}

#[test]
fn review_keys_are_unbound_with_review_disabled() {
    let repo = Repo::new();
    let path = repo.write("doc.md", DOC);
    let mut c = config(None);
    c.review.enabled = false;
    let mut app = repo.app(StartTarget::File(path), c);
    leader(&mut app, "r");
    assert_eq!(app.status(), "No mapping", "<leader>r is no prefix");
    leader(&mut app, "rl");
    assert!(app.picker().is_none());
}

#[test]
fn kinds_show_in_the_list_the_k_popup_and_the_status_preview() {
    let repo = Repo::new();
    let mut r = repo.review();
    let kind = |k: &str| Some(k.to_string());
    r.add(Comment::on_file("doc.md", 5..=5, "beta", "fix this").with_kind(kind("issue")))
        .unwrap();
    r.add(Comment::on_file("doc.md", 5..=5, "beta", "plain"))
        .unwrap();
    r.add(diff_comment("doc.md", "0123456789abcdef", "why?").with_kind(kind("question")))
        .unwrap();
    let mut app = repo.open("doc.md", DOC);
    goto_text(&mut app, "beta");
    assert_eq!(app.review_preview().as_deref(), Some("● [ISSUE] fix this"));
    assert!(status_line(&app).contains("● [ISSUE] fix this"));
    keys(&mut app, "K");
    assert_eq!(
        app.hover_popup(),
        Some("● line 5 [ISSUE]\nfix this\n\n● line 5\nplain")
    );
    send(&mut app, key(KeyCode::Esc));
    leader(&mut app, "rl");
    assert_eq!(
        picker_labels(&app),
        [
            "doc.md:2-3 @01234567  [QUESTION] why?",
            "doc.md:5  [ISSUE] fix this",
            "doc.md:5  plain",
        ]
    );
}

#[test]
fn leader_rr_labels_typed_items_with_a_legend_of_the_configured_kinds() {
    let repo = Repo::new();
    let out = repo.root.parent().unwrap().join("out.md");
    let mut r = repo.review();
    r.add(Comment::on_file("doc.md", 5..=5, "beta", "keep it").with_kind(Some("praise".into())))
        .unwrap();
    r.add(Comment::on_file("doc.md", 9..=9, "delta", "untyped"))
        .unwrap();
    let path = repo.write("doc.md", DOC);
    let script = format!("cat > '{}'", out.display());
    let mut config = send_config(&["sh", "-c", &script]);
    config.review.kinds = vec![
        debrief_review::Kind {
            id: "issue".into(),
            definition: Some("Fix it.".into()),
        },
        debrief_review::Kind {
            id: "praise".into(),
            definition: Some("Leave it as it is.".into()),
        },
    ];
    let mut app = repo.app(StartTarget::File(path), config);
    leader(&mut app, "rr");
    app.run_pending_effect();
    let md = std::fs::read_to_string(&out).unwrap();
    assert!(md.contains("## Item 1 [PRAISE]\n"), "{md}");
    assert!(md.contains("## Item 2\n"), "{md}");
    assert!(md.contains("- PRAISE: Leave it as it is.\n"), "{md}");
    assert!(!md.contains("ISSUE"), "only used kinds in the legend: {md}");
}

// The comment box.

/// Draw at `w`x`h`; the screen's lines, the box (as the app places it,
/// in the content pane: the whole width with no sidebar) and the terminal
/// cursor.
fn draw_box(app: &App, w: u16, h: u16) -> (Vec<String>, Option<CommentBox>, (u16, u16)) {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| ramble::ui::draw(f, app)).unwrap();
    let buf = term.backend().buffer().clone();
    let lines = (0..h)
        .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect();
    let pos = term.get_cursor_position().unwrap();
    (
        lines,
        app.comment_box(Rect::new(0, 0, w, h - 1)),
        (pos.x, pos.y),
    )
}

/// `n` paragraphs `p1`..`pn`, a blank line apart.
fn paras(n: usize) -> String {
    (1..=n).map(|i| format!("p{i}\n\n")).collect()
}

#[test]
fn the_box_opens_below_a_selection_in_the_middle_and_keeps_it_highlighted() {
    let repo = Repo::new();
    let mut app = repo.open("doc.md", DOC);
    goto_text(&mut app, "beta");
    keys(&mut app, "Vjjc");
    assert_eq!(app.mode(), Mode::Comment);
    let (ya, yb) = (row_of(&app, "beta"), row_of(&app, "gamma"));
    assert_eq!(app.comment_rows(), Some((ya, yb)));
    let (lines, bx, cur) = draw_box(&app, COLS, ROWS);
    let bx = bx.expect("box open");
    assert_eq!(bx.rect.y as usize, yb - app.scroll() + 1, "right below");
    assert_eq!(
        (bx.rect.x, bx.rect.width),
        (1, COLS - 2),
        "pane less a margin"
    );
    assert_eq!(bx.rect.height, 3);
    assert_eq!(bx.title, " comment L5-7 ");
    assert!(lines[bx.rect.y as usize].contains("╭ comment L5-7 "));
    assert!(lines[bx.rect.y as usize + 2].contains("Enter save"));
    assert_eq!(cur, (bx.rect.x + 1, bx.rect.y + 1), "cursor in the box");
    // The rows stay drawn, with the selection background.
    assert!(lines[ya - app.scroll()].contains("beta"));
    let mut term = Terminal::new(TestBackend::new(COLS, ROWS)).unwrap();
    term.draw(|f| ramble::ui::draw(f, &app)).unwrap();
    let buf = term.backend().buffer();
    let bg = |y: usize| buf[(5, (y - app.scroll()) as u16)].bg;
    assert_eq!(bg(ya), bg(yb));
    assert_eq!(bg(ya), bg(ya + 1), "the blank row between too");
    assert_ne!(bg(ya), bg(row_of(&app, "Title")), "highlighted");
    keys(&mut app, "ab");
    let (lines, _, cur) = draw_box(&app, COLS, ROWS);
    assert_eq!(cur, (bx.rect.x + 3, bx.rect.y + 1));
    assert!(lines[bx.rect.y as usize + 1].contains("│ab"));
}

#[test]
fn the_box_opens_above_rows_at_the_bottom() {
    let repo = Repo::new();
    let mut app = repo.open("doc.md", &paras(40));
    keys(&mut app, "G");
    let r = row_of(&app, "p40");
    goto(&mut app, r);
    keys(&mut app, "cc");
    assert_eq!(app.mode(), Mode::Comment, "{}", app.status());
    let (lines, bx, _) = draw_box(&app, COLS, ROWS);
    let bx = bx.unwrap();
    let y = (r - app.scroll()) as u16;
    assert!(y >= ROWS - 3, "the row is at the bottom: {y}");
    assert_eq!(bx.rect.bottom(), y, "directly above the row");
    assert!(lines[y as usize].contains("p40"), "not covered");
}

#[test]
fn a_multi_line_body_is_saved_with_its_newlines_and_the_box_grows() {
    let repo = Repo::new();
    let mut app = repo.open("doc.md", DOC);
    goto_text(&mut app, "beta");
    keys(&mut app, "ccone");
    send(&mut app, ctrl('j'));
    keys(&mut app, "two");
    send(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
    keys(&mut app, "three");
    let (_, bx, cur) = draw_box(&app, COLS, ROWS);
    let bx = bx.unwrap();
    assert_eq!(bx.lines, ["one", "two", "three"]);
    assert_eq!(bx.rect.height, 5);
    assert_eq!(cur, (bx.rect.x + 6, bx.rect.y + 3));
    send(&mut app, key(KeyCode::Up));
    keys(&mut app, "!");
    assert_eq!(app.comment_text(), Some("one\ntwo!\nthree"));
    send(&mut app, key(KeyCode::Down));
    for i in 0..9 {
        send(&mut app, key(KeyCode::End));
        send(&mut app, ctrl('j'));
        keys(&mut app, &format!("l{i}"));
    }
    let (_, bx, cur) = draw_box(&app, COLS, ROWS);
    let bx = bx.unwrap();
    assert_eq!(bx.rect.height, 10, "8 text rows, then it scrolls");
    assert_eq!(bx.lines.last().map(String::as_str), Some("l8"));
    assert_eq!(cur.1, bx.rect.y + 8);
    send(&mut app, key(KeyCode::Enter));
    let body = only(&repo).2;
    assert!(body.starts_with("one\ntwo!\nthree\nl0\n"), "{body:?}");
    assert_eq!(body.lines().count(), 12);
}

#[test]
fn esc_closes_the_box_and_drops_the_highlight() {
    let repo = Repo::new();
    let mut app = repo.open("doc.md", DOC);
    goto_text(&mut app, "beta");
    keys(&mut app, "ccdraft");
    send(&mut app, key(KeyCode::Esc));
    let (lines, bx, _) = draw_box(&app, COLS, ROWS);
    assert!(bx.is_none());
    assert!(app.comment_rows().is_none());
    assert!(!lines.iter().any(|l| l.contains("comment L")));
    assert!(repo.comments().is_empty());
}

#[test]
fn the_box_follows_a_resize_and_a_reflow() {
    let repo = Repo::new();
    let long = "word ".repeat(30);
    let doc = format!("{}{}\n\nafter\n", paras(3), long.trim());
    let mut app = repo.open("doc.md", &doc);
    goto_text(&mut app, "after");
    keys(&mut app, "cc");
    keys(&mut app, &"note ".repeat(15));
    let (_, wide, _) = draw_box(&app, COLS, ROWS);
    let wide = wide.unwrap();
    app.event(AppEvent::Resize(30, 12));
    let r = row_of(&app, "after");
    assert_eq!(app.comment_rows(), Some((r, r)), "follows the reflow");
    let (lines, narrow, _) = draw_box(&app, 30, 12);
    let narrow = narrow.unwrap();
    assert_eq!(narrow.rect.width, 28);
    assert!(narrow.rect.height > wide.rect.height, "rewrapped taller");
    let y = (r - app.scroll()) as u16;
    assert!(
        narrow.rect.bottom() <= y || narrow.rect.y > y,
        "not over the row: {:?} row {y}",
        narrow.rect
    );
    assert!(lines[y as usize].contains("after"));
    assert!(narrow.rect.bottom() <= 11, "inside the pane");
    send(&mut app, key(KeyCode::Enter));
    assert_eq!(only(&repo).2, "note ".repeat(15).trim());
}

/// `n` source lines `line1`..`linen`, viewed raw (a row per line).
fn raw_lines(repo: &Repo, n: usize, w: u16, h: u16) -> App {
    let body: String = (1..=n).map(|i| format!("line{i}\n")).collect();
    let mut app = repo.open("doc.md", &body);
    app.event(AppEvent::Resize(w, h));
    keys(&mut app, "gR");
    assert_eq!(row_text(&app, 0).trim_end(), "line1");
    app
}

/// The box covers none of rows `lo..=hi` that are on screen.
fn off_the_rows(app: &App, bx: &CommentBox, lo: usize, hi: usize) -> bool {
    let s = app.scroll();
    (lo.max(s)..=hi).all(|r| {
        let y = (r - s) as u16;
        y < bx.rect.y || y >= bx.rect.bottom()
    })
}

#[test]
fn a_selection_taller_than_the_screen_shows_its_last_rows_above_the_box() {
    let repo = Repo::new();
    let mut app = raw_lines(&repo, 60, 60, 12);
    keys(&mut app, "ggVGc");
    assert_eq!(app.mode(), Mode::Comment, "{}", app.status());
    assert_eq!(app.comment_rows(), Some((0, 59)));
    let (lines, bx, _) = draw_box(&app, 60, 12);
    let bx = bx.unwrap();
    assert!(off_the_rows(&app, &bx, 0, 59), "{:?}", bx.rect);
    let y = (59 - app.scroll()) as u16;
    assert_eq!(bx.rect.y, y + 1, "the last row just above the box");
    assert!(
        lines[y as usize].contains("line60"),
        "{}",
        lines[y as usize]
    );
    // The box grows; the rows move up out of its way.
    for _ in 0..3 {
        send(&mut app, ctrl('j'));
    }
    let (lines, bx, _) = draw_box(&app, 60, 12);
    let bx = bx.unwrap();
    assert_eq!(bx.rect.height, 6);
    assert!(off_the_rows(&app, &bx, 0, 59), "{:?}", bx.rect);
    assert!(lines[bx.rect.y as usize - 1].contains("line60"));
}

#[test]
fn a_one_row_comment_at_heights_6_and_8_never_covers_its_row() {
    let repo = Repo::new();
    for (w, h) in [(40, 6), (100, 6), (40, 8), (100, 8)] {
        let mut app = raw_lines(&repo, 40, w, h);
        for r in 0..12 {
            for (anchor, body) in [("zt", 1), ("zz", 1), ("zb", 1), ("zz", 3), ("zb", 3)] {
                goto(&mut app, r);
                keys(&mut app, anchor);
                keys(&mut app, "cc");
                assert_eq!(app.mode(), Mode::Comment, "{}", app.status());
                // A three-line body: a 5-row box.
                for _ in 1..body {
                    send(&mut app, ctrl('j'));
                }
                let (lines, bx, _) = draw_box(&app, w, h);
                let bx = bx.unwrap();
                assert!(
                    off_the_rows(&app, &bx, r, r),
                    "{w}x{h} row {r} {anchor} body {body}: {:?} scroll {}",
                    bx.rect,
                    app.scroll()
                );
                let y = r - app.scroll();
                assert!(lines[y].contains(&format!("line{}", r + 1)));
                send(&mut app, key(KeyCode::Esc));
            }
        }
    }
}

/// The box right beside rendered row `r` (below or above), the row shown.
fn beside(app: &App, bx: &CommentBox, r: usize) -> bool {
    let y = (r - app.scroll()) as u16;
    bx.rect.y == y + 1 || bx.rect.bottom() == y
}

#[test]
fn page_keys_scroll_the_page_behind_the_box_which_follows_its_rows() {
    let repo = Repo::new();
    let mut app = raw_lines(&repo, 80, 60, 12);
    goto(&mut app, 19);
    keys(&mut app, "zzccdraft");
    let (_, bx, _) = draw_box(&app, 60, 12);
    assert!(beside(&app, &bx.unwrap(), 19));
    // Down a page: the row scrolls off the top, the box pins there.
    send(&mut app, key(KeyCode::PageDown));
    assert_eq!(app.mode(), Mode::Comment);
    assert!(app.scroll() > 19, "scrolled past the row: {}", app.scroll());
    let (lines, bx, _) = draw_box(&app, 60, 12);
    assert_eq!(bx.unwrap().rect.y, 0, "pinned to the top edge");
    assert!(lines[1].contains("draft"), "{}", lines[1]);
    // Back up: beside its row again, the text kept.
    send(&mut app, key(KeyCode::PageUp));
    let (_, bx, _) = draw_box(&app, 60, 12);
    assert!(app.scroll() <= 19);
    assert!(beside(&app, &bx.unwrap(), 19));
    assert_eq!(app.comment_text(), Some("draft"));
    send(&mut app, key(KeyCode::Enter));
    assert_eq!(only(&repo).2, "draft");
}

#[test]
fn the_wheel_scrolls_the_page_behind_the_box() {
    let repo = Repo::new();
    let mut app = raw_lines(&repo, 80, 60, 12);
    goto(&mut app, 19);
    keys(&mut app, "zzccdraft");
    // Draw once so the layout knows where the text is.
    draw_box(&app, 60, 12);
    let before = app.scroll();
    let wheel = |app: &mut App, kind| {
        let m = MouseEvent {
            kind,
            column: 40,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        app.event(AppEvent::Mouse(m, Instant::now()));
        draw_box(app, 60, 12);
    };
    for _ in 0..4 {
        wheel(&mut app, MouseEventKind::ScrollDown);
    }
    assert_eq!(app.mode(), Mode::Comment);
    assert!(app.scroll() > before, "{before} -> {}", app.scroll());
    let (_, bx, _) = draw_box(&app, 60, 12);
    let bx = bx.unwrap();
    let r = 19;
    assert!(
        r < app.scroll() && bx.rect.y == 0 || beside(&app, &bx, r),
        "follows its row: {:?} scroll {}",
        bx.rect,
        app.scroll()
    );
    for _ in 0..4 {
        wheel(&mut app, MouseEventKind::ScrollUp);
    }
    let (_, bx, _) = draw_box(&app, 60, 12);
    assert!(beside(&app, &bx.unwrap(), r), "scroll {}", app.scroll());
    assert_eq!(app.comment_text(), Some("draft"));
}
