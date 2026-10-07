//! Markdown parsing into a [`Document`] with byte ranges for blocks,
//! headings and links. No rendering.
//!
//! Interface fixed by the orchestrator before parallel units start; the
//! `doc` unit (t02) implements [`parse`] and may add private helpers, but
//! must keep these public names and field types.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashSet;
use std::ops::Range;
use std::path::PathBuf;

use pulldown_cmark::{
    BlockQuoteKind, CodeBlockKind, Event, LinkType, Options, Parser, Tag, TagEnd,
};
use unicode_general_category::{GeneralCategory, get_general_category};

/// A parsed markdown document. `source` is the full input text; every
/// range below is a byte range into it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub source: String,
    /// Top-level blocks in document order; nested blocks live in
    /// `children` / `items`.
    pub blocks: Vec<Block>,
    /// Every heading, in order, with its GitHub-style slug.
    pub headings: Vec<Heading>,
    /// Every link (inline, reference, autolink, wikilink), in order.
    pub links: Vec<Link>,
    /// Inline code spans outside any link, in order: candidates for
    /// code-path links. The app keeps those naming an existing file
    /// ([`Document::add_links`]); `doc` itself does no I/O.
    pub code_spans: Vec<CodeSpan>,
    /// Byte range of a leading YAML front-matter block (`---` ... `---`),
    /// fences included. It is metadata: no block, heading or link.
    pub front_matter: Option<Range<usize>>,
    /// True when the input had invalid UTF-8 that was replaced with U+FFFD.
    pub lossy: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    /// 1..=6
    pub level: u8,
    /// Plain text of the heading (inline markup stripped).
    pub text: String,
    /// GitHub slug: lowercase, spaces to `-`, punctuation except `-`/`_`
    /// removed; duplicates get `-1`, `-2`, ... in document order.
    pub slug: String,
    /// Byte range of the whole heading construct.
    pub range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkKind {
    /// `[text](dest)`, `[text][ref]`, `<https://...>`, `<mailto:...>`.
    /// Bare URLs are plain text.
    Markdown,
    /// `[[target]]` or `[[target|label]]`.
    Wiki,
    /// An inline code span naming an existing file (added by the app, not
    /// by [`parse`]). Language-server results never apply to these.
    CodePath,
}

/// An inline code span outside any link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeSpan {
    /// Full span, including the backticks.
    pub range: Range<usize>,
    /// The code text (what the renderer draws).
    pub text_range: Range<usize>,
    /// The code text as CommonMark reads it.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub kind: LinkKind,
    /// Destination as written: a URL, a relative path, `path#anchor`,
    /// `#anchor`, or a wikilink target.
    pub dest: String,
    /// Full source span, including brackets and `(dest)`.
    pub range: Range<usize>,
    /// Source span of the visible text only (what the renderer draws).
    /// For a wikilink without a label this is the target text. When the
    /// text crosses lines inside a container it spans the `> ` / indent
    /// prefixes too; the drawn, prefix-free pieces come from [`inlines`].
    pub text_range: Range<usize>,
    /// [`LinkKind::CodePath`] only: the existing file the text names.
    pub resolved: Option<PathBuf>,
    /// [`LinkKind::CodePath`] only: the `:LINE` suffix, if any.
    pub line: Option<usize>,
}

