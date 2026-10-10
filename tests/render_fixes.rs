//! Regression tests for code fences inside containers, sanitized footnote
//! labels, and the `[render] theme` key.

use std::io::Write;
use std::process::{Command, Stdio};

use ramble::config::Config;
use ramble::render::{RenderedPage, Theme, render};

fn page(src: &str) -> (ramble::doc::Document, RenderedPage) {
    let d = ramble::doc::parse(src.to_string());
    let p = render(&d, 60, &Theme::catppuccin_mocha());
    (d, p)
}

fn rows(p: &RenderedPage) -> Vec<String> {
    p.lines
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect()
}

/// `ramble --print` of `input` on stdin, with no user config.
fn print(input: &[u8]) -> String {
    let tmp = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_ramble"))
        .args(["--print", "--width", "60", "--config"])
        .arg(tmp.path().join("none.toml"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap()
}

/// `s` without SGR sequences (`ESC [ ... m`).
fn strip_sgr(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            let mut seq = String::new();
            for d in chars.by_ref() {
                seq.push(d);
                if d == 'm' {
                    break;
                }
            }
            if seq.starts_with('[') && seq.ends_with('m') {
                continue;
            }
            out.push(c);
            out.push_str(&seq);
        } else {
            out.push(c);
        }
    }
    out
}

const QUOTE_FENCE: &str = "> ```text\n> first\n> second\n> ```\n";
const LIST_FENCE: &str = "- item\n\n  ```text\n  first\n    second\n  ```\n";

#[test]
fn fence_in_blockquote_has_no_quote_markers() {
    let (d, p) = page(QUOTE_FENCE);
    assert_eq!(rows(&p), ["│ first", "│ second"]);
    // Each code row maps back to its own line's text.
    let second = d.source.find("second").unwrap();
    assert_eq!(p.srcmap.source_at(1, 2), Some(second));
}

#[test]
fn fence_in_list_item_keeps_only_its_own_indent() {
    let (_, p) = page(LIST_FENCE);
    assert_eq!(rows(&p), ["• item", "  first", "    second"]);
}

#[test]
fn fences_in_containers_print_without_prefixes() {
    let quote = strip_sgr(&print(QUOTE_FENCE.as_bytes()));
    assert_eq!(quote, "│ first\n│ second\n");
    let list = strip_sgr(&print(LIST_FENCE.as_bytes()));
    assert_eq!(list, "• item\n  first\n    second\n");
}

#[test]
fn top_level_fence_keeps_blank_lines() {
    let (_, p) = page("```rust\nfn a() {}\n\nlet x = 1;\n```\n");
    assert_eq!(rows(&p), ["fn a() {}", "", "let x = 1;"]);
}

#[test]
fn footnote_label_control_chars_are_sanitized() {
    let out = print(b"[^a\x1bc]: body\n");
    assert!(!strip_sgr(&out).contains('\x1b'), "{out:?}");
    assert!(strip_sgr(&out).starts_with("[a?c] body"), "{out:?}");
}

#[test]
fn long_footnote_label_is_sanitized_too() {
    let (_, p) = page("[^long\x07label]: body\n");
    let r = rows(&p);
    assert_eq!(r[0], "[long?label]");
    assert!(r.iter().all(|row| !row.chars().any(char::is_control)));
}

fn load(src: &str) -> anyhow::Result<Config> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, src).unwrap();
    Config::load(&path)
}

#[test]
fn theme_catppuccin_mocha_is_accepted() {
    assert_eq!(
        load("[render]\ntheme = \"catppuccin-mocha\"\n").unwrap(),
        Config::default()
    );
}

#[test]
fn other_theme_is_a_config_error() {
    let e = format!(
        "{:#}",
        load("[render]\nmax_width = 80\ntheme = \"gruvbox\"\n").unwrap_err()
    );
    assert!(e.contains("line 3"), "{e}");
    assert!(e.contains("unknown theme \"gruvbox\""), "{e}");
    assert!(e.contains("catppuccin-mocha"), "{e}");
}

/// `s` with every `\n` as `\r\n`.
fn crlf(s: &str) -> String {
    s.replace('\n', "\r\n")
}

#[test]
fn crlf_top_level_fence_has_no_extra_rows() {
    let src = crlf("```text\nfirst\n\nsecond\n```\n");
    let (d, p) = page(&src);
    assert_eq!(rows(&p), ["first", "", "second"]);
    let second = d.source.find("second").unwrap();
    assert_eq!(p.srcmap.source_at(2, 0), Some(second));
    assert_eq!(p.source_lines, [2, 3, 4]);
}

