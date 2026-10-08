//! Front matter (`---` YAML or `+++` TOML) as a list of top-level entries
//! for the folded marker and the expanded table in the normal view.
//!
//! Parsing is generous, best effort: ramble is not a linter. A real parser
//! decides each value's shape; the displayed text comes from the source
//! (a line scan), and when the parser fails the line scan alone is used.
//! See `docs/specs/2026-10-08-front-matter.md`.

use std::collections::HashSet;
use std::ops::Range;

/// Which delimiters the block used.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FmKind {
    /// `---` ... `---`
    #[default]
    Yaml,
    /// `+++` ... `+++`
    Toml,
}

/// A top-level value as shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FmValue {
    /// The source text, surrounding quotes stripped.
    Scalar(String),
    List(Vec<String>),
    /// A nested map, shown as `{…}`.
    Map,
}

/// One top-level entry. `src` is the byte range in the parsed text of
/// the lines it came from (the whole text when it can't be located).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FmEntry {
    pub key: String,
    pub value: FmValue,
    pub src: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FrontMatter {
    /// In source order.
    pub entries: Vec<FmEntry>,
    /// False when no key could be extracted from a non-blank block.
    pub parsed: bool,
}

impl FmValue {
    /// The value as drawn: lists joined with `, `, maps as `{…}`.
    pub fn display(&self) -> String {
        match self {
            FmValue::Scalar(s) => s.clone(),
            FmValue::List(items) => items.join(", "),
            FmValue::Map => "{…}".into(),
        }
    }
}

/// The text between the delimiter lines of the block at `range` in
/// `source` (the range covers the fences, as `Document::front_matter`).
pub fn body(source: &str, range: &Range<usize>) -> Range<usize> {
    let block = source.get(range.clone()).unwrap_or("");
    let start = block.find('\n').map_or(block.len(), |i| i + 1);
    let end = block.rfind('\n').unwrap_or(0).max(start);
    range.start + start..range.start + end
}

/// Parse the text between the delimiters. Never fails.
pub fn parse(src: &str, kind: FmKind) -> FrontMatter {
    let scanned = scan(src);
    let real = match kind {
        FmKind::Yaml => yaml_entries(src),
        FmKind::Toml => toml_entries(src),
    };
    let entries = match real {
        Some(real) => combine(real, scanned, src, kind),
        None => scanned,
    };
    let parsed = !entries.is_empty() || src.trim().is_empty();
    FrontMatter { entries, parsed }
}

/// The parser's entries combined with the line scan's. When the scan
/// found the same top-level keys as the parser, keys, order and text come
/// from the source and the parser only decides each value's shape;
/// duplicate keys all show, in source order. Otherwise (TOML tables,
/// dotted keys, a flow mapping) the parser's entries are used, with the
/// scan's text and lines where a key matches.
fn combine(
    real: Vec<(String, FmValue)>,
    mut scanned: Vec<FmEntry>,
    src: &str,
    kind: FmKind,
) -> Vec<FmEntry> {
    let norm: Vec<String> = scanned
        .iter()
        .map(|e| match kind {
            FmKind::Yaml if !src[e.src.start..].trim_start().starts_with(['"', '\'']) => {
                yaml_key(&e.key)
            }
            _ => e.key.clone(),
        })
        .collect();
    let covers = real.iter().all(|(k, _)| norm.contains(k))
        && norm.iter().all(|n| real.iter().any(|(k, _)| k == n));
    if covers && !scanned.is_empty() {
        let mut real = real;
        // The last occurrence of a key is the one the parser kept.
        for (i, n) in norm.iter().enumerate().rev() {
            if let Some(j) = real.iter().position(|(k, _)| k == n) {
                let (_, shape) = real.swap_remove(j);
                let value = std::mem::replace(&mut scanned[i].value, FmValue::Map);
                scanned[i].value = pick(shape, value);
            }
        }
        return scanned;
    }
    real.into_iter()
        .map(|(key, shape)| {
            let found = norm.iter().position(|n| *n == key).map(|i| &scanned[i]);
            FmEntry {
                value: match found {
                    Some(e) => pick(shape, e.value.clone()),
                    None => shape,
                },
                src: found.map_or(0..src.len(), |e| e.src.clone()),
                key,
            }
        })
        .collect()
}

