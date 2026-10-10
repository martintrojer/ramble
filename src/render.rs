//! Layout of a [`Document`](crate::doc::Document) for one width into
//! styled lines plus a source map.

use std::ops::Range;
use std::sync::OnceLock;

use mdroots::syntax::{Field, Frontmatter};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::FontStyle;
use syntect::parsing::{SyntaxReference, SyntaxSet};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::doc::{self, AlertKind, Alignment, Block, Document, Inline, ListItem};

/// Colours and styles. `Theme::catppuccin_mocha()` is the default.
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    /// syntect theme name inside two-face's embedded set.
    pub code_theme: String,
    /// Convert LaTeX math to Unicode (`[render] math`). Off: math shows
    /// as its raw source, delimiters included, in the math colour.
    pub math: bool,
}

impl Theme {
    pub fn catppuccin_mocha() -> Self {
        Self {
            name: "catppuccin-mocha".into(),
            code_theme: "catppuccin-mocha".into(),
            math: true,
        }
    }
}

/// A screen span: one drawn row and a column range on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScreenSpan {
    pub row: usize,
    pub col_start: usize,
    /// Exclusive.
    pub col_end: usize,
}

/// One drawn inline segment: the source bytes it came from and where it
/// was drawn. Segments never overlap on screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub src: Range<usize>,
    pub span: ScreenSpan,
    /// Index into `Document::links` when the segment is link text.
    pub link: Option<usize>,
}

/// Two-way map between source byte ranges and screen spans.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SrcMap {
    /// Sorted by (row, col_start).
    pub segments: Vec<Segment>,
}

impl SrcMap {
    /// Screen spans drawn from any byte in `src`. Empty when none of
    /// those bytes is drawn.
    pub fn spans_for(&self, src: Range<usize>) -> Vec<ScreenSpan> {
        self.segments
            .iter()
            .filter(|s| {
                if src.is_empty() {
                    s.src.contains(&src.start)
                } else {
                    s.src.start < src.end && src.start < s.src.end
                }
            })
            .map(|s| s.span)
            .collect()
    }

    /// The source byte drawn at (row, col), or the nearest drawn byte on
    /// that row, or `None` for an empty row. Inside a segment the byte is
    /// scaled from the column, so it can fall inside a multi-byte char:
    /// move it to a char boundary before slicing the source with it.
    pub fn source_at(&self, row: usize, col: usize) -> Option<usize> {
        let lo = self.segments.partition_point(|s| s.span.row < row);
        let hi = self.segments.partition_point(|s| s.span.row <= row);
        let on_row = &self.segments[lo..hi];
        if let Some(s) = on_row
            .iter()
            .find(|s| (s.span.col_start..s.span.col_end).contains(&col))
        {
            let cols = s.span.col_end - s.span.col_start;
            let len = s.src.end - s.src.start;
            let off = (col - s.span.col_start) * len / cols.max(1);
            return Some((s.src.start + off).min(s.src.end - 1));
        }
        on_row
            .iter()
            .min_by_key(|s| {
                if col < s.span.col_start {
                    s.span.col_start - col
                } else {
                    col + 1 - s.span.col_end
                }
            })
            .map(|s| {
                if col < s.span.col_start {
                    s.src.start
                } else {
                    s.src.end - 1
                }
            })
    }

    /// The first screen row drawn from a byte at or after `byte`.
    pub fn row_for(&self, byte: usize) -> Option<usize> {
        self.segments
            .iter()
            .find(|s| s.src.end > byte)
            .map(|s| s.span.row)
    }
}

#[derive(Debug, Clone)]
pub struct RenderedPage {
    pub lines: Vec<Line<'static>>,
    pub srcmap: SrcMap,
    /// For each rendered row, the 1-based source line it starts on (for
    /// `<leader>o` and review markers). Rows with no source (blank
    /// separators) repeat the previous value.
    pub source_lines: Vec<usize>,
    /// For each rendered row, whether it has a source anchor: drawn
    /// source text, or a fallback byte (blank code lines, empty raw
    /// lines). False for blank separators.
    pub anchored: Vec<bool>,
}

/// Lay out `doc` at `width` columns, front matter left out (print mode).
pub fn render(doc: &Document, width: u16, theme: &Theme) -> RenderedPage {
    render_with(doc, width, theme, &|r| doc::inlines(doc, r))
}

/// [`render`] for the normal view: front matter, if any, first, as a
/// folded marker row or (`expanded`) the marker and one row per entry.
pub fn render_page(doc: &Document, width: u16, theme: &Theme, expanded: bool) -> RenderedPage {
    let inlines = |r| doc::inlines(doc, r);
    let mut r = Renderer::new(doc, width, &inlines);
    r.math = theme.math;
    if let Some(fm) = &doc.fm {
        r.front_matter(fm, expanded);
    }
    r.blocks(&doc.blocks, true);
    r.finish()
}

/// Text of the folded front-matter marker row.
pub fn front_matter_marker(fm: &Frontmatter, expanded: bool) -> String {
    if expanded {
        return "▾ front matter".into();
    }
    match fm.fields().len() {
        _ if !fm.parsed() => "▸ front matter · unparsed".into(),
        1 => "▸ front matter · 1 key".into(),
        n => format!("▸ front matter · {n} keys"),
    }
}

/// Widest key column in the expanded front matter.
const FM_KEY_MAX: usize = 20;

/// [`render`] with the inline walker injected, so layout can be tested
/// with hand-built inlines.
#[doc(hidden)]
pub fn render_with(
    doc: &Document,
    width: u16,
    theme: &Theme,
    inlines: &dyn Fn(Range<usize>) -> Vec<Inline>,
) -> RenderedPage {
    // v1 always uses the embedded Catppuccin Mocha code theme.
    let mut r = Renderer::new(doc, width, inlines);
    r.math = theme.math;
    r.blocks(&doc.blocks, true);
    r.finish()
}

