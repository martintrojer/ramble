//! Config loading over built-in defaults: servers, leader key, launchers,
//! render, sidebar, review, send and mouse options.
//!
//! A user file overrides the defaults one field at a time. `[[lsp.server]]`
//! replaces the server list (empty by default: every page uses the built-in
//! mdroots backend); `[[launch]]` entries merge by name.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail};
use debrief_review::Kind;
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
    pub send: SendConfig,
    pub mouse: MouseConfig,
    /// Problems the file had that didn't stop it loading (dropped comment
    /// kinds); the app shows them in the status line at start.
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MouseConfig {
    /// Capture the mouse (click, drag-to-copy, wheel); false leaves it to
    /// the terminal or tmux.
    pub enabled: bool,
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
    /// `always`, `never` or `auto` (peek, D11). A start with no page shows
    /// the tree regardless.
    pub show: SidebarShow,
    pub default: SidebarMode,
    /// Columns, not counting the border: fitted to the rows, or fixed.
    pub width: SidebarWidth,
    /// Bounds of the auto width; `max_width` is also capped at 35% of the
    /// terminal width.
    pub min_width: u16,
    pub max_width: u16,
    /// Share of the sidebar given to files in split mode.
    pub split_ratio: f32,
    /// Also list non-markdown files in the tree.
    pub show_all: bool,
    /// What `auto` shows while a page is loaded.
    pub reading: SidebarReading,
    /// Hide the sidebar while the terminal is narrower than this many
    /// columns (0 disables).
    pub auto_hide_below: u16,
    /// Which edge of the screen the sidebar is drawn at (D9).
    pub side: SidebarSide,
}

/// `sidebar.show` (D11): whether the sidebar is drawn while a page is
/// shown. No other spellings are accepted: anything else, the old
/// `true`/`false` included, is an error naming the key and the three values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarShow {
    /// Shown; `<leader>e` hides it.
    #[default]
    Always,
    /// Hidden; `<leader>e` shows it.
    Never,
    /// Peek: shown while you use it, hidden while you read.
    Auto,
}

impl<'de> Deserialize<'de> for SidebarShow {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = SidebarShow;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str(r#""always", "never" or "auto""#)
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<SidebarShow, E> {
                match v {
                    "always" => Ok(SidebarShow::Always),
                    "never" => Ok(SidebarShow::Never),
                    "auto" => Ok(SidebarShow::Auto),
                    _ => Err(E::custom(format!("{SHOW_EXPECTED}, got \"{v}\""))),
                }
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<SidebarShow, E> {
                Err(E::custom(format!("{SHOW_EXPECTED}, got {v}")))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<SidebarShow, E> {
                Err(E::custom(format!("{SHOW_EXPECTED}, got {v}")))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<SidebarShow, E> {
                Err(E::custom(format!("{SHOW_EXPECTED}, got {v}")))
            }
        }
        d.deserialize_any(V)
    }
}

/// The `sidebar.show` error, before what was found.
const SHOW_EXPECTED: &str = "sidebar.show: expected \"always\", \"never\" or \"auto\"";

/// `sidebar.side`: the screen edge the sidebar sits at (`:Sidebar
/// left|right` changes it at runtime).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SidebarSide {
    #[default]
    Left,
    Right,
}

/// `sidebar.width`: `"auto"` fits the rows (spec D5), a number is used as
/// given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarWidth {
    Auto,
    Fixed(u16),
}

/// `sidebar.width` as written: `"auto"` or a number.
#[derive(Deserialize)]
#[serde(untagged)]
enum RawSidebarWidth {
    Fixed(u16),
    Name(String),
}