/// An unquoted YAML key as the parser reads it (`1.10` -> `1.1`, `~` ->
/// empty), so source keys can be matched to parsed ones.
fn yaml_key(key: &str) -> String {
    use saphyr::{LoadableYamlNode, Yaml};
    Yaml::load_from_str(key)
        .ok()
        .and_then(|d| d.first().and_then(yaml_scalar))
        .unwrap_or_else(|| key.to_string())
}

/// The parser's shape, with the scan's text when the shapes agree.
fn pick(shape: FmValue, scanned: FmValue) -> FmValue {
    match (&shape, &scanned) {
        (FmValue::Scalar(_), FmValue::Scalar(_)) | (FmValue::List(_), FmValue::List(_)) => scanned,
        _ => shape,
    }
}

// ---------------------------------------------------------------------------
// Real parsers: top-level keys and value shapes, `None` unless a mapping.

fn yaml_entries(src: &str) -> Option<Vec<(String, FmValue)>> {
    use saphyr::{LoadableYamlNode, Yaml};
    let docs = Yaml::load_from_str(src).ok()?;
    let Some(Yaml::Mapping(map)) = docs.first() else {
        return None;
    };
    Some(
        map.iter()
            .filter_map(|(k, v)| Some((yaml_scalar(k)?, yaml_shape(v))))
            .collect(),
    )
}

fn yaml_scalar(y: &saphyr::Yaml) -> Option<String> {
    use saphyr::{Scalar, Yaml};
    match y {
        Yaml::Value(s) => Some(match s {
            Scalar::Null => String::new(),
            Scalar::Boolean(b) => b.to_string(),
            Scalar::Integer(i) => i.to_string(),
            Scalar::FloatingPoint(f) => f.to_string(),
            Scalar::String(s) => s.to_string(),
        }),
        Yaml::Representation(s, ..) => Some(s.to_string()),
        Yaml::Tagged(_, inner) => yaml_scalar(inner),
        _ => None,
    }
}

fn yaml_shape(y: &saphyr::Yaml) -> FmValue {
    use saphyr::Yaml;
    match y {
        Yaml::Mapping(_) => FmValue::Map,
        Yaml::Sequence(items) => FmValue::List(
            items
                .iter()
                .map(|i| yaml_scalar(i).unwrap_or_else(|| yaml_shape(i).display()))
                .collect(),
        ),
        Yaml::Tagged(_, inner) => yaml_shape(inner),
        y => FmValue::Scalar(yaml_scalar(y).unwrap_or_default()),
    }
}

fn toml_entries(src: &str) -> Option<Vec<(String, FmValue)>> {
    let table: toml::Table = src.parse().ok()?;
    Some(
        table
            .into_iter()
            .map(|(k, v)| (k, toml_shape(&v)))
            .collect(),
    )
}

fn toml_shape(v: &toml::Value) -> FmValue {
    match v {
        toml::Value::Table(_) => FmValue::Map,
        toml::Value::Array(items) => FmValue::List(items.iter().map(toml_text).collect()),
        v => FmValue::Scalar(toml_text(v)),
    }
}

