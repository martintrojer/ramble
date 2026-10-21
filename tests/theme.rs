//! `[theme]`: built-in schemes, scheme files, overrides, errors, `fill`.

use std::path::Path;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ramble::app::{App, MdrootsOptions, StartOptions, StartTarget};
use ramble::config::{Config, SidebarShow};
use ramble::theme::{self, MOCHA, Palette};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Color;

fn c(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// Load `src` as the config file in `dir`.
fn load_in(dir: &Path, src: &str) -> anyhow::Result<Config> {
    let path = dir.join("config.toml");
    std::fs::write(&path, src).unwrap();
    Config::load(&path)
}

fn load(src: &str) -> anyhow::Result<Config> {
    load_in(tempfile::tempdir().unwrap().path(), src)
}

fn palette_of(src: &str) -> Palette {
    load(src).unwrap().theme.resolve().unwrap().palette
}

fn err(src: &str) -> String {
    format!("{:#}", load(src).unwrap_err())
}

#[test]
fn mocha_is_todays_colours() {
    let want = [
        (MOCHA.text, 0xcdd6f4),
        (MOCHA.purple, 0xcba6f7),
        (MOCHA.blue, 0x89b4fa),
        (MOCHA.green, 0xa6e3a1),
        (MOCHA.muted, 0x6c7086),
        (MOCHA.red, 0xf38ba8),
        (MOCHA.yellow, 0xf9e2af),
        (MOCHA.orange, 0xfab387),
        (MOCHA.cyan, 0x94e2d5),
        (MOCHA.bg, 0x1e1e2e),
        (MOCHA.bg_dark, 0x181825),
        (MOCHA.surface0, 0x313244),
        (MOCHA.surface1, 0x45475a),
        (MOCHA.surface2, 0x585b70),
        (MOCHA.sky, 0x89dceb),
        (MOCHA.pink, 0xf5c2e7),
        (MOCHA.cursor_bg, 0x313244),
        (MOCHA.selection_bg, 0x45475a),
        (MOCHA.add_bg, 0x394545),
        (MOCHA.del_bg, 0x483346),
        (MOCHA.hunk_bg, 0x322e46),
    ];
    for (got, hex) in want {
        assert_eq!(got, c(hex), "{hex:06x}");
    }
}

#[test]
fn every_builtin_loads_and_mocha_is_the_default() {
    assert_eq!(theme::BUILTINS.len(), 8);
    for name in theme::BUILTINS {
        let cfg = load(&format!("[theme]\nname = \"{name}\"\n")).unwrap();
        let r = cfg
            .theme
            .resolve()
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(Some(r.palette), theme::builtin(name), "{name}");
        // Building the syntect theme works for the built-in's syntax name.
        theme::Theme::new(&cfg.resolve_theme().unwrap());
    }
    assert_eq!(theme::builtin("catppuccin-mocha"), Some(MOCHA));
    assert_eq!(palette_of(""), MOCHA);
    assert_eq!(theme::builtin("nope"), None);
}

#[test]
fn gruvbox_light_is_pinned_to_the_spec_table() {
    let p = theme::builtin("gruvbox-light").unwrap();
    let want = [
        (p.bg, 0xfbf1c7),
        (p.bg_dark, 0xf2e5bc),
        (p.surface0, 0xe4dab5),
        (p.surface1, 0xd4ccaa),
        (p.surface2, 0xc5bd9e),
        (p.muted, 0xa59d85),
        (p.text, 0x3c3836),
        (p.red, 0x9d0006),
        (p.orange, 0xaf3a03),
        (p.yellow, 0xb57614),
        (p.green, 0x79740e),
        (p.cyan, 0x427b58),
        (p.sky, 0x427b58),
        (p.blue, 0x076678),
        (p.purple, 0x8f3f71),
        (p.pink, 0x8f3f71),
        // Roles follow the palette.
        (p.cursor_bg, 0xe4dab5),
        (p.selection_bg, 0xd4ccaa),
    ];
    for (got, hex) in want {
        assert_eq!(got, c(hex), "{hex:06x}");
    }
    assert_eq!(p.add_bg, theme::blend(p.green, p.bg, 20));
    assert_eq!(p.del_bg, theme::blend(p.red, p.bg, 20));
    assert_eq!(p.hunk_bg, theme::blend(p.purple, p.bg, 12));
    let r = load("[theme]\nname = \"gruvbox-light\"\n")
        .unwrap()
        .theme
        .resolve()
        .unwrap();
    assert_eq!(r.syntax, "gruvbox-light");
}

#[test]
fn blend_is_integer_arithmetic_per_channel() {
    assert_eq!(MOCHA.add_bg, theme::blend(MOCHA.green, MOCHA.bg, 20));
    assert_eq!(
        theme::blend(c(0xff0000), c(0x000000), 20),
        Color::Rgb(51, 0, 0)
    );
}

#[test]
fn an_override_replaces_one_colour() {
    let p = palette_of("[theme.colors]\nblue = \"#112233\"\n");
    assert_eq!(p.blue, c(0x112233));
    assert_eq!(
        Palette {
            blue: MOCHA.blue,
            ..p
        },
        MOCHA
    );
    // Over another scheme, the rest stays that scheme's.
    let p = palette_of("[theme]\nname = \"catppuccin-latte\"\n[theme.colors]\nred = \"AABBCC\"\n");
    assert_eq!(p.red, c(0xaabbcc));
    assert_eq!(p.blue, theme::builtin("catppuccin-latte").unwrap().blue);
}

#[test]
fn cursor_bg_follows_a_surface0_override_when_unset() {
    let p = palette_of("[theme.colors]\nsurface0 = \"102030\"\n");
    assert_eq!(p.surface0, c(0x102030));
    assert_eq!(p.cursor_bg, c(0x102030));
    assert_eq!(p.selection_bg, MOCHA.surface1, "selection_bg: surface1");
    let p = palette_of("[theme.colors]\nbg = \"#000000\"\ngreen = \"#00ff00\"\n");
    assert_eq!(p.add_bg, theme::blend(c(0x00ff00), c(0x000000), 20));
}

#[test]
fn a_role_override_wins_even_when_it_equals_mocha() {
    let p = palette_of("[theme.colors]\ncursor_bg = \"#585b70\"\n");
    assert_eq!(p.cursor_bg, c(0x585b70));
    assert_eq!(p.surface0, MOCHA.surface0);
    // An explicit Mocha value is kept, so it wins after a palette change.
    let p = palette_of(
        "[theme.colors]\nsurface0 = \"#000000\"\ncursor_bg = \"#313244\"\nselection_bg = \"#ABCDEF\"\n",
    );
    assert_eq!(p.cursor_bg, MOCHA.cursor_bg);
    assert_eq!(p.selection_bg, c(0xabcdef));
    let cfg = load("[theme.colors]\ncursor_bg = \"#313244\"\n").unwrap();
    assert_eq!(cfg.theme.colors.cursor_bg, Some([0x31, 0x32, 0x44]));
}

const BASE16: &str = r##"system: "base16"
name: "Test"
author: "me"
variant: "dark"
palette:
  base00: "#101010"
  base01: "202020"
  base02: "#303030"   # not used
  base03: "#404040"
  base04: "#505050"
  base05: "#e0e0e0"
  base06: "#606060"
  base07: "#707070"
  base08: "#aa0000"
  base09: "#bb5500"
  base0A: "#cccc00"
  base0B: "#00dd00"
  base0C: "#00eeee"
  base0D: "#0000ff"
  base0E: "#ee00ee"
  base0F: "#808080"
"##;

fn base24() -> String {
    format!("{BASE16}  base10: \"#000000\"\n  base15: \"#11aaff\"\n  base17: \"#ff66aa\"\n")
}

fn file_palette(name: &str, text: &str) -> Palette {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(name), text).unwrap();
    let cfg = load_in(dir.path(), &format!("[theme]\nname = \"{name}\"\n")).unwrap();
    cfg.theme.resolve().unwrap().palette
}