impl TryFrom<RawSidebarWidth> for SidebarWidth {
    type Error = String;
    fn try_from(w: RawSidebarWidth) -> Result<SidebarWidth, String> {
        match w {
            RawSidebarWidth::Fixed(n) => Ok(SidebarWidth::Fixed(n)),
            RawSidebarWidth::Name(s) if s == "auto" => Ok(SidebarWidth::Auto),
            RawSidebarWidth::Name(s) => Err(format!(
                "sidebar.width: expected \"auto\" or a number, got \"{s}\""
            )),
        }
    }
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

#[derive(Debug, Clone, PartialEq)]
pub struct KeysConfig {
    pub leader: char,
    /// Show the next-key box after a pause in a key sequence.
    pub clue: bool,
    /// `C-h/j/k/l` move between panes (as `C-w h/j/k/l`), handing off to
    /// tmux at the edge; false leaves them unbound.
    pub pane_nav: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LspConfig {
    /// Tried in order; the first whose root marker is found wins. Empty by
    /// default; pages no server selects use the mdroots backend.
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
    /// Comment mode (`c`, `cc`) and markers from the debrief-review batch.
    pub enabled: bool,
    /// Kinds Tab cycles through in the comment box, in order; the
    /// export's legend defines them. Omitted: debrief-review's built-ins.
    pub kinds: Vec<Kind>,
}

/// How `<leader>rr` hands the review batch back (shared in meaning with
/// debrief's `[send]`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SendConfig {
    /// Command run with the markdown on stdin; empty (or a failure) copies
    /// it to the clipboard instead.
    pub command: Vec<String>,
    /// Replaces the export's opening paragraph; `""` drops it.
    pub preamble: Option<String>,
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
                show: SidebarShow::Always,
                default: SidebarMode::Auto,
                width: SidebarWidth::Auto,
                min_width: 16,
                max_width: 48,
                split_ratio: 0.5,
                show_all: false,
                reading: SidebarReading::Outline,
                auto_hide_below: 80,
                side: SidebarSide::Left,
            },
            keys: KeysConfig {
                leader: ' ',
                clue: true,
                pane_nav: true,
            },
            lsp: LspConfig { server: vec![] },
            launch: vec![launcher(
                "edit",
                "<leader>o",
                &["${editor}", "+${line}", "${file}"],
                false,
            )],
            review: ReviewConfig {
                enabled: true,
                kinds: debrief_review::builtin_kinds(),
            },
            send: SendConfig::default(),
            mouse: MouseConfig { enabled: true },
            warnings: Vec::new(),
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
    send: Option<RawSend>,
    mouse: Option<RawMouse>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMouse {
    enabled: Option<bool>,
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
    show: Option<SidebarShow>,
    default: Option<SidebarMode>,
    width: Option<toml::Spanned<RawSidebarWidth>>,
    min_width: Option<u16>,
    max_width: Option<u16>,
    split_ratio: Option<f32>,
    show_all: Option<bool>,
    reading: Option<SidebarReading>,
    auto_hide_below: Option<u16>,
    side: Option<SidebarSide>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKeys {
    leader: Option<char>,
    clue: Option<bool>,
    pane_nav: Option<bool>,
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
    kinds: Option<Vec<RawKind>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKind {
    id: String,
    definition: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSend {
    command: Option<Vec<String>>,
    preamble: Option<String>,
}

/// 1-based line of byte offset `at` in `src`.
fn line_of(src: &str, at: usize) -> usize {
    src.as_bytes()[..at.min(src.len())]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        + 1
}

/// `[review] kinds` with ids trimmed and lowercased; an empty or repeated
/// id is dropped with a warning (tuicr's rule).
fn clean_kinds(kinds: Vec<RawKind>, warnings: &mut Vec<String>) -> Vec<Kind> {
    let mut out: Vec<Kind> = Vec::new();
    for k in kinds {
        let id = k.id.trim().to_lowercase();
        if id.is_empty() {
            warnings.push("[review] kinds: dropped a kind with an empty id".into());
        } else if out.iter().any(|o| o.id == id) {
            warnings.push(format!("[review] kinds: dropped duplicate kind `{id}`"));
        } else {
            out.push(Kind {
                id,
                definition: k.definition,
            });
        }
    }
    out
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
            set(&mut c.sidebar.default, s.default);
            set(&mut c.sidebar.show, s.show);
            if let Some(w) = s.width {
                let at = w.span().start;
                c.sidebar.width = w.into_inner().try_into().map_err(|e| (Some(at), e))?;
            }
            set(&mut c.sidebar.min_width, s.min_width);
            set(&mut c.sidebar.max_width, s.max_width);
            set(&mut c.sidebar.split_ratio, s.split_ratio);
            set(&mut c.sidebar.show_all, s.show_all);
            set(&mut c.sidebar.reading, s.reading);
            set(&mut c.sidebar.auto_hide_below, s.auto_hide_below);
            set(&mut c.sidebar.side, s.side);
        }
        if let Some(k) = raw.keys {
            set(&mut c.keys.leader, k.leader);
            set(&mut c.keys.clue, k.clue);
            set(&mut c.keys.pane_nav, k.pane_nav);
        }
        if let Some(l) = raw.lsp {
            set(&mut c.lsp.server, l.server);
        }
        if let Some(r) = raw.review {
            set(&mut c.review.enabled, r.enabled);
            if let Some(kinds) = r.kinds {
                c.review.kinds = clean_kinds(kinds, &mut c.warnings);
            }
        }
        if let Some(s) = raw.send {
            set(&mut c.send.command, s.command);
            if s.preamble.is_some() {
                c.send.preamble = s.preamble;
            }
        }
        if let Some(m) = raw.mouse {
            set(&mut c.mouse.enabled, m.enabled);
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