fn toml_text(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        toml::Value::Table(_) => "{…}".into(),
        v => v.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Line scan.

/// An entry being collected.
struct Building {
    key: String,
    /// The inline value and folded continuation lines.
    parts: Vec<String>,
    items: Vec<String>,
    map: bool,
    src: Range<usize>,
}

impl Building {
    fn finish(self) -> FmEntry {
        let value = if self.map {
            FmValue::Map
        } else if !self.items.is_empty() {
            FmValue::List(self.items)
        } else {
            let text = self.parts.join(" ");
            if text.starts_with('[') && text.ends_with(']') {
                FmValue::List(split_inline(&text[1..text.len() - 1]))
            } else if text.starts_with('{') {
                FmValue::Map
            } else {
                FmValue::Scalar(unquote(&text).to_string())
            }
        };
        FmEntry {
            key: self.key,
            value,
            src: self.src,
        }
    }

    /// No value on the key line and nothing after it yet.
    fn empty(&self) -> bool {
        self.parts.is_empty() && self.items.is_empty()
    }
}

/// Walk the block line by line: `key: value` / `key = value` at the
/// smallest indentation starts an entry; more indented lines are list
/// items, nested keys (a map) or folded continuation.
fn scan(src: &str) -> Vec<FmEntry> {
    let mut lines = Vec::new();
    let mut at = 0;
    for line in src.split_inclusive('\n') {
        let text = line.trim_end_matches(['\n', '\r']);
        lines.push((at, text));
        at += line.len();
    }
    let indent = |t: &str| t.len() - t.trim_start_matches([' ', '\t']).len();
    let content = |t: &str| {
        let t = t.trim();
        !t.is_empty() && !t.starts_with('#')
    };
    let Some(base) = lines
        .iter()
        .filter(|(_, t)| content(t) && !is_item(t.trim()) && split_entry(t.trim()).is_some())
        .map(|(_, t)| indent(t))
        .min()
    else {
        return Vec::new();
    };

    let quoted = quoted_continuations(&lines, base);
    let mut out = Vec::new();
    let mut cur: Option<Building> = None;
    let start = |at: usize, t: &str, (k, v): (String, String)| Building {
        key: k,
        parts: if v.is_empty() || is_block_indicator(&v) {
            Vec::new()
        } else {
            vec![v]
        },
        items: Vec::new(),
        map: false,
        src: at..at + t.len(),
    };
    for (n, &(at, line)) in lines.iter().enumerate() {
        if quoted.contains(&n)
            && let Some(c) = &mut cur
        {
            c.parts.push(line.trim().to_string());
            c.src.end = at + line.len();
            continue;
        }
        if !content(line) {
            continue;
        }
        let t = line.trim();
        let ind = indent(line);
        if ind <= base {
            match (ind == base).then(|| split_entry(t)).flatten() {
                Some(kv) if !is_item(t) => {
                    out.extend(cur.take().map(Building::finish));
                    cur = Some(start(at, line, kv));
                }
                // `key:` then `- item` at the same indentation.
                _ if is_item(t) && cur.as_ref().is_some_and(|c| c.parts.is_empty() && !c.map) => {
                    let c = cur.as_mut().expect("checked");
                    c.items.push(unquote(t[1..].trim()).to_string());
                    c.src.end = at + line.len();
                }
                _ => out.extend(cur.take().map(Building::finish)),
            }
            continue;
        }
        let Some(c) = &mut cur else { continue };
        if is_item(t) {
            c.items.push(unquote(t[1..].trim()).to_string());
        } else if let Some(kv) = split_entry(t).filter(|_| !c.parts.is_empty() && !c.map) {
            // Bad indentation: a key under a scalar starts its own entry.
            out.extend(cur.take().map(Building::finish));
            cur = Some(start(at, line, kv));
            continue;
        } else if split_entry(t).is_some() && (c.map || c.empty()) {
            c.map = true;
        } else if !c.map {
            c.parts.push(t.to_string());
        }
        c.src.end = at + line.len();
    }
    out.extend(cur.map(Building::finish));
    out
}

/// Indices of lines inside a quoted value that runs past its key line:
/// TOML `"""` / `\'\'\'` strings up to their closing fence, and YAML `"` /
/// `'` scalars continued on lines that don't look like a new entry. An
/// unclosed quote absorbs nothing.
fn quoted_continuations(lines: &[(usize, &str)], base: usize) -> HashSet<usize> {
    let mut out = HashSet::new();
    let indent = |t: &str| t.len() - t.trim_start_matches([' ', '\t']).len();
    let mut n = 0;
    while n < lines.len() {
        let line = lines[n].1;
        n += 1;
        let Some((_, v)) = split_entry(line.trim()).filter(|_| indent(line) == base) else {
            continue;
        };
        let fence = ["\"\"\"", "\'\'\'"].into_iter().find(|f| v.starts_with(f));
        let open = match fence {
            Some(f) => !v[3..].contains(f),
            None => v.starts_with(['"', '\'']) && closing_quote(&v).is_none(),
        };
        if !open {
            continue;
        }
        let close = lines[n..].iter().position(|(_, t)| match fence {
            Some(f) => t.contains(f),
            None => t.contains(&v[..1]),
        });
        let entry_before = |end: usize| {
            fence.is_none()
                && lines[n..n + end]
                    .iter()
                    .any(|(_, t)| indent(t) <= base && split_entry(t.trim()).is_some())
        };
        if let Some(end) = close.filter(|&e| !entry_before(e + 1)) {
            out.extend(n..=n + end);
            n += end + 1;
        }
    }
    out
}

fn is_item(t: &str) -> bool {
    t == "-" || t.starts_with("- ") || t.starts_with("-\t")
}

/// `|`, `>`, `|-`, `>+2` and the like: a YAML block scalar header.
fn is_block_indicator(v: &str) -> bool {
    let mut cs = v.chars();
    matches!(cs.next(), Some('|' | '>')) && cs.all(|c| matches!(c, '-' | '+' | '0'..='9'))
}

/// `key: value` or `key = value`: the key (quotes trimmed) and the value,
/// with an unquoted trailing ` # comment` removed. A `:` must be followed
/// by whitespace or the end of the line, so `http://` is not a key.
fn split_entry(t: &str) -> Option<(String, String)> {
    let (key, rest) = match t.chars().next() {
        Some(q @ ('"' | '\'')) => {
            let end = t[1..].find(q)? + 1;
            (&t[1..end], t[end + 1..].trim_start())
        }
        _ => {
            let i = t.char_indices().find_map(|(i, c)| {
                let next = t[i + c.len_utf8()..].chars().next();
                match c {
                    '=' => Some(i),
                    ':' if next.is_none_or(char::is_whitespace) => Some(i),
                    _ => None,
                }
            })?;
            (t[..i].trim(), &t[i..])
        }
    };
    let rest = rest.strip_prefix([':', '='])?;
    if key.is_empty() || key.starts_with(['#', '[', '{']) {
        return None;
    }
    let mut value = rest.trim();
    if let Some(end) = closing_quote(value) {
        let after = value[end..].trim_start();
        if after.is_empty() || after.starts_with('#') {
            value = &value[..end];
        }
    } else if let Some(i) = comment_start(value) {
        value = value[..i].trim_end();
    }
    Some((key.to_string(), value.to_string()))
}

/// Where an unquoted value's ` # comment` starts. A `#` inside `[...]` or
/// `{...}` (Obsidian's `tags: [#a, #b]`) is part of the value, and so is
/// everything after an unclosed bracket.
fn comment_start(v: &str) -> Option<usize> {
    let mut depth = 0usize;
    let mut quote = None;
    // A value that starts with `#` is shown, as before (generous).
    let mut prev = '#';
    for (i, c) in v.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') if depth > 0 => quote = Some(c),
            (None, '[' | '{') => depth += 1,
            (None, ']' | '}') => depth = depth.saturating_sub(1),
            (None, '#') if depth == 0 && prev.is_whitespace() => return Some(i),
            _ => {}
        }
        prev = c;
    }
    None
}

