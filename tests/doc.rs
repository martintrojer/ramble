//! Tests for `ramble::doc`: parsing, ranges, slugs and inline segments.

use std::fs;

use proptest::prelude::*;
use ramble::doc::{
    AlertKind, Alignment, Block, Document, Inline, InlineStyle, LinkKind, from_bytes, inlines,
    parse,
};

fn fixture(name: &str) -> Document {
    let path = format!("{}/tests/fixtures/doc/{name}", env!("CARGO_MANIFEST_DIR"));
    parse(fs::read_to_string(path).unwrap())
}

fn s(doc: &Document, r: &std::ops::Range<usize>) -> String {
    doc.source[r.clone()].to_owned()
}

/// Concatenated source text of the drawn `Text` segments in `range`.
fn drawn(doc: &Document, range: std::ops::Range<usize>) -> String {
    inlines(doc, range)
        .iter()
        .map(|i| match i {
            Inline::Text { range, .. } | Inline::Code { range, .. } => s(doc, range),
            Inline::SoftBreak => " ".into(),
            Inline::HardBreak => "\n".into(),
            Inline::FootnoteRef { label, .. } => format!("[{label}]"),
            Inline::Math { tex, .. } => format!("${tex}$"),
        })
        .collect()
}

fn first_paragraph_inline(doc: &Document) -> std::ops::Range<usize> {
    doc.blocks
        .iter()
        .find_map(|b| match b {
            Block::Paragraph { inline, .. } => Some(inline.clone()),
            _ => None,
        })
        .expect("a paragraph")
}

#[test]
fn heading_slugs() {
    let doc = fixture("headings.md");
    let got: Vec<(u8, &str, &str)> = doc
        .headings
        .iter()
        .map(|h| (h.level, h.text.as_str(), h.slug.as_str()))
        .collect();
    assert_eq!(
        got,
        vec![
            (1, "Hello World", "hello-world"),
            (2, "Hello World", "hello-world-1"),
            (2, "Hello, World!", "hello-world-2"),
            (3, "What's new? 😀 Emoji", "whats-new--emoji"),
            (2, "日本語", "日本語"),
            (2, "snake_case and-dash", "snake_case-and-dash"),
            (1, "Custom styled", "my-id"),
            (2, "Hello World", "hello-world-3"),
        ]
    );
    assert_eq!(s(&doc, &doc.headings[0].range), "# Hello World\n");
}

#[test]
fn heading_slugs_keep_combining_marks() {
    let doc = parse("## हिन्दी\n\n## e\u{301}te!\n\n## \u{20DD}x 😀\n".to_owned());
    let slugs: Vec<&str> = doc.headings.iter().map(|h| h.slug.as_str()).collect();
    // Mn (virama U+094D, acute U+0301) and Me (U+20DD) are kept like
    // github-slugger; punctuation and emoji are still dropped.
    assert_eq!(slugs, ["हिन्दी", "e\u{301}te", "\u{20DD}x-"]);
}