/// Lay out `doc`'s source as-is for the raw view: one source line per row,
/// soft-wrapped by grapheme at `width`, highlighted with syntect's
/// Markdown syntax. Segments carry the index of the `doc.links` entry
/// whose full range holds their bytes, so link actions work unchanged.
pub fn render_raw(doc: &Document, width: u16, theme: &Theme) -> RenderedPage {
    let _ = theme;
    let none = |_: Range<usize>| Vec::new();
    let mut r = Renderer::new(doc, width, &none);
    r.raw(&doc.links);
    RenderedPage {
        lines: r.lines,
        srcmap: SrcMap {
            segments: r.segments,
        },
        source_lines: r.source_lines,
        anchored: r.anchored,
    }
}

/// Write `page` as ANSI-coloured text (truecolor) with a trailing newline
/// per row. Used by print mode.
pub fn to_ansi(page: &RenderedPage) -> String {
    let mut out = String::new();
    for line in &page.lines {
        let mut styled = false;
        for span in &line.spans {
            let codes = sgr_codes(line.style.patch(span.style));
            if styled {
                out.push_str("\x1b[0m");
            }
            styled = !codes.is_empty();
            if styled {
                out.push_str("\x1b[");
                out.push_str(&codes.join(";"));
                out.push('m');
            }
            out.push_str(&span.content);
        }
        out.push_str("\x1b[0m\n");
    }
    out
}

fn sgr_codes(style: Style) -> Vec<String> {
    let mut codes = Vec::new();
    for (color, base) in [(style.fg, 38), (style.bg, 48)] {
        match color {
            Some(Color::Rgb(r, g, b)) => codes.push(format!("{base};2;{r};{g};{b}")),
            Some(Color::Indexed(i)) => codes.push(format!("{base};5;{i}")),
            _ => {}
        }
    }
    let m = style.add_modifier;
    for (flag, code) in [
        (Modifier::BOLD, "1"),
        (Modifier::DIM, "2"),
        (Modifier::ITALIC, "3"),
        (Modifier::UNDERLINED, "4"),
        (Modifier::CROSSED_OUT, "9"),
    ] {
        if m.contains(flag) {
            codes.push(code.to_string());
        }
    }
    codes
}

/// Catppuccin Mocha palette used by the renderer.
pub mod palette {
    use ratatui::style::Color;

    pub const TEXT: Color = Color::Rgb(0xcd, 0xd6, 0xf4);
    pub const MAUVE: Color = Color::Rgb(0xcb, 0xa6, 0xf7);
    pub const BLUE: Color = Color::Rgb(0x89, 0xb4, 0xfa);
    pub const GREEN: Color = Color::Rgb(0xa6, 0xe3, 0xa1);
    pub const OVERLAY: Color = Color::Rgb(0x6c, 0x70, 0x86);
    pub const RED: Color = Color::Rgb(0xf3, 0x8b, 0xa8);
    pub const YELLOW: Color = Color::Rgb(0xf9, 0xe2, 0xaf);
    pub const PEACH: Color = Color::Rgb(0xfa, 0xb3, 0x87);
    pub const TEAL: Color = Color::Rgb(0x94, 0xe2, 0xd5);
}

// ---------------------------------------------------------------------------
// Layout internals

/// One drawn grapheme.
#[derive(Debug, Clone)]
struct Cell {
    text: String,
    w: usize,
    style: Style,
    src: Option<Range<usize>>,
    link: Option<usize>,
}

impl Cell {
    fn deco(text: &str, style: Style) -> Self {
        Self {
            text: text.to_string(),
            w: UnicodeWidthStr::width(text),
            style,
            src: None,
            link: None,
        }
    }
}

fn deco_cells(text: &str, style: Style) -> Vec<Cell> {
    text.graphemes(true).map(|g| Cell::deco(g, style)).collect()
}

fn width_of(cells: &[Cell]) -> usize {
    cells.iter().map(|c| c.w).sum()
}

/// Drawn text and width of a grapheme. Control characters become `?`.
fn sanitize(g: &str) -> (String, usize) {
    if g.chars().any(char::is_control) {
        ("?".to_string(), 1)
    } else {
        (g.to_string(), UnicodeWidthStr::width(g))
    }
}

/// A word-wrap token.
enum Tok {
    Word(Vec<Cell>),
    /// Collapsible whitespace; `src` is the first whitespace grapheme, if any.
    Space(Option<Range<usize>>),
    Break,
}

/// Greedy word wrap of `toks` to `avail` columns. Words longer than
/// `avail` are broken by grapheme.
fn wrap(toks: Vec<Tok>, avail: usize, base: Style) -> Vec<Vec<Cell>> {
    let avail = avail.max(1);
    let mut rows = Vec::new();
    let mut row: Vec<Cell> = Vec::new();
    let mut rw = 0;
    let mut pending: Option<Option<Range<usize>>> = None;
    for tok in toks {
        match tok {
            Tok::Space(src) => {
                if !row.is_empty() {
                    pending = Some(src);
                }
            }
            Tok::Break => {
                rows.push(std::mem::take(&mut row));
                rw = 0;
                pending = None;
            }
            Tok::Word(cells) => {
                if cells.is_empty() {
                    continue;
                }
                let ww = width_of(&cells);
                let space = pending.take().filter(|_| !row.is_empty());
                let sw = usize::from(space.is_some());
                if rw + sw + ww <= avail {
                    if let Some(src) = space {
                        let prev = row.last().expect("row is non-empty");
                        let (style, link) = if prev.link.is_some() && prev.link == cells[0].link {
                            (prev.style, prev.link)
                        } else {
                            (base, None)
                        };
                        row.push(Cell {
                            text: " ".into(),
                            w: 1,
                            style,
                            src,
                            link,
                        });
                        rw += 1;
                    }
                    rw += ww;
                    row.extend(cells);
                } else if ww <= avail {
                    rows.push(std::mem::replace(&mut row, cells));
                    rw = ww;
                } else {
                    if !row.is_empty() {
                        rows.push(std::mem::take(&mut row));
                        rw = 0;
                    }
                    for c in cells {
                        if rw + c.w > avail && !row.is_empty() {
                            rows.push(std::mem::take(&mut row));
                            rw = 0;
                        }
                        rw += c.w;
                        row.push(c);
                    }
                }
            }
        }
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}

/// `cells` cut to `max` columns, ending in `…` when cut.
fn clip(cells: Vec<Cell>, max: usize) -> Vec<Cell> {
    if width_of(&cells) <= max {
        return cells;
    }
    let mut out = Vec::new();
    let mut w = 0;
    for c in cells {
        if w + c.w + 1 > max {
            break;
        }
        w += c.w;
        out.push(c);
    }
    while out.last().is_some_and(|c| c.text == " ") {
        out.pop();
    }
    if max > 0 {
        let style = out.last().map_or_else(base_style, |c| c.style);
        out.push(Cell::deco("…", style));
    }
    out
}

/// Split one row into rows no wider than `avail`, by grapheme.
fn hard_break(cells: Vec<Cell>, avail: usize) -> Vec<Vec<Cell>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut rw = 0;
    for c in cells {
        if rw + c.w > avail && !row.is_empty() {
            rows.push(std::mem::take(&mut row));
            rw = 0;
        }
        rw += c.w;
        row.push(c);
    }
    if !row.is_empty() || rows.is_empty() {
        rows.push(row);
    }
    rows
}