/// Block-level structure. Inline content is kept as a source range and
/// re-walked by the renderer with [`inlines`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    Heading {
        level: u8,
        range: Range<usize>,
        inline: Range<usize>,
    },
    Paragraph {
        range: Range<usize>,
        inline: Range<usize>,
    },
    /// `lang` is the fence info string's first token, cut at the first `,`
    /// or whitespace, with surrounding `{`/`}` and a leading `.` stripped,
    /// lowercased (`rust,ignore` -> `rust`, `{.python}` -> `python`). None
    /// for indented blocks and empty info strings. `lines` are the code's
    /// text pieces with container prefixes (`> `, list indentation) already
    /// skipped; `code` spans the first to the last piece.
    CodeBlock {
        lang: Option<String>,
        range: Range<usize>,
        code: Range<usize>,
        lines: Vec<Range<usize>>,
    },
    BlockQuote {
        range: Range<usize>,
        alert: Option<AlertKind>,
        children: Vec<Block>,
    },
    List {
        range: Range<usize>,
        start: Option<u64>,
        items: Vec<ListItem>,
    },
    Table {
        range: Range<usize>,
        alignments: Vec<Alignment>,
        header: Vec<Range<usize>>,
        rows: Vec<Vec<Range<usize>>>,
    },
    Rule {
        range: Range<usize>,
    },
    /// Raw HTML block, shown as literal text.
    Html {
        range: Range<usize>,
    },
    /// `[^label]: ...` definition.
    FootnoteDefinition {
        label: String,
        range: Range<usize>,
        children: Vec<Block>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListItem {
    pub range: Range<usize>,
    /// `Some(checked)` for a task-list item.
    pub task: Option<bool>,
    pub children: Vec<Block>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    None,
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

/// One inline segment inside a block's inline range, as the renderer sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    /// Text drawn as-is. `range` is the source bytes of exactly that text.
    Text {
        range: Range<usize>,
        style: InlineStyle,
    },
    /// Inline code; `range` covers the code text without backticks.
    /// `link` indexes `Document::links` when the code is link text (inside
    /// a link, or a code-path link).
    Code {
        range: Range<usize>,
        link: Option<usize>,
    },
    /// Soft line break (reflowed to a space).
    SoftBreak,
    /// Hard line break.
    HardBreak,
    /// Footnote reference `[^label]`; `range` is the label text.
    FootnoteRef { label: String, range: Range<usize> },
    /// Math: `$…$` (`display: false`) or `$$…$$` (`display: true`).
    Math {
        /// The LaTeX as CommonMark reads it: no delimiters, no container
        /// prefixes. Convert this, never a slice of the source.
        tex: String,
        display: bool,
        /// Where converted output maps: the LaTeX without delimiters or
        /// container prefixes. For math over several lines this is the
        /// first non-blank line (prefix-free), so it never covers `> `.
        range: Range<usize>,
        /// The whole construct, delimiters included, one range per source
        /// line with container prefixes skipped (shown when math is off).
        raw: Vec<Range<usize>>,
        /// Index into `Document::links` when the math is link text.
        link: Option<usize>,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InlineStyle {
    pub strong: bool,
    pub emphasis: bool,
    pub strikethrough: bool,
    /// Index into `Document::links` when this text is a link's visible text.
    pub link: Option<usize>,
}

/// Parse markdown. Never fails: invalid UTF-8 is handled by the caller
/// via [`from_bytes`].
pub fn parse(source: String) -> Document {
    let events = events(&source);
    let mut i = 0;
    let blocks = parse_blocks(&source, &events, &mut i);
    let headings = collect_headings(&events);
    let links = collect_links(&events);
    let code_spans = collect_code_spans(&source, &events);
    let front_matter = events.iter().find_map(|(event, range)| match event {
        Event::Start(Tag::MetadataBlock(_)) => Some(range.clone()),
        _ => None,
    });
    Document {
        source,
        blocks,
        headings,
        links,
        code_spans,
        front_matter,
        lossy: false,
    }
}

impl Document {
    /// Merge `extra` into `links`, keeping them sorted by source position.
    /// Link indices shift; call before rendering. [`inlines`] finds a
    /// link by its range, so nothing else needs rebuilding.
    pub fn add_links(&mut self, extra: Vec<Link>) {
        self.links.extend(extra);
        self.links.sort_by_key(|l| l.range.start);
    }
}

/// Decode bytes (lossy UTF-8) and parse. Returns `None` when the first
/// 8 KiB contain a NUL byte (treated as binary).
pub fn from_bytes(bytes: &[u8]) -> Option<Document> {
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return None;
    }
    let (text, lossy) = match String::from_utf8_lossy(bytes) {
        Cow::Borrowed(s) => (s.to_owned(), false),
        Cow::Owned(s) => (s, true),
    };
    let mut doc = parse(text);
    doc.lossy = lossy;
    Some(doc)
}

/// Walk the inline content of `inline` (a block's inline range) into
/// segments in source order.
pub fn inlines(doc: &Document, inline: Range<usize>) -> Vec<Inline> {
    let mut out = Vec::new();
    if inline.is_empty() {
        return out;
    }
    with_inline_events(&doc.source, |events| {
        let (mut strong, mut emphasis, mut strike) = (0u32, 0u32, 0u32);
        let mut link_stack: Vec<Option<usize>> = Vec::new();
        // Every event starting inside `inline` sits at or after this index
        // (`max_start` is a prefix maximum, so it is sorted).
        let first = events.partition_point(|e| e.max_start < inline.start);
        for e in &events[first..] {
            let range = e.range.clone();
            if !matches!(e.ev, Ev::End(_)) && range.start >= inline.end {
                break;
            }
            if range.start < inline.start || range.end > inline.end {
                continue;
            }
            let style = InlineStyle {
                strong: strong > 0,
                emphasis: emphasis > 0,
                strikethrough: strike > 0,
                link: link_stack.last().copied().flatten(),
            };
            match &e.ev {
                Ev::Start(Mark::Strong) => strong += 1,
                Ev::End(Mark::Strong) => strong = strong.saturating_sub(1),
                Ev::Start(Mark::Emphasis) => emphasis += 1,
                Ev::End(Mark::Emphasis) => emphasis = emphasis.saturating_sub(1),
                Ev::Start(Mark::Strike) => strike += 1,
                Ev::End(Mark::Strike) => strike = strike.saturating_sub(1),
                Ev::Start(Mark::Link) => link_stack.push(link_index(doc, &range)),
                Ev::End(Mark::Link) => {
                    link_stack.pop();
                }
                Ev::Text => push_lines(&mut out, &doc.source, range, |range| Inline::Text {
                    range,
                    style,
                }),
                Ev::Code => {
                    let link = style.link.or_else(|| link_index(doc, &range));
                    push_lines(
                        &mut out,
                        &doc.source,
                        code_content(&doc.source, range),
                        |range| Inline::Code { range, link },
                    )
                }
                Ev::SoftBreak => out.push(Inline::SoftBreak),
                Ev::HardBreak => out.push(Inline::HardBreak),
                Ev::Math { tex, display } => out.push(math_inline(
                    &doc.source,
                    range,
                    tex.clone(),
                    *display,
                    style.link,
                )),
                Ev::FootnoteRef(label) => out.push(Inline::FootnoteRef {
                    label: label.clone(),
                    range: range.start + 2..range.end - 1,
                }),
            }
        }
    });
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mark {
    Strong,
    Emphasis,
    Strike,
    Link,
}

/// Owned, inline-only view of a pulldown-cmark event.
#[derive(Debug, Clone)]
enum Ev {
    Start(Mark),
    End(Mark),
    Text,
    Code,
    SoftBreak,
    HardBreak,
    FootnoteRef(String),
    Math { tex: String, display: bool },
}

#[derive(Debug, Clone)]
struct InlineEvent {
    ev: Ev,
    range: Range<usize>,
    /// Maximum `range.start` over this and all earlier events.
    max_start: usize,
}

thread_local! {
    /// Inline events of the last source seen by [`inlines`]. A renderer
    /// calls `inlines` once per block; caching avoids a full re-parse per
    /// call (never parse a substring: reference links and container
    /// prefixes need the whole document).
    static INLINE_CACHE: RefCell<Option<(String, Vec<InlineEvent>)>> =
        const { RefCell::new(None) };
}

fn with_inline_events(src: &str, f: impl FnOnce(&[InlineEvent])) {
    INLINE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if !matches!(&*cache, Some((s, _)) if s == src) {
            *cache = Some((src.to_owned(), inline_events(src)));
        }
        if let Some((_, events)) = &*cache {
            f(events);
        }
    });
}

fn inline_events(src: &str) -> Vec<InlineEvent> {
    let mut max_start = 0;
    Parser::new_ext(src, options())
        .into_offset_iter()
        .filter_map(|(event, range)| {
            let ev = match event {
                Event::Start(Tag::Strong) => Ev::Start(Mark::Strong),
                Event::End(TagEnd::Strong) => Ev::End(Mark::Strong),
                Event::Start(Tag::Emphasis) => Ev::Start(Mark::Emphasis),
                Event::End(TagEnd::Emphasis) => Ev::End(Mark::Emphasis),
                Event::Start(Tag::Strikethrough) => Ev::Start(Mark::Strike),
                Event::End(TagEnd::Strikethrough) => Ev::End(Mark::Strike),
                Event::Start(Tag::Link { .. }) => Ev::Start(Mark::Link),
                Event::End(TagEnd::Link) => Ev::End(Mark::Link),
                // Inline HTML is shown literally.
                Event::Text(_) | Event::InlineHtml(_) => Ev::Text,
                Event::InlineMath(tex) => Ev::Math {
                    tex: tex.to_string(),
                    display: false,
                },
                Event::DisplayMath(tex) => Ev::Math {
                    tex: tex.to_string(),
                    display: true,
                },
                Event::Code(_) => Ev::Code,
                Event::SoftBreak => Ev::SoftBreak,
                Event::HardBreak => Ev::HardBreak,
                Event::FootnoteReference(label) => Ev::FootnoteRef(label.to_string()),
                _ => return None,
            };
            max_start = max_start.max(range.start);
            Some(InlineEvent {
                ev,
                range,
                max_start,
            })
        })
        .collect()
}

type Events<'a> = [(Event<'a>, Range<usize>)];

fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM
        | Options::ENABLE_WIKILINKS
        | Options::ENABLE_HEADING_ATTRIBUTES
        | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
        | Options::ENABLE_MATH
}

fn events(src: &str) -> Vec<(Event<'_>, Range<usize>)> {
    Parser::new_ext(src, options()).into_offset_iter().collect()
}

fn link_index(doc: &Document, range: &Range<usize>) -> Option<usize> {
    let from = doc.links.partition_point(|l| l.range.start < range.start);
    doc.links[from..]
        .iter()
        .take_while(|l| l.range.start == range.start)
        .position(|l| l.range == *range)
        .map(|p| from + p)
}

/// Push `range` as one segment per source line, joined by soft breaks.
/// Continuation lines skip their container prefix (`> ` of block quotes,
/// list-item indentation), so no segment ever covers prefix bytes. A
/// paragraph continuation line cannot start with `>` (that would open a
/// block quote), so skipping leading whitespace and `>` is exact except
/// for a lazy line indented 4+ spaces whose text starts with `>`.
fn push_lines(
    out: &mut Vec<Inline>,
    src: &str,
    range: Range<usize>,
    make: impl Fn(Range<usize>) -> Inline,
) {
    let mut start = range.start;
    let mut first = true;
    loop {
        if !first {
            out.push(Inline::SoftBreak);
            let rest = &src[start..range.end];
            start += rest.len() - rest.trim_start_matches([' ', '\t', '>']).len();
        }
        first = false;
        let nl = src[start..range.end].find('\n').map(|n| start + n);
        let mut end = nl.unwrap_or(range.end);
        if src[start..end].ends_with('\r') {
            end -= 1;
        }
        if start < end || (nl.is_none() && start == range.start) {
            out.push(make(start..end));
        }
        match nl {
            Some(nl) => start = nl + 1,
            None => break,
        }
    }
}