#[test]
fn a_base16_file_loads_with_the_spec_mapping_and_the_sky_pink_fallback() {
    let p = file_palette("t.yaml", BASE16);
    assert_eq!(p.bg, c(0x101010));
    assert_eq!(p.bg_dark, c(0x202020));
    assert_eq!(p.text, c(0xe0e0e0));
    assert_eq!(p.red, c(0xaa0000));
    assert_eq!(p.orange, c(0xbb5500));
    assert_eq!(p.yellow, c(0xcccc00));
    assert_eq!(p.green, c(0x00dd00));
    assert_eq!(p.cyan, c(0x00eeee));
    assert_eq!(p.blue, c(0x0000ff));
    assert_eq!(p.purple, c(0xee00ee));
    assert_eq!(p.sky, p.cyan, "no base15: cyan");
    assert_eq!(p.pink, p.purple, "no base17: purple");
    assert_eq!(p.surface0, theme::blend(p.text, p.bg, 12));
    assert_eq!(p.surface1, theme::blend(p.text, p.bg, 20));
    assert_eq!(p.surface2, theme::blend(p.text, p.bg, 28));
    assert_eq!(p.muted, theme::blend(p.text, p.bg, 45));
    assert_eq!(p.cursor_bg, p.surface0);
}

#[test]
fn a_base24_file_uses_base15_and_base17() {
    let p = file_palette("t.yml", &base24());
    assert_eq!(p.sky, c(0x11aaff));
    assert_eq!(p.pink, c(0xff66aa));
    assert_eq!(p.cyan, c(0x00eeee));
}

