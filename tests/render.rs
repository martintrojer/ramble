//! Render tests. Most build `Document`s by hand and inject an inline walker
//! through `render_with`; a few render real `doc::parse` output.

use std::ops::Range;

use proptest::prelude::*;
use ramble::doc::{
    AlertKind, Alignment, Block, Document, Inline, InlineStyle, Link, LinkKind, ListItem,
};
use ramble::render::{RenderedPage, Theme, palette, render, render_with, to_ansi};
use ratatui::style::{Color, Modifier, Style};
use unicode_width::UnicodeWidthStr;

// ---------------------------------------------------------------------------
// Helpers

/// Byte range of the first occurrence of `needle` in `src`.
fn rng(src: &str, needle: &str) -> Range<usize> {
    let start = src
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} not in source"));
    start..start + needle.len()
}

/// Byte range of the first occurrence of `needle` at or after `from`.
fn rng_from(src: &str, from: usize, needle: &str) -> Range<usize> {
    let start = from + src[from..].find(needle).expect("needle after offset");
    start..start + needle.len()
}

fn doc(source: &str, blocks: Vec<Block>, links: Vec<Link>) -> Document {
    Document {
        source: source.to_string(),
        blocks,
        headings: Vec::new(),
        links,
        code_spans: Vec::new(),
        front_matter: None,
        front_matter_kind: Default::default(),
        fm: None,
        lossy: false,
    }
}

fn para(src: &str, text: &str) -> Block {
    let r = rng(src, text);
    Block::Paragraph {
        range: r.clone(),
        inline: r,
    }
}

