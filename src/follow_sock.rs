//! Follow the editor's cursor (`--follow`, `--goto`): the socket side.
//!
//! A following ramble listens on `<dir>/ramble-<uid>/<scope>.<pid>.sock`.
//! Editor hooks write `<line> <absolute path>\n` to every socket in their
//! scope. The scope is the tmux window (`t<server pid>w<window id>`), or
//! `any` outside tmux; a sender in scope `any` writes to every socket.
//! See docs/specs/2026-10-22-follow-cursor.md.

use std::io::{BufRead, BufReader, ErrorKind, Read, Write};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The scope of a sender or listener outside tmux.
pub const ANY: &str = "any";

/// How long a sender waits on one socket, and a listener on one client.
const IO_TIMEOUT: Duration = Duration::from_millis(500);

/// The most a listener reads from one client: far more than a few
/// `<line> <path>` messages, so a stuck client can't grow memory.
const MAX_READ: u64 = 64 * 1024;

/// `<base>/ramble-<uid>`, where base is the first set, non-empty one of
/// `$XDG_RUNTIME_DIR`, `$TMPDIR`, else `/tmp`.
pub fn socket_dir(env: impl Fn(&str) -> Option<String>, uid: u32) -> PathBuf {
    let base = ["XDG_RUNTIME_DIR", "TMPDIR"]
        .iter()
        .find_map(|k| env(k).filter(|v| !v.is_empty()))
        .unwrap_or_else(|| "/tmp".into());
    Path::new(&base).join(format!("ramble-{uid}"))
}

/// [`socket_dir`] for this process.
pub fn default_dir() -> PathBuf {
    // SAFETY: geteuid has no preconditions and cannot fail.
    let uid = unsafe { libc::geteuid() };
    socket_dir(|k| std::env::var(k).ok(), uid)
}

/// Create `dir` mode 0700, or check that an existing one is ours
/// ([`check_dir`]).
pub fn ensure_dir(dir: &Path) -> std::io::Result<()> {
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => check_dir(dir),
        Err(e) => Err(e),
    }
}

/// Check that `dir` is a real directory (not a symlink) we own that no one
/// else can read or write. Senders check it too: on a shared `/tmp`
/// another user could pre-create it and collect our paths.
pub fn check_dir(dir: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(dir)?;
    // SAFETY: as in default_dir.
    let uid = unsafe { libc::geteuid() };
    let bad = if !meta.is_dir() {
        Some("not a directory")
    } else if meta.uid() != uid {
        Some("owned by another user")
    } else if meta.permissions().mode() & 0o077 != 0 {
        Some("accessible by other users")
    } else {
        None
    };
    match bad {
        Some(why) => Err(std::io::Error::other(format!("{}: {why}", dir.display()))),
        None => Ok(()),
    }
}

