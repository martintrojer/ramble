//! Launchers (spec § Launchers): `<leader>` key sequences from config that
//! run a command with the TUI suspended, then reload the page.

use std::path::{Path, PathBuf};

use anyhow::Context;

use super::{App, Effect};
use crate::config::Launcher;

/// Files or folders whose presence marks a VCS root.
const VCS_MARKERS: [&str; 4] = [".git", ".jj", ".hg", ".sl"];

/// The nearest ancestor of `path` (a file or folder) holding a VCS marker.
pub fn vcs_root(path: &Path) -> Option<PathBuf> {
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    abs.ancestors()
        .find(|d| VCS_MARKERS.iter().any(|m| d.join(m).exists()))
        .map(Path::to_path_buf)
}

/// A fully expanded launcher command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchCommand {
    /// The launcher's name (for status messages).
    pub name: String,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
}

/// How a launched command ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Code(i32),
    Signal(i32),
}

/// Runs a launcher command in the terminal and waits for it.
pub(super) type Runner = dyn FnMut(&LaunchCommand) -> anyhow::Result<Exit>;

/// Reads an environment variable (injectable so tests never touch the
/// process environment).
pub(super) type EnvLookup = dyn Fn(&str) -> Option<String>;

/// Values for the `${...}` variables of a launcher command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchVars {
    pub file: Option<PathBuf>,
    pub line: usize,
    pub dir: PathBuf,
    pub vcs_root: Option<PathBuf>,
    /// `$VISUAL`, else `$EDITOR`, else `vi`, split on whitespace.
    pub editor: Vec<String>,
}

/// Substitute variables in each argument (no shell). An argument that is
/// exactly `${editor}` expands to the editor's words. `Err` names the
/// reason the launch is unavailable.
pub fn expand(command: &[String], vars: &LaunchVars) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for arg in command {
        if arg == "${editor}" {
            out.extend(vars.editor.iter().cloned());
            continue;
        }
        let mut s = String::new();
        let mut rest = arg.as_str();
        while let Some(i) = rest.find("${") {
            s.push_str(&rest[..i]);
            let after = &rest[i + 2..];
            let Some(j) = after.find('}') else {
                return Err(format!("unterminated variable in {arg:?}"));
            };
            let value = match &after[..j] {
                "file" => vars
                    .file
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .ok_or("no file (stdin document)")?,
                "line" => vars.line.to_string(),
                "dir" => vars.dir.display().to_string(),
                "vcs_root" => vars
                    .vcs_root
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .ok_or("not in a VCS repository")?,
                "editor" => vars.editor.join(" "),
                other => return Err(format!("unknown variable ${{{other}}}")),
            };
            s.push_str(&value);
            rest = &after[j + 1..];
        }
        s.push_str(rest);
        out.push(s);
    }
    Ok(out)
}

/// Parse a launcher key like `<leader>rr` into the chars after the leader.
pub fn parse_key(key: &str) -> Result<Vec<char>, String> {
    let rest = key
        .strip_prefix("<leader>")
        .ok_or_else(|| format!("key {key:?} must start with <leader>"))?;
    if let Some(i) = rest.find('<') {
        let tok = rest[i..].split_inclusive('>').next().unwrap_or(&rest[i..]);
        return Err(format!("unknown key {tok}"));
    }
    if rest.is_empty() {
        return Err(format!("key {key:?} has nothing after <leader>"));
    }
    Ok(rest.chars().collect())
}

/// Launcher key bindings: the chars after the leader, and the index into
/// `Config.launch`. Bad keys are skipped with a message.
pub(super) fn bindings(launchers: &[Launcher]) -> (Vec<(Vec<char>, usize)>, Vec<String>) {
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for (i, l) in launchers.iter().enumerate() {
        let Some(key) = &l.key else { continue };
        match parse_key(key) {
            Ok(seq) => out.push((seq, i)),
            Err(e) => errors.push(format!("launcher {}: {e}", l.name)),
        }
    }
    (out, errors)
}

/// What the keys typed after the leader mean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LeaderMatch {
    Launch(usize),
    Pending,
    NoMapping,
}