/// The language of a fence info string; see [`Block::CodeBlock`].
fn fence_lang(info: &str) -> Option<String> {
    let first = info.trim_start().trim_start_matches('{').trim_start();
    let end = first
        .find(|c: char| c == ',' || c == '}' || c.is_whitespace())
        .unwrap_or(first.len());
    let lang = first[..end].trim_start_matches('.');
    (!lang.is_empty()).then(|| lang.to_lowercase())
}

/// An [`Inline::Math`] for the math event at `range` (delimiters included).
fn math_inline(
    src: &str,
    range: Range<usize>,
    tex: String,
    display: bool,
    link: Option<usize>,
) -> Inline {
    let pieces = |r: Range<usize>| {
        let mut out = Vec::new();
        push_lines(&mut out, src, r, |range| Inline::Text {
            range,
            style: InlineStyle::default(),
        });
        out.into_iter()
            .filter_map(|i| match i {
                Inline::Text { range, .. } => Some(range),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    // A math event spans both delimiters.
    let delim = if display { 2 } else { 1 };
    let start = (range.start + delim).min(range.end);
    let inner = start..range.end.saturating_sub(delim).max(start);
    let first = pieces(inner.clone())
        .into_iter()
        .find_map(|r| {
            let s = &src[r.clone()];
            let t = s.trim_start();
            let start = r.start + s.len() - t.len();
            let end = start + t.trim_end().len();
            (start < end).then_some(start..end)
        })
        .unwrap_or(inner.start..inner.start);
    Inline::Math {
        tex,
        display,
        range: first,
        raw: pieces(range),
        link,
    }
}

/// Inline code span without the backtick fences and the single padding
/// space CommonMark strips.
fn code_content(src: &str, range: Range<usize>) -> Range<usize> {
    let s = &src[range.clone()];
    let ticks = s.len() - s.trim_start_matches('`').len();
    let (mut start, mut end) = (range.start + ticks, range.end - ticks);
    if start > end {
        return range.start..range.start;
    }
    let inner = &src[start..end];
    let pad = |c: char| c == ' ' || c == '\n';
    if inner.len() >= 2 && inner.starts_with(pad) && inner.ends_with(pad) && !inner.chars().all(pad)
    {
        start += 1;
        end -= 1;
    }
    start..end
}

fn is_inline_event(event: &Event<'_>) -> bool {
    match event {
        Event::Start(tag) => matches!(
            tag,
            Tag::Emphasis
                | Tag::Strong
                | Tag::Strikethrough
                | Tag::Superscript
                | Tag::Subscript
                | Tag::Link { .. }
                | Tag::Image { .. }
        ),
        Event::End(tag) => matches!(
            tag,
            TagEnd::Emphasis
                | TagEnd::Strong
                | TagEnd::Strikethrough
                | TagEnd::Superscript
                | TagEnd::Subscript
                | TagEnd::Link
                | TagEnd::Image
        ),
        Event::Text(_)
        | Event::Code(_)
        | Event::InlineMath(_)
        | Event::DisplayMath(_)
        | Event::InlineHtml(_)
        | Event::FootnoteReference(_)
        | Event::SoftBreak
        | Event::HardBreak => true,
        Event::Html(_) | Event::Rule | Event::TaskListMarker(_) => false,
    }
}

/// Consume inline events from `i` and return their source span. Stops at
/// the first block-level event (or an `End` of an enclosing block).
fn inline_run(events: &Events<'_>, i: &mut usize, at: usize) -> Range<usize> {
    let mut span: Option<Range<usize>> = None;
    while let Some((event, range)) = events.get(*i) {
        if !is_inline_event(event) {
            break;
        }
        span = Some(match span {
            None => range.clone(),
            Some(s) => s.start.min(range.start)..s.end.max(range.end),
        });
        *i += 1;
    }
    span.unwrap_or(at..at)
}

/// Skip to just past the `End` matching the `Start` at `events[*i - 1]`.
fn skip_to_end(events: &Events<'_>, i: &mut usize) {
    let mut depth = 1usize;
    while let Some((event, _)) = events.get(*i) {
        *i += 1;
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) => {
                depth -= 1;
                if depth == 0 {
                    return;
                }
            }
            _ => {}
        }
    }
}

/// Parse sibling blocks until the `End` of the enclosing container
/// (consumed) or the end of input.
fn parse_blocks(src: &str, events: &Events<'_>, i: &mut usize) -> Vec<Block> {
    let mut blocks = Vec::new();
    while let Some((event, range)) = events.get(*i) {
        let range = range.clone();
        if is_inline_event(event) {
            // Tight list items carry inline content without a Paragraph.
            let inline = inline_run(events, i, range.start);
            blocks.push(Block::Paragraph {
                range: inline.clone(),
                inline,
            });
            continue;
        }
        *i += 1;
        let tag = match event {
            Event::End(_) => return blocks,
            Event::Rule => {
                blocks.push(Block::Rule { range });
                continue;
            }
            Event::Html(_) => {
                blocks.push(Block::Html { range });
                continue;
            }
            Event::Start(tag) => tag,
            _ => continue, // TaskListMarker: read by the enclosing item.
        };
        match tag {
            Tag::Paragraph => {
                let inline = inline_run(events, i, range.start);
                skip_to_end(events, i);
                blocks.push(Block::Paragraph { range, inline });
            }
            Tag::Heading { level, .. } => {
                let inline = inline_run(events, i, range.start);
                skip_to_end(events, i);
                blocks.push(Block::Heading {
                    level: *level as u8,
                    range,
                    inline,
                });
            }
            Tag::CodeBlock(kind) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => fence_lang(info),
                    CodeBlockKind::Indented => None,
                };
                let mut code: Option<Range<usize>> = None;
                let mut lines = Vec::new();
                while let Some((Event::Text(_), r)) = events.get(*i) {
                    lines.push(r.clone());
                    code = Some(match code {
                        None => r.clone(),
                        Some(c) => c.start..r.end,
                    });
                    *i += 1;
                }
                skip_to_end(events, i);
                let code = code.unwrap_or_else(|| {
                    let at = match kind {
                        CodeBlockKind::Fenced(_) => src[range.clone()]
                            .find('\n')
                            .map_or(range.end, |n| range.start + n + 1),
                        CodeBlockKind::Indented => range.start,
                    };
                    at..at
                });
                blocks.push(Block::CodeBlock {
                    lang,
                    range,
                    code,
                    lines,
                });
            }
            Tag::BlockQuote(kind) => {
                let alert = kind.map(|k| match k {
                    BlockQuoteKind::Note => AlertKind::Note,
                    BlockQuoteKind::Tip => AlertKind::Tip,
                    BlockQuoteKind::Important => AlertKind::Important,
                    BlockQuoteKind::Warning => AlertKind::Warning,
                    BlockQuoteKind::Caution => AlertKind::Caution,
                });
                let children = parse_blocks(src, events, i);
                blocks.push(Block::BlockQuote {
                    range,
                    alert,
                    children,
                });
            }
            Tag::List(start) => {
                let start = *start;
                let mut items = Vec::new();
                while let Some((event, item_range)) = events.get(*i) {
                    *i += 1;
                    match event {
                        Event::Start(Tag::Item) => {
                            let task = task_marker(events, *i);
                            let children = parse_blocks(src, events, i);
                            items.push(ListItem {
                                range: item_range.clone(),
                                task,
                                children,
                            });
                        }
                        Event::End(_) => break,
                        _ => {}
                    }
                }
                blocks.push(Block::List {
                    range,
                    start,
                    items,
                });
            }
            Tag::Table(aligns) => {
                let alignments = aligns
                    .iter()
                    .map(|a| match a {
                        pulldown_cmark::Alignment::None => Alignment::None,
                        pulldown_cmark::Alignment::Left => Alignment::Left,
                        pulldown_cmark::Alignment::Center => Alignment::Center,
                        pulldown_cmark::Alignment::Right => Alignment::Right,
                    })
                    .collect();
                let mut header = Vec::new();
                let mut rows: Vec<Vec<Range<usize>>> = Vec::new();
                let mut in_head = false;
                while let Some((event, r)) = events.get(*i) {
                    *i += 1;
                    match event {
                        Event::Start(Tag::TableHead) => in_head = true,
                        Event::End(TagEnd::TableHead) => in_head = false,
                        Event::Start(Tag::TableRow) => rows.push(Vec::new()),
                        Event::Start(Tag::TableCell) => {
                            if in_head {
                                header.push(r.clone());
                            } else if let Some(row) = rows.last_mut() {
                                row.push(r.clone());
                            }
                            skip_to_end(events, i);
                        }
                        Event::End(TagEnd::Table) => break,
                        _ => {}
                    }
                }
                blocks.push(Block::Table {
                    range,
                    alignments,
                    header,
                    rows,
                });
            }
            Tag::HtmlBlock => {
                skip_to_end(events, i);
                blocks.push(Block::Html { range });
            }
            Tag::FootnoteDefinition(label) => {
                let label = label.to_string();
                let children = parse_blocks(src, events, i);
                blocks.push(Block::FootnoteDefinition {
                    label,
                    range,
                    children,
                });
            }
            // Front matter (metadata, recorded in `Document::front_matter`),
            // not enabled (definition lists) or unexpected: keep the walk
            // balanced.
            _ => skip_to_end(events, i),
        }
    }
    blocks
}