#[test]
fn a_scheme_file_s_syntax_follows_its_variant() {
    let syntax = |text: &str| {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("t.yaml"), text).unwrap();
        let cfg = load_in(dir.path(), "[theme]\nname = \"t.yaml\"\n").unwrap();
        cfg.theme.resolve().unwrap().syntax
    };
    assert_eq!(syntax(BASE16), "Catppuccin Mocha");
    assert_eq!(
        syntax(&BASE16.replace("variant: \"dark\"\n", "")),
        "Catppuccin Mocha"
    );
    assert_eq!(
        syntax(&BASE16.replace("\"dark\"", "\"light\"")),
        "Catppuccin Latte"
    );
}

#[test]
fn scheme_paths_resolve_against_the_config_dir() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("schemes")).unwrap();
    std::fs::write(dir.path().join("schemes/t.yaml"), BASE16).unwrap();
    let cfg = load_in(dir.path(), "[theme]\nname = \"schemes/t.yaml\"\n").unwrap();
    assert_eq!(cfg.theme.resolve().unwrap().palette.bg, c(0x101010));
    // An absolute path is used as it is.
    let abs = dir.path().join("schemes/t.yaml");
    let cfg = load_in(
        tempfile::tempdir().unwrap().path(),
        &format!("[theme]\nname = \"{}\"\n", abs.display()),
    )
    .unwrap();
    assert_eq!(cfg.theme.resolve().unwrap().palette.bg, c(0x101010));
    // A missing file is an error naming it.
    let e = err("[theme]\nname = \"nope/x.yaml\"\n");
    assert!(e.contains("nope/x.yaml") && e.contains("line 2"), "{e}");
}

#[test]
fn a_scheme_file_missing_a_slot_or_with_a_bad_colour_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("t.yaml"),
        BASE16.replace("  base0D: \"#0000ff\"\n", ""),
    )
    .unwrap();
    let e = format!(
        "{:#}",
        load_in(dir.path(), "[theme]\nname = \"t.yaml\"\n").unwrap_err()
    );
    assert!(e.contains("missing base0D") && e.contains("t.yaml"), "{e}");
    assert!(theme::parse_scheme(&BASE16.replace("#0000ff", "#00zzff"), "f").is_err());
}

#[test]
fn non_string_colours_name_their_key_and_line() {
    for (value, ty) in [
        ("123", "integer"),
        ("true", "boolean"),
        ("[1]", "array"),
        ("{ a = 1 }", "table"),
        ("1.5", "float"),
    ] {
        let e = err(&format!("[theme.colors]\nblue = {value}\n"));
        assert!(
            e.contains("theme.colors.blue") && e.contains("line 2") && e.contains(ty),
            "{value}: {e}"
        );
    }
}

