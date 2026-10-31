//! End-to-end: the real binary in an isolated tmux server.

use std::path::Path;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

struct Tmux {
    sock: std::path::PathBuf,
    home: std::path::PathBuf,
    /// mdroots' cache for the spawned ramble, outside the browsed tree.
    _cache: tempfile::TempDir,
}

impl Tmux {
    fn cmd(&self, args: &[&str]) -> Output {
        Command::new("tmux")
            .arg("-S")
            .arg(&self.sock)
            .args(args)
            .env("HOME", &self.home)
            .env_remove("TMUX")
            .output()
            .expect("run tmux")
    }

    fn capture(&self) -> String {
        self.capture_of("e2e")
    }

    fn capture_of(&self, target: &str) -> String {
        String::from_utf8_lossy(&self.cmd(&["capture-pane", "-p", "-t", target]).stdout).into()
    }

    /// Poll the pane until `pred` holds, or panic with the last screen.
    fn wait_for(&self, what: &str, pred: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let screen = self.capture();
            if pred(&screen) {
                return screen;
            }
            if Instant::now() > deadline {
                panic!("timed out waiting for {what}; screen:\n{screen}");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Tmux {
    fn drop(&mut self) {
        let _ = self.cmd(&["kill-server"]);
    }
}

/// The content part of the first row (right of the sidebar border, if any).
fn first_line(screen: &str) -> &str {
    let line = screen.lines().next().unwrap_or("");
    line.rsplit_once('│').map_or(line, |(_, c)| c).trim_end()
}

#[test]
fn g_then_gg_in_tmux() {
    let dir = tempfile::tempdir().unwrap();
    let doc = dir.path().join("long.md");
    let body: String = (1..=120).map(|i| format!("line {i:03}\n\n")).collect();
    std::fs::write(&doc, body).unwrap();
    let Some(tmux) = start(dir.path(), &doc) else {
        return;
    };

    let screen = tmux.wait_for("first page", |s| first_line(s) == "line 001");
    assert!(
        screen.contains("long.md"),
        "status line shows the file:\n{screen}"
    );
    assert!(screen.contains("Top"), "{screen}");

    tmux.cmd(&["send-keys", "-t", "e2e", "G"]);
    let screen = tmux.wait_for("end of page", |s| s.contains("line 120"));
    assert!(!screen.contains("line 001"), "{screen}");
    assert!(screen.contains("100%"), "{screen}");

    tmux.cmd(&["send-keys", "-t", "e2e", "g", "g"]);
    let screen = tmux.wait_for("top of page", |s| first_line(s) == "line 001");
    assert!(!screen.contains("line 120"), "{screen}");

    tmux.cmd(&["send-keys", "-t", "e2e", "q"]);
    let deadline = Instant::now() + Duration::from_secs(10);
    while tmux.cmd(&["has-session", "-t", "e2e"]).status.success() {
        assert!(Instant::now() < deadline, "ramble did not quit on q");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn line_flag_opens_at_that_source_line() {
    let dir = tempfile::tempdir().unwrap();
    let doc = dir.path().join("long.md");
    // "line N" is on source line 2N - 1.
    let body: String = (1..=120).map(|i| format!("line {i:03}\n\n")).collect();
    std::fs::write(&doc, body).unwrap();
    let Some(tmux) = start_with(dir.path(), &doc, "--line 99") else {
        return;
    };
    let screen = tmux.wait_for("line 50 on top", |s| first_line(s) == "line 050");
    assert!(!screen.contains("line 001"), "{screen}");
    tmux.cmd(&["send-keys", "-t", "e2e", "q"]);
}

/// Start the real binary on `doc` in a fresh isolated tmux server.
fn start(dir: &Path, doc: &Path) -> Option<Tmux> {
    start_with(dir, doc, "")
}

/// [`start`] with extra `flags` before the file.
fn start_with(dir: &Path, doc: &Path, flags: &str) -> Option<Tmux> {
    start_env(dir, doc, "", flags)
}

/// [`start_with`] with `env` (`NAME=value ...`, quoted) set for ramble.
fn start_env(dir: &Path, doc: &Path, env: &str, flags: &str) -> Option<Tmux> {
    if Command::new("tmux").arg("-V").output().is_err() {
        eprintln!("skipping tui_e2e: tmux not found on PATH");
        return None;
    }
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    // The pages have no zk/marksman marker, so mdroots serves them: its
    // cache goes to a temp dir, never the user's.
    let cache = tempfile::tempdir().unwrap();
    // XDG_* point into the temp home too, so the user's real
    // `$XDG_CONFIG_HOME/ramble/config.toml` can't leak into the run.
    let xdg = |name: &str| {
        let d = home.join(name.to_lowercase());
        std::fs::create_dir_all(&d).unwrap();
        quote(&d)
    };
    let shell_cmd = format!(
        "env HOME={} XDG_RUNTIME_DIR={} XDG_CONFIG_HOME={} XDG_DATA_HOME={} XDG_CACHE_HOME={} MDROOTS_CACHE_DIR={} {env} {} {flags} {}",
        quote(&home),
        quote(&run_dir(dir)),
        xdg("XDG_CONFIG_HOME"),
        xdg("XDG_DATA_HOME"),
        xdg("XDG_CACHE_HOME"),
        quote(cache.path()),
        quote(Path::new(env!("CARGO_BIN_EXE_ramble"))),
        quote(doc)
    );
    let tmux = Tmux {
        sock: dir.join("sock"),
        home: home.clone(),
        _cache: cache,
    };
    let out = tmux.cmd(&[
        "-f",
        "/dev/null",
        "new-session",
        "-d",
        "-s",
        "e2e",
        "-x",
        "80",
        "-y",
        "24",
        &shell_cmd,
    ]);
    assert!(out.status.success(), "tmux new-session: {out:?}");
    Some(tmux)
}

#[test]
fn follow_link_and_back_in_tmux() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.md");
    std::fs::write(&a, "# Page A\n\nGo to [the b page](b.md) now\n").unwrap();
    std::fs::write(dir.path().join("b.md"), "# Page B\n\nbody of b\n").unwrap();
    let Some(tmux) = start(dir.path(), &a) else {
        return;
    };
    tmux.wait_for("a.md", |s| first_line(s) == "Page A");

    // Rows: "Page A", rule, blank, link line. `w w w` lands on "the".
    tmux.cmd(&["send-keys", "-t", "e2e", "j", "j", "j", "w", "w", "w"]);
    tmux.cmd(&["send-keys", "-t", "e2e", "g", "d"]);
    let screen = tmux.wait_for("b.md after gd", |s| first_line(s) == "Page B");
    assert!(screen.contains("body of b"), "{screen}");
    assert!(screen.contains("← 1"), "{screen}");

    tmux.cmd(&["send-keys", "-t", "e2e", "C-o"]);
    let screen = tmux.wait_for("a.md after C-o", |s| first_line(s) == "Page A");
    assert!(screen.contains("the b page"), "{screen}");
    assert!(screen.contains("← 0"), "{screen}");

    // C-] (sent once) follows the same link: the cursor was restored onto it.
    tmux.cmd(&["send-keys", "-t", "e2e", "C-]"]);
    tmux.wait_for("b.md after C-]", |s| first_line(s) == "Page B");

    tmux.cmd(&["send-keys", "-t", "e2e", "q"]);
}

/// `$XDG_RUNTIME_DIR` for the spawned rambles: `--follow` sockets stay in
/// the test's dir.
fn run_dir(dir: &Path) -> std::path::PathBuf {
    let d = dir.join("run");
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn follow_moves_to_the_line_another_pane_in_the_window_sends() {
    let dir = tempfile::tempdir().unwrap();
    let doc = dir.path().join("long.md");
    let body: String = (1..=120).map(|i| format!("line {i:03}\n\n")).collect();
    std::fs::write(&doc, body).unwrap();
    let Some(tmux) = start_with(dir.path(), &doc, "--follow") else {
        return;
    };
    tmux.wait_for("first page", |s| first_line(s) == "line 001");
    // Wait for the socket: the listener binds just after the first draw.
    let run = run_dir(dir.path());
    let deadline = Instant::now() + Duration::from_secs(10);
    while !std::fs::read_dir(&run)
        .into_iter()
        .flatten()
        .flatten()
        .any(|d| std::fs::read_dir(d.path()).is_ok_and(|mut e| e.next().is_some()))
    {
        assert!(Instant::now() < deadline, "no follow socket in {run:?}");
        std::thread::sleep(Duration::from_millis(50));
    }

    // Like an editor hook: `--goto` from another pane in the same window.
    // Source line 179 is "line 090".
    let goto = format!(
        "env XDG_RUNTIME_DIR={} {} --goto {}:179",
        quote(&run),
        quote(Path::new(env!("CARGO_BIN_EXE_ramble"))),
        quote(&doc)
    );
    let out = tmux.cmd(&["split-window", "-d", "-t", "e2e", &goto]);
    assert!(out.status.success(), "split-window: {out:?}");
    let screen = tmux.wait_for("line 090 on screen", |s| s.contains("line 090"));
    assert!(!screen.contains("line 001"), "{screen}");

    tmux.cmd(&["send-keys", "-t", "e2e", "q"]);
}

/// `<leader>O` in tmux: the editor (a stub that runs `--goto`, as an
/// editor hook would) opens in a split beside ramble without suspending
/// it, ramble starts listening, and the stub's first position moves it.
#[test]
fn leader_shift_o_splits_the_editor_and_follows_it() {
    let dir = tempfile::tempdir().unwrap();
    let doc = dir.path().join("long.md");
    let body: String = (1..=120).map(|i| format!("line {i:03}\n\n")).collect();
    std::fs::write(&doc, body).unwrap();
    let run = run_dir(dir.path());
    // The split pane gets tmux's environment, not ramble's: the stub sets
    // the runtime dir itself. Source line 179 is "line 090".
    let editor = dir.path().join("editor");
    std::fs::write(
        &editor,
        format!(
            "#!/bin/sh\nXDG_RUNTIME_DIR={} {} --goto {}:179\nsleep 600\n",
            quote(&run),
            quote(Path::new(env!("CARGO_BIN_EXE_ramble"))),
            quote(&doc)
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&editor, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let env = format!("VISUAL={}", quote(&editor));
    let Some(tmux) = start_env(dir.path(), &doc, &env, "") else {
        return;
    };
    tmux.wait_for("first page", |s| first_line(s) == "line 001");
    tmux.cmd(&["send-keys", "-t", "e2e", "Space", "O"]);
    // The editor pane has focus now; ramble is pane 0.
    let deadline = Instant::now() + Duration::from_secs(10);
    let screen = loop {
        let s = tmux.capture_of("e2e.0");
        if s.contains("line 090") {
            break s;
        }
        assert!(Instant::now() < deadline, "did not follow:\n{s}");
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(!screen.contains("line 001"), "{screen}");
    let panes = tmux.cmd(&["display-message", "-p", "-t", "e2e", "#{window_panes}"]);
    assert_eq!(String::from_utf8_lossy(&panes.stdout).trim(), "2");
}

fn quote(p: &Path) -> String {
    format!("'{}'", p.display().to_string().replace('\'', r"'\''"))
}
