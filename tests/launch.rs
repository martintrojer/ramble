//! Launchers (spec § Launchers, § Testing "launchers"): a recording runner
//! stands in for the terminal; no real editor is spawned.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ramble::app::{App, AppEvent, Effect, Exit, FsEvent, LaunchCommand, StartOptions, StartTarget};
use ramble::config::Config;
use tempfile::TempDir;

const SIZE: (u16, u16) = (60, 20);

type Calls = Rc<RefCell<Vec<LaunchCommand>>>;

fn no_env(_: &str) -> Option<String> {
    None
}

fn keys(app: &mut App, s: &str) {
    for c in s.chars() {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
}

/// An app on `dir/sub/a.md` with a recording runner that exits with `code`.
fn launch_app(mut config: Config, vcs: bool, code: Exit) -> (TempDir, App, Calls) {
    config.lsp.server = vec![]; // N5: tests never start a real server
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    if vcs {
        std::fs::create_dir(root.join(".jj")).unwrap();
    }
    std::fs::create_dir(root.join("sub")).unwrap();
    let path = root.join("sub/a.md");
    std::fs::write(&path, "# A\n\nline three\n\nline five\n").unwrap();
    let calls: Calls = Rc::default();
    let rec = calls.clone();
    let app = App::new(
        StartOptions {
            target: StartTarget::File(path),
            tree_root: root,
            config,
            review_cache: None,
        },
        SIZE,
    )
    .unwrap()
    .with_env(no_env)
    .with_runner(move |cmd| {
        rec.borrow_mut().push(cmd.clone());
        Ok(code)
    });
    (dir, app, calls)
}

/// The default config without language servers (N5).
fn no_lsp() -> Config {
    let mut c = Config::default();
    c.lsp.server = vec![];
    c
}

fn root(dir: &TempDir) -> PathBuf {
    dir.path().canonicalize().unwrap()
}

fn s(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|x| x.to_string()).collect()
}

/// Move to the row reading `text`.
fn goto(app: &mut App, text: &str) {
    let p = app.page().unwrap();
    let row = (0..p.rendered.lines.len())
        .find(|&r| {
            p.rendered.lines[r]
                .spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
                == text
        })
        .unwrap();
    keys(app, "gg");
    for _ in 0..row {
        keys(app, "j");
    }
}

#[test]
fn leader_o_expands_editor_line_and_file_with_vcs_cwd() {
    let (dir, app, calls) = launch_app(Config::default(), true, Exit::Code(0));
    let mut app = app.with_env(|k| (k == "EDITOR").then(|| "nvim --clean".to_string()));
    goto(&mut app, "line five");
    keys(&mut app, " o");
    let Some(Effect::Launch(cmd)) = app.pending_effect().cloned() else {
        panic!(
            "no launch queued: {:?} / {}",
            app.pending_effect(),
            app.status()
        );
    };
    let file = root(&dir).join("sub/a.md");
    assert_eq!(
        cmd.argv,
        s(&["nvim", "--clean", "+5", file.to_str().unwrap()])
    );
    assert_eq!(cmd.cwd, root(&dir), "cwd is the VCS root");
    app.run_pending_effect();
    assert_eq!(calls.borrow().len(), 1);
    assert_eq!(app.status(), "");
}

#[test]
fn no_vcs_cwd_is_dir_and_editor_defaults_to_vi() {
    let (dir, mut app, calls) = launch_app(Config::default(), false, Exit::Code(0));
    keys(&mut app, " o");
    app.run_pending_effect();
    let c = &calls.borrow()[0];
    assert_eq!(c.argv[0], "vi");
    assert_eq!(c.argv[1], "+1");
    assert_eq!(c.cwd, root(&dir).join("sub"));
}