/// A per-row prefix (list marker, quote bar). `first` and `rest` have
/// the same width.
struct Prefix {
    first: Vec<Cell>,
    rest: Vec<Cell>,
    used: bool,
}

fn base_style() -> Style {
    Style::new().fg(palette::TEXT)
}

fn heading_style(level: u8) -> Style {
    let fg = match level {
        1 | 2 => palette::MAUVE,
        3 => palette::PEACH,
        4 => palette::YELLOW,
        5 => palette::GREEN,
        _ => palette::TEAL,
    };
    Style::new().fg(fg).add_modifier(Modifier::BOLD)
}

fn alert_info(kind: AlertKind) -> (&'static str, Color) {
    match kind {
        AlertKind::Note => ("Note", palette::BLUE),
        AlertKind::Tip => ("Tip", palette::GREEN),
        AlertKind::Important => ("Important", palette::MAUVE),
        AlertKind::Warning => ("Warning", palette::YELLOW),
        AlertKind::Caution => ("Caution", palette::RED),
    }
}

/// Fence names syntect doesn't know, mapped to tokens it does. `console`
/// and friends highlight every line as bash (prompts and output alike).
/// The `text` family is explicitly plain.
const LANG_ALIASES: &[(&str, &str)] = &[
    ("python3", "python"),
    ("shell", "bash"),
    ("console", "bash"),
    ("shellsession", "bash"),
    ("sh-session", "bash"),
    ("jsx", "tsx"),
    ("jsonc", "json"),
    ("json5", "json"),
    ("golang", "go"),
    ("lean4", "lean"),
    ("objc", "objective-c"),
    ("csharp", "c#"),
    ("containerfile", "dockerfile"),
];

const PLAIN_LANGS: &[&str] = &["text", "txt", "plain", "none"];

fn find_syntax(lang: &str) -> Option<&'static SyntaxReference> {
    let lang = lang.to_lowercase();
    if PLAIN_LANGS.contains(&lang.as_str()) {
        return None;
    }
    let ss = syntaxes();
    let token = LANG_ALIASES
        .iter()
        .find(|(alias, _)| *alias == lang)
        .map_or(lang.as_str(), |(_, target)| target);
    ss.find_syntax_by_token(token)
        .filter(|s| s.name != ss.find_syntax_plain_text().name)
}

/// The syntax name a code fence's `lang` highlights with, or None for plain
/// text (unknown names, and `text`/`txt`/`plain`/`none`).
pub fn resolve_syntax(lang: &str) -> Option<&'static str> {
    find_syntax(lang).map(|s| s.name.as_str())
}

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(two_face::syntax::extra_newlines)
}

fn code_theme() -> &'static syntect::highlighting::Theme {
    static THEME: OnceLock<syntect::highlighting::Theme> = OnceLock::new();
    THEME.get_or_init(|| {
        two_face::theme::extra()
            .get(two_face::theme::EmbeddedThemeName::CatppuccinMocha)
            .clone()
    })
}

