//! Colour themes: the palette and its role colours, the built-in schemes,
//! base16/base24 scheme files, and the resolution of the `[theme]` config
//! into a [`Theme`]. See `docs/specs/2026-10-21-theming.md`.
//!
//! Keep the tables and names in step with debrief's `theme.rs`.

use std::path::{Path, PathBuf};

use ratatui::style::Color;
use syntect::highlighting::Theme as SyntaxTheme;

/// One list of keys for the [`Palette`] and its override table
/// ([`Colors`]): the 16 palette colours, then the 5 roles.
macro_rules! keys {
    ($($k:ident),* $(,)?) => {
        /// The colours of a theme: 16 palette colours and 5 roles.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct Palette {
            $(pub $k: Color,)*
        }

        /// `[theme.colors]`: any subset of the keys, unresolved. An
        /// explicit key stays set even when it equals the Mocha value.
        #[derive(Debug, Clone, Default, PartialEq, Eq)]
        pub struct Colors {
            $(pub $k: Option<[u8; 3]>,)*
        }

        /// Every key `[theme.colors]` accepts.
        pub const KEYS: &[&str] = &[$(stringify!($k)),*];

        impl Colors {
            /// Set `key`; false when it is not a key.
            pub fn set(&mut self, key: &str, v: [u8; 3]) -> bool {
                $(if key == stringify!($k) {
                    self.$k = Some(v);
                    return true;
                })*
                false
            }
        }
    };
}

keys!(
    bg,
    bg_dark,
    surface0,
    surface1,
    surface2,
    muted,
    text,
    red,
    orange,
    yellow,
    green,
    cyan,
    sky,
    blue,
    purple,
    pink,
    cursor_bg,
    selection_bg,
    add_bg,
    del_bg,
    hunk_bg,
);

