//! Config loading, overlay semantics, errors and the embedded default file.

use std::path::Path;

use ramble::config::{
    self, Config, Launcher, PositionEncoding, ServerKind, SidebarMode, SidebarReading, SidebarShow,
    SidebarSide, SidebarWidth,
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
    assert_eq!(c.sidebar.show, SidebarShow::Always);
    assert_eq!(c.sidebar.default, SidebarMode::Auto);
    assert_eq!(c.sidebar.width, SidebarWidth::Auto);
    assert_eq!(c.sidebar.min_width, 16);
    assert_eq!(c.sidebar.max_width, 48);
    assert_eq!(c.sidebar.auto_hide_below, 80);
    assert_eq!(c.sidebar.split_ratio, 0.5);
    assert!(!c.sidebar.show_all);
    assert_eq!(c.sidebar.reading, SidebarReading::Outline);
    assert_eq!(c.sidebar.side, SidebarSide::Left);
    assert_eq!(c.keys.leader, ' ');
    assert!(c.mouse.enabled);
    assert!(
        c.lsp.server.is_empty(),
        "mdroots serves every page by default"
    );
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
        [launcher(
            "edit",
            "<leader>o",
            &["${editor}", "+${line}", "${file}"],
            false
        ),]
    );
    assert!(c.review.enabled);
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
fn send_takes_a_command_and_a_preamble() {
    let c = Config::default();
    assert!(c.send.command.is_empty());
    assert_eq!(c.send.preamble, None);
    let c = load_str("[send]\ncommand = [\"sh\", \"-c\", \"cat\"]\npreamble = \"\"\n").unwrap();
    assert_eq!(c.send.command, ["sh", "-c", "cat"]);
    assert_eq!(c.send.preamble.as_deref(), Some(""));
    assert!(load_str("[send]\nrecipient = \"x\"\n").is_err());
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
fn a_zk_server_entry_is_exactly_the_server_list() {
    let c = load_str(
        "[[lsp.server]]\nkind = \"zk\"\ncommand = [\"zk\", \"lsp\"]\nroot_markers = [\".zk\"]\n",
    )
    .unwrap();
    assert_eq!(c.lsp.server.len(), 1);
    let s = &c.lsp.server[0];
    assert_eq!(s.kind, ServerKind::Zk);
    assert_eq!(s.command, ["zk", "lsp"]);
    assert_eq!(s.root_markers, [".zk"]);
    assert_eq!(s.position_encoding, None);
}

#[test]
fn launch_replace_disable_append() {
    let c = load_str(
        r#"
[[launch]]
name = "edit"
command = ["hx", "${file}"]

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
    assert_eq!(names(&c), ["edit", "open"]);
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
    assert_eq!(c.launch[1].key.as_deref(), Some("<leader>x"));
    assert!(c.launch[1].needs_vcs);
    let c = load_str("[[launch]]\nname = \"edit\"\ndisabled = true\n").unwrap();
    assert!(c.launch.is_empty(), "disabled removes a default");
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
fn sidebar_show_and_auto_hide_below_parse() {
    let c = load_str("[sidebar]\nshow = \"never\"\nauto_hide_below = 0\n").unwrap();
    assert_eq!(c.sidebar.show, SidebarShow::Never);
    assert_eq!(c.sidebar.auto_hide_below, 0);
    assert_eq!(c.sidebar.default, SidebarMode::Auto);
}

#[test]
fn sidebar_width_auto_number_and_bounds_parse() {
    let c = load_str("[sidebar]\nwidth = \"auto\"\nmin_width = 12\nmax_width = 60\n").unwrap();
    assert_eq!(c.sidebar.width, SidebarWidth::Auto);
    assert_eq!(c.sidebar.min_width, 12);
    assert_eq!(c.sidebar.max_width, 60);
    // A number keeps its old meaning.
    let c = load_str("[sidebar]\nwidth = 30\n").unwrap();
    assert_eq!(c.sidebar.width, SidebarWidth::Fixed(30));
    let e = load_str("[sidebar]\nwidth = \"wide\"\n")
        .unwrap_err()
        .to_string();
    assert!(e.contains("line 2"), "{e}");
    assert!(e.contains("\"auto\" or a number"), "{e}");
}

#[test]
fn sidebar_show_takes_always_never_auto() {
    for (v, want) in [
        ("always", SidebarShow::Always),
        ("never", SidebarShow::Never),
        ("auto", SidebarShow::Auto),
    ] {
        let c = load_str(&format!("[sidebar]\nshow = \"{v}\"\n")).unwrap();
        assert_eq!(c.sidebar.show, want, "{v}");
    }
}

#[test]
fn legacy_sidebar_spellings_are_errors() {
    // D11: no compatibility shims.
    let show = r#"sidebar.show: expected "always", "never" or "auto""#;
    for (src, got) in [
        ("[sidebar]\nshow = true\n", "got true"),
        ("[sidebar]\nshow = false\n", "got false"),
        ("[sidebar]\nshow = \"off\"\n", "got \"off\""),
        ("[sidebar]\nshow = 1\n", "got 1"),
    ] {
        let e = err_str(src);
        assert!(e.contains("line 2"), "{src}: {e}");
        assert!(e.contains(show), "{src}: {e}");
        assert!(e.contains(got), "{src}: {e}");
    }
    let e = err_str("[sidebar]\ndefault = \"off\"\n");
    assert!(e.contains("line 2"), "{e}");
    assert!(
        e.contains("`auto`, `files`, `outline`, `split`"),
        "default names its values: {e}"
    );
    let c = load_str("[sidebar]\ndefault = \"files\"\n").unwrap();
    assert_eq!(c.sidebar.show, SidebarShow::Always);
    assert_eq!(c.sidebar.default, SidebarMode::Files);
}

#[test]
fn mouse_can_be_turned_off() {
    let c = load_str("[mouse]\nenabled = false\n").unwrap();
    assert!(!c.mouse.enabled);
    assert!(err_str("[mouse]\nwheel = 3\n").contains("unknown field"));
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
        "[send]",
        "[mouse]",
        "[[launch]]",
    ] {
        assert!(uncommented.lines().any(|l| l == header), "missing {header}");
    }
    assert_eq!(
        uncommented.lines().filter(|l| *l == "[[launch]]").count(),
        1
    );
    // The server examples are documentation only: no server by default.
    assert!(!uncommented.lines().any(|l| l == "[[lsp.server]]"));
    assert_eq!(load_str(&uncommented).unwrap(), Config::default());
}

#[test]
fn sidebar_side_parses_left_and_right_and_rejects_others() {
    let c = load_str("[sidebar]\nside = \"right\"\n").unwrap();
    assert_eq!(c.sidebar.side, SidebarSide::Right);
    let mut want = Config::default();
    want.sidebar.side = SidebarSide::Right;
    assert_eq!(c, want, "only the side changes");
    let c = load_str("[sidebar]\nside = \"left\"\n").unwrap();
    assert_eq!(c.sidebar.side, SidebarSide::Left);
    assert!(err_str("[sidebar]\nside = \"top\"\n").contains("unknown variant"));
}
