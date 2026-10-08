//! Config loading over built-in defaults: servers, leader key, launchers,
//! render, sidebar and review options.
//!
//! A user file overrides the defaults one field at a time. `[[lsp.server]]`
//! replaces the default server list; `[[launch]]` entries merge by name.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail};
use serde::Deserialize;

/// The commented default config written by `--init-config`.
pub const DEFAULT_TOML: &str = include_str!("../config.default.toml");

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub render: RenderConfig,
    pub sidebar: SidebarConfig,
    pub keys: KeysConfig,
    pub lsp: LspConfig,
    pub launch: Vec<Launcher>,
    pub review: ReviewConfig,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RenderConfig {
    /// Upper bound on the reflow width.
    pub max_width: u16,
    pub theme: String,
    /// Convert LaTeX math to Unicode; off shows the raw source.
    pub math: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SidebarConfig {
    /// Shown at start (a start with no page shows the tree regardless).
    pub show: bool,
    pub default: SidebarMode,
    pub width: u16,
    /// Share of the sidebar given to files in split mode.
    pub split_ratio: f32,
    /// Also list non-markdown files in the tree.
    pub show_all: bool,
    /// What `auto` shows while a page is loaded.
    pub reading: SidebarReading,
    /// Hide the sidebar while the terminal is narrower than this many
    /// columns (0 disables).
    pub auto_hide_below: u16,
}

/// `sidebar.reading`: the mode `auto` uses while a page is loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SidebarReading {
    Outline,
    Split,
}

impl From<SidebarReading> for SidebarMode {
    fn from(r: SidebarReading) -> SidebarMode {
        match r {
            SidebarReading::Outline => SidebarMode::Outline,
            SidebarReading::Split => SidebarMode::Split,
        }
    }
}

/// What the sidebar shows. Whether it is shown is separate
/// (`sidebar.show`, `<leader>e`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SidebarMode {
    Auto,
    Files,
    Outline,
    Split,
}

