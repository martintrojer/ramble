//! Regression tests for doc parsing / rendering fixes.

use ramble::doc::parse;
use ramble::render::{Theme, render};

fn rows(src: &str) -> Vec<String> {
    let doc = parse(src.to_owned());
    render(&doc, 40, &Theme::catppuccin_mocha())
        .lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect()
}

#[test]
fn loose_task_items_render_checkbox_and_text() {
    let text = rows("- [ ] alpha\n\n- [x] beta\n\n- gamma\n").join("\n");
    assert!(text.contains("[ ] alpha"), "{text}");
    assert!(text.contains("[x] beta"), "{text}");
    assert!(text.contains("gamma"), "{text}");
}

#[test]
fn tight_task_items_render_checkbox_and_text() {
    let text = rows("- [ ] alpha\n- [x] beta\n- gamma\n").join("\n");
    assert!(text.contains("[ ] alpha"), "{text}");
    assert!(text.contains("[x] beta"), "{text}");
    assert!(text.contains("gamma"), "{text}");
}