const fn rgb(c: u32) -> Color {
    Color::Rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// Catppuccin Mocha, the default.
pub const MOCHA: Palette = Palette {
    bg: rgb(0x1e1e2e),
    bg_dark: rgb(0x181825),
    surface0: rgb(0x313244),
    surface1: rgb(0x45475a),
    surface2: rgb(0x585b70),
    muted: rgb(0x6c7086),
    text: rgb(0xcdd6f4),
    red: rgb(0xf38ba8),
    orange: rgb(0xfab387),
    yellow: rgb(0xf9e2af),
    green: rgb(0xa6e3a1),
    cyan: rgb(0x94e2d5),
    sky: rgb(0x89dceb),
    blue: rgb(0x89b4fa),
    purple: rgb(0xcba6f7),
    pink: rgb(0xf5c2e7),
    cursor_bg: rgb(0x313244),
    selection_bg: rgb(0x45475a),
    add_bg: rgb(0x394545),
    del_bg: rgb(0x483346),
    hunk_bg: rgb(0x322e46),
};

/// `blend(fg over bg, pct)`: per channel `(fg*pct + bg*(100-pct)) / 100`.
/// Only RGB colours blend; anything else gives `bg`.
pub fn blend(fg: Color, bg: Color, pct: u32) -> Color {
    match (fg, bg) {
        (Color::Rgb(fr, fg_, fb), Color::Rgb(br, bg_, bb)) => {
            let ch = |f: u8, b: u8| ((u32::from(f) * pct + u32::from(b) * (100 - pct)) / 100) as u8;
            Color::Rgb(ch(fr, br), ch(fg_, bg_), ch(fb, bb))
        }
        _ => bg,
    }
}

/// The 16 palette colours a scheme provides, in [`Palette`] order.
type Base = [Color; 16];

/// Built-in schemes: name, the 16 colours (bg, bg_dark, surface0-2, muted,
/// text, red, orange, yellow, green, cyan, sky, blue, purple, pink), and
/// the default `syntax`.
#[rustfmt::skip]
const SCHEMES: &[(&str, [u32; 16], &str)] = &[
    ("catppuccin-mocha", [0x1e1e2e, 0x181825, 0x313244, 0x45475a, 0x585b70, 0x6c7086, 0xcdd6f4, 0xf38ba8, 0xfab387, 0xf9e2af, 0xa6e3a1, 0x94e2d5, 0x89dceb, 0x89b4fa, 0xcba6f7, 0xf5c2e7], "Catppuccin Mocha"),
    ("catppuccin-macchiato", [0x24273a, 0x1e2030, 0x363a4f, 0x494d64, 0x5b6078, 0x6e738d, 0xcad3f5, 0xed8796, 0xf5a97f, 0xeed49f, 0xa6da95, 0x8bd5ca, 0x91d7e3, 0x8aadf4, 0xc6a0f6, 0xf5bde6], "Catppuccin Macchiato"),
    ("catppuccin-frappe", [0x303446, 0x292c3c, 0x414559, 0x51576d, 0x626880, 0x737994, 0xc6d0f5, 0xe78284, 0xef9f76, 0xe5c890, 0xa6d189, 0x81c8be, 0x99d1db, 0x8caaee, 0xca9ee6, 0xf4b8e4], "Catppuccin Frappe"),
    ("catppuccin-latte", [0xeff1f5, 0xe6e9ef, 0xccd0da, 0xbcc0cc, 0xacb0be, 0x9ca0b0, 0x4c4f69, 0xd20f39, 0xfe640b, 0xdf8e1d, 0x40a02b, 0x179299, 0x04a5e5, 0x1e66f5, 0x8839ef, 0xea76cb], "Catppuccin Latte"),
    ("solarized-dark", [0x002b36, 0x073642, 0x113942, 0x1d424b, 0x294c53, 0x426066, 0x93a1a1, 0xdc322f, 0xcb4b16, 0xb58900, 0x859900, 0x2aa198, 0x2aa198, 0x268bd2, 0x6c71c4, 0xd33682], "Solarized (dark)"),
    ("solarized-light", [0xfdf6e3, 0xeee8d5, 0xe9e5d5, 0xdcdacd, 0xcecfc4, 0xb2b8b1, 0x586e75, 0xdc322f, 0xcb4b16, 0xb58900, 0x859900, 0x2aa198, 0x2aa198, 0x268bd2, 0x6c71c4, 0xd33682], "Solarized (light)"),
    ("gruvbox-dark", [0x282828, 0x1d2021, 0x3f3d38, 0x4f4b43, 0x5e5a4e, 0x7f7866, 0xebdbb2, 0xfb4934, 0xfe8019, 0xfabd2f, 0xb8bb26, 0x8ec07c, 0x8ec07c, 0x83a598, 0xd3869b, 0xd3869b], "gruvbox-dark"),
    ("gruvbox-light", [0xfbf1c7, 0xf2e5bc, 0xe4dab5, 0xd4ccaa, 0xc5bd9e, 0xa59d85, 0x3c3836, 0x9d0006, 0xaf3a03, 0xb57614, 0x79740e, 0x427b58, 0x427b58, 0x076678, 0x8f3f71, 0x8f3f71], "gruvbox-light"),
];

/// The built-in scheme names, as `name` takes them.
pub const BUILTINS: &[&str] = &[
    "catppuccin-mocha",
    "catppuccin-macchiato",
    "catppuccin-frappe",
    "catppuccin-latte",
    "solarized-dark",
    "solarized-light",
    "gruvbox-dark",
    "gruvbox-light",
];

fn scheme(name: &str) -> Option<(Base, &'static str)> {
    let (_, c, syntax) = SCHEMES.iter().find(|(n, ..)| *n == name)?;
    Some((c.map(rgb), syntax))
}

/// The 16 palette colours, in field order.
fn from_base(b: Base) -> Palette {
    let [
        bg,
        bg_dark,
        surface0,
        surface1,
        surface2,
        muted,
        text,
        red,
        orange,
        yellow,
        green,
        cyan,
        sky,
        blue,
        purple,
        pink,
    ] = b;
    let mut p = Palette {
        bg,
        bg_dark,
        surface0,
        surface1,
        surface2,
        muted,
        text,
        red,
        orange,
        yellow,
        green,
        cyan,
        sky,
        blue,
        purple,
        pink,
        cursor_bg: bg,
        selection_bg: bg,
        add_bg: bg,
        del_bg: bg,
        hunk_bg: bg,
    };
    p.derive_roles(&Colors::default());
    p
}

impl Palette {
    /// The roles, from the final palette: an override in `c` wins.
    fn derive_roles(&mut self, c: &Colors) {
        let o = |v: Option<[u8; 3]>, d: Color| v.map_or(d, |[r, g, b]| Color::Rgb(r, g, b));
        self.cursor_bg = o(c.cursor_bg, self.surface0);
        self.selection_bg = o(c.selection_bg, self.surface1);
        self.add_bg = o(c.add_bg, blend(self.green, self.bg, 20));
        self.del_bg = o(c.del_bg, blend(self.red, self.bg, 20));
        self.hunk_bg = o(c.hunk_bg, blend(self.purple, self.bg, 12));
    }
}

/// The built-in scheme `name`, or `None`.
pub fn builtin(name: &str) -> Option<Palette> {
    scheme(name).map(|(b, _)| from_base(b))
}

// ---------------------------------------------------------------------------
// Scheme files

/// A base16/base24 scheme file, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemeFile {
    /// `base00`–`base17`; the base24 slots may be missing.
    pub slots: [Option<[u8; 3]>; 24],
    /// `variant: "light"`.
    pub light: bool,
}

