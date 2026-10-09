//! A text box: the comment prompt's editing model, free of ratatui.
//!
//! COPY RULE: this file is byte-identical in debrief
//! (`crates/debrief/src/app/textbox.rs`) and ramble (`src/app/textbox.rs`).
//! Change both copies together, tests included, and `cmp` them.
//!
//! Adapted from tuicr (https://github.com/agavra/tuicr), src/text_edit.rs,
//! src/handler.rs and src/ui/comment_panel.rs.
//! Copyright (c) 2025 tuicr contributors. MIT License; see THIRD-PARTY.md.
//!
//! The text is stored with `\n` between lines. The cursor is a byte offset
//! that always sits on a grapheme boundary; widths are display cells
//! (`unicode-width`). Words are runs of non-whitespace. Rows are the
//! lines hard-wrapped at a display width (not word-wrapped); Up/Down move
//! by row and keep the column they started from.

use std::cell::Cell;
use std::ops::Range;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// One editing key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edit {
    Insert(char),
    /// Backspace: the grapheme before the cursor.
    Backspace,
    /// Delete: the grapheme after the cursor.
    Delete,
    /// `C-w`, `Alt-Backspace`: back to the start of the word.
    DeleteWordBack,
    /// `C-u`: back to the start of the line.
    DeleteToLineStart,
    Left,
    Right,
    /// `Alt-b`, `C-Left`, `Alt-Left`.
    WordLeft,
    /// `Alt-f`, `C-Right`, `Alt-Right`.
    WordRight,
    /// `Home`, `C-a`.
    Home,
    End,
    Up,
    Down,
}

impl Edit {
    /// The edit `key` stands for. `None` leaves the key to the caller
    /// (Enter, Esc, Tab, `C-e`, ...).
    pub fn from_key(key: &KeyEvent) -> Option<Edit> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        Some(match key.code {
            KeyCode::Char('w') if ctrl => Edit::DeleteWordBack,
            KeyCode::Char('u') if ctrl => Edit::DeleteToLineStart,
            KeyCode::Char('a') if ctrl => Edit::Home,
            KeyCode::Char('b') if alt => Edit::WordLeft,
            KeyCode::Char('f') if alt => Edit::WordRight,
            KeyCode::Char(c) if !ctrl && !alt && !c.is_control() => Edit::Insert(c),
            KeyCode::Backspace if alt || ctrl => Edit::DeleteWordBack,
            KeyCode::Backspace => Edit::Backspace,
            KeyCode::Delete => Edit::Delete,
            KeyCode::Left if alt || ctrl => Edit::WordLeft,
            KeyCode::Right if alt || ctrl => Edit::WordRight,
            KeyCode::Left => Edit::Left,
            KeyCode::Right => Edit::Right,
            KeyCode::Home => Edit::Home,
            KeyCode::End => Edit::End,
            KeyCode::Up => Edit::Up,
            KeyCode::Down => Edit::Down,
            _ => return None,
        })
    }
}

/// Text with a cursor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextBox {
    text: String,
    /// Byte offset, on a grapheme boundary.
    cursor: usize,
    /// The column Up/Down aim for; any other edit clears it.
    goal_col: Option<usize>,
    /// The first byte the one-line view shows (see [`TextBox::line_view`]).
    scroll: Cell<usize>,
}

impl TextBox {
    pub fn new() -> Self {
        Self::default()
    }

