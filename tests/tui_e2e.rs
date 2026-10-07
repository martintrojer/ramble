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
    if Command::new("tmux").arg("-V").output().is_err() {
        eprintln!("skipping tui_e2e: tmux not found on PATH");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let doc = dir.path().join("long.md");
    let body: String = (1..=120).map(|i| format!("line {i:03}\n\n")).collect();
    std::fs::write(&doc, body).unwrap();

    let tmux = Tmux {
        sock: dir.path().join("sock"),
        home: home.clone(),
    };
    let bin = env!("CARGO_BIN_EXE_ramble");
    let shell_cmd = format!(
        "env HOME={} {} {}",
        quote(&home),
        quote(Path::new(bin)),
        quote(&doc)
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

fn quote(p: &Path) -> String {
    format!("'{}'", p.display().to_string().replace('\'', r"'\''"))
}