#[test]
fn unknown_colour_keys_are_rejected_whatever_the_value() {
    for value in ["\"#123456\"", "\"nope\"", "123", "true", "[1]", "{ a = 1 }"] {
        let e = err(&format!("[theme.colors]\nbrown = {value}\n"));
        assert!(
            e.contains("unknown key `brown`") && e.contains("line 2") && e.contains("cursor_bg"),
            "{value}: {e}"
        );
    }
}

#[test]
fn bad_values_give_errors_naming_the_key_or_value() {
    let e = err("[theme.colors]\nblue = \"#12345\"\n");
    assert!(
        e.contains("theme.colors.blue") && e.contains("line 2"),
        "{e}"
    );
    assert!(e.contains("#12345"), "{e}");
    let e = err("[theme.colors]\nbrown = \"#123456\"\n");
    assert!(
        e.contains("brown") && e.contains("line 2") && e.contains("cursor_bg"),
        "{e}"
    );
    let e = err("[theme]\nname = \"gruvbox\"\n");
    assert!(e.contains("\"gruvbox\"") && e.contains("line 2"), "{e}");
    assert!(
        e.contains("catppuccin-mocha") && e.contains("gruvbox-light"),
        "{e}"
    );
    let e = err("[theme]\nsyntax = \"Nope\"\n");
    assert!(e.contains("\"Nope\"") && e.contains("line 2"), "{e}");
    assert!(e.contains("Dracula") && e.contains("Nord"), "{e}");
    let e = err("[theme]\ncolour = 1\n");
    assert!(e.contains("colour"), "{e}");
}

#[test]
fn syntax_is_matched_without_regard_to_case_and_overrides_the_default() {
    let cfg = load("[theme]\nname = \"gruvbox-dark\"\nsyntax = \"dRaCuLa\"\n").unwrap();
    assert_eq!(cfg.theme.resolve().unwrap().syntax, "Dracula");
    assert_eq!(
        theme::Theme::new(&cfg.resolve_theme().unwrap())
            .syntax
            .name
            .as_deref(),
        Some("Dracula")
    );
}

#[test]
fn render_theme_is_deprecated_but_still_loads() {
    let mut cfg = load("[render]\ntheme = \"catppuccin-mocha\"\n").unwrap();
    assert_eq!(
        cfg.warnings,
        ["[render] theme is deprecated; use [theme] name"]
    );
    assert_eq!(cfg.theme.resolve().unwrap().palette, MOCHA);
    cfg.warnings.clear();
    assert_eq!(cfg, Config::default());
    // [theme] name wins.
    let cfg =
        load("[render]\ntheme = \"catppuccin-mocha\"\n[theme]\nname = \"gruvbox-dark\"\n").unwrap();
    assert_eq!(cfg.theme.resolve().unwrap().palette.bg, c(0x282828));
    // Any other value is the same error as [theme] name's.
    let e = err("[render]\ntheme = \"gruvbox\"\n");
    assert!(
        e.contains("line 2") && e.contains("unknown theme \"gruvbox\""),
        "{e}"
    );
    assert!(e.contains("gruvbox-dark"), "{e}");
}

#[test]
fn the_default_file_has_a_theme_section_with_every_key() {
    let src = ramble::config::DEFAULT_TOML;
    for key in theme::KEYS {
        if ["add_bg", "del_bg", "hunk_bg"].contains(key) {
            continue;
        }
        let line = format!("# {key} = ");
        assert!(src.lines().any(|l| l.starts_with(&line)), "{key}");
    }
    for key in ["name", "syntax", "fill"] {
        assert!(src.contains(&format!("# {key} = ")), "{key}");
    }
    assert!(!src.contains("# theme = "), "no deprecated value line");
}

const SIZE: (u16, u16) = (40, 8);