    /// `text` (CRLF and CR as LF) with the cursor at its end.
    pub fn from_text(text: &str) -> Self {
        let text = normalize_newlines(text);
        let cursor = text.len();
        Self {
            text,
            cursor,
            ..Self::default()
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn into_text(self) -> String {
        self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Apply `edit`; `width` is the wrap width for Up/Down (0: no wrap).
    pub fn apply(&mut self, edit: Edit, width: usize) {
        match edit {
            Edit::Insert(c) => self.insert(c),
            Edit::Backspace => self.backspace(),
            Edit::Delete => self.delete(),
            Edit::DeleteWordBack => self.delete_word_back(),
            Edit::DeleteToLineStart => self.delete_to_line_start(),
            Edit::Left => self.left(),
            Edit::Right => self.right(),
            Edit::WordLeft => self.word_left(),
            Edit::WordRight => self.word_right(),
            Edit::Home => self.home(),
            Edit::End => self.end(),
            Edit::Up => self.up(width),
            Edit::Down => self.down(width),
        }
    }

    /// Put the cursor at `at` (moved back to a grapheme boundary) and clear
    /// the goal column.
    fn set_cursor(&mut self, at: usize) {
        self.cursor = floor_boundary(&self.text, at);
        self.goal_col = None;
    }

    /// After an edit, put the cursor at `at` moved forward to a grapheme
    /// boundary of the new text: graphemes can join across the edit point
    /// (a combining mark, ZWJ, regional indicators, Hangul jamo), and the
    /// text before the cursor stays before it.
    fn edited(&mut self, at: usize) {
        self.cursor = ceil_boundary(&self.text, at);
        self.goal_col = None;
    }

    pub fn insert(&mut self, c: char) {
        let mut b = [0; 4];
        self.insert_str(c.encode_utf8(&mut b));
    }

    /// Insert `s` at the cursor, CRLF and CR as LF (a multi-line paste).
    pub fn insert_str(&mut self, s: &str) {
        let s = normalize_newlines(s);
        self.text.insert_str(self.cursor, &s);
        self.edited(self.cursor + s.len());
    }

    /// Insert `s` as one line: newlines and tabs become spaces, other
    /// control characters are dropped (a paste into a one-line prompt).
    pub fn insert_line(&mut self, s: &str) {
        let line: String = normalize_newlines(s)
            .chars()
            .filter_map(|c| match c {
                '\n' | '\t' => Some(' '),
                c if c.is_control() => None,
                c => Some(c),
            })
            .collect();
        self.insert_str(&line);
    }

    pub fn backspace(&mut self) {
        let prev = prev_boundary(&self.text, self.cursor);
        self.text.replace_range(prev..self.cursor, "");
        self.edited(prev);
    }

    pub fn delete(&mut self) {
        let next = next_boundary(&self.text, self.cursor);
        self.text.replace_range(self.cursor..next, "");
        self.edited(self.cursor);
    }

    /// Delete back over whitespace, then over the word before it.
    pub fn delete_word_back(&mut self) {
        let start = word_back_start(&self.text, self.cursor);
        self.text.replace_range(start..self.cursor, "");
        self.edited(start);
    }

    /// Delete back to the start of the cursor's line (not past a newline).
    pub fn delete_to_line_start(&mut self) {
        let start = line_start(&self.text, self.cursor);
        self.text.replace_range(start..self.cursor, "");
        self.edited(start);
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.edited(0);
    }

    pub fn left(&mut self) {
        self.set_cursor(prev_boundary(&self.text, self.cursor));
    }

    pub fn right(&mut self) {
        self.set_cursor(next_boundary(&self.text, self.cursor));
    }

    /// To the start of this word, or of the previous one.
    pub fn word_left(&mut self) {
        self.set_cursor(word_left(&self.text, self.cursor));
    }

    /// To the start of the next word.
    pub fn word_right(&mut self) {
        self.set_cursor(word_right(&self.text, self.cursor));
    }

    /// To the start of the line.
    pub fn home(&mut self) {
        self.set_cursor(line_start(&self.text, self.cursor));
    }

    /// To the end of the line.
    pub fn end(&mut self) {
        self.set_cursor(line_end(&self.text, self.cursor));
    }

    /// One row up at wrap width `width` (0: no wrap).
    pub fn up(&mut self, width: usize) {
        self.vertical(width, false);
    }

    /// One row down at wrap width `width` (0: no wrap).
    pub fn down(&mut self, width: usize) {
        self.vertical(width, true);
    }

    /// The rows at wrap width `width` (0: no wrap), as byte ranges of the
    /// text without their newline. An empty text has one empty row.
    pub fn rows(&self, width: usize) -> Vec<Range<usize>> {
        let mut rows = Vec::new();
        let mut start = 0;
        for line in self.text.split('\n') {
            for seg in wrap_segments(line, width) {
                rows.push(start..start + seg.len());
                start += seg.len();
            }
            start += 1;
        }
        rows
    }

    /// The cursor's row and display column at wrap width `width`. At a
    /// soft-wrap boundary the cursor is on the following row, so the column
    /// equals `width` only at the end of a full last row of a line: leave a
    /// spare cell for it.
    pub fn cursor_cell(&self, width: usize) -> (usize, usize) {
        let rows = self.rows(width);
        let row = cursor_row(&rows, self.cursor);
        (row, self.text[rows[row].start..self.cursor].width())
    }

    /// The one-line view at `width` cells: the visible byte range and the
    /// cursor's column in it. The view scrolls only as far as it must to
    /// keep the cursor's grapheme on screen (at the end, a spare cell for
    /// the cursor), and shows as much of the text as fits.
    pub fn line_view(&self, width: usize) -> (Range<usize>, usize) {
        let t = self.text.as_str();
        if width == 0 {
            return (self.cursor..self.cursor, 0);
        }
        // The cells from `s` through the cursor's grapheme.
        let through = |s: usize| match next_boundary(t, self.cursor) {
            e if e > self.cursor => t[s..e].width(),
            _ => t[s..self.cursor].width() + 1,
        };
        let mut s = floor_boundary(t, self.scroll.get().min(self.cursor));
        while s < self.cursor && through(s) > width {
            s = next_boundary(t, s);
        }
        while s > 0 && t[prev_boundary(t, s)..].width() < width {
            s = prev_boundary(t, s);
        }
        self.scroll.set(s);
        let mut end = s;
        for g in t[s..].graphemes(true) {
            if t[s..end + g.len()].width() > width {
                break;
            }
            end += g.len();
        }
        (s..end, t[s..self.cursor].width())
    }

    /// Up/Down by displayed row, after tuicr's `move_comment_cursor`.
    fn vertical(&mut self, width: usize, down: bool) {
        let rows = self.rows(width);
        let current = cursor_row(&rows, self.cursor);
        let target = if down {
            current.checked_add(1)
        } else {
            current.checked_sub(1)
        };
        let Some(target) = target.filter(|&i| i < rows.len()) else {
            return;
        };
        let column = *self
            .goal_col
            .get_or_insert_with(|| self.text[rows[current].start..self.cursor].width());
        let row = rows[target].clone();
        let text = &self.text[row.clone()];
        // At a soft-wrap boundary the cursor belongs to the next row, so it
        // can sit after a row's text only at a newline or the text's end.
        let end_stop =
            (!rows.get(target + 1).is_some_and(|n| n.start == row.end)).then_some(text.len());
        let byte = text
            .grapheme_indices(true)
            .map(|(b, _)| b)
            .chain(end_stop)
            .rfind(|&b| text[..b].width() <= column)
            .unwrap_or(0);
        self.cursor = row.start + byte;
    }
}

/// CRLF and lone CR as LF.
fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

/// The row holding byte `cursor`: the last row starting at or before it.
fn cursor_row(rows: &[Range<usize>], cursor: usize) -> usize {
    rows.iter().rposition(|r| r.start <= cursor).unwrap_or(0)
}

/// `line` split into pieces at most `width` cells wide, at grapheme
/// boundaries (a grapheme wider than `width` gets a piece of its own).
/// `width` 0, or a line that fits, gives the line itself.
pub fn wrap_segments(line: &str, width: usize) -> Vec<&str> {
    if width == 0 || line.width() <= width {
        return vec![line];
    }
    let mut segs = Vec::new();
    let mut start = 0;
    let mut w = 0;
    for (i, g) in line.grapheme_indices(true) {
        let gw = g.width();
        if w + gw > width && i > start {
            segs.push(&line[start..i]);
            start = i;
            w = 0;
        }
        w += gw;
    }
    segs.push(&line[start..]);
    segs
}

/// The last grapheme boundary at or before `at`.
fn floor_boundary(s: &str, at: usize) -> usize {
    let at = at.min(s.len());
    s.grapheme_indices(true)
        .map(|(i, _)| i)
        .chain(std::iter::once(s.len()))
        .take_while(|&i| i <= at)
        .last()
        .unwrap_or(0)
}

/// The first grapheme boundary at or after `at`.
fn ceil_boundary(s: &str, at: usize) -> usize {
    s.grapheme_indices(true)
        .map(|(i, _)| i)
        .find(|&i| i >= at)
        .unwrap_or(s.len())
}

/// The grapheme boundary before `at` (0 at the start).
fn prev_boundary(s: &str, at: usize) -> usize {
    s.grapheme_indices(true)
        .map(|(i, _)| i)
        .take_while(|&i| i < at)
        .last()
        .unwrap_or(0)
}

/// The grapheme boundary after `at` (the length at the end).
fn next_boundary(s: &str, at: usize) -> usize {
    s.grapheme_indices(true)
        .map(|(i, g)| i + g.len())
        .find(|&e| e > at)
        .unwrap_or(s.len())
}

fn line_start(s: &str, at: usize) -> usize {
    s[..at].rfind('\n').map_or(0, |p| p + 1)
}

fn line_end(s: &str, at: usize) -> usize {
    s[at..].find('\n').map_or(s.len(), |p| at + p)
}

/// Whether grapheme `g` is whitespace: its first char is.
fn is_space(g: &str) -> bool {
    g.chars().next().is_some_and(char::is_whitespace)
}

/// Where `C-w` deletes back to: over whitespace graphemes, then over
/// non-whitespace ones.
fn word_back_start(s: &str, at: usize) -> usize {
    let before: Vec<_> = s
        .grapheme_indices(true)
        .take_while(|&(i, _)| i < at)
        .collect();
    let mut gs = before.into_iter().rev().peekable();
    let mut start = at;
    for space in [true, false] {
        while let Some((i, _)) = gs.next_if(|&(_, g)| is_space(g) == space) {
            start = i;
        }
    }
    start
}

/// The start of the word at or before `at` (over whitespace first).
fn word_left(s: &str, at: usize) -> usize {
    word_back_start(s, at)
}

/// The start of the next word: past this word, then past whitespace.
fn word_right(s: &str, at: usize) -> usize {
    let mut gs = s
        .grapheme_indices(true)
        .skip_while(|&(i, _)| i < at)
        .peekable();
    for space in [false, true] {
        while gs.next_if(|&(_, g)| is_space(g) == space).is_some() {}
    }
    gs.peek().map_or(s.len(), |&(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box holding `text` with the cursor at the `|` in it.
    fn tb(text: &str) -> TextBox {
        let at = text.find('|').expect("a | marks the cursor");
        let mut t = TextBox::from_text(&text.replace('|', ""));
        t.cursor = at;
        t
    }

    /// The text with `|` at the cursor.
    fn show(t: &TextBox) -> String {
        let mut s = t.text().to_string();
        s.insert(t.cursor(), '|');
        s
    }

    fn after(text: &str, edits: &[Edit]) -> String {
        let mut t = tb(text);
        for &e in edits {
            t.apply(e, 0);
        }
        show(&t)
    }

    fn key(code: KeyCode, m: KeyModifiers) -> Option<Edit> {
        Edit::from_key(&KeyEvent::new(code, m))
    }

    #[test]
    fn keys_map_to_edits() {
        let (n, c, a) = (KeyModifiers::NONE, KeyModifiers::CONTROL, KeyModifiers::ALT);
        let s = KeyModifiers::SHIFT;
        assert_eq!(key(KeyCode::Char('x'), n), Some(Edit::Insert('x')));
        assert_eq!(key(KeyCode::Char('X'), s), Some(Edit::Insert('X')));
        assert_eq!(key(KeyCode::Char('w'), c), Some(Edit::DeleteWordBack));
        assert_eq!(key(KeyCode::Backspace, a), Some(Edit::DeleteWordBack));
        assert_eq!(key(KeyCode::Char('u'), c), Some(Edit::DeleteToLineStart));
        assert_eq!(key(KeyCode::Char('a'), c), Some(Edit::Home));
        assert_eq!(key(KeyCode::Char('b'), a), Some(Edit::WordLeft));
        assert_eq!(key(KeyCode::Char('f'), a), Some(Edit::WordRight));
        assert_eq!(key(KeyCode::Left, c), Some(Edit::WordLeft));
        assert_eq!(key(KeyCode::Right, a), Some(Edit::WordRight));
        assert_eq!(key(KeyCode::Backspace, n), Some(Edit::Backspace));
        assert_eq!(key(KeyCode::Delete, n), Some(Edit::Delete));
        assert_eq!(key(KeyCode::Left, n), Some(Edit::Left));
        assert_eq!(key(KeyCode::Right, n), Some(Edit::Right));
        assert_eq!(key(KeyCode::Home, n), Some(Edit::Home));
        assert_eq!(key(KeyCode::End, n), Some(Edit::End));
        assert_eq!(key(KeyCode::Up, n), Some(Edit::Up));
        assert_eq!(key(KeyCode::Down, n), Some(Edit::Down));
        // Left to the caller: C-e (the editor), other ctrl / alt letters,
        // Enter, Esc, Tab.
        assert_eq!(key(KeyCode::Char('e'), c), None);
        assert_eq!(key(KeyCode::Char('x'), c), None);
        assert_eq!(key(KeyCode::Char('x'), a), None);
        assert_eq!(key(KeyCode::Enter, n), None);
        assert_eq!(key(KeyCode::Esc, n), None);
        assert_eq!(key(KeyCode::Tab, n), None);
    }

    #[test]
    fn insert_at_the_cursor() {
        assert_eq!(after("ac|", &[Edit::Insert('d')]), "acd|");
        assert_eq!(after("a|c", &[Edit::Insert('b')]), "ab|c");
        assert_eq!(after("|", &[Edit::Insert('좋')]), "좋|");
        assert_eq!(after("a|b", &[Edit::Insert('🦀')]), "a🦀|b");
    }

    #[test]
    fn a_combining_mark_joins_the_grapheme_before_it() {
        let mut t = tb("e|x");
        t.insert('\u{301}');
        assert_eq!(show(&t), "e\u{301}|x");
        t.backspace();
        assert_eq!(show(&t), "|x", "the whole grapheme goes");
    }

    #[test]
    fn insert_str_keeps_newlines_as_lf() {
        let mut t = tb("a|b");
        t.insert_str("1\r\n2\r3\n");
        assert_eq!(show(&t), "a1\n2\n3\n|b");
        assert_eq!(TextBox::from_text("x\r\ny").text(), "x\ny");
        assert_eq!(TextBox::from_text("xy").cursor(), 2);
    }

    #[test]
    fn insert_line_flattens_newlines_and_tabs() {
        let mut t = tb("see |");
        t.insert_line("first\r\nsecond\tthird\n\u{7}G");
        assert_eq!(show(&t), "see first second third G|");
    }

    #[test]
    fn backspace_takes_one_grapheme() {
        assert_eq!(after("hello|", &[Edit::Backspace]), "hell|");
        assert_eq!(after("좋아|", &[Edit::Backspace]), "좋|");
        assert_eq!(after("좋아|요", &[Edit::Backspace]), "좋|요");
        assert_eq!(after("hi🦀|", &[Edit::Backspace]), "hi|");
        assert_eq!(after("|hello", &[Edit::Backspace]), "|hello");
        // A flag is two code points, one grapheme.
        assert_eq!(after("a🇸🇪|", &[Edit::Backspace]), "a|");
        assert_eq!(after("a\n|b", &[Edit::Backspace]), "a|b");
    }

    #[test]
    fn delete_takes_the_grapheme_after() {
        assert_eq!(after("a|bc", &[Edit::Delete]), "a|c");
        assert_eq!(after("|좋아", &[Edit::Delete]), "|아");
        assert_eq!(after("|🇸🇪x", &[Edit::Delete]), "|x");
        assert_eq!(after("ab|", &[Edit::Delete]), "ab|");
        assert_eq!(after("a|\nb", &[Edit::Delete]), "a|b");
    }

    #[test]
    fn left_and_right_step_over_graphemes() {
        assert_eq!(after("ab|", &[Edit::Left]), "a|b");
        assert_eq!(after("|ab", &[Edit::Left]), "|ab");
        assert_eq!(after("좋|아", &[Edit::Right]), "좋아|");
        assert_eq!(after("좋아|", &[Edit::Right]), "좋아|");
        assert_eq!(after("x🇸🇪|", &[Edit::Left]), "x|🇸🇪");
        assert_eq!(after("|e\u{301}x", &[Edit::Right]), "e\u{301}|x");
        let s = "좋아요";
        let mut t = TextBox::from_text(s);
        let mut seen = vec![t.cursor()];
        for _ in 0..4 {
            t.left();
            seen.push(t.cursor());
        }
        assert_eq!(seen, [9, 6, 3, 0, 0]);
    }

    #[test]
    fn home_and_end_go_to_the_line_ends() {
        assert_eq!(after("ab|c", &[Edit::Home]), "|abc");
        assert_eq!(after("ab|c", &[Edit::End]), "abc|");
        assert_eq!(after("one\ntw|o\nthree", &[Edit::Home]), "one\n|two\nthree");
        assert_eq!(after("one\ntw|o\nthree", &[Edit::End]), "one\ntwo|\nthree");
    }

    #[test]
    fn word_motions_skip_whitespace_runs() {
        assert_eq!(after("hello world|", &[Edit::WordLeft]), "hello |world");
        assert_eq!(after("hello wo|rld", &[Edit::WordLeft]), "hello |world");
        assert_eq!(after("hello |world", &[Edit::WordLeft]), "|hello world");
        assert_eq!(after("  |x", &[Edit::WordLeft]), "|  x");
        assert_eq!(after("|hello world", &[Edit::WordRight]), "hello |world");
        assert_eq!(
            after("hel|lo   world", &[Edit::WordRight]),
            "hello   |world"
        );
        assert_eq!(
            after("hello  | world", &[Edit::WordRight]),
            "hello   |world"
        );
        assert_eq!(after("hello |world", &[Edit::WordRight]), "hello world|");
        assert_eq!(after("안녕 아가|", &[Edit::WordLeft]), "안녕 |아가");
        assert_eq!(after("|안녕 아가", &[Edit::WordRight]), "안녕 |아가");
        assert_eq!(after("a\n|b", &[Edit::WordLeft]), "|a\nb");
    }

    #[test]
    fn edits_leave_the_cursor_on_a_boundary_of_the_new_text() {
        // Deleting what separates two graphemes can join them.
        assert_eq!(after("🇸|x🇪", &[Edit::Delete]), "🇸🇪|");
        assert_eq!(after("🇸|x🇪", &[Edit::Delete, Edit::Backspace]), "|");
        assert_eq!(after("🇸x|🇪", &[Edit::Backspace]), "🇸🇪|");
        assert_eq!(
            after("\u{1100}|x\u{1161}", &[Edit::Delete]),
            "\u{1100}\u{1161}|"
        );
        assert_eq!(
            after("\u{1100}|x\u{1161}", &[Edit::Delete, Edit::Insert('Z')]),
            "\u{1100}\u{1161}Z|"
        );
        assert_eq!(after("a|x\u{301}", &[Edit::Delete]), "a|");
        assert_eq!(after("a|\n\u{301}", &[Edit::Delete]), "a\u{301}|");
        assert_eq!(
            after("e x\n|\u{301}", &[Edit::DeleteWordBack]),
            "e \u{301}|"
        );
        assert_eq!(after("e\n|\u{301}", &[Edit::Backspace]), "e\u{301}|");
        let joined = "\u{1f469}\u{200d}\u{1f4bb}";
        assert_eq!(
            after("\u{1f469}\u{200d}|x\u{1f4bb}", &[Edit::Delete]),
            format!("{joined}|")
        );
        // Inserting can join graphemes on either side of the cursor.
        assert_eq!(after("a|\u{301}", &[Edit::Insert('e')]), "ae\u{301}|");
        assert_eq!(after("|🇪", &[Edit::Insert('🇸')]), "🇸🇪|");
        assert_eq!(
            after("\u{1f469}|\u{1f4bb}", &[Edit::Insert('\u{200d}')]),
            format!("{joined}|")
        );
        let mut t = tb("\u{1f469}|\u{1f4bb}");
        t.insert_line("\u{200d}");
        assert_eq!(show(&t), format!("{joined}|"));
    }

    #[test]
    fn word_motions_step_over_graphemes() {
        // " \u{301}" is one whitespace grapheme.
        assert_eq!(after("|a \u{301}b", &[Edit::WordRight]), "a \u{301}|b");
        assert_eq!(after("a| \u{301}b", &[Edit::WordRight]), "a \u{301}|b");
        assert_eq!(
            after("|a \u{301}b c", &[Edit::WordRight, Edit::WordRight]),
            "a \u{301}b |c"
        );
        assert_eq!(after("a \u{301}b|", &[Edit::WordLeft]), "a \u{301}|b");
        assert_eq!(after("a \u{301}|b", &[Edit::WordLeft]), "|a \u{301}b");
        assert_eq!(after("a \u{301}b|", &[Edit::DeleteWordBack]), "a \u{301}|");
        // Every motion makes progress until the end or the start.
        for s in ["a \u{301}b", " \u{301}\u{301} x\u{301} 🇸🇪 y"] {
            let mut t = TextBox::from_text(s);
            t.home();
            while t.cursor() < s.len() {
                let at = t.cursor();
                t.word_right();
                assert!(t.cursor() > at, "{s:?}: Alt-f stuck at {at}");
            }
            while t.cursor() > 0 {
                let at = t.cursor();
                t.word_left();
                assert!(t.cursor() < at, "{s:?}: Alt-b stuck at {at}");
            }
        }
    }

    #[test]
    fn delete_word_back() {
        assert_eq!(after("hello world|", &[Edit::DeleteWordBack]), "hello |");
        assert_eq!(after("hello   |", &[Edit::DeleteWordBack]), "|");
        assert_eq!(after("hello wo|rld", &[Edit::DeleteWordBack]), "hello |rld");
        assert_eq!(after("|hello", &[Edit::DeleteWordBack]), "|hello");
        assert_eq!(after("안녕 아가브라|", &[Edit::DeleteWordBack]), "안녕 |");
        assert_eq!(after("x 🇸🇪🦀|", &[Edit::DeleteWordBack]), "x |");
    }

    #[test]
    fn delete_to_line_start() {
        assert_eq!(after("hello wo|rld", &[Edit::DeleteToLineStart]), "|rld");
        assert_eq!(after("a\nbc|d", &[Edit::DeleteToLineStart]), "a\n|d");
        assert_eq!(after("a\n|d", &[Edit::DeleteToLineStart]), "a\n|d");
        let mut t = tb("ab|c");
        t.clear();
        assert_eq!(show(&t), "|");
    }

    #[test]
    fn wrap_segments_by_display_width() {
        assert_eq!(wrap_segments("abcdef", 0), ["abcdef"]);
        assert_eq!(wrap_segments("abc", 3), ["abc"]);
        assert_eq!(wrap_segments("abcdefg", 3), ["abc", "def", "g"]);
        // Wide graphemes take two cells and never split.
        assert_eq!(wrap_segments("좋아요", 4), ["좋아", "요"]);
        assert_eq!(wrap_segments("a좋아", 4), ["a좋", "아"]);
        assert_eq!(wrap_segments("좋x", 1), ["좋", "x"]);
        assert_eq!(wrap_segments("🇸🇪🇸🇪", 3), ["🇸🇪", "🇸🇪"]);
        assert_eq!(wrap_segments("", 3), [""]);
    }

    #[test]
    fn rows_wrap_each_line() {
        let t = TextBox::from_text("abcde\n\nxy");
        assert_eq!(t.rows(3), [0..3, 3..5, 6..6, 7..9]);
        assert_eq!(t.rows(0), [0..5, 6..6, 7..9]);
        assert_eq!(TextBox::new().rows(5), vec![Range { start: 0, end: 0 }]);
    }

    #[test]
    fn cursor_cell_counts_display_columns() {
        assert_eq!(tb("ab|cde\nxy").cursor_cell(3), (0, 2));
        // At a soft wrap the cursor is on the next row.
        assert_eq!(tb("abc|de").cursor_cell(3), (1, 0));
        assert_eq!(tb("abc|").cursor_cell(3), (0, 3));
        assert_eq!(tb("abcde\nx|y").cursor_cell(3), (2, 1));
        assert_eq!(tb("좋아|요").cursor_cell(0), (0, 4));
        assert_eq!(tb("좋아|요").cursor_cell(4), (1, 0));
    }

    #[test]
    fn up_and_down_move_by_displayed_row() {
        // Rows at width 3: "abc", "def", "gh", "xy".
        let mut t = tb("ab|cdefgh\nxy");
        t.down(3);
        assert_eq!(show(&t), "abcde|fgh\nxy");
        t.down(3);
        assert_eq!(show(&t), "abcdefgh|\nxy", "the end of a line is a stop");
        t.down(3);
        assert_eq!(show(&t), "abcdefgh\nxy|");
        t.down(3);
        assert_eq!(show(&t), "abcdefgh\nxy|", "no row below");
        t.up(3);
        t.up(3);
        assert_eq!(show(&t), "abcde|fgh\nxy");
        t.left();
        t.up(3);
        assert_eq!(show(&t), "a|bcdefgh\nxy", "a motion resets the goal");
        t.up(3);
        assert_eq!(show(&t), "a|bcdefgh\nxy", "no row above");
    }

    #[test]
    fn up_and_down_keep_the_goal_column_over_a_short_row() {
        let mut t = tb("abcd|e\nx\nabcdef");
        t.down(0);
        assert_eq!(show(&t), "abcde\nx|\nabcdef");
        t.down(0);
        assert_eq!(show(&t), "abcde\nx\nabcd|ef");
        // A soft-wrap boundary is not a stop: the cursor stays on its row.
        let mut t = tb("abcdef|");
        t.up(3);
        assert_eq!(show(&t), "ab|cdef");
        let mut t = tb("abcdef\nx|y");
        t.up(3);
        assert_eq!(show(&t), "abcd|ef\nxy");
        t.up(3);
        assert_eq!(show(&t), "a|bcdef\nxy");
    }

    #[test]
    fn up_and_down_over_logical_lines_without_wrap() {
        let mut t = tb("long li|ne\nab\nanother");
        t.down(0);
        assert_eq!(show(&t), "long line\nab|\nanother");
        t.down(0);
        assert_eq!(show(&t), "long line\nab\nanother|");
    }

    #[test]
    fn up_and_down_land_on_wide_grapheme_boundaries() {
        // Column 3 falls inside the second wide grapheme: land before it.
        let mut t = tb("abc|\n좋아요");
        t.down(0);
        assert_eq!(show(&t), "abc\n좋|아요");
        let mut t = tb("좋아|요\nabcdef");
        t.down(0);
        assert_eq!(show(&t), "좋아요\nabcd|ef");
    }

    #[test]
    fn line_view_scrolls_to_keep_the_cursor_visible() {
        let mut t = TextBox::from_text("abcdefghij");
        // The cursor's cell needs room too.
        assert_eq!(t.line_view(4), (7..10, 3));
        for _ in 0..4 {
            t.left();
        }
        assert_eq!(t.line_view(4), (6..10, 0), "no scroll while visible");
        t.left();
        assert_eq!(t.line_view(4), (5..9, 0));
        t.home();
        assert_eq!(t.line_view(4), (0..4, 0));
        t.end();
        t.delete_word_back();
        assert_eq!(t.line_view(4), (0..0, 0));
        assert_eq!(TextBox::from_text("ab").line_view(10), (0..2, 2));
        assert_eq!(TextBox::from_text("ab").line_view(0), (2..2, 0));
    }

    #[test]
    fn line_view_shows_more_after_a_delete() {
        let mut t = TextBox::from_text("abcdefghij");
        t.line_view(4);
        t.backspace();
        t.backspace();
        assert_eq!(t.line_view(4), (5..8, 3));
        t.clear();
        t.insert_str("xy");
        assert_eq!(t.line_view(4), (0..2, 2));
    }

    #[test]
    fn line_view_with_wide_graphemes() {
        let t = TextBox::from_text("좋아요");
        assert_eq!(t.line_view(5), (3..9, 4));
        let mut t = TextBox::from_text("a좋아요b");
        t.home();
        assert_eq!(t.line_view(5), (0..7, 0), "a좋아 is 5 cells");
        t.end();
        assert_eq!(t.line_view(5), (7..11, 3));
        // The wide grapheme under the cursor fits whole.
        let mut t = TextBox::from_text("abc좋");
        t.left();
        assert_eq!(t.line_view(4), (1..6, 2));
    }
}