fn syntect_style(st: syntect::highlighting::Style) -> Style {
    let fg = st.foreground;
    let mut style = Style::new().fg(Color::Rgb(fg.r, fg.g, fg.b));
    if st.font_style.contains(FontStyle::BOLD) {
        style = style.add_modifier(Modifier::BOLD);
    }
    if st.font_style.contains(FontStyle::ITALIC) {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if st.font_style.contains(FontStyle::UNDERLINE) {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    style
}

struct Renderer<'a> {
    src: &'a str,
    width: usize,
    inlines: &'a dyn Fn(Range<usize>) -> Vec<Inline>,
    newlines: Vec<usize>,
    prefixes: Vec<Prefix>,
    list_depth: usize,
    math: bool,
    lines: Vec<Line<'static>>,
    segments: Vec<Segment>,
    source_lines: Vec<usize>,
    anchored: Vec<bool>,
}

impl<'a> Renderer<'a> {
    fn new(
        doc: &'a Document,
        width: u16,
        inlines: &'a dyn Fn(Range<usize>) -> Vec<Inline>,
    ) -> Self {
        let src = doc.source.as_str();
        Self {
            src,
            width: usize::from(width.max(1)),
            inlines,
            newlines: src.match_indices('\n').map(|(i, _)| i).collect(),
            prefixes: Vec::new(),
            list_depth: 0,
            math: true,
            lines: Vec::new(),
            segments: Vec::new(),
            source_lines: Vec::new(),
            anchored: Vec::new(),
        }
    }

    fn finish(self) -> RenderedPage {
        RenderedPage {
            lines: self.lines,
            srcmap: SrcMap {
                segments: self.segments,
            },
            source_lines: self.source_lines,
            anchored: self.anchored,
        }
    }

    /// The front-matter block `fm`: the marker row, then when `expanded`
    /// one row per entry (or the raw lines when nothing was extracted),
    /// then a blank row. Every row maps to source
    /// bytes, so the cursor and yank work on them.
    fn front_matter(&mut self, fm: &Frontmatter, expanded: bool) {
        let dim = Style::new().fg(palette::OVERLAY);
        let range = &fm.range;
        let body = fm.inner();
        let text = self.slice(&body);
        let marker = front_matter_marker(fm, expanded);
        // Folded, the marker stands for the whole block; expanded, for
        // the opening fence, so entry bytes resolve to their own rows.
        let src = Some(if expanded {
            range.start..body.start.max(range.start + 1)
        } else {
            range.clone()
        });
        let cells = marker
            .graphemes(true)
            .map(|g| Cell {
                src: src.clone(),
                ..Cell::deco(g, dim)
            })
            .collect();
        self.emit(cells, Some(range.start));
        if expanded {
            if fm.fields().is_empty() {
                let mut at = body.start;
                for line in text.split_inclusive('\n') {
                    let r = at..at + line.trim_end_matches(['\n', '\r']).len();
                    at += line.len();
                    let cells = self.text_cells(&r, dim, None);
                    self.emit(cells, Some(r.start));
                }
            } else {
                self.front_matter_entries(fm.fields());
            }
        }
        self.blank();
    }

    /// One row per entry: the dim key padded to the widest (at most
    /// [`FM_KEY_MAX`] columns), two spaces, the value clipped with `…`.
    fn front_matter_entries(&mut self, fields: &[Field]) {
        let dim = Style::new().fg(palette::OVERLAY);
        let avail = self.avail();
        let key_w = fields
            .iter()
            .map(|e| UnicodeWidthStr::width(e.key.as_str()))
            .max()
            .unwrap_or(0)
            .min(FM_KEY_MAX)
            .min(avail.saturating_sub(3).max(1));
        for e in fields {
            let entry = e.range.clone();
            let line = self.slice(&entry);
            let key = self.mapped(&e.key, line, entry.start, &entry, dim, 0);
            let mut cells = clip(key, key_w);
            let pad = key_w.saturating_sub(width_of(&cells)) + 2;
            cells.extend((0..pad).map(|_| Cell::deco(" ", base_style())));
            let after = line
                .find(':')
                .or_else(|| line.find('='))
                .map_or(0, |i| i + 1);
            let value = e.value.display();
            let value = self.mapped(&value, line, entry.start, &entry, base_style(), after);
            let room = avail.saturating_sub(width_of(&cells));
            cells.extend(clip(value, room));
            self.emit_row(cells, Some(entry.start));
        }
    }

    /// Cells for `text`, each mapped to its own bytes when `text` appears
    /// verbatim in `line` (at `line_start`) at or after byte `from` of the
    /// line, else all to `whole`.
    fn mapped(
        &self,
        text: &str,
        line: &str,
        line_start: usize,
        whole: &Range<usize>,
        style: Style,
        from: usize,
    ) -> Vec<Cell> {
        let found = line
            .get(from..)
            .and_then(|l| l.find(text))
            .filter(|_| !text.is_empty());
        match found {
            Some(i) => {
                let start = line_start + from + i;
                self.text_cells(&(start..start + text.len()), style, None)
            }
            None => text
                .graphemes(true)
                .map(|g| {
                    let (t, w) = sanitize(g);
                    Cell {
                        text: t,
                        w,
                        style,
                        src: Some(whole.clone()),
                        link: None,
                    }
                })
                .collect(),
        }
    }

    /// `range` as text, or "" when it is not a valid slice.
    fn slice(&self, range: &Range<usize>) -> &'a str {
        self.src.get(range.clone()).unwrap_or("")
    }

    fn line_of(&self, byte: usize) -> usize {
        self.newlines.partition_point(|&n| n < byte) + 1
    }

    fn prefix_width(&self) -> usize {
        self.prefixes.iter().map(|p| width_of(&p.first)).sum()
    }

    /// Prefixes are dropped when they leave no more than one column.
    fn show_prefixes(&self) -> bool {
        self.width > self.prefix_width() + 1
    }

    /// Content columns available on a row.
    fn avail(&self) -> usize {
        if self.show_prefixes() {
            self.width - self.prefix_width()
        } else {
            self.width
        }
    }

    /// Emit one logical row, hard-breaking it when wider than `avail`.
    fn emit(&mut self, cells: Vec<Cell>, fallback: Option<usize>) {
        for row in hard_break(cells, self.avail()) {
            self.emit_row(row, fallback);
        }
    }

    fn emit_row(&mut self, content: Vec<Cell>, fallback: Option<usize>) {
        let show = self.show_prefixes();
        let mut cells = Vec::new();
        for p in &mut self.prefixes {
            if show {
                cells.extend(if p.used { &p.rest } else { &p.first }.iter().cloned());
            }
            p.used = true;
        }
        let byte = content
            .iter()
            .filter_map(|c| c.src.as_ref().map(|s| s.start))
            .min()
            .or(fallback);
        cells.extend(content);

        let row = self.lines.len();
        let mut col = 0;
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut text = String::new();
        let mut style = None;
        for c in cells {
            if style != Some(c.style) {
                if let Some(s) = style
                    && !text.is_empty()
                {
                    spans.push(Span::styled(std::mem::take(&mut text), s));
                }
                style = Some(c.style);
            }
            text.push_str(&c.text);
            if let Some(src) = c.src.filter(|s| !s.is_empty() && c.w > 0) {
                match self.segments.last_mut() {
                    Some(last)
                        if last.span.row == row
                            && last.span.col_end == col
                            && last.link == c.link
                            && src.start >= last.src.start
                            && src.start <= last.src.end =>
                    {
                        last.src.end = last.src.end.max(src.end);
                        last.span.col_end += c.w;
                    }
                    _ => self.segments.push(Segment {
                        src,
                        span: ScreenSpan {
                            row,
                            col_start: col,
                            col_end: col + c.w,
                        },
                        link: c.link,
                    }),
                }
            }
            col += c.w;
        }
        if let Some(s) = style
            && !text.is_empty()
        {
            spans.push(Span::styled(text, s));
        }
        self.lines.push(Line::from(spans));
        let prev = self.source_lines.last().copied().unwrap_or(1);
        let line = byte.map_or(prev, |b| self.line_of(b).max(prev));
        self.source_lines.push(line);
        self.anchored.push(byte.is_some());
    }

    fn blank(&mut self) {
        self.emit_row(Vec::new(), None);
    }

    fn with_prefix(&mut self, first: Vec<Cell>, rest: Vec<Cell>, f: impl FnOnce(&mut Self)) {
        self.prefixes.push(Prefix {
            first,
            rest,
            used: false,
        });
        f(self);
        self.prefixes.pop();
    }

    /// Cells for the source text in `range`, one per grapheme.
    fn text_cells(&self, range: &Range<usize>, style: Style, link: Option<usize>) -> Vec<Cell> {
        let mut cells = Vec::new();
        for (i, g) in self.slice(range).grapheme_indices(true) {
            let b = range.start + i;
            self.push_grapheme(&mut cells, g, b, style, link);
        }
        cells
    }

    fn push_grapheme(
        &self,
        cells: &mut Vec<Cell>,
        g: &str,
        b: usize,
        style: Style,
        link: Option<usize>,
    ) {
        let src = Some(b..b + g.len());
        if g == "\t" {
            for _ in 0..4 {
                cells.push(Cell {
                    text: " ".into(),
                    w: 1,
                    style,
                    src: src.clone(),
                    link,
                });
            }
            return;
        }
        let (text, w) = sanitize(g);
        if w > 0 {
            cells.push(Cell {
                text,
                w,
                style,
                src,
                link,
            });
        }
    }

    /// Turn inline content into wrap tokens.
    fn tokens(&self, inlines: Vec<Inline>, base: Style) -> Vec<Tok> {
        let mut toks = Vec::new();
        let mut word: Vec<Cell> = Vec::new();
        let flush = |word: &mut Vec<Cell>, toks: &mut Vec<Tok>| {
            if !word.is_empty() {
                toks.push(Tok::Word(std::mem::take(word)));
            }
        };
        let space = |toks: &mut Vec<Tok>, src: Option<Range<usize>>| {
            if !matches!(toks.last(), Some(Tok::Space(_))) {
                toks.push(Tok::Space(src));
            }
        };
        for inline in inlines {
            let (range, style, link) = match inline.clone() {
                Inline::Text { range, style: s } => {
                    let mut style = base;
                    if s.strong {
                        style = style.add_modifier(Modifier::BOLD);
                    }
                    if s.emphasis {
                        style = style.add_modifier(Modifier::ITALIC);
                    }
                    if s.strikethrough {
                        style = style.add_modifier(Modifier::CROSSED_OUT);
                    }
                    if s.link.is_some() {
                        style = style.fg(palette::BLUE).add_modifier(Modifier::UNDERLINED);
                    }
                    (range, style, s.link)
                }
                Inline::Code { range, link } => {
                    let mut style = base.fg(palette::GREEN);
                    if link.is_some() {
                        // Code colour stays; the link colour goes on the underline.
                        style = style
                            .add_modifier(Modifier::UNDERLINED)
                            .underline_color(palette::BLUE);
                    }
                    (range, style, link)
                }
                Inline::SoftBreak => {
                    flush(&mut word, &mut toks);
                    space(&mut toks, None);
                    continue;
                }
                Inline::HardBreak => {
                    flush(&mut word, &mut toks);
                    toks.push(Tok::Break);
                    continue;
                }
                Inline::Math { .. } => {
                    for tok in self.math_tokens(&inline, base) {
                        match tok {
                            Tok::Word(cells) => word.extend(cells),
                            Tok::Space(src) => {
                                flush(&mut word, &mut toks);
                                space(&mut toks, src);
                            }
                            Tok::Break => {
                                flush(&mut word, &mut toks);
                                toks.push(Tok::Break);
                            }
                        }
                    }
                    continue;
                }
                Inline::FootnoteRef { label, range } => {
                    let style = Style::new().fg(palette::PEACH);
                    word.push(Cell::deco("[", style));
                    let cells = self.text_cells(&range, style, None);
                    if cells.is_empty() {
                        word.extend(deco_cells(&label, style));
                    } else {
                        word.extend(cells);
                    }
                    word.push(Cell::deco("]", style));
                    continue;
                }
            };
            for (i, g) in self.slice(&range).grapheme_indices(true) {
                let b = range.start + i;
                if g.chars().all(char::is_whitespace) {
                    flush(&mut word, &mut toks);
                    space(&mut toks, Some(b..b + g.len()));
                } else {
                    self.push_grapheme(&mut word, g, b, style, link);
                }
            }
        }
        flush(&mut word, &mut toks);
        toks
    }

    fn wrapped_inline(&self, range: &Range<usize>, base: Style, avail: usize) -> Vec<Vec<Cell>> {
        let inlines = (self.inlines)(range.clone());
        wrap(self.tokens(inlines, base), avail, base)
    }

    fn blocks(&mut self, blocks: &[Block], separate: bool) {
        for (i, block) in blocks.iter().enumerate() {
            if i > 0 && separate {
                self.blank();
            }
            self.block(block);
        }
    }

    fn block(&mut self, block: &Block) {
        match block {
            Block::Heading {
                level,
                range,
                inline,
            } => {
                let style = heading_style(*level);
                let rows = self.wrapped_inline(inline, style, self.avail());
                if rows.is_empty() {
                    self.emit(Vec::new(), Some(range.start));
                }
                for row in rows {
                    self.emit(row, Some(range.start));
                }
                if *level <= 2 {
                    let ch = if *level == 1 { "━" } else { "─" };
                    let rule = ch.repeat(self.avail());
                    let style = Style::new().fg(if *level == 1 {
                        palette::MAUVE
                    } else {
                        palette::OVERLAY
                    });
                    self.emit(deco_cells(&rule, style), Some(range.start));
                }
            }
            Block::Paragraph { range, inline } => {
                if self.standalone_math(range, inline) {
                    return;
                }
                for row in self.wrapped_inline(inline, base_style(), self.avail()) {
                    self.emit(row, Some(range.start));
                }
            }
            Block::CodeBlock {
                lang, code, lines, ..
            } if self.math && lang.as_deref() == Some("math") => self.math_fence(code, lines),
            Block::CodeBlock { lang, code, .. } => self.code_block(lang.as_deref(), code),
            Block::BlockQuote {
                range,
                alert,
                children,
            } => {
                let color = alert.map_or(palette::OVERLAY, |k| alert_info(k).1);
                let bar = deco_cells("│ ", Style::new().fg(color));
                self.with_prefix(bar.clone(), bar, |r| {
                    if let Some(kind) = alert {
                        let (label, color) = alert_info(*kind);
                        let style = Style::new().fg(color).add_modifier(Modifier::BOLD);
                        r.emit(deco_cells(label, style), Some(range.start));
                        if !children.is_empty() {
                            r.blank();
                        }
                    }
                    r.blocks(children, true);
                });
            }
            Block::List { start, items, .. } => self.list(*start, items),
            Block::Table {
                range,
                alignments,
                header,
                rows,
            } => self.table(range, alignments, header, rows),
            Block::Rule { range } => {
                let rule = "─".repeat(self.avail());
                self.emit(
                    deco_cells(&rule, Style::new().fg(palette::OVERLAY)),
                    Some(range.start),
                );
            }
            Block::Html { range } => {
                let style = Style::new().fg(palette::OVERLAY);
                let text = self.slice(range);
                let mut offset = range.start;
                for line in text.split_inclusive('\n') {
                    let body = line.trim_end_matches(['\n', '\r']);
                    let cells = self.text_cells(&(offset..offset + body.len()), style, None);
                    self.emit(cells, Some(offset));
                    offset += line.len();
                }
            }
            Block::FootnoteDefinition {
                label,
                range,
                children,
            } => {
                let style = Style::new().fg(palette::PEACH);
                let tag = format!("[{label}]");
                if UnicodeWidthStr::width(tag.as_str()) <= 6 {
                    let first = deco_cells(&format!("{tag} "), style);
                    let rest = deco_cells(&" ".repeat(width_of(&first)), style);
                    self.with_prefix(first, rest, |r| {
                        if children.is_empty() {
                            r.emit(Vec::new(), Some(range.start));
                        }
                        r.blocks(children, true);
                    });
                } else {
                    self.emit(deco_cells(&tag, style), Some(range.start));
                    let pad = deco_cells("    ", style);
                    self.with_prefix(pad.clone(), pad, |r| r.blocks(children, true));
                }
            }
        }
    }

    fn list(&mut self, start: Option<u64>, items: &[ListItem]) {
        self.list_depth += 1;
        let bullet = ["•", "◦", "▪"][(self.list_depth - 1) % 3];
        let numbers: Vec<String> = match start {
            Some(n) => (0..items.len() as u64)
                .map(|i| format!("{}.", n.saturating_add(i)))
                .collect(),
            None => vec![bullet.to_string(); items.len()],
        };
        let marker_w = numbers
            .iter()
            .map(|s| UnicodeWidthStr::width(s.as_str()))
            .max()
            .unwrap_or(1);
        for (item, number) in items.iter().zip(numbers) {
            let mstyle = Style::new().fg(palette::BLUE);
            let pad = marker_w - UnicodeWidthStr::width(number.as_str());
            let mut first = deco_cells(&format!("{}{number} ", " ".repeat(pad)), mstyle);
            match item.task {
                Some(true) => first.extend(deco_cells("[x] ", Style::new().fg(palette::GREEN))),
                Some(false) => first.extend(deco_cells("[ ] ", Style::new().fg(palette::OVERLAY))),
                None => {}
            }
            let rest = deco_cells(&" ".repeat(width_of(&first)), mstyle);
            self.with_prefix(first, rest, |r| {
                if item.children.is_empty() {
                    r.emit(Vec::new(), Some(item.range.start));
                }
                r.blocks(&item.children, false);
            });
        }
        self.list_depth -= 1;
    }

    fn code_block(&mut self, lang: Option<&str>, code: &Range<usize>) {
        let ss = syntaxes();
        let syntax = lang
            .and_then(find_syntax)
            .unwrap_or_else(|| ss.find_syntax_plain_text());
        let mut hl = HighlightLines::new(syntax, code_theme());
        let fallback = Style::new().fg(palette::GREEN);
        let mut offset = code.start;
        for line in self.slice(code).split_inclusive('\n') {
            let mut cells = Vec::new();
            let pieces = hl
                .highlight_line(line, ss)
                .map(|v| {
                    v.into_iter()
                        .map(|(st, s)| (syntect_style(st), s))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|_| vec![(fallback, line)]);
            let mut b = offset;
            for (style, piece) in pieces {
                for (i, g) in piece.grapheme_indices(true) {
                    if !matches!(g, "\n" | "\r\n" | "\r") {
                        self.push_grapheme(&mut cells, g, b + i, style, None);
                    }
                }
                b += piece.len();
            }
            self.emit(cells, Some(offset));
            offset += line.len();
        }
    }

    /// The raw view: every source line, highlighted as Markdown.
    fn raw(&mut self, links: &[doc::Link]) {
        let ss = syntaxes();
        let syntax = ss
            .find_syntax_by_name("Markdown")
            .unwrap_or_else(|| ss.find_syntax_plain_text());
        let mut hl = HighlightLines::new(syntax, code_theme());
        let mut offset = 0;
        for line in self.src.split_inclusive('\n') {
            let end = offset + line.len();
            let near: Vec<(usize, &Range<usize>)> = links
                .iter()
                .enumerate()
                .filter(|(_, l)| l.range.start < end && offset < l.range.end)
                .map(|(i, l)| (i, &l.range))
                .collect();
            let pieces = hl
                .highlight_line(line, ss)
                .map(|v| {
                    v.into_iter()
                        .map(|(st, s)| (syntect_style(st), s))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|_| vec![(base_style(), line)]);
            let mut cells = Vec::new();
            let mut b = offset;
            for (style, piece) in pieces {
                for (i, g) in piece.grapheme_indices(true) {
                    if matches!(g, "\n" | "\r\n" | "\r") {
                        continue;
                    }
                    let at = b + i;
                    let link = near.iter().find(|(_, r)| r.contains(&at)).map(|(i, _)| *i);
                    self.push_grapheme(&mut cells, g, at, style, link);
                }
                b += piece.len();
            }
            self.emit(cells, Some(offset));
            offset = end;
        }
    }

    fn table(
        &mut self,
        range: &Range<usize>,
        alignments: &[Alignment],
        header: &[Range<usize>],
        rows: &[Vec<Range<usize>>],
    ) {
        let ncol = rows
            .iter()
            .map(Vec::len)
            .chain([alignments.len(), header.len()])
            .max()
            .unwrap_or(0);
        if ncol == 0 {
            return;
        }
        let header_style = base_style().add_modifier(Modifier::BOLD);
        let all: Vec<(&[Range<usize>], Style)> = std::iter::once((header, header_style))
            .chain(rows.iter().map(|r| (r.as_slice(), base_style())))
            .collect();
        let mut nat = vec![1; ncol];
        for (cells, style) in &all {
            for (c, r) in cells.iter().enumerate() {
                let w = self
                    .wrapped_inline(r, *style, usize::MAX)
                    .iter()
                    .map(|row| width_of(row))
                    .max()
                    .unwrap_or(0);
                nat[c] = nat[c].max(w);
            }
        }
        let overhead = 3 * ncol + 1;
        let avail = self.avail();
        let mut widths = nat.clone();
        if nat.iter().sum::<usize>() + overhead > avail && avail >= overhead + ncol {
            let mut remaining = avail - overhead;
            let mut order: Vec<usize> = (0..ncol).collect();
            order.sort_by_key(|&i| nat[i]);
            for (k, &i) in order.iter().enumerate() {
                let share = remaining / (ncol - k);
                widths[i] = nat[i].min(share).max(1);
                remaining = remaining.saturating_sub(widths[i]);
            }
        }

        let border = Style::new().fg(palette::OVERLAY);
        let rule = |l: &str, m: &str, r: &str| {
            let mid: Vec<String> = widths.iter().map(|w| "─".repeat(w + 2)).collect();
            deco_cells(&format!("{l}{}{r}", mid.join(m)), border)
        };
        let top = rule("┌", "┬", "┐");
        let sep = rule("├", "┼", "┤");
        let bottom = rule("└", "┴", "┘");
        self.emit(top, Some(range.start));
        for (n, (cells, style)) in all.iter().enumerate() {
            let wrapped: Vec<Vec<Vec<Cell>>> = (0..ncol)
                .map(|c| {
                    cells
                        .get(c)
                        .map(|r| self.wrapped_inline(r, *style, widths[c]))
                        .unwrap_or_default()
                })
                .collect();
            let height = wrapped.iter().map(Vec::len).max().unwrap_or(0).max(1);
            // A row whose cells draw nothing (an empty body row) still
            // comes from its own source line: anchor it to its first cell.
            let anchor = cells.first().map_or(range.start, |r| r.start);
            for j in 0..height {
                let mut row = deco_cells("│", border);
                for c in 0..ncol {
                    let content = wrapped[c].get(j).cloned().unwrap_or_default();
                    let slack = widths[c].saturating_sub(width_of(&content));
                    let left = match alignments.get(c) {
                        Some(Alignment::Right) => slack,
                        Some(Alignment::Center) => slack / 2,
                        _ => 0,
                    };
                    row.extend(deco_cells(&" ".repeat(left + 1), base_style()));
                    row.extend(content);
                    row.extend(deco_cells(&" ".repeat(slack - left + 1), base_style()));
                    row.extend(deco_cells("│", border));
                }
                self.emit(row, Some(anchor));
            }
            if n == 0 {
                self.emit(sep.clone(), Some(range.start));
            }
        }
        self.emit(bottom, Some(range.start));
    }
}

// ---------------------------------------------------------------------------
// Math

fn math_style(base: Style, link: Option<usize>) -> Style {
    let style = base.fg(palette::TEAL);
    if link.is_some() {
        style
            .add_modifier(Modifier::UNDERLINED)
            .underline_color(palette::BLUE)
    } else {
        style
    }
}

/// True when `{`/`}` (ignoring `\{`, `\}`) nest properly.
fn braces_balanced(s: &str) -> bool {
    let mut depth = 0i32;
    let mut escaped = false;
    for c in s.chars() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

/// The converters never fail, so judge their output: a conversion fails
/// when the output is blank, still holds a `\command`, or has unbalanced
/// braces, or when the input does (`\frac{a}{`) or uses an environment
/// (`\begin{…}`, which neither converter lays out).
fn converted(tex: &str, out: &str) -> bool {
    let has_command = out
        .as_bytes()
        .windows(2)
        .any(|w| w[0] == b'\\' && w[1].is_ascii_alphabetic());
    !out.trim().is_empty()
        && !has_command
        && braces_balanced(out)
        && braces_balanced(tex)
        && !tex.contains("\\begin{")
}

/// The LaTeX on one line, trimmed (converters do not expect newlines).
fn math_source(tex: &str) -> String {
    tex.replace(['\r', '\n'], " ").trim().to_string()
}

/// One-line Unicode for `tex`, or the trimmed LaTeX when conversion fails.
fn math_one_line(tex: &str) -> String {
    let tex = math_source(tex);
    let out = latex_to_unicode::latex_to_unicode(&tex);
    if converted(&tex, &out) { out } else { tex }
}

/// 2-D rows for display math, or `None` when conversion fails.
fn math_grid(tex: &str) -> Option<Vec<String>> {
    let tex = math_source(tex);
    let block = term_maths::render(&tex);
    let rows: Vec<String> = block
        .cells()
        .iter()
        .map(|row| row.concat().trim_end().to_string())
        .collect();
    converted(&tex, &rows.join("\n")).then_some(rows)
}

impl Renderer<'_> {
    /// A drawn math cell mapping to `src` (the whole math range).
    fn math_cell(g: &str, src: &Range<usize>, style: Style, link: Option<usize>) -> Option<Cell> {
        let (text, w) = sanitize(g);
        (w > 0).then(|| Cell {
            text,
            w,
            style,
            src: Some(src.clone()),
            link,
        })
    }

    /// Wrap tokens for an [`Inline::Math`] drawn on one line: converted
    /// Unicode (every cell maps to the math range), or with math off the
    /// raw source pieces (each grapheme maps to its own byte).
    fn math_tokens(&self, inline: &Inline, base: Style) -> Vec<Tok> {
        let Inline::Math {
            tex,
            range,
            raw,
            link,
            ..
        } = inline
        else {
            return Vec::new();
        };
        let style = math_style(base, *link);
        let mut toks = Vec::new();
        let mut word = Vec::new();
        let space = |word: &mut Vec<Cell>, toks: &mut Vec<Tok>, src| {
            if !word.is_empty() {
                toks.push(Tok::Word(std::mem::take(word)));
            }
            toks.push(Tok::Space(src));
        };
        if self.math {
            for g in math_one_line(tex).graphemes(true) {
                if g.chars().all(char::is_whitespace) {
                    space(&mut word, &mut toks, Some(range.clone()));
                } else {
                    word.extend(Self::math_cell(g, range, style, *link));
                }
            }
        } else {
            for (k, r) in raw.iter().enumerate() {
                if k > 0 {
                    space(&mut word, &mut toks, None);
                }
                for (i, g) in self.slice(r).grapheme_indices(true) {
                    let b = r.start + i;
                    if g.chars().all(char::is_whitespace) {
                        space(&mut word, &mut toks, Some(b..b + g.len()));
                    } else {
                        self.push_grapheme(&mut word, g, b, style, *link);
                    }
                }
            }
        }
        if !word.is_empty() {
            toks.push(Tok::Word(word));
        }
        toks
    }

    /// A paragraph whose only content is one `$$…$$` is drawn as a
    /// display block. Returns false (nothing drawn) otherwise, and always
    /// with math off (the paragraph then reflows its raw source).
    fn standalone_math(&mut self, range: &Range<usize>, inline: &Range<usize>) -> bool {
        if !self.math {
            return false;
        }
        let inlines = (self.inlines)(inline.clone());
        let mut math = inlines.iter().filter(|i| {
            !matches!(i, Inline::SoftBreak | Inline::HardBreak)
                && !matches!(i, Inline::Text { range, .. } if self.slice(range).trim().is_empty())
        });
        let (Some(m @ Inline::Math { display: true, .. }), None) = (math.next(), math.next())
        else {
            return false;
        };
        self.display_math(m, range.start);
        true
    }

    /// A fenced ```` ```math ```` block, drawn like a standalone `$$` block.
    fn math_fence(&mut self, code: &Range<usize>, pieces: &[Range<usize>]) {
        // The pieces already skip container prefixes, so a `>` left in them
        // is LaTeX. A top-level piece may hold several lines.
        let mut tex = String::new();
        let mut first: Option<Range<usize>> = None;
        for piece in pieces {
            let mut offset = piece.start;
            for line in self.slice(piece).split_inclusive('\n') {
                let body = line.trim_end_matches(['\n', '\r']);
                let text = body.trim();
                if first.is_none() && !text.is_empty() {
                    let start = offset + body.len() - body.trim_start().len();
                    first = Some(start..start + text.len());
                }
                tex.push_str(text);
                if line.ends_with('\n') {
                    tex.push('\n');
                }
                offset += line.len();
            }
        }
        let range = first.unwrap_or(code.start..code.start);
        let m = Inline::Math {
            tex,
            display: true,
            raw: vec![range.clone()],
            range,
            link: None,
        };
        self.display_math(&m, code.start);
    }

    /// Display math: the 2-D grid centred in the available width, or one
    /// wrapped line when it does not convert or does not fit.
    fn display_math(&mut self, m: &Inline, fallback: usize) {
        let Inline::Math { tex, range, .. } = m else {
            return;
        };
        let style = math_style(base_style(), None);
        let avail = self.avail();
        if let Some(rows) = math_grid(tex) {
            let w = rows
                .iter()
                .map(|r| UnicodeWidthStr::width(r.as_str()))
                .max()
                .unwrap_or(0);
            if w <= avail {
                let pad = " ".repeat((avail - w) / 2);
                for row in rows {
                    let mut cells = deco_cells(&pad, base_style());
                    cells.extend(
                        row.graphemes(true)
                            .filter_map(|g| Self::math_cell(g, range, style, None)),
                    );
                    self.emit(cells, Some(fallback));
                }
                return;
            }
        }
        let toks = self.math_tokens(m, base_style());
        let rows = wrap(toks, avail, base_style());
        if rows.is_empty() {
            self.emit(Vec::new(), Some(fallback));
        }
        for row in rows {
            self.emit(row, Some(fallback));
        }
    }
}