/// Task marker of the item whose children start at `i`: directly inside a
/// tight item, or as the first event of a loose item's first paragraph.
fn task_marker(events: &Events<'_>, i: usize) -> Option<bool> {
    let at = |k: usize| match events.get(k) {
        Some((Event::TaskListMarker(checked), _)) => Some(*checked),
        _ => None,
    };
    at(i).or_else(|| match events.get(i) {
        Some((Event::Start(Tag::Paragraph), _)) => at(i + 1),
        _ => None,
    })
}

fn collect_links(events: &Events<'_>) -> Vec<Link> {
    let mut links = Vec::new();
    // (index into links, visible text span so far)
    let mut open: Vec<(usize, Option<Range<usize>>)> = Vec::new();
    for (event, range) in events {
        match event {
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                ..
            }) => {
                let kind = if matches!(link_type, LinkType::WikiLink { .. }) {
                    LinkKind::Wiki
                } else {
                    LinkKind::Markdown
                };
                open.push((links.len(), None));
                links.push(Link {
                    kind,
                    dest: dest_url.to_string(),
                    range: range.clone(),
                    text_range: range.start..range.start,
                    resolved: None,
                    line: None,
                });
            }
            Event::End(TagEnd::Link) => {
                if let Some((idx, span)) = open.pop() {
                    let link = &mut links[idx];
                    link.text_range = span.unwrap_or_else(|| {
                        // `[](x)`: empty text just inside the bracket(s).
                        let at = (link.range.start + 1).min(link.range.end);
                        at..at
                    });
                }
            }
            Event::Text(_)
            | Event::Code(_)
            | Event::InlineHtml(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_) => {
                for (_, span) in &mut open {
                    *span = Some(match span.take() {
                        None => range.clone(),
                        Some(s) => s.start.min(range.start)..s.end.max(range.end),
                    });
                }
            }
            _ => {}
        }
    }
    links
}