pub(super) fn match_leader(bindings: &[(Vec<char>, usize)], typed: &[char]) -> LeaderMatch {
    let longer = bindings
        .iter()
        .any(|(seq, _)| seq.len() > typed.len() && seq.starts_with(typed));
    if longer {
        return LeaderMatch::Pending;
    }
    match bindings.iter().find(|(seq, _)| seq == typed) {
        Some(&(_, i)) => LeaderMatch::Launch(i),
        None => LeaderMatch::NoMapping,
    }
}

/// Editor words: `$VISUAL`, else `$EDITOR`, else `vi`.
pub(super) fn editor_words(env: &EnvLookup) -> Vec<String> {
    let cmd = ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(|v| env(v))
        .find(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "vi".into());
    cmd.split_whitespace().map(str::to_string).collect()
}

impl App {
    /// `:Launch <name>`: queue the named launcher (status if unknown or
    /// unavailable).
    pub fn launch(&mut self, name: &str) {
        match self.config.launch.iter().position(|l| l.name == name) {
            Some(i) => self.launch_index(i),
            None => self.set_status(format!("No launcher named {name}")),
        }
    }

    /// The command launcher `i` would run, or why it is unavailable.
    pub fn launch_command(&self, name: &str) -> Result<LaunchCommand, String> {
        let l = self
            .config
            .launch
            .iter()
            .find(|l| l.name == name)
            .ok_or_else(|| format!("No launcher named {name}"))?;
        self.build_launch(l)
    }

    fn build_launch(&self, l: &Launcher) -> Result<LaunchCommand, String> {
        let vars = self.launch_vars();
        let unavailable = |why: &str| format!("{}: unavailable: {why}", l.name);
        if l.needs_vcs && !self.has_vcs_root() {
            return Err(unavailable("not in a VCS repository"));
        }
        let argv = expand(&l.command, &vars).map_err(|e| unavailable(&e))?;
        if argv.is_empty() {
            return Err(unavailable("empty command"));
        }
        Ok(LaunchCommand {
            name: l.name.clone(),
            argv,
            cwd: vars.vcs_root.clone().unwrap_or(vars.dir),
        })
    }

    pub(super) fn launch_index(&mut self, i: usize) {
        let Some(l) = self.config.launch.get(i) else {
            return;
        };
        match self.build_launch(l) {
            Ok(cmd) => self.pending_effect = Some(Effect::Launch(cmd)),
            Err(msg) => self.set_status(msg),
        }
    }

    /// The current file lies in a VCS repository (`needs_vcs` launchers).
    pub(crate) fn has_vcs_root(&self) -> bool {
        self.launch_vars().vcs_root.is_some()
    }

    fn launch_vars(&self) -> LaunchVars {
        let file = self
            .page
            .as_ref()
            .and_then(|p| p.path.as_deref())
            .map(|p| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()));
        let line = self
            .page
            .as_ref()
            .and_then(|p| p.rendered.source_lines.get(self.cursor.row).copied())
            .unwrap_or(1)
            .max(1);
        let dir = self.link_dir();
        let dir = std::path::absolute(&dir).unwrap_or(dir);
        let vcs_root = file.as_deref().and_then(vcs_root);
        LaunchVars {
            file,
            line,
            dir,
            vcs_root,
            editor: editor_words(self.env.as_ref()),
        }
    }

    /// Run a launcher command (TUI already suspended), then reload.
    pub(super) fn run_launch(&mut self, cmd: &LaunchCommand) {
        let result = (self.runner)(cmd);
        self.reload();
        self.launch_reloaded_at = Some(std::time::Instant::now());
        match result {
            Ok(Exit::Code(0)) => {}
            Ok(Exit::Code(n)) => self.set_status(format!("{}: exited with status {n}", cmd.name)),
            Ok(Exit::Signal(n)) => self.set_status(format!("{}: killed by signal {n}", cmd.name)),
            Err(e) => self.set_status(format!("{}: {e:#}", cmd.name)),
        }
    }
}