/// `sidebar.default` as written: [`SidebarMode`] plus the legacy `off`,
/// which means `show = false` with `default = "auto"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RawSidebarMode {
    Auto,
    Off,
    Files,
    Outline,
    Split,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KeysConfig {
    pub leader: char,
    /// Show the next-key box after a pause in a key sequence.
    pub clue: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LspConfig {
    /// Tried in order; the first whose root marker is found wins.
    pub server: Vec<ServerConfig>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    pub kind: ServerKind,
    pub command: Vec<String>,
    #[serde(default)]
    pub root_markers: Vec<String>,
    #[serde(default)]
    pub position_encoding: Option<PositionEncoding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerKind {
    Zk,
    Marksman,
    Generic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum PositionEncoding {
    #[serde(rename = "utf-8")]
    Utf8,
    #[serde(rename = "utf-16")]
    Utf16,
    #[serde(rename = "utf-32")]
    Utf32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Launcher {
    pub name: String,
    pub key: Option<String>,
    pub command: Vec<String>,
    pub needs_vcs: bool,
    pub disabled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReviewConfig {
    pub enabled: bool,
    pub command: String,
}

fn strings(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|s| s.to_string()).collect()
}

fn launcher(name: &str, key: &str, command: &[&str], needs_vcs: bool) -> Launcher {
    Launcher {
        name: name.into(),
        key: Some(key.into()),
        command: strings(command),
        needs_vcs,
        disabled: false,
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            render: RenderConfig {
                max_width: 100,
                theme: "catppuccin-mocha".into(),
                math: true,
            },
            sidebar: SidebarConfig {
                show: true,
                default: SidebarMode::Auto,
                width: 30,
                split_ratio: 0.5,
                show_all: false,
                reading: SidebarReading::Outline,
                auto_hide_below: 80,
            },
            keys: KeysConfig {
                leader: ' ',
                clue: true,
            },
            lsp: LspConfig {
                server: vec![
                    ServerConfig {
                        kind: ServerKind::Zk,
                        command: strings(&["zk", "lsp"]),
                        root_markers: strings(&[".zk"]),
                        position_encoding: None,
                    },
                    ServerConfig {
                        kind: ServerKind::Marksman,
                        command: strings(&["marksman", "server"]),
                        root_markers: strings(&[".marksman.toml", ".git"]),
                        position_encoding: None,
                    },
                ],
            },
            launch: vec![
                launcher(
                    "edit",
                    "<leader>o",
                    &["${editor}", "+${line}", "${file}"],
                    false,
                ),
                launcher(
                    "review",
                    "<leader>rr",
                    &["tuicr", "--file", "${file}", "--line", "${line}"],
                    false,
                ),
                launcher(
                    "review-changes",
                    "<leader>rw",
                    &["tuicr", "-w", "-p", "${file}", "--line", "${line}"],
                    true,
                ),
                launcher(
                    "review-dir",
                    "<leader>rd",
                    &["tuicr", "--file", "${dir}"],
                    false,
                ),
            ],
            review: ReviewConfig {
                enabled: true,
                command: "tuicr".into(),
            },
        }
    }
}

// Partial shapes of the user file: every field optional, unknown keys rejected.

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    render: Option<RawRender>,
    sidebar: Option<RawSidebar>,
    keys: Option<RawKeys>,
    lsp: Option<RawLsp>,
    launch: Option<Vec<toml::Spanned<RawLauncher>>>,
    review: Option<RawReview>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRender {
    max_width: Option<u16>,
    theme: Option<String>,
    math: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSidebar {
    show: Option<bool>,
    default: Option<RawSidebarMode>,
    width: Option<u16>,
    split_ratio: Option<f32>,
    show_all: Option<bool>,
    reading: Option<SidebarReading>,
    auto_hide_below: Option<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKeys {
    leader: Option<char>,
    clue: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLsp {
    server: Option<Vec<ServerConfig>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLauncher {
    name: String,
    key: Option<String>,
    command: Option<Vec<String>>,
    #[serde(default)]
    needs_vcs: bool,
    #[serde(default)]
    disabled: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawReview {
    enabled: Option<bool>,
    command: Option<String>,
}

/// 1-based line of byte offset `at` in `src`.
fn line_of(src: &str, at: usize) -> usize {
    src.as_bytes()[..at.min(src.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        + 1
}

fn set<T>(slot: &mut T, v: Option<T>) {
    if let Some(v) = v {
        *slot = v;
    }
}

impl Config {
    /// `~/.config/ramble/config.toml` (respecting `$XDG_CONFIG_HOME`).
    pub fn default_path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("ramble").join("config.toml"))
    }

    /// Load `path` over the defaults. A missing file yields the defaults.
    /// A parse error is returned with its line number in the message.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let src = std::fs::read_to_string(path)
            .map_err(|e| anyhow!("cannot read config {}: {e}", path.display()))?;
        Self::parse(&src).map_err(|(at, msg)| {
            let msg = msg.trim_end();
            match at {
                Some(at) => anyhow!(
                    "config error at {} line {}: {msg}",
                    path.display(),
                    line_of(&src, at)
                ),
                None => anyhow!("config error at {}: {msg}", path.display()),
            }
        })
    }

    /// Parse `src` over the defaults. Errors carry a byte offset if known.
    fn parse(src: &str) -> Result<Self, (Option<usize>, String)> {
        let raw: RawConfig =
            toml::from_str(src).map_err(|e| (e.span().map(|s| s.start), e.message().into()))?;
        let mut c = Self::default();
        if let Some(r) = raw.render {
            set(&mut c.render.max_width, r.max_width);
            set(&mut c.render.theme, r.theme);
            set(&mut c.render.math, r.math);
        }
        if let Some(s) = raw.sidebar {
            if let Some(d) = s.default {
                c.sidebar.default = match d {
                    RawSidebarMode::Off => {
                        c.sidebar.show = false;
                        SidebarMode::Auto
                    }
                    RawSidebarMode::Auto => SidebarMode::Auto,
                    RawSidebarMode::Files => SidebarMode::Files,
                    RawSidebarMode::Outline => SidebarMode::Outline,
                    RawSidebarMode::Split => SidebarMode::Split,
                };
            }
            // After the legacy `off`, so an explicit `show` wins.
            set(&mut c.sidebar.show, s.show);
            set(&mut c.sidebar.width, s.width);
            set(&mut c.sidebar.split_ratio, s.split_ratio);
            set(&mut c.sidebar.show_all, s.show_all);
            set(&mut c.sidebar.reading, s.reading);
            set(&mut c.sidebar.auto_hide_below, s.auto_hide_below);
        }
        if let Some(k) = raw.keys {
            set(&mut c.keys.leader, k.leader);
            set(&mut c.keys.clue, k.clue);
        }
        if let Some(l) = raw.lsp {
            set(&mut c.lsp.server, l.server);
        }
        if let Some(r) = raw.review {
            set(&mut c.review.enabled, r.enabled);
            set(&mut c.review.command, r.command);
        }
        for entry in raw.launch.unwrap_or_default() {
            let at = entry.span().start;
            let l = entry.into_inner();
            let pos = c.launch.iter().position(|d| d.name == l.name);
            if l.disabled {
                if let Some(i) = pos {
                    c.launch.remove(i);
                }
                continue;
            }
            let Some(command) = l.command else {
                return Err((
                    Some(at),
                    format!("launcher `{}` is missing `command`", l.name),
                ));
            };
            let new = Launcher {
                name: l.name,
                key: l.key,
                command,
                needs_vcs: l.needs_vcs,
                disabled: false,
            };
            match pos {
                Some(i) => c.launch[i] = new,
                None => c.launch.push(new),
            }
        }
        Ok(c)
    }
}

/// Write the commented default config to `path`, creating parent
/// directories. Refuses to overwrite an existing file.
pub fn write_default(path: &Path) -> anyhow::Result<()> {
    if path.exists() {
        bail!("{} already exists", path.display());
    }
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)
            .map_err(|e| anyhow!("cannot create {}: {e}", dir.display()))?;
    }
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| anyhow!("cannot write {}: {e}", path.display()))?;
    std::io::Write::write_all(&mut f, DEFAULT_TOML.as_bytes())
        .map_err(|e| anyhow!("cannot write {}: {e}", path.display()))?;
    Ok(())
}