fn app_with_theme(dir: &Path, theme_toml: &str, show: SidebarShow) -> App {
    let path = dir.join("doc.md");
    std::fs::write(&path, "# Title\n\nbody\n").unwrap();
    let mut config = load_in(dir, theme_toml).unwrap();
    config.sidebar.auto_hide_below = 0;
    config.sidebar.show = show;
    App::new(
        StartOptions {
            target: StartTarget::File(path),
            tree_root: dir.to_path_buf(),
            config,
            review_cache: None,
            mdroots: MdrootsOptions::memory(),
        },
        SIZE,
    )
    .unwrap()
}

fn draw(app: &App) -> ratatui::buffer::Buffer {
    let mut term = Terminal::new(TestBackend::new(SIZE.0, SIZE.1)).unwrap();
    term.draw(|f| ramble::ui::draw(f, app)).unwrap();
    term.backend().buffer().clone()
}

#[test]
fn fill_paints_bg_under_the_content_and_the_sidebar() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_with_theme(
        dir.path(),
        "[theme]\nname = \"solarized-light\"\nfill = true\n",
        SidebarShow::Always,
    );
    let p = theme::builtin("solarized-light").unwrap();
    let buf = draw(&app);
    let text_area = app.layout().text;
    // A blank cell in the content area, at the far right of a blank row.
    let (x, y) = (SIZE.0 - 1, 5);
    assert_eq!(buf[(x, y)].symbol(), " ");
    assert_eq!(buf[(x, y)].bg, p.bg);
    assert_eq!(buf[(x, y)].fg, p.text);
    // And in the sidebar, whose area is cleared first on a peek.
    assert_eq!(buf[(0, y)].bg, p.bg);
    assert!(text_area.is_some_and(|r| r.x > 0), "the sidebar is shown");
}

#[test]
fn without_fill_blank_cells_have_no_background() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_with_theme(
        dir.path(),
        "[theme]\nname = \"solarized-light\"\n",
        SidebarShow::Always,
    );
    let buf = draw(&app);
    assert_eq!(buf[(SIZE.0 - 1, 5)].bg, Color::Reset);
    assert_eq!(buf[(0, 5)].bg, Color::Reset);
}

/// An Auto sidebar opened with `C-w h` at a width too narrow to sit beside
/// the page: drawn over the text, after a `Clear`.
fn open_overlay(dir: &Path, theme_toml: &str) -> App {
    let mut app = app_with_theme(dir, theme_toml, SidebarShow::Auto);
    app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    app.handle_key(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE));
    assert!(app.sidebar_overlay(), "the sidebar is an overlay");
    app
}

#[test]
fn fill_repaints_the_auto_sidebar_overlay_after_its_clear() {
    let dir = tempfile::tempdir().unwrap();
    let app = open_overlay(
        dir.path(),
        "[theme]\nname = \"solarized-light\"\nfill = true\n",
    );
    let p = theme::builtin("solarized-light").unwrap();
    let buf = draw(&app);
    let side = app.layout().sidebar.expect("the overlay is laid out");
    assert!(side.width > 0 && side.width < SIZE.0, "{side:?}");
    // A blank row of the cleared area: its background and text colour are
    // the theme's, not the terminal's.
    let y = SIZE.1 - 2;
    for x in side.x..side.right() {
        assert_eq!(buf[(x, y)].bg, p.bg, "x={x}");
    }
    // The border column (the page side of a left sidebar) is `muted`.
    let border = side.right() - 1;
    for x in side.x..border {
        assert_eq!(buf[(x, y)].fg, p.text, "x={x}");
    }
    assert_eq!(buf[(border, y)].fg, p.muted);
}

#[test]
fn without_fill_the_auto_sidebar_overlay_has_no_background() {
    let dir = tempfile::tempdir().unwrap();
    let app = open_overlay(dir.path(), "[theme]\nname = \"solarized-light\"\n");
    let buf = draw(&app);
    let side = app.layout().sidebar.expect("the overlay is laid out");
    let y = SIZE.1 - 2;
    for x in side.x..side.right() {
        assert_eq!(buf[(x, y)].bg, Color::Reset, "x={x}");
    }
}

