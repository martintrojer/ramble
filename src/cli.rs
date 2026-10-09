//! Command-line arguments: tree root, start target, TTY detection, --print, --init-config.
//!
//! [`plan`] turns parsed [`Args`] plus an injected [`Env`] into a [`Plan`];
//! `main.rs` only executes it. Nothing here reads the process environment
//! except [`Env::from_process`].

use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};

use clap::Parser;

use crate::app::{StartOptions, StartTarget};
use crate::config::Config;

/// Label used for a document read from stdin.
pub const STDIN_LABEL: &str = "[stdin]";

/// Directory markers that make a directory a VCS root.
const VCS_MARKERS: [&str; 3] = [".git", ".jj", ".sl"];

#[derive(Debug, Parser)]
#[command(name = "ramble", version, about)]
pub struct Args {
    /// Markdown file to open, or directory to browse.
    pub path: Option<PathBuf>,
    /// Render once to stdout and exit (automatic when stdout is not a terminal).
    #[arg(long)]
    pub print: bool,
    /// Render width in print mode.
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u16).range(1..))]
    pub width: Option<u16>,
    /// Write the commented default config and exit.
    #[arg(long)]
    pub init_config: bool,
    /// Config file to use instead of the default location.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,
}

/// Everything `plan` needs from the process, injectable for tests.
pub struct Env {
    pub stdin_is_tty: bool,
    pub stdout_is_tty: bool,
    /// Raw `$COLUMNS`.
    pub columns: Option<String>,
    /// Width of stdout when it is a terminal.
    pub term_width: Option<u16>,
    pub cwd: PathBuf,
    pub home: PathBuf,
    pub stdin: Box<dyn Read>,
}

impl Env {
    pub fn from_process() -> Self {
        let stdout_is_tty = std::io::stdout().is_terminal();
        let term_width = if stdout_is_tty {
            crossterm::terminal::size().ok().map(|(w, _)| w)
        } else {
            None
        };
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| cwd.clone());
        Self {
            stdin_is_tty: std::io::stdin().is_terminal(),
            stdout_is_tty,
            columns: std::env::var("COLUMNS").ok(),
            term_width,
            cwd,
            home,
            stdin: Box::new(std::io::stdin()),
        }
    }
}

/// What `main` should do.
#[derive(Debug)]
pub enum Plan {
    /// Render `bytes` once at `width` to stdout. `label` names the source
    /// in messages (a path or `[stdin]`).
    Print {
        bytes: Vec<u8>,
        label: String,
        width: u16,
    },
    Interactive(StartOptions),
    /// Write the default config. `None` means `Config::default_path()`.
    InitConfig(Option<PathBuf>),
    UsageError {
        msg: String,
        code: i32,
    },
}

fn error(msg: String, code: i32) -> Plan {
    Plan::UsageError { msg, code }
}

/// Decide what to run. Reads only `env`, the path argument, and `env.stdin`.
pub fn plan(args: &Args, env: &mut Env, config: &Config) -> Plan {
    if args.init_config {
        return Plan::InitConfig(args.config.clone());
    }
    let print = args.print || !env.stdout_is_tty;

    let arg = match &args.path {
        Some(p) => match env.cwd.join(p).canonicalize() {
            Ok(abs) => Some((p, abs)),
            Err(e) => return error(format!("ramble: {}: {e}", p.display()), 1),
        },
        None => None,
    };

    let target = match &arg {
        Some((_, abs)) if abs.is_dir() => StartTarget::Dir(abs.clone()),
        Some((_, abs)) => StartTarget::File(abs.clone()),
        None if !env.stdin_is_tty => {
            let mut bytes = Vec::new();
            if let Err(e) = env.stdin.read_to_end(&mut bytes) {
                return error(format!("ramble: reading stdin: {e}"), 1);
            }
            if print {
                return Plan::Print {
                    bytes,
                    label: STDIN_LABEL.into(),
                    width: print_width(args.width, env, config.render.max_width),
                };
            }
            // v1: the lossy flag is not carried for stdin documents.
            StartTarget::Stdin(String::from_utf8_lossy(&bytes).into_owned())
        }
        None => StartTarget::Dir(env.cwd.clone()),
    };

    if print {
        return match (&target, &arg) {
            (StartTarget::File(abs), Some((shown, _))) => match std::fs::read(abs) {
                Ok(bytes) => Plan::Print {
                    bytes,
                    label: shown.display().to_string(),
                    width: print_width(args.width, env, config.render.max_width),
                },
                Err(e) => error(format!("ramble: {}: {e}", shown.display()), 1),
            },
            _ => error("ramble: --print needs a file or stdin".into(), 2),
        };
    }

    let tree_root = tree_root(
        arg.as_ref().map(|(_, abs)| abs.as_path()),
        &env.cwd,
        &env.home,
    );
    Plan::Interactive(StartOptions {
        target,
        tree_root,
        config: config.clone(),
        review_cache: None,
    })
}

/// Tree root: a file argument's parent dir; a dir argument itself; with no
/// path argument, the nearest ancestor of `cwd` holding `.git`, `.jj` or
/// `.sl`, else `home`. Paths are canonicalized when possible.
pub fn tree_root(arg: Option<&Path>, cwd: &Path, home: &Path) -> PathBuf {
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    match arg {
        Some(p) => {
            let p = canon(&cwd.join(p));
            if p.is_dir() {
                p
            } else {
                p.parent().map(Path::to_path_buf).unwrap_or(p)
            }
        }
        None => canon(cwd)
            .ancestors()
            .find(|d| VCS_MARKERS.iter().any(|m| d.join(m).exists()))
            .map(Path::to_path_buf)
            .unwrap_or_else(|| canon(home)),
    }
}

/// Print-mode width: `--width`, else a positive `$COLUMNS`, else the
/// terminal width, else 80; then capped at `max_width`.
pub fn print_width(width: Option<u16>, env: &Env, max_width: u16) -> u16 {
    width
        .or_else(|| {
            env.columns
                .as_deref()
                .and_then(|c| c.trim().parse::<u16>().ok())
                .filter(|&c| c > 0)
        })
        .or(env.term_width.filter(|&w| w > 0))
        .unwrap_or(80)
        .min(max_width)
}