/// A config with two `<leader>r…` launchers, one needing a repo.
fn dir_and_vcs_launchers() -> Config {
    let mut c = Config::default();
    let l = |name: &str, key: &str, cmd: &[&str], needs_vcs| ramble::config::Launcher {
        name: name.into(),
        key: Some(key.into()),
        command: s(cmd),
        needs_vcs,
        disabled: false,
    };
    c.launch
        .push(l("browse", "<leader>rd", &["lf", "${dir}"], false));
    c.launch.push(l(
        "changes",
        "<leader>rw",
        &["jjui", "${file}", "+${line}"],
        true,
    ));
    c
}

#[test]
fn dir_variable_and_needs_vcs() {
    let (dir, mut app, calls) = launch_app(dir_and_vcs_launchers(), false, Exit::Code(0));
    keys(&mut app, " rd");
    app.run_pending_effect();
    let sub = root(&dir).join("sub");
    assert_eq!(calls.borrow()[0].argv, s(&["lf", sub.to_str().unwrap()]));

    keys(&mut app, " rw");
    assert!(app.pending_effect().is_none());
    assert_eq!(
        app.status(),
        "changes: unavailable: not in a VCS repository"
    );
    assert_eq!(calls.borrow().len(), 1);
}

#[test]
fn needs_vcs_launcher_runs_inside_a_repo() {
    let (dir, mut app, calls) = launch_app(dir_and_vcs_launchers(), true, Exit::Code(0));
    keys(&mut app, " rw");
    app.run_pending_effect();
    let file = root(&dir).join("sub/a.md");
    assert_eq!(
        calls.borrow()[0].argv,
        s(&["jjui", file.to_str().unwrap(), "+1"])
    );
}

#[test]
fn the_old_review_keys_are_unbound_by_default() {
    let (_d, mut app, calls) = launch_app(Config::default(), false, Exit::Code(0));
    for k in [" rr", " rw", " rd"] {
        keys(&mut app, k);
        assert!(app.pending_effect().is_none(), "{k}");
        assert_eq!(app.status(), "No mapping", "{k}");
    }
    assert!(calls.borrow().is_empty());
}

#[test]
fn leader_r_waits_and_unknown_cancels() {
    let (_d, mut app, calls) = launch_app(dir_and_vcs_launchers(), false, Exit::Code(0));
    keys(&mut app, " r");
    assert!(app.pending_effect().is_none());
    assert_eq!(app.status(), "");
    keys(&mut app, "x");
    assert_eq!(app.status(), "No mapping");
    // The cancelled sequence does not leak: `j` is a motion again.
    keys(&mut app, "j");
    assert_eq!(app.cursor().row, 1);
    assert!(calls.borrow().is_empty());
}

#[test]
fn stdin_page_cannot_launch_editor() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::new(
        StartOptions {
            target: StartTarget::Stdin("# S\n".into()),
            tree_root: dir.path().to_path_buf(),
            config: no_lsp(),
            review_cache: None,
        },
        SIZE,
    )
    .unwrap()
    .with_env(no_env)
    .with_runner(|_| panic!("must not run"));
    keys(&mut app, " o");
    assert!(app.pending_effect().is_none());
    assert_eq!(app.status(), "edit: unavailable: no file (stdin document)");
}

#[test]
fn non_zero_exit_and_signal_show_in_status() {
    let (_d, mut app, _c) = launch_app(dir_and_vcs_launchers(), false, Exit::Code(2));
    keys(&mut app, " rd");
    app.run_pending_effect();
    assert_eq!(app.status(), "browse: exited with status 2");

    let (_d, mut app, _c) = launch_app(Config::default(), false, Exit::Signal(9));
    app.launch("edit");
    app.run_pending_effect();
    assert_eq!(app.status(), "edit: killed by signal 9");

    app.launch("nope");
    assert_eq!(app.status(), "No launcher named nope");
}