/// For a value starting with a quote, the byte just past its closing
/// quote: `\"` is escaped inside `"`, `''` inside `'`.
fn closing_quote(v: &str) -> Option<usize> {
    let q = v.chars().next().filter(|c| matches!(c, '"' | '\''))?;
    let mut chars = v.char_indices().skip(1).peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' if q == '"' => {
                chars.next();
            }
            c if c == q && q == '\'' && chars.peek().is_some_and(|&(_, n)| n == q) => {
                chars.next();
            }
            c if c == q => return Some(i + 1),
            _ => {}
        }
    }
    None
}

/// Strip one pair of surrounding quotes, or a TOML `"""` / `\'\'\'` pair
/// (and the space next to it).
fn unquote(s: &str) -> &str {
    for f in ["\"\"\"", "\'\'\'"] {
        if s.len() >= 6 && s.starts_with(f) && s.ends_with(f) {
            return s[3..s.len() - 3].trim();
        }
    }
    let b = s.as_bytes();
    if b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && b[b.len() - 1] == b[0] {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Split `a, "b, c", d` on commas outside quotes; items unquoted, empty
/// items dropped.
fn split_inline(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut quote = None;
    let mut item = String::new();
    for c in s.chars() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, ',') => {
                out.push(std::mem::take(&mut item));
                continue;
            }
            _ => {}
        }
        item.push(c);
    }
    out.push(item);
    out.iter()
        .map(|i| unquote(i.trim()).to_string())
        .filter(|i| !i.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kv(fm: &FrontMatter) -> Vec<(&str, FmValue)> {
        fm.entries
            .iter()
            .map(|e| (e.key.as_str(), e.value.clone()))
            .collect()
    }

    fn s(v: &str) -> FmValue {
        FmValue::Scalar(v.into())
    }

    fn l(v: &[&str]) -> FmValue {
        FmValue::List(v.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn valid_yaml_scalars_lists_maps_quotes_dates() {
        let src = "title: \"Hello: world\"\ndate: 2024-01-05\nversion: 1.10\n\
                   tags: [rust, 'tui']\nauthors:\n  - Ann\n  - Bob\n\
                   meta:\n  a: 1\n  b: 2\nempty:\nflag: yes\n";
        let fm = parse(src, FmKind::Yaml);
        assert!(fm.parsed);
        assert_eq!(
            kv(&fm),
            [
                ("title", s("Hello: world")),
                ("date", s("2024-01-05")),
                ("version", s("1.10")),
                ("tags", l(&["rust", "tui"])),
                ("authors", l(&["Ann", "Bob"])),
                ("meta", FmValue::Map),
                ("empty", s("")),
                ("flag", s("yes")),
            ]
        );
        let authors = &fm.entries[4];
        assert_eq!(&src[authors.src.clone()], "authors:\n  - Ann\n  - Bob");
    }

    #[test]
    fn yaml_block_scalar_and_compact_list() {
        let src = "desc: >\n  one\n  two\ntags:\n- a\n- b\n";
        let fm = parse(src, FmKind::Yaml);
        assert_eq!(kv(&fm), [("desc", s("one two")), ("tags", l(&["a", "b"]))]);
    }

    #[test]
    fn broken_yaml_bad_indentation() {
        let fm = parse("title: X\n author: Y\ntags: [a]\n", FmKind::Yaml);
        assert_eq!(
            kv(&fm),
            [("title", s("X")), ("author", s("Y")), ("tags", l(&["a"]))]
        );
    }

    #[test]
    fn broken_yaml_tab() {
        let fm = parse("tags:\n\t- a\n\t- b\ntitle: T\n", FmKind::Yaml);
        assert_eq!(kv(&fm), [("tags", l(&["a", "b"])), ("title", s("T"))]);
    }

    #[test]
    fn broken_yaml_unclosed_quote() {
        let fm = parse("title: \"Hello\nauthor: Ann\n", FmKind::Yaml);
        assert_eq!(kv(&fm), [("title", s("\"Hello")), ("author", s("Ann"))]);
    }

    #[test]
    fn broken_yaml_duplicate_key() {
        let src = "a: 1\nb: 2\na: 3\n";
        let fm = parse(src, FmKind::Yaml);
        assert!(fm.parsed);
        assert_eq!(kv(&fm), [("a", s("1")), ("b", s("2")), ("a", s("3"))]);
        let lines: Vec<&str> = fm.entries.iter().map(|e| &src[e.src.clone()]).collect();
        assert_eq!(lines, ["a: 1", "b: 2", "a: 3"]);
    }

    #[test]
    fn non_string_yaml_keys_keep_their_source_text_and_lines() {
        let src = "1.10: y\n007: z\nnull: x\n~: n\ntags:\n  - a\n";
        let fm = parse(src, FmKind::Yaml);
        assert_eq!(
            kv(&fm),
            [
                ("1.10", s("y")),
                ("007", s("z")),
                ("null", s("x")),
                ("~", s("n")),
                ("tags", l(&["a"])),
            ]
        );
        let lines: Vec<&str> = fm.entries.iter().map(|e| &src[e.src.clone()]).collect();
        assert_eq!(
            lines,
            ["1.10: y", "007: z", "null: x", "~: n", "tags:\n  - a"]
        );
    }

    #[test]
    fn toml_tables_and_dotted_keys_use_the_parser_keys() {
        let fm = parse("a.b = 1\n[t]\nk = 2\n", FmKind::Toml);
        assert_eq!(kv(&fm), [("a", FmValue::Map), ("t", FmValue::Map)]);
    }

    #[test]
    fn toml_front_matter() {
        let src = "title = \"T\"\ndate = 2024-01-05\ntags = [\"rust\", \"tui\"]\n\
                   nums = [\n  1,\n  2,\n]\n[extra]\nk = 1\n";
        let fm = parse(src, FmKind::Toml);
        assert_eq!(
            kv(&fm),
            [
                ("title", s("T")),
                ("date", s("2024-01-05")),
                ("tags", l(&["rust", "tui"])),
                ("nums", l(&["1", "2"])),
                ("extra", FmValue::Map),
            ]
        );
    }

    #[test]
    fn broken_toml_falls_back_to_key_value_lines() {
        let fm = parse("title = \"T\nauthor = Ann\nx = [1, 2\n", FmKind::Toml);
        assert_eq!(
            kv(&fm),
            [("title", s("\"T")), ("author", s("Ann")), ("x", s("[1, 2"))]
        );
    }

    #[test]
    fn empty_block() {
        for kind in [FmKind::Yaml, FmKind::Toml] {
            let fm = parse("", kind);
            assert!(fm.parsed);
            assert!(fm.entries.is_empty());
            assert!(parse("\n  \n", kind).parsed);
        }
    }

    #[test]
    fn only_comments_or_stray_text_is_unparsed() {
        for src in ["# a comment\n# another\n", "just some text\nmore text\n"] {
            for kind in [FmKind::Yaml, FmKind::Toml] {
                let fm = parse(src, kind);
                assert!(!fm.parsed, "{src:?} {kind:?}");
                assert!(fm.entries.is_empty());
            }
        }
    }

    #[test]
    fn stray_lines_and_comments_are_skipped() {
        let fm = parse(
            "# c\ntitle: A # note\nstray text\nurl: http://x.y\n",
            FmKind::Yaml,
        );
        assert_eq!(kv(&fm), [("title", s("A")), ("url", s("http://x.y"))]);
    }

    #[test]
    fn quoted_keys_and_inline_list_with_quoted_commas() {
        let fm = parse("\"a: b\": 1\nt: [\"x, y\", z,]\n", FmKind::Yaml);
        assert_eq!(kv(&fm), [("a: b", s("1")), ("t", l(&["x, y", "z"]))]);
    }

    #[test]
    fn a_comment_after_a_quoted_value_is_dropped() {
        let fm = parse(
            "q: \"x\" # c\nc2: '#000' # trailing\ne: \"a\\\"b\" # c\n",
            FmKind::Yaml,
        );
        assert_eq!(
            kv(&fm),
            [("q", s("x")), ("c2", s("#000")), ("e", s("a\\\"b"))]
        );
        let fm = parse("s = 'x' # c\nt = \"#y\" # d\n", FmKind::Toml);
        assert_eq!(kv(&fm), [("s", s("x")), ("t", s("#y"))]);
        // Through the line scan alone (broken YAML), too.
        let fm = parse("q: 'x' # c\n bad: [\n", FmKind::Yaml);
        assert_eq!(kv(&fm)[0], ("q", s("x")));
        assert_eq!(closing_quote("'it''s' # x"), Some(7));
        assert_eq!(closing_quote("\"a\\\"b\" # x"), Some(6));
        assert_eq!(closing_quote("'open"), None);
    }

    #[test]
    fn multi_line_quoted_strings_are_joined_and_cover_all_their_lines() {
        let src = "multi = \"\"\"\nline1\nline2\"\"\"\nlit = \'\'\'\na\nb\'\'\'\nz = 1\n";
        let fm = parse(src, FmKind::Toml);
        assert_eq!(
            kv(&fm),
            [
                ("multi", s("line1 line2")),
                ("lit", s("a b")),
                ("z", s("1"))
            ]
        );
        assert_eq!(
            &src[fm.entries[0].src.clone()],
            "multi = \"\"\"\nline1\nline2\"\"\""
        );
        // A YAML double-quoted scalar continued on an unindented line.
        let src = "title: \"a\nb\"\nz: 1\n";
        let fm = parse(src, FmKind::Yaml);
        assert_eq!(kv(&fm), [("title", s("a b")), ("z", s("1"))]);
        assert_eq!(&src[fm.entries[0].src.clone()], "title: \"a\nb\"");
        // An indented continuation that looks like a key is still text.
        let fm = parse("title: \"a\n  k: v\"\nz: 1\n", FmKind::Yaml);
        assert_eq!(kv(&fm), [("title", s("a k: v")), ("z", s("1"))]);
        // Broken YAML keeps the scan's text.
        let fm = parse("title: \"open\nz: 1\n", FmKind::Yaml);
        assert_eq!(kv(&fm), [("title", s("\"open")), ("z", s("1"))]);
        // ... even when a later line has a quote.
        let fm = parse("title: \"open\nz: 1\nw: \"x\"\n", FmKind::Yaml);
        assert_eq!(
            kv(&fm),
            [("title", s("\"open")), ("z", s("1")), ("w", s("x"))]
        );
    }

    #[test]
    fn hashes_inside_an_inline_list_are_values() {
        let fm = parse("tags: [#a, #b] # c\nm: {k: #v}\nt: x #y\n", FmKind::Yaml);
        assert_eq!(
            kv(&fm),
            [
                ("tags", l(&["#a", "#b"])),
                ("m", FmValue::Map),
                ("t", s("x")),
            ]
        );
        // Unclosed bracket: generous, keep the rest.
        let fm = parse("tags: [#a, #b\n", FmKind::Yaml);
        assert_eq!(kv(&fm), [("tags", s("[#a, #b"))]);
        // A bracket inside a quoted item doesn't close the list.
        let fm = parse("t: [\"x ] #\", #y] # c\nh: #h\n", FmKind::Yaml);
        assert_eq!(kv(&fm), [("t", l(&["x ] #", "#y"])), ("h", s("#h"))]);
    }

    #[test]
    fn crlf_lines() {
        let src = "title: T\r\ntags:\r\n  - a\r\nz: 1\r\n";
        let fm = parse(src, FmKind::Yaml);
        assert_eq!(
            kv(&fm),
            [("title", s("T")), ("tags", l(&["a"])), ("z", s("1"))]
        );
        assert_eq!(&src[fm.entries[1].src.clone()], "tags:\r\n  - a");
        // Through the scan alone (a tab makes it invalid YAML).
        let fm = parse("a: x\r\n\tb: y\r\n", FmKind::Yaml);
        assert_eq!(kv(&fm), [("a", s("x")), ("b", s("y"))]);
    }

    #[test]
    fn tagged_yaml_values_take_the_inner_shape() {
        let fm = parse(
            "t: !!str 5\nl: !custom [a, b]\nm: !x {k: 1}\n",
            FmKind::Yaml,
        );
        assert_eq!(
            kv(&fm),
            [
                ("t", s("!!str 5")),
                ("l", l(&["a", "b"])),
                ("m", FmValue::Map),
            ]
        );
    }

    #[test]
    fn body_strips_the_delimiter_lines() {
        let src = "---\na: 1\n---\n\n# H\n";
        let r = 0..src.find("---\n\n").unwrap() + 3;
        assert_eq!(&src[body(src, &r)], "a: 1");
        let src = "---\n---\n";
        assert_eq!(body(src, &(0..7)), 4..4);
    }
}
