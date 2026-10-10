//! fix-misc: comment temp file permissions, `:e ~/`, pickers in tiny terminals.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use debrief_review::{Comment, Review};
use ramble::app::{App, AppEvent, Exit, StartOptions, StartTarget};
use ramble::config::{Config, SidebarShow};
use ratatui::Terminal;
use ratatui::backend::TestBackend;

const DOC: &str = "# Title\n\nalpha\n\nbeta\n\ngamma\n";

fn config() -> Config {
    let mut c = Config::default();
    c.lsp.server = vec![];
    c.sidebar.show = SidebarShow::Never;
    c
}

fn app(root: &Path, cache: &Path, doc: &Path) -> App {
    App::new(
        StartOptions {
            target: StartTarget::File(doc.to_path_buf()),
            tree_root: root.to_path_buf(),
            config: config(),
            review_cache: Some(cache.to_path_buf()),
            mdroots: ramble::app::MdrootsOptions::memory(),
        },
        (60, 24),
    )
    .unwrap()
}

fn keys(app: &mut App, s: &str) {
    for c in s.chars() {
        app.event(AppEvent::Key(KeyEvent::new(
            KeyCode::Char(c),
            KeyModifiers::NONE,
        )));
    }
}

fn repo() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let root = base.join("repo");
    std::fs::create_dir_all(root.join(".git")).unwrap();
    let doc = root.join("doc.md");
    std::fs::write(&doc, DOC).unwrap();
    (dir, root, doc)
}

#[cfg(unix)]
#[test]
fn comment_temp_file_is_private_and_removed() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, root, doc) = repo();
    let cache = dir.path().join("cache");
    let seen: Rc<RefCell<Option<(u32, std::path::PathBuf)>>> = Rc::default();
    let s = seen.clone();
    let mut app = app(&root, &cache, &doc).with_runner(move |cmd| {
        let f = Path::new(cmd.argv.last().unwrap());
        let mode = std::fs::metadata(f).unwrap().permissions().mode() & 0o777;
        *s.borrow_mut() = Some((mode, f.to_path_buf()));
        Ok(Exit::Code(0))
    });
    let row = (0..app.page().unwrap().rendered.lines.len())
        .find(|&r| {
            let t: String = app.page().unwrap().rendered.lines[r]
                .spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect();
            t.contains("gamma")
        })
        .unwrap();
    keys(&mut app, &format!("{}Gccx", row + 1));
    app.event(AppEvent::Key(KeyEvent::new(
        KeyCode::Char('e'),
        KeyModifiers::CONTROL,
    )));
    app.run_pending_effect();
    let (mode, file) = seen.borrow().clone().expect("editor ran");
    assert_eq!(mode, 0o600);
    assert!(!file.exists(), "removed after the editor exits");
}

#[test]
fn edit_command_expands_a_leading_tilde() {
    let (dir, root, doc) = repo();
    let home = dir.path().canonicalize().unwrap().join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("x.md"), "# Home page\n").unwrap();
    let h = home.display().to_string();
    let mut app = app(&root, &dir.path().join("cache"), &doc)
        .with_env(move |k| (k == "HOME").then(|| h.clone()));
    app.execute("e ~/x.md");
    let page = app.page().unwrap();
    assert_eq!(page.path.as_deref(), Some(home.join("x.md").as_path()));
}

#[test]
fn pickers_do_not_panic_in_tiny_terminals() {
    let (dir, root, doc) = repo();
    let cache = dir.path().join("cache");
    Review::open(&cache, &root)
        .unwrap()
        .add(Comment::on_file("doc.md", 3..=3, "alpha", "note"))
        .unwrap();
    let mut app = app(&root, &cache, &doc);
    keys(&mut app, " rl");
    assert!(app.picker().is_some(), "picker open: {}", app.status());
    for (w, h) in [(80, 1), (80, 2), (80, 3), (1, 1), (3, 5), (80, 0)] {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| ramble::ui::draw(f, &app)).unwrap();
    }
    keys(&mut app, "g?");
    let mut term = Terminal::new(TestBackend::new(80, 1)).unwrap();
    term.draw(|f| ramble::ui::draw(f, &app)).unwrap();
}