/// A tiny inline walker for tests: links (from `doc.links`), `**strong**`,
/// `` `code` ``, `[^label]` footnote refs, and newlines as soft breaks.
fn walk(d: &Document, range: Range<usize>) -> Vec<Inline> {
    let src = &d.source;
    let mut out = Vec::new();
    let mut i = range.start;
    let mut text_start = i;
    let mut strong = false;
    let flush = |out: &mut Vec<Inline>, from: usize, to: usize, strong: bool| {
        if from < to {
            out.push(Inline::Text {
                range: from..to,
                style: InlineStyle {
                    strong,
                    ..Default::default()
                },
            });
        }
    };
    while i < range.end {
        let rest = &src[i..range.end];
        if let Some((idx, link)) = d.links.iter().enumerate().find(|(_, l)| l.range.start == i) {
            flush(&mut out, text_start, i, strong);
            out.push(Inline::Text {
                range: link.text_range.clone(),
                style: InlineStyle {
                    strong,
                    link: Some(idx),
                    ..Default::default()
                },
            });
            i = link.range.end;
            text_start = i;
        } else if rest.starts_with("**") {
            flush(&mut out, text_start, i, strong);
            strong = !strong;
            i += 2;
            text_start = i;
        } else if let Some(code) = rest.strip_prefix('`') {
            flush(&mut out, text_start, i, strong);
            let end = i + 1 + code.find('`').expect("closing backtick");
            out.push(Inline::Code {
                range: i + 1..end,
                link: None,
            });
            i = end + 1;
            text_start = i;
        } else if let Some(body) = rest.strip_prefix('$') {
            flush(&mut out, text_start, i, strong);
            let display = body.starts_with('$');
            let d = 1 + usize::from(display);
            let len = rest[d..].find(&"$$"[..d]).expect("closing $");
            let range = i + d..i + d + len;
            let whole = i..range.end + d;
            out.push(Inline::Math {
                tex: src[range.clone()].to_string(),
                display,
                range: range.clone(),
                raw: vec![whole],
                link: None,
            });
            i = range.end + d;
            text_start = i;
        } else if rest.starts_with("[^") {
            flush(&mut out, text_start, i, strong);
            let end = i + rest.find(']').expect("closing bracket");
            out.push(Inline::FootnoteRef {
                label: src[i + 2..end].to_string(),
                range: i + 2..end,
            });
            i = end + 1;
            text_start = i;
        } else if rest.starts_with('\n') {
            flush(&mut out, text_start, i, strong);
            out.push(Inline::SoftBreak);
            i += 1;
            text_start = i;
        } else {
            i += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
    flush(&mut out, text_start, range.end, strong);
    out
}

fn draw(d: &Document, width: u16) -> RenderedPage {
    render_with(d, width, &Theme::catppuccin_mocha(), &|r| walk(d, r))
}

fn color_name(c: Color) -> String {
    let named = [
        (palette::TEXT, "text"),
        (palette::MAUVE, "mauve"),
        (palette::BLUE, "blue"),
        (palette::GREEN, "green"),
        (palette::OVERLAY, "overlay"),
        (palette::RED, "red"),
        (palette::YELLOW, "yellow"),
        (palette::PEACH, "peach"),
        (palette::TEAL, "teal"),
    ];
    match named.iter().find(|(n, _)| *n == c) {
        Some((_, name)) => (*name).to_string(),
        None => match c {
            Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
            other => format!("{other:?}"),
        },
    }
}

fn style_tag(s: Style) -> String {
    let mut tag = s.fg.map(color_name).unwrap_or_default();
    for (m, ch) in [
        (Modifier::BOLD, 'b'),
        (Modifier::ITALIC, 'i'),
        (Modifier::UNDERLINED, 'u'),
        (Modifier::CROSSED_OUT, 's'),
    ] {
        if s.add_modifier.contains(m) {
            tag.push(if tag.is_empty() { ch } else { ',' });
            if tag.ends_with(',') {
                tag.push(ch);
            }
        }
    }
    tag
}

/// Plain text with style markers: `{tag}text{/}` per styled span (plain
/// text colour untagged), prefixed by the row's source line.
fn snapshot_text(page: &RenderedPage) -> String {
    let mut out = String::new();
    for (line, src_line) in page.lines.iter().zip(&page.source_lines) {
        out.push_str(&format!("{src_line:>3}|"));
        for span in &line.spans {
            let tag = style_tag(span.style);
            if tag.is_empty() || tag == "text" || span.content.trim().is_empty() {
                out.push_str(&span.content);
            } else {
                out.push_str(&format!("{{{tag}}}{}{{/}}", span.content));
            }
        }
        out.push('\n');
    }
    out
}

fn strip_sgr(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for c in chars.by_ref() {
                if c == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Fixture

const FIXTURE: &str = "# Ramble héading 😀

## Second level

A paragraph with **strong** text, `code`, and a [link](https://example.com) that should reflow across several rows at narrow widths. 日本語のテキストも折り返す。Footnote[^1].

- first item
- [x] done task
- [ ] open task
  - nested item

> plain quote

> [!WARNING]
> alert body

```rust
fn main() {
\tprintln!(\"hi\");
}
```

| Name | Value |
|:-----|------:|
| é | 😀 |
| 漢字 | long cell text here |

---

<div>raw html</div>

Math $\\sum_{i=1}^n x_i$ and $\\mathbb{R}^n$ inline.

$$\\frac{a+b}{c}$$

[^1]: The footnote.
";

fn fixture() -> Document {
    let s = FIXTURE;
    let link_range = rng(s, "[link](https://example.com)");
    let link_text = rng(s, "link](").start..rng(s, "link](").start + 4;
    let links = vec![Link {
        kind: LinkKind::Markdown,
        dest: "https://example.com".into(),
        range: link_range,
        text_range: link_text,
        resolved: None,
        line: None,
    }];
    let p = rng(s, "A paragraph");
    let p = p.start..rng(s, "Footnote[^1].").end;
    let item = |line: &str, text: &str, task: Option<bool>, extra: Vec<Block>| {
        let r = rng(s, line);
        let mut children = vec![Block::Paragraph {
            range: rng(s, text),
            inline: rng(s, text),
        }];
        children.extend(extra);
        ListItem {
            range: r,
            task,
            children,
        }
    };
    let nested = Block::List {
        range: rng(s, "  - nested item"),
        start: None,
        items: vec![item("- nested item", "nested item", None, vec![])],
    };
    let list_start = rng(s, "- first item").start;
    let list_end = rng(s, "  - nested item").end;
    let code_fence = rng(s, "```rust");
    let code_end = rng_from(s, code_fence.end, "```");
    let table = rng(s, "| Name").start..rng(s, "long cell text here |").end;
    let row2 = rng(s, "| 漢字").start;
    let cell = |from: usize, t: &str| rng_from(s, from, t);
    let blocks = vec![
        Block::Heading {
            level: 1,
            range: rng(s, "# Ramble héading 😀"),
            inline: rng(s, "Ramble héading 😀"),
        },
        Block::Heading {
            level: 2,
            range: rng(s, "## Second level"),
            inline: rng(s, "Second level"),
        },
        Block::Paragraph {
            range: p.clone(),
            inline: p,
        },
        Block::List {
            range: list_start..list_end,
            start: None,
            items: vec![
                item("- first item", "first item", None, vec![]),
                item("- [x] done task", "done task", Some(true), vec![]),
                ListItem {
                    range: rng(s, "- [ ] open task").start..list_end,
                    ..item("- [ ] open task", "open task", Some(false), vec![nested])
                },
            ],
        },
        Block::BlockQuote {
            range: rng(s, "> plain quote"),
            alert: None,
            children: vec![para(s, "plain quote")],
        },
        Block::BlockQuote {
            range: rng(s, "> [!WARNING]").start..rng(s, "> alert body").end,
            alert: Some(AlertKind::Warning),
            children: vec![para(s, "alert body")],
        },
        Block::CodeBlock {
            lang: Some("rust".into()),
            range: code_fence.start..code_end.end,
            code: code_fence.end + 1..code_end.start,
            lines: std::iter::once(code_fence.end + 1..code_end.start).collect(),
        },
        Block::Table {
            range: table.clone(),
            alignments: vec![Alignment::Left, Alignment::Right],
            header: vec![cell(table.start, "Name"), cell(table.start, "Value")],
            rows: vec![
                vec![cell(table.start, "é"), cell(table.start, "😀")],
                vec![cell(row2, "漢字"), cell(row2, "long cell text here")],
            ],
        },
        Block::Rule {
            range: rng(s, "---\n\n<div>").start..rng(s, "---\n\n<div>").start + 3,
        },
        Block::Html {
            range: rng(s, "<div>raw html</div>"),
        },
        para(s, "Math $\\sum_{i=1}^n x_i$ and $\\mathbb{R}^n$ inline."),
        para(s, "$$\\frac{a+b}{c}$$"),
        Block::FootnoteDefinition {
            label: "1".into(),
            range: rng(s, "[^1]: The footnote."),
            children: vec![para(s, "The footnote.")],
        },
    ];
    doc(s, blocks, links)
}

// ---------------------------------------------------------------------------
// Golden snapshots

#[test]
fn snapshot_width_40() {
    insta::assert_snapshot!(snapshot_text(&draw(&fixture(), 40)));
}

#[test]
fn snapshot_width_80() {
    insta::assert_snapshot!(snapshot_text(&draw(&fixture(), 80)));
}

// ---------------------------------------------------------------------------
// Example-based checks

#[test]
fn empty_document_and_tiny_widths_never_panic() {
    let empty = doc("", vec![], vec![]);
    for w in 0..=12 {
        let page = draw(&empty, w);
        assert!(page.lines.is_empty());
        let page = draw(&fixture(), w);
        assert_eq!(page.lines.len(), page.source_lines.len());
        let _ = to_ansi(&page);
    }
}

#[test]
fn fixture_rows_fit_width() {
    let d = fixture();
    for w in 10..=100u16 {
        let page = draw(&d, w);
        for (i, line) in page.lines.iter().enumerate() {
            assert!(line.width() <= w as usize, "w={w} row {i}: {line:?}");
        }
    }
}

#[test]
fn link_text_has_link_segment_and_markup_is_not_drawn() {
    let d = fixture();
    let page = draw(&d, 40);
    let link = &d.links[0];
    let segs: Vec<_> = page
        .srcmap
        .segments
        .iter()
        .filter(|s| s.link == Some(0))
        .collect();
    assert!(!segs.is_empty());
    for s in &segs {
        assert!(s.src.start >= link.text_range.start && s.src.end <= link.text_range.end);
    }
    // The `(https://example.com)` destination and `**` markers are never drawn.
    let dest = rng(&d.source, "(https://example.com)");
    assert!(page.srcmap.spans_for(dest).is_empty());
    let star = rng(&d.source, "**strong");
    assert!(page.srcmap.spans_for(star.start..star.start + 2).is_empty());
}

#[test]
fn code_rows_map_to_their_source_lines() {
    let d = fixture();
    let page = draw(&d, 80);
    let line = rng(&d.source, "fn main() {");
    let rows: Vec<_> = page
        .srcmap
        .spans_for(line.clone())
        .iter()
        .map(|s| s.row)
        .collect();
    assert_eq!(rows.len(), 1, "one segment for one code row");
    let row = rows[0];
    let src_line = d.source[..line.start].matches('\n').count() + 1;
    assert_eq!(page.source_lines[row], src_line);
    // The tab expands to 4 spaces.
    let next: String = page.lines[row + 1]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect();
    assert!(next.starts_with("    println!"), "{next:?}");
}

#[test]
fn row_for_finds_first_segment_at_or_after_byte() {
    let d = fixture();
    let page = draw(&d, 40);
    let segs = &page.srcmap.segments;
    for s in segs {
        let row = page.srcmap.row_for(s.src.start).expect("a row");
        assert!(row <= s.span.row);
    }
    // A byte inside link syntax maps to the next drawn row.
    let dest = rng(&d.source, "https://example.com");
    let expect = segs
        .iter()
        .find(|s| s.src.end > dest.start)
        .map(|s| s.span.row);
    assert_eq!(page.srcmap.row_for(dest.start), expect);
    assert_eq!(page.srcmap.row_for(d.source.len()), None);
    assert!(page.source_lines.windows(2).all(|w| w[0] <= w[1]));
}

#[test]
fn source_at_snaps_to_nearest_and_empty_rows_are_none() {
    let d = fixture();
    let page = draw(&d, 40);
    let blank = page
        .lines
        .iter()
        .position(|l| l.width() == 0)
        .expect("a blank row");
    assert_eq!(page.srcmap.source_at(blank, 0), None);
    // Far right of the heading row snaps to its last drawn byte.
    let heading = rng(&d.source, "Ramble héading 😀");
    assert_eq!(page.srcmap.source_at(0, 39), Some(heading.end - 1));
}

#[test]
fn to_ansi_has_truecolor_and_plain_text_within_width() {
    let d = fixture();
    let page = draw(&d, 40);
    let ansi = to_ansi(&page);
    assert!(ansi.contains("\x1b[38;2;"));
    let plain = strip_sgr(&ansi);
    assert!(plain.contains("Ramble héading 😀"));
    assert!(plain.contains("日本語"));
    assert_eq!(plain.lines().count(), page.lines.len());
    for line in plain.lines() {
        assert!(UnicodeWidthStr::width(line) <= 40, "{line:?}");
    }
    assert!(ansi.ends_with("\x1b[0m\n"));
}

fn row_text(page: &RenderedPage, row: usize) -> String {
    page.lines[row]
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect()
}

#[test]
fn narrow_table_wraps_cells_inside_borders() {
    let src = "| Name | Value |\n|:-----|------:|\n| é | 😀 |\n| 漢字 | long cell text here |\n";
    let d = ramble::doc::parse(src.to_string());
    // Natural width is 30; at 24 the columns shrink and cells wrap.
    let page = render(&d, 24, &Theme::catppuccin_mocha());
    let rows: Vec<String> = (0..page.lines.len()).map(|r| row_text(&page, r)).collect();
    assert!(!rows.is_empty());
    let width = UnicodeWidthStr::width(rows[0].as_str());
    for row in &rows {
        let first = row.chars().next().expect("non-empty row");
        let last = row.chars().last().expect("non-empty row");
        assert!("┌├└│".contains(first), "{row:?}");
        assert!("┐┤┘│".contains(last), "{row:?}");
        assert_eq!(UnicodeWidthStr::width(row.as_str()), width, "{row:?}");
        assert!(width <= 24, "{row:?}");
    }
    // The long cell wraps at a word boundary onto two rows.
    let long = rows
        .iter()
        .position(|r| r.contains("long cell"))
        .expect("long cell row");
    assert!(rows[long + 1].contains("text here"), "{rows:#?}");
}

#[test]
fn control_chars_draw_as_question_marks() {
    for (src, drawn) in [
        ("```\nx\u{1b}y\u{7}z\n```\n", "x?y?z"),
        ("a\u{7}b\n", "a?b"),
    ] {
        let d = ramble::doc::parse(src.to_string());
        let page = render(&d, 40, &Theme::catppuccin_mocha());
        let text = row_text(&page, 0);
        assert_eq!(text.trim_end(), drawn, "{src:?}");
        assert!(!text.chars().any(char::is_control), "{text:?}");
        assert_eq!(page.lines[0].width(), drawn.len());
        let seg = &page.srcmap.segments[0];
        assert_eq!(seg.span.col_end - seg.span.col_start, drawn.len());
    }
}

#[test]
fn touching_links_get_separate_segments() {
    // Two links whose visible text is contiguous in the source.
    let src = "ab";
    let links = vec![
        Link {
            kind: LinkKind::Markdown,
            dest: "x".into(),
            range: 0..1,
            text_range: 0..1,
            resolved: None,
            line: None,
        },
        Link {
            kind: LinkKind::Markdown,
            dest: "y".into(),
            range: 1..2,
            text_range: 1..2,
            resolved: None,
            line: None,
        },
    ];
    let d = doc(src, vec![para(src, "ab")], links);
    let page = draw(&d, 40);
    for (i, link) in d.links.iter().enumerate() {
        let segs: Vec<_> = page
            .srcmap
            .segments
            .iter()
            .filter(|s| s.link == Some(i))
            .collect();
        assert_eq!(segs.len(), 1, "link {i}: {:?}", page.srcmap.segments);
        assert_eq!(segs[0].src, link.text_range, "link {i}");
    }
    assert_eq!(page.srcmap.segments.len(), 2);
}

// ---------------------------------------------------------------------------
// Properties

const WORDS: &[&str] = &[
    "a",
    "the",
    "word",
    "é",
    "café",
    "😀",
    "😀😀",
    "漢字",
    "日本語のテキスト",
    "x",
    "supercalifragilisticexpialidocious",
    "**bold**",
    "`code`",
    "naïve",
];

#[derive(Debug, Clone)]
enum Piece {
    Word(usize),
    Link(Vec<usize>),
}

#[derive(Debug, Clone)]
struct GenPara {
    container: u8,
    pieces: Vec<Piece>,
}

fn piece() -> impl Strategy<Value = Piece> {
    prop_oneof![
        4 => (0..WORDS.len()).prop_map(Piece::Word),
        1 => prop::collection::vec(0..WORDS.len(), 1..4).prop_map(Piece::Link),
    ]
}

fn gen_doc() -> impl Strategy<Value = Vec<GenPara>> {
    prop::collection::vec(
        (0u8..3, prop::collection::vec(piece(), 1..30))
            .prop_map(|(container, pieces)| GenPara { container, pieces }),
        0..5,
    )
}

/// Build source, blocks and links from generated paragraphs. Link text uses
/// plain words only (no markup), so it is always visible.
fn build(paras: &[GenPara]) -> Document {
    let mut src = String::new();
    let mut blocks = Vec::new();
    let mut links = Vec::new();
    for p in paras {
        let block_start = src.len();
        let marker = match p.container {
            1 => "> ",
            2 => "- ",
            _ => "",
        };
        src.push_str(marker);
        let inline_start = src.len();
        for (i, piece) in p.pieces.iter().enumerate() {
            if i > 0 {
                src.push(if i % 7 == 0 { '\n' } else { ' ' });
            }
            match piece {
                Piece::Word(w) => src.push_str(WORDS[*w]),
                Piece::Link(ws) => {
                    let start = src.len();
                    src.push('[');
                    let text_start = src.len();
                    let text: Vec<&str> = ws
                        .iter()
                        .map(|w| WORDS[*w].trim_matches(['*', '`']))
                        .collect();
                    src.push_str(&text.join(" "));
                    let text_end = src.len();
                    src.push_str("](dest)");
                    links.push(Link {
                        kind: LinkKind::Markdown,
                        dest: "dest".into(),
                        range: start..src.len(),
                        text_range: text_start..text_end,
                        resolved: None,
                        line: None,
                    });
                }
            }
        }
        let inline = inline_start..src.len();
        let range = block_start..src.len();
        let paragraph = Block::Paragraph {
            range: inline.clone(),
            inline,
        };
        blocks.push(match p.container {
            1 => Block::BlockQuote {
                range,
                alert: None,
                children: vec![paragraph],
            },
            2 => Block::List {
                range: range.clone(),
                start: None,
                items: vec![ListItem {
                    range,
                    task: None,
                    children: vec![paragraph],
                }],
            },
            _ => paragraph,
        });
        src.push_str("\n\n");
    }
    doc(&src, blocks, links)
}

fn block_ranges(blocks: &[Block], out: &mut Vec<Range<usize>>) {
    for b in blocks {
        match b {
            Block::Paragraph { range, .. } => out.push(range.clone()),
            Block::BlockQuote { children, .. } => block_ranges(children, out),
            Block::List { items, .. } => {
                for i in items {
                    block_ranges(&i.children, out);
                }
            }
            _ => {}
        }
    }
}

proptest! {
    #[test]
    fn wrapping_never_exceeds_width(paras in gen_doc(), width in 10u16..120) {
        let d = build(&paras);
        let page = draw(&d, width);
        for line in &page.lines {
            prop_assert!(line.width() <= width as usize, "{:?}", line);
        }
        let ansi = to_ansi(&page);
        for line in strip_sgr(&ansi).lines() {
            prop_assert!(UnicodeWidthStr::width(line) <= width as usize);
        }
    }

    #[test]
    fn tiny_widths_never_panic(paras in gen_doc(), width in 0u16..10) {
        let d = build(&paras);
        let page = draw(&d, width);
        prop_assert_eq!(page.lines.len(), page.source_lines.len());
    }

    #[test]
    fn srcmap_properties(paras in gen_doc(), width in 10u16..120) {
        let d = build(&paras);
        let page = draw(&d, width);
        let map = &page.srcmap;

        // Every link's visible text has a screen span.
        for (i, link) in d.links.iter().enumerate() {
            prop_assert!(!map.spans_for(link.text_range.clone()).is_empty(), "link {}", i);
            prop_assert!(map.segments.iter().any(|s| s.link == Some(i)));
        }

        let mut ranges = Vec::new();
        block_ranges(&d.blocks, &mut ranges);
        let mut last = None;
        for seg in &map.segments {
            // Sorted, non-overlapping.
            let key = (seg.span.row, seg.span.col_start);
            if let Some(prev) = last {
                prop_assert!(prev <= key);
            }
            last = Some((seg.span.row, seg.span.col_end));
            // Inside its block.
            prop_assert!(
                ranges.iter().any(|r| r.start <= seg.src.start && seg.src.end <= r.end),
                "{:?} outside blocks", seg
            );
            // Screen -> source stays in the segment, for every cell.
            for col in seg.span.col_start..seg.span.col_end {
                let b = map.source_at(seg.span.row, col);
                prop_assert!(b.is_some_and(|b| seg.src.contains(&b)), "{:?} col {} -> {:?}", seg, col, b);
            }
            // Source -> screen contains the segment.
            prop_assert!(map.spans_for(seg.src.clone()).contains(&seg.span));
            // row_for is at or before this segment's row.
            prop_assert!(map.row_for(seg.src.start).is_some_and(|r| r <= seg.span.row));
        }
        prop_assert!(page.source_lines.windows(2).all(|w| w[0] <= w[1]));
    }
}

// ---------------------------------------------------------------------------
// Math

fn render_md(src: &str, width: u16, math: bool) -> (Document, RenderedPage) {
    let d = ramble::doc::parse(src.to_string());
    let theme = Theme { math };
    let page = render(&d, width, &theme);
    (d, page)
}

fn all_rows(page: &RenderedPage) -> Vec<String> {
    (0..page.lines.len()).map(|r| row_text(page, r)).collect()
}

#[test]
fn inline_math_renders_unicode_in_math_colour() {
    let (_, page) = render_md("Sum $\\sum_{i=1}^n x_i$ here.\n", 80, true);
    assert_eq!(row_text(&page, 0), "Sum ∑ᵢ₌₁ⁿ xᵢ here.");
    let span = page.lines[0]
        .spans
        .iter()
        .find(|s| s.content.contains('∑'))
        .expect("math span");
    assert_eq!(span.style.fg, Some(palette::TEAL));
}

#[test]
fn bad_math_keeps_raw_source() {
    for (tex, want) in [
        ("\\foo{x}", "\\foo{x}"),
        (
            "\\begin{matrix}a&b\\end{matrix}",
            "\\begin{matrix}a&b\\end{matrix}",
        ),
        ("x^2", "x²"),
    ] {
        let (_, page) = render_md(&format!("A ${tex}$ b\n"), 80, true);
        assert_eq!(row_text(&page, 0), format!("A {want} b"), "{tex}");
    }
    // CommonMark does not read `$\frac{a}{$` as math; a math fence does.
    let (_, page) = render_md("```math\n\\frac{a}{\n```\n", 40, true);
    assert_eq!(all_rows(&page), vec!["\\frac{a}{"]);
}

#[test]
fn display_math_draws_centred_fraction() {
    let src = "$$\n\\frac{a+b}{c}\n$$\n";
    let (d, page) = render_md(src, 21, true);
    let rows = all_rows(&page);
    assert_eq!(rows, vec!["        a + b", "       ───────", "          c"]);
    // Every math row maps into the LaTeX, not the delimiters.
    let inner = rng(src, "\\frac{a+b}{c}");
    for row in 0..3 {
        let segs: Vec<_> = page
            .srcmap
            .segments
            .iter()
            .filter(|s| s.span.row == row)
            .collect();
        assert!(!segs.is_empty(), "row {row}");
        for seg in segs {
            assert!(
                seg.src.start >= inner.start && seg.src.end <= inner.end,
                "row {row}: {:?} outside {inner:?} ({:?})",
                seg.src,
                &d.source[seg.src.clone()]
            );
        }
    }
    assert_eq!(page.source_lines, vec![2, 2, 2]);
}

#[test]
fn display_math_in_quote_maps_past_prefix() {
    let src = "> $$\n> \\frac{1}{2}\n> $$\n";
    let (_, page) = render_md(src, 40, true);
    let rows = all_rows(&page);
    assert_eq!(rows.len(), 3, "{rows:#?}");
    assert!(rows.iter().all(|r| r.starts_with("│ ")), "{rows:#?}");
    let inner = rng(src, "\\frac{1}{2}");
    for seg in &page.srcmap.segments {
        assert!(seg.src.start >= inner.start && seg.src.end <= inner.end);
    }
}

#[test]
fn too_wide_display_math_falls_back_to_one_line() {
    let src = "$$\\sum_{i=1}^n i = \\frac{n(n+1)}{2}$$\n";
    let (_, wide) = render_md(src, 40, true);
    assert_eq!(wide.lines.len(), 3);
    let (_, narrow) = render_md(src, 16, true);
    assert_eq!(all_rows(&narrow), vec!["∑ᵢ₌₁ⁿ i =", "(n(n+1))/2"]);
}

#[test]
fn display_math_inside_text_renders_inline() {
    let (_, page) = render_md("half $$\\frac{1}{2}$$ done\n", 40, true);
    assert_eq!(all_rows(&page), vec!["half 1/2 done"]);
}

#[test]
fn math_fence_renders_like_display_math() {
    let (_, page) = render_md("```math\n\\frac{a}{b}\n```\n", 3, true);
    assert_eq!(all_rows(&page), vec![" a", "───", " b"]);
}

#[test]
fn math_off_shows_raw_source() {
    let src = "A $x^2$ b\n\n$$\\frac{1}{2}$$\n";
    let (_, page) = render_md(src, 40, false);
    assert_eq!(all_rows(&page), vec!["A $x^2$ b", "", "$$\\frac{1}{2}$$"]);
    let span = page.lines[0]
        .spans
        .iter()
        .find(|s| s.content.contains("x^2"))
        .expect("math span");
    assert_eq!(span.style.fg, Some(palette::TEAL));
    // Raw source maps byte for byte.
    let spans = page.srcmap.spans_for(rng(src, "$x^2$"));
    assert_eq!((spans[0].col_start, spans[0].col_end), (0, 9));
    assert_eq!(page.srcmap.source_at(0, 4), Some(4));
}

#[test]
fn math_fence_keeps_gt_in_latex() {
    // A fence line starting with `>` is LaTeX, not a quote prefix.
    let (_, page) = render_md("```math\nx\n> 0\n```\n", 40, true);
    let rows = all_rows(&page);
    assert!(rows.iter().any(|r| r.contains('>')), "{rows:#?}");
}

#[test]
fn math_fence_in_quote_skips_only_the_prefix() {
    let src = "> ```math\n> a\n> > b\n> ```\n";
    let (d, page) = render_md(src, 40, true);
    let rows = all_rows(&page);
    assert!(rows.iter().all(|r| r.starts_with("│ ")), "{rows:#?}");
    let body: String = rows.iter().map(|r| &r["│ ".len()..]).collect();
    assert_eq!(body.matches('>').count(), 1, "{rows:#?}");
    // No segment covers a `> ` container prefix.
    let prefixes = [rng(src, "> a"), rng_from(src, 13, "> > b")];
    for seg in &page.srcmap.segments {
        for p in &prefixes {
            let p = p.start..p.start + 2;
            assert!(
                seg.src.end <= p.start || seg.src.start >= p.end,
                "{:?} covers prefix {p:?} ({:?})",
                seg.src,
                &d.source[seg.src.clone()]
            );
        }
    }
}

#[test]
fn too_wide_display_math_measures_the_container_width() {
    // The grid is 20 columns; the list item leaves 21 - 2 = 19 at width 21.
    let src = "- $$\\sum_{i=1}^n i = \\frac{n(n+1)}{2}$$\n";
    let (_, narrow) = render_md(src, 21, true);
    assert_eq!(all_rows(&narrow), vec!["• ∑ᵢ₌₁ⁿ i =", "  (n(n+1))/2"]);
    let (_, wide) = render_md(src, 22, true);
    assert_eq!(wide.lines.len(), 3, "{:#?}", all_rows(&wide));
    let quoted = src.replacen("- ", "> ", 1);
    let (_, page) = render_md(&quoted, 21, true);
    assert_eq!(all_rows(&page), vec!["│ ∑ᵢ₌₁ⁿ i =", "│ (n(n+1))/2"]);
}

#[test]
fn display_math_placement() {
    // Text after the math keeps the paragraph inline.
    let (_, page) = render_md("$$\\frac{a}{b}$$ tail\n", 40, true);
    assert_eq!(all_rows(&page), vec!["a/b tail"]);
    // A heading is not a paragraph: inline.
    let (_, page) = render_md("# $$\\frac{a}{b}$$\n", 40, true);
    assert_eq!(row_text(&page, 0), "a/b");
    // A list item's lone math is a display block.
    let (_, page) = render_md("- $$\\frac{a}{b}$$\n", 40, true);
    let rows = all_rows(&page);
    assert_eq!(rows.len(), 3, "{rows:#?}");
    assert!(
        rows[0].starts_with("• ") && rows[1].contains("───"),
        "{rows:#?}"
    );
}

#[test]
fn math_off_draws_math_fence_as_code() {
    let (_, page) = render_md("```math\n\\frac{a}{b}\n```\n", 40, false);
    assert_eq!(all_rows(&page), vec!["\\frac{a}{b}"]);
}

#[test]
fn blank_or_unbalanced_conversion_keeps_raw_source() {
    // `\,` converts to nothing; keep the source rather than draw blank.
    let (_, page) = render_md("A $\\,$ b\n", 40, true);
    assert_eq!(row_text(&page, 0), "A \\, b");
    // The grid for `\{x` is `{x` (unbalanced): fall back to one line.
    let (_, page) = render_md("$$\\{x$$\n", 40, true);
    assert_eq!(all_rows(&page), vec!["\\{x"]);
}