#[test]
fn crlf_fence_in_blockquote_has_no_extra_rows() {
    let (d, p) = page(&crlf(QUOTE_FENCE));
    assert_eq!(rows(&p), ["│ first", "│ second"]);
    let second = d.source.find("second").unwrap();
    assert_eq!(p.srcmap.source_at(1, 2), Some(second));
}

#[test]
fn crlf_fence_in_list_item_has_no_extra_rows() {
    let (d, p) = page(&crlf(LIST_FENCE));
    assert_eq!(rows(&p), ["• item", "  first", "    second"]);
    let second = d.source.find("second").unwrap();
    assert_eq!(p.srcmap.source_at(2, 4), Some(second));
}

#[test]
fn crlf_fences_print_without_extra_rows() {
    let top = strip_sgr(&print(crlf("```text\nfirst\nsecond\n```\n").as_bytes()));
    assert_eq!(top, "first\nsecond\n");
    let quote = strip_sgr(&print(crlf(QUOTE_FENCE).as_bytes()));
    assert_eq!(quote, "│ first\n│ second\n");
    let list = strip_sgr(&print(crlf(LIST_FENCE).as_bytes()));
    assert_eq!(list, "• item\n  first\n    second\n");
}

// A tab the container prefix only partly consumes leaves code-owned
// columns the parser synthesizes as spaces with no source bytes.
const TAB_LIST_FENCE: &str = "- item\n\n  ```text\n\tfirst\n\tsecond\n  ```\n";
const TAB_QUOTE_FENCE: &str = "> ```text\n>\tfirst\n>\tsecond\n> ```\n";

#[test]
fn partial_tab_in_list_fence_keeps_code_indent() {
    let (d, p) = page(TAB_LIST_FENCE);
    assert_eq!(rows(&p), ["• item", "    first", "    second"]);
    let second = d.source.find("second").unwrap();
    assert_eq!(p.srcmap.source_at(2, 4), Some(second));
}

#[test]
fn partial_tab_in_quote_fence_keeps_code_indent() {
    let (d, p) = page(TAB_QUOTE_FENCE);
    assert_eq!(rows(&p), ["│   first", "│   second"]);
    let second = d.source.find("second").unwrap();
    assert_eq!(p.srcmap.source_at(1, 4), Some(second));
}

#[test]
fn partial_tab_in_indented_code_keeps_code_indent() {
    // `>` then two tabs: the quote marker and its space take two columns,
    // the indented block four, and the code keeps the remaining two.
    let (_, p) = page(">\t\tcode\n");
    assert_eq!(rows(&p), ["│   code"]);
}

#[test]
fn partial_tab_fences_print_with_code_indent() {
    let list = strip_sgr(&print(TAB_LIST_FENCE.as_bytes()));
    assert_eq!(list, "• item\n    first\n    second\n");
    let quote = strip_sgr(&print(TAB_QUOTE_FENCE.as_bytes()));
    assert_eq!(quote, "│   first\n│   second\n");
    let crlf_list = strip_sgr(&print(crlf(TAB_LIST_FENCE).as_bytes()));
    assert_eq!(crlf_list, "• item\n    first\n    second\n");
}

/// Asserts the `pad` synthesized blanks before `text` on `row` (drawn
/// after a `prefix`-column container prefix) map to the tab before
/// `text`, and the first text column maps to `text`.
fn assert_pads_map_to_tab(src: &str, row: usize, prefix: usize, pad: usize, text: &str) {
    let (d, p) = page(src);
    let at = d.source.find(text).unwrap();
    assert_eq!(&d.source[at - 1..at], "\t");
    for col in prefix..prefix + pad {
        assert_eq!(p.srcmap.source_at(row, col), Some(at - 1), "pad col {col}");
    }
    assert_eq!(p.srcmap.source_at(row, prefix + pad), Some(at), "text col");
}

#[test]
fn partial_tab_pad2_maps_to_tab() {
    // List content starts at column 2; the tab reaches column 4.
    assert_pads_map_to_tab(TAB_LIST_FENCE, 2, 2, 2, "second");
}

#[test]
fn partial_tab_pad3_maps_to_tab() {
    // A one-space fence indent: the tab after it leaves three columns.
    assert_pads_map_to_tab(" ```text\n\tfirst\n ```\n", 0, 0, 3, "first");
}

#[test]
fn partial_tab_pad_before_multibyte_maps_to_tab() {
    let src = "> ```text\n>\téééééééééé\n> ```\n";
    assert_pads_map_to_tab(src, 0, 2, 2, "é");
}