#[test]
fn a_theme_changes_what_the_page_is_drawn_in() {
    let dir = tempfile::tempdir().unwrap();
    let app = app_with_theme(
        dir.path(),
        "[theme]\nname = \"gruvbox-dark\"\n",
        SidebarShow::Never,
    );
    let p = theme::builtin("gruvbox-dark").unwrap();
    let buf = draw(&app);
    // The heading's first letter is in the heading colour (purple).
    assert_eq!(buf[(0, 0)].fg, p.purple);
    // The status line is on bg_dark.
    assert_eq!(buf[(0, SIZE.1 - 1)].bg, p.bg_dark);
}

/// Load a config whose `name` is a scheme file, then remove or change the
/// file: the validated resolution is still what is used.
#[test]
fn a_loaded_scheme_file_is_not_read_again() {
    let dir = tempfile::tempdir().unwrap();
    let scheme = dir.path().join("t.yaml");
    std::fs::write(&scheme, BASE16).unwrap();
    let toml = "[theme]\nname = \"t.yaml\"\n";
    let cfg = load_in(dir.path(), toml).unwrap();
    let want = cfg.resolve_theme().unwrap();
    assert_eq!(want.palette.bg, c(0x101010));

    std::fs::write(&scheme, BASE16.replace("#101010", "#202020")).unwrap();
    assert_eq!(cfg.resolve_theme().unwrap(), want, "changed file");
    std::fs::remove_file(&scheme).unwrap();
    assert_eq!(cfg.resolve_theme().unwrap(), want, "deleted file");

    // The app starts with the validated palette too.
    let doc = dir.path().join("doc.md");
    std::fs::write(&doc, "# Title\n").unwrap();
    let app = App::new(
        StartOptions {
            target: StartTarget::File(doc),
            tree_root: dir.path().to_path_buf(),
            config: cfg.clone(),
            review_cache: None,
            mdroots: MdrootsOptions::memory(),
        },
        SIZE,
    )
    .unwrap();
    assert_eq!(app.theme().palette, want.palette);
    // The cache is invisible to equality.
    assert_eq!(
        cfg,
        load_in(dir.path(), "[theme]\nname = \"catppuccin-mocha\"\n")
            .map(|mut o| {
                o.theme = cfg.theme.clone();
                o
            })
            .unwrap()
    );
}

/// Every programmatic change to `theme` after load is resolved afresh.
#[test]
fn changing_the_theme_after_load_invalidates_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("t.yaml"), BASE16).unwrap();
    let base = load_in(dir.path(), "[theme]\nname = \"t.yaml\"\n").unwrap();
    assert_eq!(base.resolve_theme().unwrap().palette.bg, c(0x101010));

    let mut cfg = base.clone();
    cfg.theme.name = Some("gruvbox-dark".into());
    assert_eq!(cfg.resolve_theme().unwrap().palette.bg, c(0x282828), "name");

    let mut cfg = base.clone();
    cfg.theme.syntax = Some("Dracula".into());
    assert_eq!(cfg.resolve_theme().unwrap().syntax, "Dracula", "syntax");

    let mut cfg = base.clone();
    cfg.theme.fill = Some(true);
    assert!(cfg.resolve_theme().unwrap().fill, "fill");

    let mut cfg = base.clone();
    cfg.theme.colors.set("bg", [1, 2, 3]);
    assert_eq!(
        cfg.resolve_theme().unwrap().palette.bg,
        c(0x010203),
        "colors"
    );

    // `dir` changes where a relative scheme path is found.
    let other = tempfile::tempdir().unwrap();
    std::fs::write(
        other.path().join("t.yaml"),
        BASE16.replace("#101010", "#303030"),
    )
    .unwrap();
    let mut cfg = base.clone();
    cfg.theme.dir = Some(other.path().to_path_buf());
    assert_eq!(cfg.resolve_theme().unwrap().palette.bg, c(0x303030), "dir");

    // A change that no longer resolves is an error, not the stale cache.
    let mut cfg = base;
    cfg.theme.name = Some("nope".into());
    assert!(cfg.resolve_theme().is_err());
}