/// `#rrggbb` or `rrggbb`, any case.
pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let h = s.strip_prefix('#').unwrap_or(s);
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let n = u32::from_str_radix(h, 16).ok()?;
    Some([(n >> 16) as u8, (n >> 8) as u8, n as u8])
}

/// The value after `key:` on a line: quoted or bare, a trailing comment
/// dropped.
fn scalar(v: &str) -> &str {
    let v = v.trim();
    for q in ['"', '\''] {
        if let Some(rest) = v.strip_prefix(q) {
            return rest.split(q).next().unwrap_or("");
        }
    }
    v.split_whitespace().next().unwrap_or("")
}

/// Parse a scheme file's text; `file` names it in errors. Every other line
/// than `baseXX:` and `variant:` is ignored.
pub fn parse_scheme(text: &str, file: &str) -> Result<SchemeFile, String> {
    let mut slots = [None; 24];
    let mut light = false;
    for line in text.lines() {
        let Some((key, val)) = line.trim().split_once(':') else {
            continue;
        };
        let key = key.trim().trim_matches(['"', '\'']);
        if key == "variant" {
            light = scalar(val).eq_ignore_ascii_case("light");
        } else if let Some(xx) = key.strip_prefix("base")
            && xx.len() == 2
            && let Ok(i) = usize::from_str_radix(xx, 16)
            && i < 24
        {
            let v = scalar(val);
            slots[i] =
                Some(parse_hex(v).ok_or_else(|| {
                    format!("{file}: {key}: bad colour {v:?} (expected #rrggbb)")
                })?);
        }
    }
    for (i, s) in slots.iter().take(16).enumerate() {
        if s.is_none() {
            return Err(format!("{file}: missing base{i:02X}"));
        }
    }
    Ok(SchemeFile { slots, light })
}

fn color(c: [u8; 3]) -> Color {
    Color::Rgb(c[0], c[1], c[2])
}

impl SchemeFile {
    /// The 16 palette colours the spec's mapping gives.
    fn base(&self) -> Base {
        let s = |i: usize| color(self.slots[i].unwrap_or_default());
        let (bg, text) = (s(0), s(5));
        let (cyan, purple) = (s(0xC), s(0xE));
        let opt = |i: usize, d: Color| self.slots[i].map_or(d, color);
        [
            bg,
            s(1),
            blend(text, bg, 12),
            blend(text, bg, 20),
            blend(text, bg, 28),
            blend(text, bg, 45),
            text,
            s(8),
            s(9),
            s(0xA),
            s(0xB),
            cyan,
            opt(0x15, cyan),
            s(0xD),
            purple,
            opt(0x17, purple),
        ]
    }

    /// The `syntax` default: Mocha for dark, Latte for light.
    pub fn syntax(&self) -> &'static str {
        if self.light {
            "Catppuccin Latte"
        } else {
            "Catppuccin Mocha"
        }
    }
}

/// Whether `name` is a scheme file path rather than a built-in name.
pub fn is_path(name: &str) -> bool {
    name.contains('/') || name.ends_with(".yaml") || name.ends_with(".yml")
}

/// `name` as a path: `~` expands to `$HOME`, a relative path is relative
/// to `dir` (the config file's directory).
pub fn scheme_path(name: &str, dir: Option<&Path>) -> PathBuf {
    if let Some(rest) = name.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    let p = Path::new(name);
    match dir {
        Some(d) if p.is_relative() => d.join(p),
        _ => p.to_path_buf(),
    }
}

/// A `[theme]` problem and the key it is about, for the config error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeError {
    /// `name` or `syntax`.
    pub key: &'static str,
    pub msg: String,
}

impl std::fmt::Display for ThemeError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str(&self.msg)
    }
}

impl std::error::Error for ThemeError {}

pub fn unknown_name(name: &str) -> String {
    format!(
        "unknown theme {name:?}: the built-ins are {}, or a path to a base16/base24 .yaml file",
        BUILTINS.join(", ")
    )
}

/// The base colours and default `syntax` of `name`: a built-in, or a
/// scheme file (a relative path resolved against `dir`).
fn load_base(name: &str, dir: Option<&Path>) -> Result<(Base, &'static str), ThemeError> {
    let err = |msg| ThemeError { key: "name", msg };
    if !is_path(name) {
        return scheme(name).ok_or_else(|| err(unknown_name(name)));
    }
    let path = scheme_path(name, dir);
    let shown = path.display().to_string();
    let text = std::fs::read_to_string(&path)
        .map_err(|e| err(format!("cannot read theme {shown}: {e}")))?;
    let f = parse_scheme(&text, &shown).map_err(err)?;
    Ok((f.base(), f.syntax()))
}

