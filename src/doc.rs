//! Markdown parsing into a [`Document`] with byte ranges for blocks,
//! headings and links. No rendering.
//!
//! Interface fixed by the orchestrator before parallel units start; the
//! `doc` unit (t02) implements [`parse`] and may add private helpers, but
//! must keep these public names and field types.

use std::ops::Range;

/// A parsed markdown document. `source` is the full input text; every
/// range below is a byte range into it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub source: String,
    /// Top-level and nested blocks in document order.
    pub blocks: Vec<Block>,
    /// Every heading, in order, with its GitHub-style slug.
    pub headings: Vec<Heading>,
    /// Every link (inline, reference, autolink, wikilink), in order.
    pub links: Vec<Link>,
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
    /// `[text](dest)`, `[text][ref]`, `<https://...>`, bare autolinks.
    Markdown,
    /// `[[target]]` or `[[target|label]]`.
    Wiki,
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
    /// For a wikilink without a label this is the target text.
    pub text_range: Range<usize>,
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
    /// `lang` is the fence info string's first word, if any.
    CodeBlock {
        lang: Option<String>,
        range: Range<usize>,
        code: Range<usize>,
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
    Code { range: Range<usize> },
    /// Soft line break (reflowed to a space).
    SoftBreak,
    /// Hard line break.
    HardBreak,
    /// Footnote reference `[^label]`; `range` is the label text.
    FootnoteRef { label: String, range: Range<usize> },
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
    let _ = source;
    todo!("t02_doc")
}

/// Decode bytes (lossy UTF-8) and parse. Returns `None` when the first
/// 8 KiB contain a NUL byte (treated as binary).
pub fn from_bytes(bytes: &[u8]) -> Option<Document> {
    let _ = bytes;
    todo!("t02_doc")
}

/// Walk the inline content of `inline` (a block's inline range) into
/// segments in source order.
pub fn inlines(doc: &Document, inline: Range<usize>) -> Vec<Inline> {
    let _ = (doc, inline);
    todo!("t02_doc")
}