/// The scope for `$TMUX` (`socket,server_pid,session`) and a tmux window
/// id (`@12`): `t<server pid>w<window digits>`. [`ANY`] when either is
/// missing or malformed.
pub fn scope_from(tmux: Option<&str>, window_id: Option<&str>) -> String {
    let pid = tmux
        .and_then(|t| t.split(',').nth(1))
        .filter(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    let win = window_id
        .map(|w| w.trim().trim_start_matches('@'))
        .filter(|w| !w.is_empty() && w.bytes().all(|b| b.is_ascii_digit()));
    match (pid, win) {
        (Some(p), Some(w)) => format!("t{p}w{w}"),
        _ => ANY.into(),
    }
}

/// This process's scope: asks tmux for the window of `$TMUX_PANE` when
/// inside tmux (both `$TMUX` and `$TMUX_PANE` set).
pub fn current_scope() -> String {
    // Without the pane, tmux would answer for the most recent client's
    // window, which may not be ours.
    let (Ok(tmux), Ok(pane)) = (std::env::var("TMUX"), std::env::var("TMUX_PANE")) else {
        return ANY.into();
    };
    let win = std::process::Command::new("tmux")
        .args(["display-message", "-p", "-t", &pane, "#{window_id}"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned());
    scope_from(Some(&tmux), win.as_deref())
}

/// One message line: `<line> <path>`, line 1-based, path absolute and
/// the rest of the line (spaces and colons allowed).
pub fn parse_line(s: &str) -> Option<(usize, PathBuf)> {
    let s = s.strip_suffix('\n').unwrap_or(s);
    let s = s.strip_suffix('\r').unwrap_or(s);
    let (n, path) = s.split_once(' ')?;
    let n: usize = n.parse().ok().filter(|&n| n >= 1)?;
    let path = Path::new(path);
    path.is_absolute().then(|| (n, path.to_path_buf()))
}

/// The message for `line` of `path`.
pub fn message(line: usize, path: &Path) -> String {
    format!("{line} {}\n", path.display())
}

/// The socket file name for `scope` and `pid`.
pub fn socket_name(scope: &str, pid: u32) -> String {
    format!("{scope}.{pid}.sock")
}

/// Whether a sender in `scope` writes to the socket named `name`.
pub fn in_scope(scope: &str, name: &str) -> bool {
    if !name.ends_with(".sock") {
        return false;
    }
    scope == ANY
        || name
            .split_once('.')
            .is_some_and(|(s, _)| s == scope || s == ANY)
}

/// Send `line` of `path` to every socket in `dir` that a sender in
/// `scope` reaches. Nothing is sent unless `dir` passes [`check_dir`]. A
/// socket that refuses the connection (its ramble is gone) is deleted.
/// Errors are swallowed: editor hooks must never fail.
pub fn send_all(dir: &Path, scope: &str, line: usize, path: &Path) {
    if check_dir(dir).is_err() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let msg = message(line, path);
    for e in entries.flatten() {
        let name = e.file_name();
        let is_socket = e.file_type().is_ok_and(|t| t.is_socket());
        if !is_socket || !in_scope(scope, &name.to_string_lossy()) {
            continue;
        }
        let sock = e.path();
        match UnixStream::connect(&sock) {
            Ok(mut s) => {
                let _ = s.set_write_timeout(Some(IO_TIMEOUT));
                let _ = s.write_all(msg.as_bytes());
            }
            Err(e) if e.kind() == ErrorKind::ConnectionRefused => {
                let _ = std::fs::remove_file(&sock);
            }
            Err(_) => {}
        }
    }
}

/// A bound follow socket. Dropping it removes the socket file.
#[derive(Debug)]
pub struct Listener {
    path: PathBuf,
}

impl Listener {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Bind `<dir>/<scope>.<pid>.sock` (creating `dir` per [`ensure_dir`])
/// and serve it on a thread: each client's last valid line goes to
/// `emit(path, line)`.
pub fn listen(
    dir: &Path,
    scope: &str,
    emit: impl Fn(PathBuf, usize) + Send + 'static,
) -> std::io::Result<Listener> {
    ensure_dir(dir)?;
    let path = dir.join(socket_name(scope, std::process::id()));
    // A leftover from an earlier process with our pid.
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    std::thread::Builder::new()
        .name("ramble-follow".into())
        .spawn(move || {
            for conn in listener.incoming() {
                let Ok(conn) = conn else {
                    // e.g. out of fds: don't spin.
                    std::thread::sleep(IO_TIMEOUT);
                    continue;
                };
                if let Some((line, path)) = read_client(conn) {
                    emit(path, line);
                }
            }
        })?;
    Ok(Listener { path })
}

/// The last valid line one client sends. Clients are served one at a
/// time, so each gets [`IO_TIMEOUT`] in total and [`MAX_READ`] bytes:
/// a client that holds its connection open can't stall the others.
fn read_client(conn: UnixStream) -> Option<(usize, PathBuf)> {
    let deadline = std::time::Instant::now() + IO_TIMEOUT;
    let mut reader = BufReader::new(conn.try_clone().ok()?.take(MAX_READ));
    let mut last = None;
    let mut buf = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() || conn.set_read_timeout(Some(left)).is_err() {
            break;
        }
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) => break,
            // A cut-off last line (timeout, cap) is dropped.
            Ok(_) if buf.ends_with(b"\n") => {
                if let Some(m) = std::str::from_utf8(&buf).ok().and_then(parse_line) {
                    last = Some(m);
                }
            }
            Ok(_) | Err(_) => break,
        }
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_dir_prefers_xdg_then_tmpdir_then_tmp() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| v.to_string())
            }
        };
        assert_eq!(
            socket_dir(env(&[("XDG_RUNTIME_DIR", "/run/u"), ("TMPDIR", "/t")]), 7),
            Path::new("/run/u/ramble-7")
        );
        assert_eq!(
            socket_dir(env(&[("XDG_RUNTIME_DIR", ""), ("TMPDIR", "/t/")]), 7),
            Path::new("/t/ramble-7")
        );
        assert_eq!(socket_dir(env(&[]), 7), Path::new("/tmp/ramble-7"));
    }

    #[test]
    fn scope_needs_both_server_pid_and_window() {
        assert_eq!(
            scope_from(Some("/tmp/tmux-1000/default,4242,0"), Some("@12\n")),
            "t4242w12"
        );
        assert_eq!(scope_from(None, Some("@12")), ANY);
        assert_eq!(scope_from(Some("/tmp/s,4242,0"), None), ANY);
        assert_eq!(scope_from(Some("/tmp/s,,0"), Some("@1")), ANY);
        assert_eq!(scope_from(Some("/tmp/s,4242,0"), Some("oops")), ANY);
    }

    #[test]
    fn parse_takes_the_rest_of_the_line_as_the_path() {
        assert_eq!(
            parse_line("42 /a b/c:d.md\n"),
            Some((42, PathBuf::from("/a b/c:d.md")))
        );
        assert_eq!(parse_line("1 /x.md\r\n"), Some((1, "/x.md".into())));
        assert_eq!(parse_line("0 /x.md"), None, "lines are 1-based");
        assert_eq!(parse_line("x /x.md"), None);
        assert_eq!(parse_line("3 rel.md"), None, "paths are absolute");
        assert_eq!(parse_line("3"), None);
        let p = Path::new("/n/a b.md");
        assert_eq!(parse_line(&message(9, p)), Some((9, p.to_path_buf())));
    }

    #[test]
    fn senders_reach_their_scope_and_any() {
        let s = |scope, pid| socket_name(scope, pid);
        assert!(in_scope("t1w2", &s("t1w2", 5)));
        assert!(in_scope("t1w2", &s(ANY, 5)));
        assert!(!in_scope("t1w2", &s("t1w3", 5)));
        assert!(!in_scope("t1w2", &s("t9w2", 5)));
        assert!(in_scope(ANY, &s("t1w3", 5)), "outside tmux: everyone");
        assert!(!in_scope(ANY, "notes.txt"));
    }
}
