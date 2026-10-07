//! End-to-end: the real binary in an isolated tmux server.

use std::path::Path;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

struct Tmux {
    sock: std::path::PathBuf,
    home: std::path::PathBuf,
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
        String::from_utf8_lossy(&self.cmd(&["capture-pane", "-p", "-t", "e2e"]).stdout).into()
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

fn first_line(screen: &str) -> &str {
    screen.lines().next().unwrap_or("").trim_end()
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

/// Start the real binary on `doc` in a fresh isolated tmux server.
fn start(dir: &Path, doc: &Path) -> Option<Tmux> {
    if Command::new("tmux").arg("-V").output().is_err() {
        eprintln!("skipping tui_e2e: tmux not found on PATH");
        return None;
    }
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let tmux = Tmux {
        sock: dir.join("sock"),
        home: home.clone(),
    };
    let bin = env!("CARGO_BIN_EXE_ramble");
    let shell_cmd = format!(
        "env HOME={} {} {}",
        quote(&home),
        quote(Path::new(bin)),
        quote(doc)
    );
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

fn quote(p: &Path) -> String {
    format!("'{}'", p.display().to_string().replace('\'', r"'\''"))
}
