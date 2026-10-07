//! In-page search (`/ ? n N * #`): literal, smartcase, hits kept as source
//! byte ranges so they survive resize and reload.

use std::ops::Range;

use super::{App, Mode};
use crate::render::{ScreenSpan, SrcMap};

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
    /// Pattern before the prompt opened (restored by Esc).
    saved: Option<(String, bool)>,
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

/// Screen spans drawn from `src`, trimmed to the columns of `src` within
/// each segment (a hit across a wrap yields one span per row).
pub(super) fn hit_spans(map: &SrcMap, src: &Range<usize>) -> Vec<ScreenSpan> {
    map.segments
        .iter()
        .filter(|s| s.src.start < src.end && src.start < s.src.end)
        .map(|s| {
            let cols = s.span.col_end - s.span.col_start;
            let len = (s.src.end - s.src.start).max(1);
            let at = |b: usize| s.span.col_start + (b - s.src.start) * cols / len;
            let a = at(src.start.max(s.src.start));
            let b = at(src.end.min(s.src.end)).max(a + 1).min(s.span.col_end);
            ScreenSpan {
                row: s.span.row,
                col_start: a,
                col_end: b,
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
            .flat_map(|h| hit_spans(&p.rendered.srcmap, h))
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
        self.search.saved = Some((self.search.pattern.clone(), self.search.whole_word));
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
            if let Some((pat, ww)) = saved {
                self.search.pattern = pat;
                self.search.whole_word = ww;
            }
            self.refresh_search();
        }
        self.search_next(true, 1);
    }

    /// Esc in the prompt: close it and restore the previous pattern.
    pub(super) fn search_cancel(&mut self) {
        self.mode = Mode::Normal;
        self.search.prompt = None;
        if let Some((pat, ww)) = self.search.saved.take() {
            self.search.pattern = pat;
            self.search.whole_word = ww;
        }
        self.refresh_search();
    }

    /// Esc in normal mode: clear the highlights.
    pub(super) fn search_clear(&mut self) {
        self.search.pattern.clear();
        self.search.hits.clear();
    }

    /// `*` (forward) / `#`: whole-word search for the word under the cursor.
    pub(super) fn search_word(&mut self, forward: bool) {
        let Some(word) = self.word_under_cursor() else {
            self.set_status("No word under cursor");
            return;
        };
        self.search.pattern = word;
        self.search.whole_word = true;
        self.search.forward = forward;
        self.refresh_search();
        self.search_next(true, 1);
    }

    fn word_under_cursor(&self) -> Option<String> {
        let p = self.page.as_ref()?;
        let byte = p
            .rendered
            .srcmap
            .source_at(self.cursor.row, self.cursor.col)?;
        let src = &p.doc.source;
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
        Some(src[start..end].to_string())
    }

    /// `n` (`same` = true) / `N`, `count` times.
    pub(super) fn search_next(&mut self, same: bool, count: usize) {
        if self.search.pattern.is_empty() {
            self.set_status("No previous search");
            return;
        }
        if self.search.hits.is_empty() {
            self.set_status(format!("Pattern not found: {}", self.search.pattern));
            return;
        }
        let forward = self.search.forward == same;
        let Some(p) = &self.page else { return };
        let map = &p.rendered.srcmap;
        // Compare screen positions, not source bytes: a blank or rule row
        // has no byte under it but still sits between the hits above and
        // below it (vim searches onward from the cursor position).
        let mut starts: Vec<(usize, usize)> = self
            .search
            .hits
            .iter()
            .filter_map(|h| hit_spans(map, &(h.start..h.start + 1)).first().copied())
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
    fn whole_word() {
        assert_eq!(
            find_all("foo food _foo foo.", "foo", true),
            vec![0..3, 14..17]
        );
        assert_eq!(find_all("Foo foo", "Foo", true), vec![0..3, 4..7]);
    }
}