// ---------------------------------------------------------------------------
// Syntax themes

/// The code-highlighting themes `syntax` takes (two-face's embedded set).
pub fn syntax_names() -> Vec<&'static str> {
    two_face::theme::EmbeddedLazyThemeSet::theme_names()
        .iter()
        .map(|n| n.as_name())
        .collect()
}

/// `syntax` matched case-insensitively against [`syntax_names`].
pub fn syntax_name(name: &str) -> Result<&'static str, String> {
    let names = syntax_names();
    names
        .iter()
        .find(|n| n.eq_ignore_ascii_case(name))
        .copied()
        .ok_or_else(|| {
            format!(
                "unknown syntax theme {name:?}: valid names are {}",
                names.join(", ")
            )
        })
}

/// The embedded syntect theme called `name` (a canonical [`syntax_name`]).
fn syntax_theme(name: &str) -> SyntaxTheme {
    let t = two_face::theme::extra();
    two_face::theme::EmbeddedLazyThemeSet::theme_names()
        .iter()
        .find(|n| n.as_name() == name)
        .map_or_else(
            || t.get(two_face::theme::EmbeddedThemeName::CatppuccinMocha),
            |n| t.get(*n),
        )
        .clone()
}

// ---------------------------------------------------------------------------
// Resolution

/// `[theme]` as written: every key optional, nothing resolved.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeConfig {
    /// A built-in name or a scheme file path.
    pub name: Option<String>,
    /// A two-face theme name.
    pub syntax: Option<String>,
    pub fill: Option<bool>,
    pub colors: Colors,
    /// The config file's directory, which a relative `name` path is
    /// relative to. Set only when `name` is a path.
    pub dir: Option<PathBuf>,
}

/// A [`ThemeConfig`] resolved except for the syntect theme itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub palette: Palette,
    /// Canonical two-face theme name.
    pub syntax: &'static str,
    pub fill: bool,
}

impl ThemeConfig {
    /// Resolve: the scheme (built-in or file; relative paths against
    /// `dir`), then the colour overrides, then the derived roles and the
    /// syntax theme name.
    pub fn resolve(&self) -> Result<Resolved, ThemeError> {
        let (base, default_syntax) = match &self.name {
            Some(n) => load_base(n, self.dir.as_deref())?,
            None => load_base("catppuccin-mocha", None)?,
        };
        let mut p = from_base(base);
        let c = &self.colors;
        let set = |slot: &mut Color, v: Option<[u8; 3]>| {
            if let Some(v) = v {
                *slot = color(v);
            }
        };
        set(&mut p.bg, c.bg);
        set(&mut p.bg_dark, c.bg_dark);
        set(&mut p.surface0, c.surface0);
        set(&mut p.surface1, c.surface1);
        set(&mut p.surface2, c.surface2);
        set(&mut p.muted, c.muted);
        set(&mut p.text, c.text);
        set(&mut p.red, c.red);
        set(&mut p.orange, c.orange);
        set(&mut p.yellow, c.yellow);
        set(&mut p.green, c.green);
        set(&mut p.cyan, c.cyan);
        set(&mut p.sky, c.sky);
        set(&mut p.blue, c.blue);
        set(&mut p.purple, c.purple);
        set(&mut p.pink, c.pink);
        p.derive_roles(c);
        let syntax = match &self.syntax {
            Some(s) => syntax_name(s).map_err(|msg| ThemeError { key: "syntax", msg })?,
            None => default_syntax,
        };
        Ok(Resolved {
            palette: p,
            syntax,
            fill: self.fill.unwrap_or(false),
        })
    }
}

/// What the app draws with.
#[derive(Debug, Clone)]
pub struct Theme {
    pub palette: Palette,
    /// The code-highlighting theme.
    pub syntax: SyntaxTheme,
    /// Paint `palette.bg` behind everything.
    pub fill: bool,
}

impl Theme {
    pub fn new(r: &Resolved) -> Self {
        Self {
            palette: r.palette,
            syntax: syntax_theme(r.syntax),
            fill: r.fill,
        }
    }

    /// Resolve `cfg` (see [`ThemeConfig::resolve`]) into a theme.
    pub fn resolve(cfg: &ThemeConfig) -> Result<Self, ThemeError> {
        cfg.resolve().map(|r| Self::new(&r))
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            palette: MOCHA,
            syntax: syntax_theme("Catppuccin Mocha"),
            fill: false,
        }
    }
}