/// Default runner: the command in the terminal, stdin from the terminal.
pub fn system_run(cmd: &LaunchCommand) -> anyhow::Result<Exit> {
    use std::io::IsTerminal;
    let stdin_is_tty = std::io::stdin().is_terminal();
    let status = process(cmd, stdin_is_tty, super::effect::open_tty)?
        .status()
        .with_context(|| format!("running {}", cmd.argv[0]))?;
    if let Some(code) = status.code() {
        return Ok(Exit::Code(code));
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            return Ok(Exit::Signal(sig));
        }
    }
    Ok(Exit::Code(-1))
}

/// The child process for `cmd`: in its cwd, stdin from the terminal
/// (`tty()` when our stdin is the document pipe).
fn process(
    cmd: &LaunchCommand,
    stdin_is_tty: bool,
    tty: impl FnOnce() -> std::io::Result<std::fs::File>,
) -> anyhow::Result<std::process::Command> {
    let (prog, args) = cmd
        .argv
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("empty command"))?;
    let mut c = std::process::Command::new(prog);
    c.args(args)
        .current_dir(&cmd.cwd)
        .stdin(super::effect::child_stdin(stdin_is_tty, tty));
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> LaunchVars {
        LaunchVars {
            file: Some("/n/a.md".into()),
            line: 7,
            dir: "/n".into(),
            vcs_root: None,
            editor: vec!["code".into(), "-w".into()],
        }
    }

    fn s(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn expands_per_argument() {
        let got = expand(
            &s(&["${editor}", "+${line}", "${file}", "${dir}/x"]),
            &vars(),
        );
        assert_eq!(got.unwrap(), s(&["code", "-w", "+7", "/n/a.md", "/n/x"]));
    }

    #[test]
    fn missing_values_are_errors() {
        let mut v = vars();
        assert!(expand(&s(&["${vcs_root}"]), &v).is_err());
        v.file = None;
        assert!(expand(&s(&["${file}"]), &v).is_err());
        assert!(expand(&s(&["${nope}"]), &vars()).is_err());
    }

    #[test]
    fn key_grammar() {
        assert_eq!(parse_key("<leader>rr").unwrap(), vec!['r', 'r']);
        assert!(parse_key("<leader><C-x>").is_err());
        assert!(parse_key("x").is_err());
        assert!(parse_key("<leader>").is_err());
    }

    #[test]
    fn leader_waits_on_strict_prefix() {
        let b = vec![(vec!['o'], 0), (vec!['r', 'r'], 1), (vec!['r', 'w'], 2)];
        assert_eq!(match_leader(&b, &[]), LeaderMatch::Pending);
        assert_eq!(match_leader(&b, &['r']), LeaderMatch::Pending);
        assert_eq!(match_leader(&b, &['r', 'w']), LeaderMatch::Launch(2));
        assert_eq!(match_leader(&b, &['o']), LeaderMatch::Launch(0));
        assert_eq!(match_leader(&b, &['x']), LeaderMatch::NoMapping);
    }

    #[test]
    fn editor_fallbacks() {
        let none = |_: &str| None;
        assert_eq!(editor_words(&none), vec!["vi"]);
        let blank_visual = |k: &str| match k {
            "VISUAL" => Some("  ".into()),
            "EDITOR" => Some("nano -w".into()),
            _ => None,
        };
        assert_eq!(editor_words(&blank_visual), vec!["nano", "-w"]);
    }

    #[test]
    fn runner_reads_the_tty_when_stdin_is_the_document_pipe() {
        let dir = tempfile::tempdir().unwrap();
        let tty = dir.path().join("tty");
        std::fs::write(&tty, "from tty").unwrap();
        let cmd = LaunchCommand {
            name: "t".into(),
            argv: s(&["sh", "-c", "pwd; cat"]),
            cwd: dir.path().to_path_buf(),
        };
        let out = process(&cmd, false, || std::fs::File::open(&tty))
            .unwrap()
            .output()
            .unwrap();
        let out = String::from_utf8(out.stdout).unwrap();
        let cwd = dir.path().canonicalize().unwrap();
        assert_eq!(out, format!("{}\nfrom tty", cwd.display()));
        // Stdin already the terminal: the tty is not opened.
        process(&cmd, true, || panic!("opened the tty")).unwrap();
    }
}