fn collect_code_spans(src: &str, events: &Events<'_>) -> Vec<CodeSpan> {
    let mut spans = Vec::new();
    // Open links and images: code inside them is not a candidate.
    let mut depth = 0usize;
    for (event, range) in events {
        match event {
            Event::Start(Tag::Link { .. } | Tag::Image { .. }) => depth += 1,
            Event::End(TagEnd::Link | TagEnd::Image) => depth = depth.saturating_sub(1),
            Event::Code(text) if depth == 0 => spans.push(CodeSpan {
                range: range.clone(),
                text_range: code_content(src, range.clone()),
                text: text.to_string(),
            }),
            _ => {}
        }
    }
    spans
}

fn collect_headings(events: &Events<'_>) -> Vec<Heading> {
    let mut headings = Vec::new();
    let mut used: HashSet<String> = HashSet::new();
    let mut current: Option<(u8, Option<String>, String, Range<usize>)> = None;
    for (event, range) in events {
        match event {
            Event::Start(Tag::Heading { level, id, .. }) => {
                current = Some((
                    *level as u8,
                    id.as_ref().map(|s| s.to_string()),
                    String::new(),
                    range.clone(),
                ));
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((level, id, text, range)) = current.take() {
                    let slug = match id {
                        Some(id) => id,
                        None => unique_slug(&slugify(&text), &used),
                    };
                    used.insert(slug.clone());
                    headings.push(Heading {
                        level,
                        text,
                        slug,
                        range,
                    });
                }
            }
            Event::Text(t) | Event::Code(t) | Event::InlineMath(t) => {
                if let Some((_, _, text, _)) = &mut current {
                    text.push_str(t);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some((_, _, text, _)) = &mut current {
                    text.push(' ');
                }
            }
            _ => {}
        }
    }
    headings
}

/// Unicode combining mark (general category Mn, Mc or Me). github-slugger
/// keeps these, e.g. the Devanagari virama or a decomposed accent.
fn is_mark(c: char) -> bool {
    matches!(
        get_general_category(c),
        GeneralCategory::NonspacingMark
            | GeneralCategory::SpacingMark
            | GeneralCategory::EnclosingMark
    )
}

/// GitHub heading slug: lowercase, keep alphanumerics, combining marks,
/// `-` and `_`, spaces to `-`, drop everything else.
fn slugify(text: &str) -> String {
    text.trim()
        .chars()
        .flat_map(char::to_lowercase)
        .filter_map(|c| match c {
            ' ' => Some('-'),
            '-' | '_' => Some(c),
            c if c.is_alphanumeric() || is_mark(c) => Some(c),
            _ => None,
        })
        .collect()
}

fn unique_slug(base: &str, used: &HashSet<String>) -> String {
    if !used.contains(base) {
        return base.to_owned();
    }
    (1..)
        .map(|n| format!("{base}-{n}"))
        .find(|s| !used.contains(s))
        .expect("unbounded counter")
}
