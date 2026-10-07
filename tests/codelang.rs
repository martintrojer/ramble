//! Language-named code fences: info-string parsing and syntax resolution.

use std::collections::HashSet;

use ramble::doc::{Block, parse};
use ramble::render::{Theme, render, resolve_syntax};

fn langs(src: &str) -> Vec<Option<String>> {
    parse(src.to_owned())
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::CodeBlock { lang, .. } => Some(lang.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn fence_lang_is_first_token_cleaned() {
    let got = langs(
        "```rust,ignore\na\n```\n\n```rust title=\"x\"\na\n```\n\n```{.python}\na\n```\n\n\
         ```{ .Haskell }\na\n```\n\n```Python3\na\n```\n\n~~~go\na\n~~~\n\n```\na\n```\n\n\
         para\n\n    indented\n",
    );
    let want = [
        Some("rust"),
        Some("rust"),
        Some("python"),
        Some("haskell"),
        Some("python3"),
        Some("go"),
        None,
        None,
    ];
    assert_eq!(got, want.map(|l| l.map(str::to_owned)).to_vec());
}

#[test]
fn aliases_resolve_to_real_syntaxes() {
    for (lang, name) in [
        ("python3", "Python"),
        ("shell", "Bourne Again Shell (bash)"),
        ("console", "Bourne Again Shell (bash)"),
        ("shellsession", "Bourne Again Shell (bash)"),
        ("sh-session", "Bourne Again Shell (bash)"),
        ("jsx", "TypeScriptReact"),
        ("jsonc", "JSON"),
        ("json5", "JSON"),
        ("golang", "Go"),
        ("lean4", "Lean 4"),
        ("objc", "Objective-C"),
        ("csharp", "C#"),
        ("containerfile", "Dockerfile"),
        ("rust", "Rust"),
        ("Rust", "Rust"),
        ("py", "Python"),
    ] {
        assert_eq!(resolve_syntax(lang), Some(name), "{lang}");
    }
}

#[test]
fn plain_and_unknown_langs_stay_plain() {
    for lang in ["text", "txt", "plain", "none", "nosuchlang", ""] {
        assert_eq!(resolve_syntax(lang), None, "{lang}");
    }
}

/// Distinct foreground colours drawn on the code rows of a one-block page.
fn code_colours(src: &str) -> usize {
    let page = render(&parse(src.to_owned()), 80, &Theme::catppuccin_mocha());
    page.lines
        .iter()
        .flat_map(|l| &l.spans)
        .filter(|s| !s.content.trim().is_empty())
        .filter_map(|s| s.style.fg)
        .collect::<HashSet<_>>()
        .len()
}

#[test]
fn attributed_fences_highlight_and_indented_do_not() {
    let body = "def f(x):\n    return \"s\" + 1\n";
    let plain = code_colours(&format!("```\n{body}```\n"));
    for info in [
        "python",
        "{.python}",
        "python,ignore",
        "python3",
        "py title=\"x\"",
    ] {
        let n = code_colours(&format!("```{info}\n{body}```\n"));
        assert!(n > plain, "{info}: {n} <= {plain}");
    }
    let indented = code_colours(&format!("    {}", body.replace('\n', "\n        ")));
    assert_eq!(indented, plain);
}