#[test]
fn user_entries_replace_defaults_and_custom_leader() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("config.toml");
    std::fs::write(
        &cfg,
        r#"
[keys]
leader = ","

[[launch]]
name = "edit"
key = "<leader>e"
command = ["myed", "${file}:${line}"]

[[launch]]
name = "browse"
key = "<leader>rr"
command = ["lf"]

[[launch]]
name = "browse"
disabled = true

[[launch]]
name = "bad"
key = "<leader><C-x>"
command = ["x"]
"#,
    )
    .unwrap();
    let config = Config::load(&cfg).unwrap();
    let (dir, mut app, calls) = launch_app(config, false, Exit::Code(0));
    assert_eq!(app.status(), "launcher bad: unknown key <C-x>");
    keys(&mut app, ",e");
    app.run_pending_effect();
    let file = root(&dir).join("sub/a.md");
    assert_eq!(
        calls.borrow()[0].argv,
        s(&["myed", &format!("{}:1", file.display())])
    );
    // `browse` is gone, `,rr` is unbound; space is no longer the leader.
    keys(&mut app, ",rr");
    assert!(app.pending_effect().is_none());
    assert_eq!(calls.borrow().len(), 1);
    keys(&mut app, " o");
    assert!(app.pending_effect().is_none());
}

#[test]
fn reload_on_return_and_watcher_event_is_suppressed() {
    let (dir, app, _c) = launch_app(Config::default(), false, Exit::Code(0));
    let file = root(&dir).join("sub/a.md");
    let f = file.clone();
    let mut app = app.with_runner(move |_| {
        std::fs::write(&f, "# A\n\nedited by launcher\n").unwrap();
        Ok(Exit::Code(0))
    });
    keys(&mut app, " o");
    app.run_pending_effect();
    assert!(
        app.page()
            .unwrap()
            .doc
            .source
            .contains("edited by launcher")
    );

    // The watcher's own Changed event right after is ignored.
    std::fs::write(&file, "# A\n\nlater\n").unwrap();
    app.event(AppEvent::FsWatch(file.clone(), FsEvent::Changed));
    assert!(
        app.page()
            .unwrap()
            .doc
            .source
            .contains("edited by launcher")
    );
}

/// spec § Testing app: `<leader>o` runs a fake editor (a script set as
/// `$VISUAL` that appends a line), then the page shows the new line.
#[cfg(unix)]
#[test]
fn leader_o_runs_fake_visual_script_then_page_shows_new_line() {
    use std::os::unix::fs::PermissionsExt;
    let (dir, app, _c) = launch_app(Config::default(), false, Exit::Code(0));
    let script = root(&dir).join("fake-editor.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\n# $1 is +LINE, $2 the file\necho 'appended by editor' >> \"$2\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let visual = script.display().to_string();
    let mut app = app
        .with_env(move |k| (k == "VISUAL").then(|| visual.clone()))
        .with_runner(run_quietly);
    keys(&mut app, " o");
    app.run_pending_effect();
    assert_eq!(app.status(), "");
    assert!(
        app.page()
            .unwrap()
            .doc
            .source
            .ends_with("appended by editor\n"),
        "{}",
        app.page().unwrap().doc.source
    );
}

/// Like the real runner, but with null stdio so the test never touches the
/// terminal.
fn run_quietly(cmd: &LaunchCommand) -> anyhow::Result<Exit> {
    let status = std::process::Command::new(&cmd.argv[0])
        .args(&cmd.argv[1..])
        .current_dir(&cmd.cwd)
        .stdin(std::process::Stdio::null())
        .status()?;
    Ok(Exit::Code(status.code().unwrap_or(-1)))
}

#[test]
fn vcs_root_walks_file_ancestors() {
    let dir = tempfile::tempdir().unwrap();
    let r = root(&dir);
    std::fs::create_dir_all(r.join("a/b")).unwrap();
    assert_eq!(ramble::app::vcs_root(&r.join("a/b/x.md")), None);
    for m in [".git", ".hg", ".sl"] {
        let d = r.join(m.trim_start_matches('.'));
        std::fs::create_dir_all(d.join(m)).unwrap();
        std::fs::create_dir_all(d.join("deep")).unwrap();
        assert_eq!(
            ramble::app::vcs_root(&d.join("deep/x.md")).as_deref(),
            Some(d.as_path())
        );
    }
}
