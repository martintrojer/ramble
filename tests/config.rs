//! Config loading, overlay semantics, errors and the embedded default file.

use std::path::Path;

use ramble::config::{
    self, Config, Launcher, PositionEncoding, ServerKind, SidebarMode, SidebarReading,
};

fn load_str(src: &str) -> anyhow::Result<Config> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, src).unwrap();
    Config::load(&path)
}

fn err_str(src: &str) -> String {
    format!("{:#}", load_str(src).unwrap_err())
}

fn names(c: &Config) -> Vec<&str> {
    c.launch.iter().map(|l| l.name.as_str()).collect()
}

#[test]
fn missing_file_yields_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let c = Config::load(&dir.path().join("nope.toml")).unwrap();
    assert_eq!(c, Config::default());
}

#[test]
fn defaults_match_spec() {
    let c = Config::default();
    assert_eq!(c.render.max_width, 100);
    assert_eq!(c.render.theme, "catppuccin-mocha");
    assert!(c.render.math);
    assert_eq!(c.sidebar.default, SidebarMode::Auto);
    assert_eq!(c.sidebar.width, 30);
    assert_eq!(c.sidebar.split_ratio, 0.5);
    assert!(!c.sidebar.show_all);
    assert_eq!(c.sidebar.reading, SidebarReading::Outline);
    assert_eq!(c.keys.leader, ' ');
    assert_eq!(c.lsp.server.len(), 2);
    assert_eq!(c.lsp.server[0].kind, ServerKind::Zk);
    assert_eq!(c.lsp.server[0].command, ["zk", "lsp"]);
    assert_eq!(c.lsp.server[0].root_markers, [".zk"]);
    assert_eq!(c.lsp.server[1].kind, ServerKind::Marksman);
    assert_eq!(c.lsp.server[1].command, ["marksman", "server"]);
    assert_eq!(c.lsp.server[1].root_markers, [".marksman.toml", ".git"]);
    // Spec § Launchers, verbatim.
    let launcher = |name: &str, key: &str, command: &[&str], needs_vcs: bool| Launcher {
        name: name.into(),
        key: Some(key.into()),
        command: command.iter().map(|s| s.to_string()).collect(),
        needs_vcs,
        disabled: false,
    };
    assert_eq!(
        c.launch,
        [
            launcher(
                "edit",
                "<leader>o",
                &["${editor}", "+${line}", "${file}"],
                false
            ),
            launcher(
                "review",
                "<leader>rr",
                &["tuicr", "--file", "${file}", "--line", "${line}"],
                false
            ),
            launcher(
                "review-changes",
                "<leader>rw",
                &["tuicr", "-w", "-p", "${file}", "--line", "${line}"],
                true
            ),
            launcher(
                "review-dir",
                "<leader>rd",
                &["tuicr", "--file", "${dir}"],
                false
            ),
        ]
    );
    assert!(c.review.enabled);
    assert_eq!(c.review.command, "tuicr");
}

#[test]
fn partial_override_keeps_other_defaults() {
    let c = load_str(
        "[render]\nmax_width = 72\nmath = false\n[sidebar]\ndefault = \"split\"\n[keys]\nleader = \",\"\n[review]\nenabled = false\n",
    )
    .unwrap();
    let mut want = Config::default();
    want.render.max_width = 72;
    want.render.math = false;
    want.sidebar.default = SidebarMode::Split;
    want.keys.leader = ',';
    want.review.enabled = false;
    assert_eq!(c, want);
}

#[test]
fn lsp_servers_replace_defaults() {
    let c = load_str(
        "[[lsp.server]]\nkind = \"generic\"\ncommand = [\"my-ls\"]\nposition_encoding = \"utf-16\"\n",
    )
    .unwrap();
    assert_eq!(c.lsp.server.len(), 1);
    let s = &c.lsp.server[0];
    assert_eq!(s.kind, ServerKind::Generic);
    assert_eq!(s.command, ["my-ls"]);
    assert!(s.root_markers.is_empty());
    assert_eq!(s.position_encoding, Some(PositionEncoding::Utf16));
}

