//! In-page search (`/ ? n N * #`): literal, smartcase, hits kept as source
//! byte ranges so they survive resize and reload.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::{App, Mode};
use crate::render::{ScreenSpan, Segment, SrcMap};

/// Search state of the current page.
#[derive(Debug, Clone, Default)]
pub(super) struct Search {
    /// The committed (or, while typing, the typed) pattern.
    pattern: String,
    /// Whole-word matching (`*` / `#`).
    whole_word: bool,
    /// Direction of the last search: `/` and `*` forward.
    forward: bool,
    /// Matches in the page source, in order. Only drawn bytes count.
    hits: Vec<Range<usize>>,
    /// The prompt text while typing; `None` outside search mode.
    prompt: Option<String>,
    /// Pattern, whole-word flag and direction before the prompt opened
    /// (restored by Esc).
    saved: Option<(String, bool, bool)>,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Literal matches of `pat` in `text`. Case-insensitive unless `pat` has
/// an uppercase letter (smartcase); whole-word searches (`*` / `#`) are
/// always case-insensitive, as in vim. Non-overlapping, in order.
pub fn find_all(text: &str, pat: &str, whole_word: bool) -> Vec<Range<usize>> {
    if pat.is_empty() {
        return Vec::new();
    }
    let fold = whole_word || !pat.chars().any(char::is_uppercase);
    let eq = |a: char, b: char| {
        if fold {
            a == b || a.to_lowercase().eq(b.to_lowercase())
        } else {
            a == b
        }
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let mut src = rest.char_indices();
        let mut end = None;
        let mut ok = true;
        for p in pat.chars() {
            match src.next() {
                Some((_, c)) if eq(c, p) => {}
                _ => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            end = Some(i + src.next().map_or(rest.len(), |(j, _)| j));
        }
        let step = rest.chars().next().map_or(1, char::len_utf8);
        match end {
            Some(e)
                if !whole_word
                    || (!text[..i].chars().next_back().is_some_and(is_word)
                        && !text[e..].chars().next().is_some_and(is_word)) =>
            {
                out.push(i..e);
                i = e.max(i + step);
            }
            _ => i += step,
        }
    }
    out
}

/// One source grapheme of a segment and the columns it is drawn on.
struct Drawn {
    src: Range<usize>,
    col: usize,
    w: usize,
}

/// The source graphemes of `seg` with their columns, when the segment
/// draws its source one grapheme per cell as the renderer does (tabs as
/// four columns, control characters as one). `None` when the widths do not
/// add up to the span (converted math, markers): callers then scale.
fn drawn_graphemes(src: &str, seg: &Segment) -> Option<Vec<Drawn>> {
    let text = src.get(seg.src.clone())?;
    let mut col = seg.span.col_start;
    let mut out = Vec::new();
    for (i, g) in text.grapheme_indices(true) {
        let w = if g == "\t" {
            4
        } else if g.chars().any(char::is_control) {
            1
        } else {
            g.width()
        };
        let start = seg.src.start + i;
        out.push(Drawn {
            src: start..start + g.len(),
            col,
            w,
        });
        col += w;
    }
    (col == seg.span.col_end).then_some(out)
}

/// The source byte of the character drawn at (`row`, `col`), always on a
/// char boundary of `src`, or `None` on a row with no drawn source.
fn char_at(map: &SrcMap, src: &str, row: usize, col: usize) -> Option<usize> {
    let mut byte = map.source_at(row, col)?;
    let lo = map.segments.partition_point(|s| s.span.row < row);
    let seg = map.segments[lo..]
        .iter()
        .take_while(|s| s.span.row == row)
        .find(|s| (s.span.col_start..s.span.col_end).contains(&col));
    if let Some(g) = seg
        .and_then(|s| drawn_graphemes(src, s))
        .and_then(|gs| gs.into_iter().find(|g| (g.col..g.col + g.w).contains(&col)))
    {
        byte = g.src.start;
    }
    // `source_at` scales columns to bytes, so it can land inside a char.
    while !src.is_char_boundary(byte) {
        byte -= 1;
    }
    Some(byte)
}

/// Screen spans drawn from `range` of `src`, trimmed to the columns of
/// `range` within each segment (a hit across a wrap yields one span per
/// row).
pub(super) fn hit_spans(map: &SrcMap, src: &str, range: &Range<usize>) -> Vec<ScreenSpan> {
    map.segments
        .iter()
        .filter(|s| s.src.start < range.end && range.start < s.src.end)
        .map(|s| {
            let cols = s.span.col_end - s.span.col_start;
            let len = (s.src.end - s.src.start).max(1);
            let at = |b: usize| s.span.col_start + (b - s.src.start) * cols / len;
            let (a, b) = match drawn_graphemes(src, s) {
                Some(gs) => {
                    let mut hit = gs
                        .iter()
                        .filter(|g| g.src.start < range.end && range.start < g.src.end);
                    let first = hit.next();
                    let last = hit.next_back().or(first);
                    (
                        first.map_or(s.span.col_start, |g| g.col),
                        last.map_or(s.span.col_start, |g| g.col + g.w),
                    )
                }
                None => (
                    at(range.start.max(s.src.start)),
                    at(range.end.min(s.src.end)),
                ),
            };
            let a = a.min(s.span.col_end.saturating_sub(1));
            ScreenSpan {
                row: s.span.row,
                col_start: a,
                col_end: b.max(a + 1).min(s.span.col_end),
            }
        })
        .collect()
}

impl App {
    /// The search prompt (`/foo` or `?foo`) while typing, else `None`.
    pub fn search_prompt(&self) -> Option<String> {
        let p = self.search.prompt.as_ref()?;
        Some(format!(
            "{}{p}",
            if self.search.forward { '/' } else { '?' }
        ))
    }

    /// Screen spans of every search hit on the current page.
    pub fn search_highlights(&self) -> Vec<ScreenSpan> {
        let Some(p) = &self.page else {
            return Vec::new();
        };
        self.search
            .hits
            .iter()
            .flat_map(|h| hit_spans(&p.rendered.srcmap, &p.doc.source, h))
            .collect()
    }

    /// Search hits as source byte ranges.
    pub fn search_hits(&self) -> &[Range<usize>] {
        &self.search.hits
    }

    /// Recompute hits from the pattern and the current source.
    pub(super) fn refresh_search(&mut self) {
        self.search.hits = match &self.page {
            Some(p) => {
                let map = &p.rendered.srcmap;
                find_all(&p.doc.source, &self.search.pattern, self.search.whole_word)
                    .into_iter()
                    .filter(|h| !map.spans_for(h.clone()).is_empty())
                    .collect()
            }
            None => Vec::new(),
        };
    }

    /// `/` (forward) or `?`: open the prompt.
    pub(super) fn search_start(&mut self, forward: bool) {
        self.search.saved = Some((
            self.search.pattern.clone(),
            self.search.whole_word,
            self.search.forward,
        ));
        self.search.prompt = Some(String::new());
        self.search.forward = forward;
        self.mode = Mode::Search;
    }

    /// A typed character (or `None` for Backspace) in the prompt.
    pub(super) fn search_edit(&mut self, c: Option<char>) {
        let Some(prompt) = &mut self.search.prompt else {
            return;
        };
        match c {
            Some(c) => prompt.push(c),
            None if prompt.is_empty() => return self.search_cancel(),
            None => {
                prompt.pop();
            }
        }
        self.search_typed();
    }

    /// Pasted text (one line) appended to the pattern being typed.
    pub(super) fn search_insert(&mut self, text: &str) {
        let Some(prompt) = &mut self.search.prompt else {
            return;
        };
        prompt.push_str(text);
        self.search_typed();
    }

    /// The prompt changed: search for what it holds now.
    fn search_typed(&mut self) {
        let Some(prompt) = &self.search.prompt else {
            return;
        };
        self.search.pattern = prompt.clone();
        self.search.whole_word = false;
        self.refresh_search();
    }

    /// Enter: commit the pattern and jump to the first hit.
    pub(super) fn search_commit(&mut self) {
        self.mode = Mode::Normal;
        let typed = self.search.prompt.take().unwrap_or_default();
        let saved = self.search.saved.take();
        if typed.is_empty() {
            // Empty pattern repeats the last one, as in vim.
            if let Some((pat, ww, _)) = saved {
                self.search.pattern = pat;
                self.search.whole_word = ww;
            }
            self.refresh_search();
        }
        self.search_next(true, 1);
    }

    /// Esc in the prompt: close it and restore the previous pattern and
    /// direction.
    pub(super) fn search_cancel(&mut self) {
        self.mode = Mode::Normal;
        self.search.prompt = None;
        if let Some((pat, ww, forward)) = self.search.saved.take() {
            self.search.pattern = pat;
            self.search.whole_word = ww;
            self.search.forward = forward;
        }
        self.refresh_search();
    }

    /// Esc in normal mode: clear the highlights.
    pub(super) fn search_clear(&mut self) {
        self.search.pattern.clear();
        self.search.hits.clear();
    }

    /// `*` (forward) / `#`: whole-word search for the word under the cursor.
    /// Both skip the word under the cursor, as in vim: `#` searches back
    /// from the word's start, not from the cursor.
    pub(super) fn search_word(&mut self, forward: bool) {
        let Some((word, start)) = self.word_under_cursor() else {
            self.set_status("No word under cursor");
            return;
        };
        self.search.pattern = word;
        self.search.whole_word = true;
        self.search.forward = forward;
        self.refresh_search();
        if !forward
            && let Some(p) = &self.page
            && let Some(s) =
                hit_spans(&p.rendered.srcmap, &p.doc.source, &(start..start + 1)).first()
        {
            self.cursor.row = s.row;
            self.cursor.col = s.col_start;
        }
        self.search_next(true, 1);
    }

    /// The word under the cursor and the source byte it starts at.
    fn word_under_cursor(&self) -> Option<(String, usize)> {
        let p = self.page.as_ref()?;
        let src = &p.doc.source;
        let byte = char_at(&p.rendered.srcmap, src, self.cursor.row, self.cursor.col)?;
        let at = src[byte..].chars().next()?;
        if !is_word(at) {
            return None;
        }
        let start = src[..byte]
            .char_indices()
            .rev()
            .take_while(|&(_, c)| is_word(c))
            .last()
            .map_or(byte, |(i, _)| i);
        let end = src[byte..]
            .char_indices()
            .find(|&(_, c)| !is_word(c))
            .map_or(src.len(), |(i, _)| byte + i);
        Some((src[start..end].to_string(), start))
    }

    /// `n` (`same` = true) / `N`, `count` times.
    /// `n` / `N` have a previous search to repeat.
    pub(crate) fn can_search_next(&self) -> bool {
        !self.search.pattern.is_empty()
    }

    pub(super) fn search_next(&mut self, same: bool, count: usize) {
        if !self.can_search_next() {
            self.set_status("No previous search");
            return;
        }
        if self.search.hits.is_empty() {
            self.set_status(format!("Pattern not found: {}", self.search.pattern));
            return;
        }
        let forward = self.search.forward == same;
        let Some(p) = &self.page else { return };
        let (map, src) = (&p.rendered.srcmap, p.doc.source.as_str());
        // Compare screen positions, not source bytes: a blank or rule row
        // has no byte under it but still sits between the hits above and
        // below it (vim searches onward from the cursor position).
        let mut starts: Vec<(usize, usize)> = self
            .search
            .hits
            .iter()
            .filter_map(|h| {
                hit_spans(map, src, &(h.start..h.start + 1))
                    .first()
                    .copied()
            })
            .map(|s| (s.row, s.col_start))
            .collect();
        starts.sort_unstable();
        starts.dedup();
        let (Some(&first), Some(&last)) = (starts.first(), starts.last()) else {
            return;
        };
        let mut cur = (self.cursor.row, self.cursor.col);
        let mut wrapped = false;
        for _ in 0..count.max(1) {
            let next = if forward {
                starts.iter().find(|&&h| h > cur)
            } else {
                starts.iter().rev().find(|&&h| h < cur)
            };
            cur = *next.unwrap_or_else(|| {
                wrapped = true;
                if forward { &first } else { &last }
            });
        }
        self.cursor.row = cur.0;
        self.set_col(cur.1);
        if wrapped {
            self.set_status(if forward {
                "search hit BOTTOM, continuing at TOP"
            } else {
                "search hit TOP, continuing at BOTTOM"
            });
        } else {
            self.status.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smartcase_and_utf8() {
        assert_eq!(
            find_all("Foo foo FOO", "foo", false),
            vec![0..3, 4..7, 8..11]
        );
        assert_eq!(find_all("Foo foo FOO", "Foo", false), vec![0..3]);
        assert_eq!(find_all("é É x", "é", false), vec![0..2, 3..5]);
        assert!(find_all("abc", "", false).is_empty());
    }

    #[test]
    fn char_at_lands_on_a_char_boundary_when_columns_are_scaled() {
        use crate::render::Segment;
        // Six bytes of "ééé" drawn on two columns (as converted math is):
        // the widths do not add up, so the column is scaled to a byte.
        let src = "ééé";
        let seg = |cols| Segment {
            src: 0..src.len(),
            span: ScreenSpan {
                row: 0,
                col_start: 0,
                col_end: cols,
            },
            link: None,
        };
        let map = SrcMap {
            segments: vec![seg(4)],
        };
        assert_eq!(map.source_at(0, 1), Some(1), "scaled byte inside \"é\"");
        assert_eq!(char_at(&map, src, 0, 1), Some(0));
        assert_eq!(char_at(&map, src, 0, 3), Some(4));
        // Widths add up: each column maps to its own grapheme.
        let map = SrcMap {
            segments: vec![seg(3)],
        };
        assert_eq!(char_at(&map, src, 0, 1), Some(2));
        assert_eq!(char_at(&map, src, 0, 9), Some(4), "past the end snaps");
    }

    #[test]
    fn whole_word() {
        assert_eq!(
            find_all("foo food _foo foo.", "foo", true),
            vec![0..3, 14..17]
        );
        assert_eq!(find_all("Foo foo", "Foo", true), vec![0..3, 4..7]);
    }
}