#[test]
fn heading_attributes_excluded_from_inline() {
    let doc = fixture("headings.md");
    let inline = doc
        .blocks
        .iter()
        .find_map(|b| match b {
            Block::Heading { inline, range, .. } if s(&doc, range).contains("{#my-id}") => {
                Some(inline.clone())
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(s(&doc, &inline), "Custom *styled*");
    let segs = inlines(&doc, inline);
    assert!(matches!(
        &segs[1],
        Inline::Text { style: InlineStyle { emphasis: true, .. }, range } if s(&doc, range) == "styled"
    ));
}

#[test]
fn links_dest_range_and_text_range() {
    let doc = fixture("links.md");
    let got: Vec<(LinkKind, String, String, String)> = doc
        .links
        .iter()
        .map(|l| {
            (
                l.kind.clone(),
                l.dest.clone(),
                s(&doc, &l.range),
                s(&doc, &l.text_range),
            )
        })
        .collect();
    use LinkKind::*;
    let e = |k, d: &str, r: &str, t: &str| (k, d.to_owned(), r.to_owned(), t.to_owned());
    assert_eq!(
        got,
        vec![
            e(
                Markdown,
                "https://example.com",
                "[inline](https://example.com)",
                "inline"
            ),
            e(Markdown, "./ref.md", "[ref][r]", "ref"),
            e(
                Markdown,
                "https://auto.example",
                "<https://auto.example>",
                "https://auto.example"
            ),
            e(Wiki, "target", "[[target]]", "target"),
            e(Wiki, "other note", "[[other note|the label]]", "the label"),
            e(
                Markdown,
                "notes/a.md#intro",
                "[doc](notes/a.md#intro)",
                "doc"
            ),
            e(
                Markdown,
                "mailto:me@example.com",
                "<mailto:me@example.com>",
                "mailto:me@example.com"
            ),
        ]
    );
}

#[test]
fn inlines_link_style_points_at_doc_links() {
    let doc = fixture("links.md");
    let segs = inlines(&doc, first_paragraph_inline(&doc));
    let linked: Vec<(usize, String)> = segs
        .iter()
        .filter_map(|i| match i {
            Inline::Text { range, style } => style.link.map(|l| (l, s(&doc, range))),
            _ => None,
        })
        .collect();
    let expect: Vec<(usize, String)> = doc
        .links
        .iter()
        .enumerate()
        .map(|(i, l)| (i, s(&doc, &l.text_range)))
        .collect();
    assert_eq!(linked, expect);
    assert!(drawn(&doc, first_paragraph_inline(&doc)).contains("bare https://bare.example stays"));
}

#[test]
fn inlines_escapes_entities_code_html_breaks() {
    let src = "a &amp; b \\* c ``x` y`` <b>hi</b>  \nnext\nline **s** ~~d~~".to_owned();
    let doc = parse(src);
    let segs = inlines(&doc, first_paragraph_inline(&doc));
    let texts: Vec<String> = segs
        .iter()
        .map(|i| match i {
            Inline::Text { range, .. } => s(&doc, range),
            Inline::Code { range, .. } => format!("`{}`", s(&doc, range)),
            Inline::SoftBreak => "<soft>".into(),
            Inline::HardBreak => "<hard>".into(),
            Inline::FootnoteRef { .. } => "<fn>".into(),
            Inline::Math { .. } => "<math>".into(),
        })
        .collect();
    assert_eq!(
        texts,
        [
            "a ", "&amp;", " b ", "* c ", "`x` y`", " ", "<b>", "hi", "</b>", "<hard>", "next",
            "<soft>", "line ", "s", " ", "d"
        ]
    );
    let style_of = |t: &str| {
        segs.iter().find_map(|i| match i {
            Inline::Text { range, style } if s(&doc, range) == t => Some(*style),
            _ => None,
        })
    };
    assert!(style_of("s").unwrap().strong);
    assert!(style_of("d").unwrap().strikethrough);
    assert_eq!(style_of("next").unwrap(), InlineStyle::default());
}

#[test]
fn inlines_inside_quote_and_list_skip_prefixes() {
    let doc = parse("> one\n> two\n\n- [x] item\n  more\n".to_owned());
    let Block::BlockQuote { children, .. } = &doc.blocks[0] else {
        panic!()
    };
    let Block::Paragraph { inline, .. } = &children[0] else {
        panic!()
    };
    assert_eq!(drawn(&doc, inline.clone()), "one two");
    let Block::List { items, .. } = &doc.blocks[1] else {
        panic!()
    };
    assert_eq!(items[0].task, Some(true));
    let Block::Paragraph { inline, .. } = &items[0].children[0] else {
        panic!()
    };
    assert_eq!(drawn(&doc, inline.clone()), "item more");
}

#[test]
fn reference_link_inside_quote_keeps_link_style() {
    let doc = parse("> see [x][r]\n\n[r]: http://r\n".to_owned());
    let Block::BlockQuote { children, .. } = &doc.blocks[0] else {
        panic!()
    };
    let Block::Paragraph { inline, .. } = &children[0] else {
        panic!()
    };
    let segs = inlines(&doc, inline.clone());
    assert!(matches!(
        segs.last(),
        Some(Inline::Text {
            style: InlineStyle { link: Some(0), .. },
            ..
        })
    ));
    assert_eq!(doc.links[0].dest, "http://r");
}

#[test]
fn blocks_tables_tasks_alerts_footnotes_code() {
    let doc = fixture("blocks.md");
    let b = &doc.blocks;
    assert_eq!(b.len(), 8, "{b:#?}");

    let Block::Table {
        alignments,
        header,
        rows,
        range,
    } = &b[0]
    else {
        panic!("{:?}", b[0])
    };
    assert!(s(&doc, range).starts_with("| Left"));
    assert_eq!(
        alignments,
        &[
            Alignment::Left,
            Alignment::Center,
            Alignment::Right,
            Alignment::None
        ]
    );
    let cells = |v: &[std::ops::Range<usize>]| -> Vec<String> {
        v.iter().map(|r| drawn(&doc, r.clone())).collect()
    };
    assert_eq!(cells(header), ["Left", "Mid", "Right", "None"]);
    assert_eq!(rows.len(), 2);
    assert_eq!(cells(&rows[1]), ["e", "f", "g", "h"]);

    let Block::List { items, start, .. } = &b[1] else {
        panic!()
    };
    assert_eq!(*start, None);
    let tasks: Vec<Option<bool>> = items.iter().map(|i| i.task).collect();
    assert_eq!(tasks, [Some(false), Some(true), None]);
    assert_eq!(s(&doc, &items[1].range), "- [x] done\n");

    let Block::BlockQuote {
        alert, children, ..
    } = &b[2]
    else {
        panic!()
    };
    assert_eq!(*alert, Some(AlertKind::Warning));
    let Block::Paragraph { inline, .. } = &children[0] else {
        panic!()
    };
    assert_eq!(s(&doc, inline), "Careful now.");
    assert!(matches!(&b[3], Block::BlockQuote { alert: None, .. }));

    let Block::Paragraph { inline, .. } = &b[4] else {
        panic!()
    };
    let segs = inlines(&doc, inline.clone());
    let Some(Inline::FootnoteRef { label, range }) = segs
        .iter()
        .find(|i| matches!(i, Inline::FootnoteRef { .. }))
    else {
        panic!("{segs:?}")
    };
    assert_eq!(label, "n");
    assert_eq!(s(&doc, range), "n");

    let Block::FootnoteDefinition {
        label,
        children,
        range,
    } = &b[5]
    else {
        panic!()
    };
    assert_eq!(label, "n");
    assert!(s(&doc, range).starts_with("[^n]: The note body."));
    assert!(
        matches!(&children[0], Block::Paragraph { inline, .. } if s(&doc, inline) == "The note body.")
    );

    let Block::CodeBlock { lang, code, .. } = &b[6] else {
        panic!()
    };
    assert_eq!(lang.as_deref(), Some("rust"));
    assert_eq!(s(&doc, code), "fn main() {}\n");
    assert!(matches!(&b[7], Block::Rule { range } if s(&doc, range).starts_with("---")));
}

#[test]
fn all_alert_kinds() {
    let doc = parse(
        "> [!NOTE]\n> a\n\n> [!TIP]\n> a\n\n> [!IMPORTANT]\n> a\n\n\
         > [!WARNING]\n> a\n\n> [!CAUTION]\n> a\n\n> [!note]\n> a\n"
            .to_owned(),
    );
    let alerts: Vec<Option<AlertKind>> = doc
        .blocks
        .iter()
        .map(|b| match b {
            Block::BlockQuote { alert, .. } => *alert,
            other => panic!("{other:?}"),
        })
        .collect();
    use AlertKind::*;
    assert_eq!(
        alerts,
        [
            Some(Note),
            Some(Tip),
            Some(Important),
            Some(Warning),
            Some(Caution),
            Some(Note)
        ]
    );
}

/// Renders segments as strings: Text verbatim, Code in backticks, breaks
/// as markers. Panics if any drawn range contains a newline.
fn seg_strings(doc: &Document, inline: std::ops::Range<usize>) -> Vec<String> {
    inlines(doc, inline)
        .iter()
        .map(|i| {
            let out = match i {
                Inline::Text { range, .. } => s(doc, range),
                Inline::Code { range, .. } => format!("`{}`", s(doc, range)),
                Inline::SoftBreak => "<soft>".into(),
                Inline::HardBreak => "<hard>".into(),
                Inline::FootnoteRef { label, .. } => format!("[^{label}]"),
                Inline::Math { range, .. } => format!("${}$", s(doc, range)),
            };
            assert!(!out.contains('\n'), "segment crosses a line: {out:?}");
            out
        })
        .collect()
}

#[test]
fn multiline_constructs_in_quote_exclude_prefix() {
    let doc = parse("> `a\n> b` and <span\n> x> [c\n> d](x)\n".to_owned());
    let Block::BlockQuote { children, .. } = &doc.blocks[0] else {
        panic!()
    };
    let Block::Paragraph { inline, .. } = &children[0] else {
        panic!()
    };
    assert_eq!(
        seg_strings(&doc, inline.clone()),
        [
            "`a`", "<soft>", "`b`", " and ", "<span", "<soft>", "x>", " ", "c", "<soft>", "d"
        ]
    );
    // text_range spans start..end of the visible text (prefix included).
    assert_eq!(s(&doc, &doc.links[0].text_range), "c\n> d");
}

#[test]
fn multiline_code_span_in_nested_list_excludes_indent() {
    let doc = parse("> - item `one\n>   two  three`\n".to_owned());
    let Block::BlockQuote { children, .. } = &doc.blocks[0] else {
        panic!()
    };
    let Block::List { items, .. } = &children[0] else {
        panic!()
    };
    let Block::Paragraph { inline, .. } = &items[0].children[0] else {
        panic!()
    };
    assert_eq!(
        seg_strings(&doc, inline.clone()),
        ["item ", "`one`", "<soft>", "`two  three`"]
    );
}

#[test]
fn single_line_code_keeps_padding_rules() {
    let doc = parse("`` `x` `` and `` ``\n".to_owned());
    assert_eq!(
        seg_strings(&doc, first_paragraph_inline(&doc)),
        ["``x``", " and ", "` `"]
    );
}

#[test]
fn loose_list_task_items() {
    let doc = parse("- [ ] a\n\n- [x] b\n\n- c\n".to_owned());
    let Block::List { items, .. } = &doc.blocks[0] else {
        panic!()
    };
    let tasks: Vec<Option<bool>> = items.iter().map(|i| i.task).collect();
    assert_eq!(tasks, [Some(false), Some(true), None]);
}

#[test]
fn nested_blocks_are_not_top_level() {
    let doc = parse("- a\n  - b\n\n> q\n".to_owned());
    assert_eq!(doc.blocks.len(), 2);
    let Block::List { items, .. } = &doc.blocks[0] else {
        panic!()
    };
    assert!(matches!(&items[0].children[1], Block::List { .. }));
}

#[test]
fn ordered_list_start_and_html_block() {
    let doc = parse("3. x\n4. y\n\n<div>\nraw\n</div>\n".to_owned());
    assert!(
        matches!(&doc.blocks[0], Block::List { start: Some(3), items, .. } if items.len() == 2)
    );
    assert!(
        matches!(&doc.blocks[1], Block::Html { range } if s(&doc, range) == "<div>\nraw\n</div>\n")
    );
}

#[test]
fn images_are_not_links() {
    let doc = parse("![alt](i.png) ![[embed]] [t](d)\n".to_owned());
    assert_eq!(doc.links.len(), 1);
    assert_eq!(doc.links[0].dest, "d");
}

#[test]
fn from_bytes_binary_and_lossy() {
    let mut bin = vec![b'a'; 100];
    bin.push(0);
    assert!(from_bytes(&bin).is_none());

    let mut late_nul = vec![b'a'; 8192];
    late_nul.push(0);
    assert!(
        from_bytes(&late_nul).is_some(),
        "NUL after 8 KiB is not binary"
    );

    let doc = from_bytes(b"ok \xff\xfe text").unwrap();
    assert!(doc.lossy);
    assert!(doc.source.contains('\u{FFFD}'));

    let doc = from_bytes("fine é".as_bytes()).unwrap();
    assert!(!doc.lossy);
    assert_eq!(doc.source, "fine é");
}

#[test]
fn non_ascii_before_link_keeps_byte_ranges() {
    let doc = parse("é😀 then [link](x.md) and [[wiki]]\n".to_owned());
    assert_eq!(s(&doc, &doc.links[0].range), "[link](x.md)");
    assert_eq!(s(&doc, &doc.links[0].text_range), "link");
    assert_eq!(doc.links[0].range.start, "é😀 then ".len());
    assert_eq!(s(&doc, &doc.links[1].text_range), "wiki");
    let segs = inlines(&doc, first_paragraph_inline(&doc));
    assert!(matches!(&segs[0], Inline::Text { range, .. } if s(&doc, range) == "é😀 then "));
}

fn word() -> impl Strategy<Value = String> {
    "[a-zA-Zé😀日]{1,6}"
}

fn piece() -> impl Strategy<Value = (String, Option<String>)> {
    prop_oneof![
        (word(), "[a-z]{1,5}(\\.md)?(#[a-z]{1,4})?")
            .prop_map(|(w, d)| (format!("[{w}]({d})"), Some(w))),
        word().prop_map(|w| (format!("[[{w}]]"), Some(w))),
        (word(), word()).prop_map(|(t, l)| (format!("[[{t}|{l}]]"), Some(l))),
        word().prop_map(|w| (w, None)),
    ]
}

proptest! {
    #[test]
    fn link_text_range_is_drawn_text(pieces in prop::collection::vec(piece(), 1..8)) {
        let src = pieces.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>().join(" ");
        let doc = parse(src.clone());
        let expected: Vec<&String> = pieces.iter().filter_map(|(_, t)| t.as_ref()).collect();
        prop_assert_eq!(doc.links.len(), expected.len(), "{}", src);
        let segs = inlines(&doc, first_paragraph_inline(&doc));
        for (i, (link, text)) in doc.links.iter().zip(expected).enumerate() {
            prop_assert!(link.range.start <= link.text_range.start && link.text_range.end <= link.range.end);
            prop_assert_eq!(&doc.source[link.text_range.clone()], text.as_str());
            let drawn: String = segs.iter().filter_map(|seg| match seg {
                Inline::Text { range, style } if style.link == Some(i) => Some(&doc.source[range.clone()]),
                _ => None,
            }).collect();
            prop_assert_eq!(drawn, text.as_str());
        }
    }
}

#[test]
fn code_spans_outside_links_and_add_links_reindexes_inline_code() {
    use ramble::doc::Link;
    let src = "`a.rs` [`in`](x) `` b c `` ![`img`](i.png)\n";
    let mut doc = parse(src.into());
    let texts: Vec<_> = doc.code_spans.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(texts, ["a.rs", "b c"]);
    let first = doc.code_spans[0].clone();
    assert_eq!(s(&doc, &first.range), "`a.rs`");
    assert_eq!(s(&doc, &first.text_range), "a.rs");
    // Inside a link the code carries that link's index.
    let all = inlines(&doc, 0..src.len() - 1);
    let code_links: Vec<_> = all
        .iter()
        .filter_map(|i| match i {
            Inline::Code { link, .. } => Some(*link),
            _ => None,
        })
        .collect();
    assert_eq!(code_links, [None, Some(0), None, None]);

    doc.add_links(vec![Link {
        kind: LinkKind::CodePath,
        dest: "a.rs".into(),
        range: first.range.clone(),
        text_range: first.text_range.clone(),
        resolved: Some("/a.rs".into()),
        line: None,
    }]);
    assert_eq!(doc.links[0].kind, LinkKind::CodePath);
    assert_eq!(doc.links[1].dest, "x");
    let code_links: Vec<_> = inlines(&doc, 0..src.len() - 1)
        .iter()
        .filter_map(|i| match i {
            Inline::Code { link, .. } => Some(*link),
            _ => None,
        })
        .collect();
    assert_eq!(code_links, [Some(0), Some(1), None, None]);
}

fn visible_rows(doc: &Document) -> Vec<String> {
    let page = ramble::render::render(doc, 40, &ramble::render::Theme::catppuccin_mocha());
    ramble::render::to_ansi(&page)
        .lines()
        .map(|l| {
            let mut out = String::new();
            let mut chars = l.chars();
            while let Some(c) = chars.next() {
                if c == '\x1b' {
                    chars.by_ref().find(|c| c.is_ascii_alphabetic());
                } else {
                    out.push(c);
                }
            }
            out.trim().to_owned()
        })
        .filter(|l| !l.is_empty())
        .collect()
}

#[test]
fn yaml_front_matter_is_metadata() {
    let src = "---\ntitle: X\ntags: [a]\nsee: \"[x](y.md)\"\n---\n\n# Real\n";
    let doc = parse(src.to_owned());
    let texts: Vec<&str> = doc.headings.iter().map(|h| h.text.as_str()).collect();
    assert_eq!(texts, vec!["Real"]);
    assert!(doc.links.is_empty());
    assert_eq!(
        doc.front_matter,
        Some(0.."---\ntitle: X\ntags: [a]\nsee: \"[x](y.md)\"\n---".len())
    );
    assert!(matches!(
        doc.blocks.as_slice(),
        [Block::Heading { level: 1, .. }]
    ));
    assert_eq!(visible_rows(&doc).first().map(String::as_str), Some("Real"));
}

#[test]
fn leading_rule_without_closing_fence_is_not_front_matter() {
    let src = "---\nJust text after a rule.\n\nMore text.\n";
    let doc = parse(src.to_owned());
    assert_eq!(doc.front_matter, None);
    assert!(matches!(
        doc.blocks.as_slice(),
        [
            Block::Rule { .. },
            Block::Paragraph { .. },
            Block::Paragraph { .. }
        ]
    ));
    assert!(doc.headings.is_empty());
}

// ---------------------------------------------------------------------------
// Math

/// The math segments of the first paragraph: (tex, display, range text).
fn maths(doc: &Document) -> Vec<(String, bool, String)> {
    inlines(doc, first_paragraph_inline(doc))
        .iter()
        .filter_map(|i| match i {
            Inline::Math {
                tex,
                display,
                range,
                ..
            } => Some((tex.clone(), *display, s(doc, range))),
            _ => None,
        })
        .collect()
}

#[test]
fn inline_and_display_math_parse() {
    let doc = parse("a $x^2$ b $$\\frac{1}{2}$$ c\n".to_owned());
    assert_eq!(
        maths(&doc),
        vec![
            ("x^2".into(), false, "x^2".into()),
            ("\\frac{1}{2}".into(), true, "\\frac{1}{2}".into()),
        ]
    );
    assert_eq!(
        drawn(&doc, first_paragraph_inline(&doc)),
        "a $x^2$ b $\\frac{1}{2}$ c"
    );
}

#[test]
fn escaped_dollar_and_code_span_are_not_math() {
    let doc = parse("\\$5 and `$x$` and $5 and $6\n".to_owned());
    assert!(maths(&doc).is_empty());
    assert_eq!(
        drawn(&doc, first_paragraph_inline(&doc)),
        "$5 and $x$ and $5 and $6"
    );
}

#[test]
fn display_math_in_quote_maps_past_prefixes() {
    let src = "> $$\n> \\frac{a}{b}\n> $$\n";
    let doc = parse(src.to_owned());
    let Block::BlockQuote { children, .. } = &doc.blocks[0] else {
        panic!("{:?}", doc.blocks);
    };
    let Block::Paragraph { inline, .. } = &children[0] else {
        panic!("{children:?}");
    };
    let segs = inlines(&doc, inline.clone());
    let [
        Inline::Math {
            tex,
            display,
            range,
            raw,
            ..
        },
    ] = segs.as_slice()
    else {
        panic!("{segs:?}");
    };
    assert!(*display);
    assert_eq!(tex.trim(), "\\frac{a}{b}");
    assert_eq!(s(&doc, range), "\\frac{a}{b}");
    let raw: Vec<String> = raw.iter().map(|r| s(&doc, r)).collect();
    assert_eq!(raw, vec!["$$", "\\frac{a}{b}", "$$"]);
}

#[test]
fn math_range_skips_padding_spaces() {
    let doc = parse("a $$ \\frac{1}{2} $$ c\n".to_owned());
    let m = maths(&doc);
    assert_eq!(m.len(), 1, "{m:?}");
    assert_eq!(m[0].2, "\\frac{1}{2}");
}
