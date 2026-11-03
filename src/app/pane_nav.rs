//! `C-h/j/k/l` (`[keys] pane_nav`): the focus moves of `C-w h/j/k/l`, as
//! in nvim. Where there is no pane that way and ramble runs inside tmux or
//! rex, the move continues into the multiplexer's pane there (`tmux
//! select-pane`, `rex focus`), as vim-tmux-navigator does.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::keys::{Action, KeyResult};
use super::launch::{Exit, LaunchCommand};
use super::sidebar::{SidebarAction, focus_toward};
use super::{App, Focus};

pub use debrief_follow::Dir;
use debrief_follow::Mux;

/// How long the multiplexer hand-off may hold up the UI.
pub const MUX_WAIT: Duration = Duration::from_millis(200);

/// The command that moves `mux`'s focus toward `dir`.
pub fn focus_command(mux: &Mux, dir: Dir) -> LaunchCommand {
    LaunchCommand {
        name: mux.name().into(),
        argv: mux.focus_argv(dir),
        cwd: ".".into(),
    }
}

impl App {
    /// `C-h/j/k/l` while `[keys] pane_nav` is on.
    pub(super) fn pane_nav_keymap(&self, keys: &[KeyEvent]) -> Option<KeyResult> {
        if !self.config.keys.pane_nav {
            return None;
        }
        let [k] = keys else { return None };
        if !k.modifiers.contains(KeyModifiers::CONTROL) {
            return None;
        }
        let dir = match k.code {
            KeyCode::Char('h') => Dir::Left,
            KeyCode::Char('j') => Dir::Down,
            KeyCode::Char('k') => Dir::Up,
            KeyCode::Char('l') => Dir::Right,
            _ => return None,
        };
        Some(KeyResult::Action(Action::PaneNav(dir)))
    }

    /// Move to the pane toward `dir` as `C-w` does; with none there
    /// inside tmux or rex, move to the multiplexer's pane that way instead.
    pub(super) fn pane_nav(&mut self, dir: Dir) {
        let a = focus_toward(dir, self.sidebar_side());
        if !self.has_pane_toward(a)
            && let Some(mux) = Mux::detect(&*self.env)
        {
            // Failure (no pane there, the multiplexer gone) is not ours
            // to report.
            let _ = (self.pane_runner)(&focus_command(&mux, dir));
            return;
        }
        self.sidebar_action(a);
    }

    /// Whether focus move `a` lands in another pane.
    fn has_pane_toward(&self, a: SidebarAction) -> bool {
        use SidebarAction as S;
        let focus = self.focus();
        let reach = self.can_focus_or_peek();
        let split = self.sidebar_panes().len() >= 2;
        match a {
            // Toward the sidebar from inside it is the screen edge.
            S::FocusSidebar => focus == Focus::Content && reach,
            S::FocusContent => focus != Focus::Content,
            S::FocusBelow => split && reach && focus != Focus::Outline,
            S::FocusAbove => split && reach && focus != Focus::Files,
            _ => true,
        }
    }
}

/// The pane runner gave up waiting; the command goes on (a slow
/// multiplexer still opens its split).
#[derive(Debug)]
pub struct StillRunning;

impl std::fmt::Display for StillRunning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("still running")
    }
}

impl std::error::Error for StillRunning {}

/// Default pane runner: start the command with no terminal I/O and wait at
/// most [`MUX_WAIT`]; a slower command finishes on its own.
pub fn spawn_capped(cmd: &LaunchCommand) -> anyhow::Result<Exit> {
    use std::process::{Command, Stdio};
    let (prog, args) = cmd
        .argv
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("empty command"))?;
    let mut child = Command::new(prog)
        .args(args)
        .current_dir(&cmd.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + MUX_WAIT;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Exit::Code(status.code().unwrap_or(-1)));
        }
        if Instant::now() >= deadline {
            // Reap it off the UI thread.
            std::thread::spawn(move || child.wait());
            return Err(anyhow::Error::new(StillRunning)
                .context(format!("{prog} still running after {MUX_WAIT:?}")));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(argv: &[&str]) -> LaunchCommand {
        LaunchCommand {
            name: "t".into(),
            argv: argv.iter().map(|s| s.to_string()).collect(),
            cwd: ".".into(),
        }
    }

    #[test]
    fn spawn_capped_returns_the_exit_code_of_a_quick_command() {
        assert!(matches!(
            spawn_capped(&cmd(&["sh", "-c", "exit 3"])),
            Ok(Exit::Code(3))
        ));
        assert!(spawn_capped(&cmd(&["/nonexistent/ramble-test"])).is_err());
    }

    #[test]
    fn spawn_capped_never_waits_much_longer_than_the_cap() {
        let t = Instant::now();
        assert!(spawn_capped(&cmd(&["sleep", "5"])).is_err());
        assert!(t.elapsed() < MUX_WAIT + Duration::from_secs(1));
    }
}
