//! cli unit tests: `plan`, `tree_root`, `print_width` with an injected `Env`.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use clap::Parser;
use ramble::app::StartTarget;
use ramble::cli::{Args, Env, Plan, STDIN_LABEL, plan, print_width, tree_root};
use ramble::config::Config;
use tempfile::TempDir;

fn args(argv: &[&str]) -> Args {
    Args::try_parse_from(std::iter::once("ramble").chain(argv.iter().copied())).unwrap()
}

/// Interactive terminal by default; tests flip fields as needed.
fn env(cwd: &Path, home: &Path) -> Env {
    Env {
        stdin_is_tty: true,
        stdout_is_tty: true,
        columns: None,
        term_width: None,
        cwd: cwd.to_path_buf(),
        home: home.to_path_buf(),
        stdin: Box::new(Cursor::new(Vec::new())),
    }
}

fn piped(mut e: Env, data: &str) -> Env {
    e.stdin_is_tty = false;
    e.stdin = Box::new(Cursor::new(data.as_bytes().to_vec()));
    e
}

fn canon(p: &Path) -> PathBuf {
    p.canonicalize().unwrap()
}

struct Fixture {
    _tmp: TempDir,
    /// `<tmp>/repo` with a `.jj` dir.
    repo: PathBuf,
    /// `<tmp>/repo/a/b`.
    nested: PathBuf,
    /// `<tmp>/repo/a/b/f.md`.
    file: PathBuf,
    /// `<tmp>/home`, no VCS.
    home: PathBuf,
}

fn fixture() -> Fixture {
    let tmp = TempDir::new().unwrap();
    let root = canon(tmp.path());
    let repo = root.join("repo");
    let nested = repo.join("a/b");
    fs::create_dir_all(&nested).unwrap();
    fs::create_dir(repo.join(".jj")).unwrap();
    let file = nested.join("f.md");
    fs::write(&file, "# f\n").unwrap();
    let home = root.join("home");
    fs::create_dir(&home).unwrap();
    Fixture {
        _tmp: tmp,
        repo,
        nested,
        file,
        home,
    }
}

fn interactive(p: Plan) -> ramble::app::StartOptions {
    match p {
        Plan::Interactive(o) => o,
        other => panic!("expected Interactive, got {other:?}"),
    }
}

#[test]
fn file_arg_wins_over_piped_stdin() {
    let f = fixture();
    let mut e = piped(env(&f.home, &f.home), "# other\n");
    let o = interactive(plan(
        &args(&[f.file.to_str().unwrap()]),
        &mut e,
        &Config::default(),
    ));
    assert_eq!(o.target, StartTarget::File(f.file.clone()));
}

#[test]
fn file_arg_relative_to_cwd() {
    let f = fixture();
    let mut e = env(&f.nested, &f.home);
    let o = interactive(plan(&args(&["f.md"]), &mut e, &Config::default()));
    assert_eq!(o.target, StartTarget::File(f.file.clone()));
}

#[test]
fn dir_arg_opens_dir() {
    let f = fixture();
    let mut e = piped(env(&f.home, &f.home), "# other\n");
    let o = interactive(plan(
        &args(&[f.nested.to_str().unwrap()]),
        &mut e,
        &Config::default(),
    ));
    assert_eq!(o.target, StartTarget::Dir(f.nested.clone()));
    assert_eq!(o.tree_root, f.nested);
}

#[test]
fn no_arg_piped_stdin_reads_stdin_lossy() {
    let f = fixture();
    let mut e = env(&f.nested, &f.home);
    e.stdin_is_tty = false;
    e.stdin = Box::new(Cursor::new(b"# hi \xff\n".to_vec()));
    let o = interactive(plan(&args(&[]), &mut e, &Config::default()));
    assert_eq!(o.target, StartTarget::Stdin("# hi \u{fffd}\n".into()));
    assert_eq!(o.tree_root, f.repo);
}

#[test]
fn no_arg_tty_stdin_opens_cwd() {
    let f = fixture();
    let mut e = env(&f.nested, &f.home);
    let o = interactive(plan(&args(&[]), &mut e, &Config::default()));
    assert_eq!(o.target, StartTarget::Dir(f.nested.clone()));
}