#[test]
fn launch_replace_disable_append() {
    let c = load_str(
        r#"
[[launch]]
name = "edit"
command = ["hx", "${file}"]

[[launch]]
name = "review"
disabled = true

[[launch]]
name = "open"
key = "<leader>x"
command = ["open", "${file}"]
needs_vcs = true

[[launch]]
name = "nonexistent"
disabled = true
"#,
    )
    .unwrap();
    assert_eq!(names(&c), ["edit", "review-changes", "review-dir", "open"]);
    assert_eq!(
        c.launch[0],
        Launcher {
            name: "edit".into(),
            key: None,
            command: vec!["hx".into(), "${file}".into()],
            needs_vcs: false,
            disabled: false,
        }
    );
    assert_eq!(c.launch[3].key.as_deref(), Some("<leader>x"));
    assert!(c.launch[3].needs_vcs);
}

#[test]
fn launch_without_command_is_an_error() {
    let e = err_str("[[launch]]\nname = \"x\"\n");
    assert!(e.contains("command"), "{e}");
    assert!(e.contains("line"), "{e}");
}

#[test]
fn unknown_key_is_an_error_naming_the_key() {
    let e = err_str("[render]\nmax_widht = 80\n");
    assert!(e.contains("max_widht"), "{e}");
    assert!(e.contains("line 2"), "{e}");
    let e = err_str("[sidbar]\n");
    assert!(e.contains("sidbar"), "{e}");
    let e = err_str("[[lsp.server]]\nkind = \"zk\"\ncommand = []\nroot = []\n");
    assert!(e.contains("root"), "{e}");
}

#[test]
fn unknown_key_in_launch_entry_is_an_error_naming_the_key() {
    let e = err_str("[[launch]]\nname = \"x\"\ncommand = [\"y\"]\nneeds_vsc = true\n");
    assert!(e.contains("needs_vsc"), "{e}");
    assert!(e.contains("line 4"), "{e}");
}

#[test]
fn syntax_error_reports_line_and_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("c.toml");
    std::fs::write(&path, "[render]\nmax_width = 80\ntheme = \n").unwrap();
    let e = format!("{:#}", Config::load(&path).unwrap_err());
    assert!(
        e.starts_with(&format!("config error at {} line 3:", path.display())),
        "{e}"
    );
    assert!(!e.contains("ramble:"), "{e}");
}

#[test]
fn bad_enum_value_is_an_error() {
    let e = err_str("[sidebar]\ndefault = \"left\"\n");
    assert!(e.contains("line 2"), "{e}");
}

#[test]
fn sidebar_reading_parses() {
    let c = load_str("[sidebar]\nreading = \"split\"\n").unwrap();
    assert_eq!(c.sidebar.reading, SidebarReading::Split);
    let e = err_str("[sidebar]\nreading = \"files\"\n");
    assert!(e.contains("line 2"), "{e}");
}

#[test]
fn write_default_creates_dirs_and_refuses_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a").join("b").join("config.toml");
    config::write_default(&path).unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        config::DEFAULT_TOML
    );
    std::fs::write(&path, "keep").unwrap();
    let e = format!("{:#}", config::write_default(&path).unwrap_err());
    assert_eq!(e, format!("{} already exists", path.display()));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep");
}

fn default_file() -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("config.default.toml"))
        .unwrap()
}

#[test]
fn commented_default_file_loads_as_defaults() {
    assert_eq!(config::DEFAULT_TOML, default_file());
    assert_eq!(load_str(&default_file()).unwrap(), Config::default());
}

#[test]
fn uncommented_default_file_equals_defaults() {
    let src = default_file();
    let uncommented: String = src
        .lines()
        .map(|l| match l.strip_prefix("# ") {
            Some(rest) if !l.starts_with("##") => rest,
            _ => l,
        })
        .collect::<Vec<_>>()
        .join("\n");
    // Guard against a vacuous pass: every section must really be present.
    for header in [
        "[render]",
        "[sidebar]",
        "[keys]",
        "[review]",
        "[[lsp.server]]",
        "[[launch]]",
    ] {
        assert!(uncommented.lines().any(|l| l == header), "missing {header}");
    }
    assert_eq!(
        uncommented.lines().filter(|l| *l == "[[launch]]").count(),
        4
    );
    assert_eq!(
        uncommented
            .lines()
            .filter(|l| *l == "[[lsp.server]]")
            .count(),
        2
    );
    assert_eq!(load_str(&uncommented).unwrap(), Config::default());
}