#[test]
fn missing_path_is_an_error() {
    let f = fixture();
    let mut e = env(&f.home, &f.home);
    match plan(&args(&["nope.md"]), &mut e, &Config::default()) {
        Plan::UsageError { msg, code } => {
            assert_eq!(code, 1);
            assert!(msg.contains("nope.md"), "{msg}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn print_flag_triggers_print() {
    let f = fixture();
    let mut e = env(&f.home, &f.home);
    match plan(
        &args(&["--print", "--width", "60", f.file.to_str().unwrap()]),
        &mut e,
        &Config::default(),
    ) {
        Plan::Print {
            bytes,
            label,
            width,
        } => {
            assert_eq!(bytes, b"# f\n");
            assert_eq!(label, f.file.display().to_string());
            assert_eq!(width, 60);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn stdout_not_tty_triggers_print_of_stdin() {
    let f = fixture();
    let mut e = piped(env(&f.home, &f.home), "# piped\n");
    e.stdout_is_tty = false;
    match plan(&args(&[]), &mut e, &Config::default()) {
        Plan::Print {
            bytes,
            label,
            width,
        } => {
            assert_eq!(bytes, b"# piped\n");
            assert_eq!(label, STDIN_LABEL);
            assert_eq!(width, 80);
        }
        other => panic!("{other:?}"),
    }
}

fn assert_usage_2(p: Plan) {
    match p {
        Plan::UsageError { msg, code } => {
            assert_eq!(code, 2);
            assert_eq!(msg, "ramble: --print needs a file or stdin");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn print_with_dir_arg_is_usage_error() {
    let f = fixture();
    let mut e = env(&f.home, &f.home);
    assert_usage_2(plan(
        &args(&["--print", f.nested.to_str().unwrap()]),
        &mut e,
        &Config::default(),
    ));
}

#[test]
fn print_with_no_arg_and_tty_stdin_is_usage_error() {
    let f = fixture();
    let mut e = env(&f.home, &f.home);
    e.stdout_is_tty = false;
    assert_usage_2(plan(&args(&[]), &mut e, &Config::default()));
}

#[test]
fn init_config_plan() {
    let f = fixture();
    let mut e = env(&f.home, &f.home);
    let p = plan(
        &args(&["--init-config", "--config", "/x/c.toml"]),
        &mut e,
        &Config::default(),
    );
    assert!(matches!(p, Plan::InitConfig(Some(ref c)) if c == Path::new("/x/c.toml")));
    let p = plan(&args(&["--init-config"]), &mut e, &Config::default());
    assert!(matches!(p, Plan::InitConfig(None)));
}

#[test]
fn width_zero_rejected() {
    assert!(Args::try_parse_from(["ramble", "--width", "0"]).is_err());
}

#[test]
fn tree_root_file_is_parent_dir() {
    let f = fixture();
    assert_eq!(tree_root(Some(&f.file), &f.home, &f.home), f.nested);
}

#[test]
fn tree_root_dir_arg_is_that_dir() {
    let f = fixture();
    assert_eq!(tree_root(Some(&f.nested), &f.home, &f.home), f.nested);
}

#[test]
fn tree_root_no_arg_finds_vcs_root() {
    let f = fixture();
    assert_eq!(tree_root(None, &f.nested, &f.home), f.repo);
}

#[test]
fn tree_root_no_arg_no_vcs_is_home() {
    let tmp = TempDir::new().unwrap();
    let cwd = canon(tmp.path()).join("x/y");
    fs::create_dir_all(&cwd).unwrap();
    let home = canon(tmp.path()).join("home");
    fs::create_dir(&home).unwrap();
    // Skip if the temp dir itself sits under a VCS checkout.
    if cwd
        .ancestors()
        .any(|d| [".git", ".jj", ".sl"].iter().any(|m| d.join(m).exists()))
    {
        eprintln!("skipped: temp dir is inside a VCS checkout");
        return;
    }
    assert_eq!(tree_root(None, &cwd, &home), home);
}

#[test]
fn print_width_precedence() {
    let tmp = TempDir::new().unwrap();
    let mut e = env(tmp.path(), tmp.path());
    assert_eq!(print_width(None, &e, 100), 80);
    e.term_width = Some(70);
    assert_eq!(print_width(None, &e, 100), 70);
    e.columns = Some("0".into());
    assert_eq!(print_width(None, &e, 100), 70, "COLUMNS=0 falls through");
    e.columns = Some("abc".into());
    assert_eq!(print_width(None, &e, 100), 70, "bad COLUMNS falls through");
    e.columns = Some("90".into());
    assert_eq!(print_width(None, &e, 100), 90);
    assert_eq!(print_width(Some(40), &e, 100), 40);
    assert_eq!(print_width(Some(140), &e, 100), 100, "capped by max_width");
    e.columns = Some("200".into());
    assert_eq!(print_width(None, &e, 100), 100);
}

#[test]
fn print_width_uses_config_max_width() {
    let f = fixture();
    let mut e = env(&f.home, &f.home);
    let mut config = Config::default();
    config.render.max_width = 50;
    match plan(
        &args(&["--print", "--width", "60", f.file.to_str().unwrap()]),
        &mut e,
        &config,
    ) {
        Plan::Print { width, .. } => assert_eq!(width, 50),
        other => panic!("{other:?}"),
    }
}

// ORCH-FOLLOWUP: enable after t02 merges (doc::from_bytes is todo!() on main).
#[test]
#[ignore]
fn binary_file_shows_message() {
    let tmp = TempDir::new().unwrap();
    let bin = tmp.path().join("b.md");
    fs::write(&bin, b"\0\x01\x02").unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_ramble"))
        .args(["--print", "--config"])
        .arg(tmp.path().join("none.toml"))
        .arg(&bin)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("looks binary"));
}

// ORCH-FOLLOWUP: enable after t02+t03 merge.
#[test]
#[ignore]
fn file_arg_beats_stdin_end_to_end() {
    let tmp = TempDir::new().unwrap();
    let f = tmp.path().join("f.md");
    fs::write(&f, "# from file\n").unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_ramble"))
        .args(["--print", "--width", "80", "--config"])
        .arg(tmp.path().join("none.toml"))
        .arg(&f)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"# from stdin\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("from file") && !s.contains("from stdin"), "{s}");
}

#[test]
fn print_without_document_exits_2_end_to_end() {
    let tmp = TempDir::new().unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_ramble"))
        .args(["--print", "--config"])
        .arg(tmp.path().join("none.toml"))
        .arg(tmp.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--print needs a file or stdin"));
}
