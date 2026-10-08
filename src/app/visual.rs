//! Visual mode (`v`, `V`, `C-v`, `gv`) and the yank operator (`y{motion}`,
//! `yy`, `Y`, `yf`, `yF`, `yu`).
//!
//! What is copied is source text, so a yanked paragraph pastes as markdown:
//! - charwise: the source bytes from the first selected drawn grapheme to
//!   the last one, inclusive. Hidden markup at the edges (the `**` before
//!   the first drawn character) is left out; markup between selected
//!   characters is kept.
//! - linewise: the whole source lines the selected rows were drawn from,
//!   each with its newline. Rows drawn from no source (blank separators)
//!   add nothing.
//! - blockwise: the rendered text of the rectangle (source columns do not
//!   line up with rendered ones), one row per line, trailing blanks
//!   trimmed.

use std::ops::Range;

use ratatui::text::Line;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::keys::{Action, KeyResult};
use super::{App, Cursor, Mode};
use crate::render::{ScreenSpan, Segment};

/// The kind of a visual selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualKind {
    /// `v`
    Char,
    /// `V`
    Line,
    /// `C-v`
    Block,
}

/// Visual-mode and yank-operator actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualAction {
    /// `v` / `V` / `C-v`: start (or switch to) a selection of this kind.
    Start(VisualKind),
    /// `gv`: reselect the last selection.
    Reselect,
    /// `Esc` / `C-c` in visual mode.
    Cancel,
    /// `o`: swap the anchor and the cursor.
    Swap,
    /// `y` in visual mode.
    Yank,
    /// `Y` in visual mode: whole lines.
    YankLines,
    /// `y` in normal mode: wait for a motion (the typed count).
    OpStart(Option<usize>),
    /// `yy` / `Y`: this many rows' source lines.
    OpLines(usize),
    /// `yf` (absolute, true) / `yF` (relative to the tree root).
    OpPath(bool),
    /// `yu`: the link target under the cursor, else the file path.
    OpLink,
    /// A key after `y` that is not a motion.
    OpCancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Selection {
    kind: VisualKind,
    anchor: Cursor,
    cursor: Cursor,
}

/// Visual and operator-pending state.
#[derive(Debug, Clone, Default)]
pub(super) struct VisualState {
    /// Anchor of the active selection (in `Mode::Visual`).
    anchor: Cursor,
    /// The last selection, for `gv`.
    last: Option<Selection>,
    /// Count typed before `y`.
    op_count: Option<usize>,
}

impl VisualState {
    /// A selection exists for `gv`.
    pub(super) fn has_last(&self) -> bool {
        self.last.is_some()
    }
}

/// How a motion extends an operator range (vim `:help exclusive`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MotionKind {
    Exclusive,
    Inclusive,
    Linewise,
}

/// Actions that move the cursor and so extend a selection.
fn is_motion(a: Action) -> bool {
    use Action as A;
    matches!(
        a,
        A::Left(_)
            | A::Right(_)
            | A::Down(_)
            | A::Up(_)
            | A::LineStart
            | A::LineEnd(_)
            | A::WordForward(_)
            | A::WordBackward(_)
            | A::WordEnd(_)
            | A::ParagraphForward(_)
            | A::ParagraphBackward(_)
            | A::GotoTop(_)
            | A::GotoBottom(_)
            | A::ScreenTop(_)
            | A::ScreenBottom(_)
            | A::ScreenMiddle
            | A::HalfPageDown
            | A::HalfPageUp
            | A::PageDown
            | A::PageUp
            | A::SearchNext(..)
            | A::SearchWord(_)
            | A::GotoMark(_)
            | A::LinkMotion(..)
            | A::LinkRepeat(..)
            | A::HeadingMotion(..)
    )
}

/// Scrolling that may move the cursor: fine in visual mode, not after `y`.
fn is_scroll(a: Action) -> bool {
    use Action as A;
    matches!(
        a,
        A::LineDown | A::LineUp | A::CenterCursor | A::CursorToTop | A::CursorToBottom
    )
}

fn motion_kind(a: Action) -> MotionKind {
    use Action as A;
    match a {
        A::Down(_)
        | A::Up(_)
        | A::GotoTop(_)
        | A::GotoBottom(_)
        | A::ScreenTop(_)
        | A::ScreenBottom(_)
        | A::ScreenMiddle
        | A::HalfPageDown
        | A::HalfPageUp
        | A::PageDown
        | A::PageUp
        | A::GotoMark(_) => MotionKind::Linewise,
        A::WordEnd(_) | A::LineEnd(_) => MotionKind::Inclusive,
        _ => MotionKind::Exclusive,
    }
}

/// One drawn grapheme of a rendered row: its column, width and byte range
/// in the row's text.
struct Grapheme {
    col: usize,
    w: usize,
    text: Range<usize>,
}

fn line_text(line: &Line<'_>) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// Drawn graphemes of `text` (zero-width ones skipped, as motions do).
fn graphemes(text: &str) -> Vec<Grapheme> {
    let mut col = 0;
    let mut out = Vec::new();
    for (i, g) in text.grapheme_indices(true) {
        let w = g.width();
        if w == 0 {
            continue;
        }
        out.push(Grapheme {
            col,
            w,
            text: i..i + g.len(),
        });
        col += w;
    }
    out
}

fn ordered(a: Cursor, b: Cursor) -> (Cursor, Cursor) {
    if (a.row, a.col) <= (b.row, b.col) {
        (a, b)
    } else {
        (b, a)
    }
}

/// `Copied …` status text.
fn copied(text: &str) -> String {
    let lines = text.lines().count();
    if text.contains('\n') {
        return format!("Copied {lines} line{}", if lines == 1 { "" } else { "s" });
    }
    let mut short: String = text.chars().take(40).collect();
    if text.chars().nth(40).is_some() {
        short.push('…');
    }
    format!("Copied {short}")
}

impl App {
    /// Keys after `y`: `yy`, `yf`, `yF`, `yu`, or a count and a motion.
    pub(super) fn op_keymap(&self, keys: &[crossterm::event::KeyEvent]) -> KeyResult {
        use VisualAction as V;
        use crossterm::event::{KeyCode, KeyModifiers};
        let count = match (self.visual.op_count, self.count) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(1).saturating_mul(b.unwrap_or(1))),
        };
        let act = |v| KeyResult::Action(Action::Visual(v));
        if let [k] = keys {
            let plain = !k.modifiers.contains(KeyModifiers::CONTROL);
            match k.code {
                KeyCode::Esc => return act(V::OpCancel),
                KeyCode::Char('y' | 'Y') if plain => return act(V::OpLines(count.unwrap_or(1))),
                KeyCode::Char('f') if plain => return act(V::OpPath(true)),
                KeyCode::Char('F') if plain => return act(V::OpPath(false)),
                KeyCode::Char('u') if plain => return act(V::OpLink),
                KeyCode::Char('0') if plain && self.count.is_none() => {
                    return KeyResult::Action(Action::LineStart);
                }
                _ => {}
            }
        }
        match self.normal_keymap_with(keys, count) {
            KeyResult::Action(a) if is_motion(a) => KeyResult::Action(a),
            KeyResult::Action(_) | KeyResult::None => act(V::OpCancel),
            r => r,
        }
    }

    /// Keys in visual mode: motions extend the selection.
    pub(super) fn visual_keymap(&self, keys: &[crossterm::event::KeyEvent]) -> KeyResult {
        use VisualAction as V;
        use crossterm::event::{KeyCode, KeyModifiers};
        let act = |v| KeyResult::Action(Action::Visual(v));
        if let [k] = keys {
            let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
            match k.code {
                KeyCode::Esc => return act(V::Cancel),
                KeyCode::Char('v') if ctrl => return act(V::Start(VisualKind::Block)),
                KeyCode::Char(c) if !ctrl => match c {
                    'y' => return act(V::Yank),
                    'Y' => return act(V::YankLines),
                    'o' => return act(V::Swap),
                    'v' => return act(V::Start(VisualKind::Char)),
                    'V' => return act(V::Start(VisualKind::Line)),
                    _ => {}
                },
                _ => {}
            }
        }
        match self.normal_keymap_with(keys, self.count) {
            KeyResult::Action(a) if is_motion(a) || is_scroll(a) => KeyResult::Action(a),
            KeyResult::Action(_) => KeyResult::None,
            r => r,
        }
    }

    /// Run an action from a key: after `y` a motion becomes `y{motion}`.
    pub(super) fn dispatch(&mut self, a: Action) {
        if self.mode == Mode::OpPending && is_motion(a) {
            self.op_motion(a);
        } else {
            self.apply(a);
        }
    }

    /// The visual selection kind, when visual mode is on.
    pub fn visual_kind(&self) -> Option<VisualKind> {
        match self.mode {
            Mode::Visual(k) => Some(k),
            _ => None,
        }
    }

    /// Status-line mode label: `VISUAL  `, `V-LINE  `, `V-BLOCK  `, or "".
    pub fn visual_label(&self) -> &'static str {
        match self.visual_kind() {
            Some(VisualKind::Char) => "VISUAL  ",
            Some(VisualKind::Line) => "V-LINE  ",
            Some(VisualKind::Block) => "V-BLOCK  ",
            None => "",
        }
    }

    /// `C-c`: cancel visual mode or a pending `y`. False when neither is on.
    pub(super) fn visual_interrupt(&mut self) -> bool {
        match self.mode {
            Mode::Visual(_) => self.visual_cancel(),
            Mode::OpPending => self.mode = Mode::Normal,
            _ => return false,
        }
        self.pending.clear();
        self.count = None;
        true
    }

    fn selection(&self) -> Option<Selection> {
        Some(Selection {
            kind: self.visual_kind()?,
            anchor: self.visual.anchor,
            cursor: self.cursor,
        })
    }

    fn visual_cancel(&mut self) {
        self.visual.last = self.selection();
        self.mode = Mode::Normal;
    }

    pub(super) fn visual_action(&mut self, a: VisualAction) {
        use VisualAction as V;
        match a {
            V::Start(kind) => match self.mode {
                Mode::Visual(k) if k == kind => self.visual_cancel(),
                Mode::Visual(_) => self.mode = Mode::Visual(kind),
                _ if self.page.is_some() => {
                    self.visual.anchor = self.cursor;
                    self.mode = Mode::Visual(kind);
                }
                _ => {}
            },
            V::Reselect => {
                let Some(s) = self.visual.last else { return };
                self.visual.anchor = s.anchor;
                self.mode = Mode::Visual(s.kind);
                self.cursor = s.cursor;
                self.want_col = s.cursor.col;
                self.clamp_cursor();
            }
            V::Cancel => self.visual_cancel(),
            V::Swap => {
                std::mem::swap(&mut self.visual.anchor, &mut self.cursor);
                self.want_col = self.cursor.col;
                self.clamp_cursor();
            }
            V::Yank | V::YankLines => {
                let Some(start) = self.yank_selection(a == V::YankLines) else {
                    return;
                };
                self.visual_cancel();
                self.cursor = start;
                self.set_col(start.col);
            }
            V::OpStart(count) => {
                if self.page.is_some() {
                    self.visual.op_count = count;
                    self.mode = Mode::OpPending;
                }
            }
            V::OpLines(n) => {
                self.mode = Mode::Normal;
                let row = self.cursor.row;
                let last = (row + n.max(1) - 1).min(self.last_row());
                let text = self.lines_text(row, last);
                self.copy_text(text);
            }
            V::OpPath(absolute) => {
                self.mode = Mode::Normal;
                self.yank_path(absolute);
            }
            V::OpLink => {
                self.mode = Mode::Normal;
                self.yank();
            }
            V::OpCancel => self.mode = Mode::Normal,
        }
    }

    /// Copy the active selection (whole lines if `lines`) and return where
    /// visual `y` puts the cursor. Visual mode stays on (mouse yanks keep
    /// the selection visible; `V::Yank` cancels after).
    pub(super) fn yank_selection(&mut self, lines: bool) -> Option<Cursor> {
        let s = self.selection()?;
        let (lo, hi) = ordered(s.anchor, s.cursor);
        let text = match s.kind {
            _ if lines => self.lines_text(lo.row, hi.row),
            VisualKind::Line => self.lines_text(lo.row, hi.row),
            VisualKind::Char => self.char_text(lo, hi),
            VisualKind::Block => {
                let (c1, c2) = (
                    s.anchor.col.min(s.cursor.col),
                    s.anchor.col.max(s.cursor.col),
                );
                self.block_text(lo.row..=hi.row, c1, c2)
            }
        };
        self.copy_text(text);
        Some(match s.kind {
            VisualKind::Block => Cursor {
                row: lo.row,
                col: s.anchor.col.min(s.cursor.col),
            },
            _ => lo,
        })
    }

    /// Start a `kind` selection from `anchor` to `cursor` (the mouse).
    pub(super) fn visual_select(&mut self, kind: VisualKind, anchor: Cursor, cursor: Cursor) {
        if self.page.is_none() {
            return;
        }
        self.visual.anchor = anchor;
        self.mode = Mode::Visual(kind);
        self.cursor = cursor;
        self.set_col(cursor.col);
    }

    /// Leave visual mode as `Esc` does.
    pub(super) fn visual_leave(&mut self) {
        if self.visual_kind().is_some() {
            self.visual_cancel();
        }
    }

    /// `y{motion}`: run the motion, yank what it covered, put the cursor at
    /// the start of the range (as vim).
    pub(super) fn op_motion(&mut self, motion: Action) {
        self.mode = Mode::Normal;
        let (start, want) = (self.cursor, self.want_col);
        self.apply(motion);
        let end = self.cursor;
        let (lo, hi) = ordered(start, end);
        let mut linewise = motion_kind(motion) == MotionKind::Linewise;
        let text = match motion_kind(motion) {
            MotionKind::Linewise => self.lines_text(lo.row, hi.row),
            MotionKind::Inclusive => self.char_text(lo, hi),
            MotionKind::Exclusive => {
                if lo == hi {
                    self.cursor = start;
                    self.want_col = want;
                    return;
                }
                // `:help exclusive-linewise`: ending in column 0 of a later
                // row from at or before the first non-blank is linewise.
                if hi.col == 0 && hi.row > lo.row && lo.col <= self.first_non_blank(lo.row) {
                    linewise = true;
                    self.lines_text(lo.row, hi.row - 1)
                } else {
                    let hi = self.before(hi, lo.row);
                    self.char_text(lo, hi)
                }
            }
        };
        if linewise {
            self.cursor.row = lo.row;
            self.want_col = want;
            self.clamp_cursor();
        } else {
            self.cursor = lo;
            self.set_col(lo.col);
        }
        self.copy_text(text);
    }

    /// Column of the first drawn non-blank grapheme of `row` (0 if none).
    fn first_non_blank(&self, row: usize) -> usize {
        let Some(line) = self.page.as_ref().and_then(|p| p.rendered.lines.get(row)) else {
            return 0;
        };
        let text = line_text(line);
        graphemes(&text)
            .iter()
            .find(|g| !text[g.text.clone()].trim().is_empty())
            .map_or(0, |g| g.col)
    }

    /// The last position before the exclusive end `hi`. At column 0 of a
    /// later row it is the end of the previous row (vim's exclusive rule).
    fn before(&self, hi: Cursor, lo_row: usize) -> Cursor {
        if hi.col > 0 {
            Cursor {
                row: hi.row,
                col: hi.col - 1,
            }
        } else if hi.row > lo_row {
            Cursor {
                row: hi.row - 1,
                col: usize::MAX,
            }
        } else {
            hi
        }
    }

    fn copy_text(&mut self, text: String) {
        if text.is_empty() {
            self.set_status("Nothing to yank");
            return;
        }
        match self.clipboard.copy(&text) {
            Ok(()) => self.set_status(copied(&text)),
            Err(e) => self.set_status(format!("copy failed: {e:#}")),
        }
    }

    fn row_segments(&self, row: usize) -> &[Segment] {
        let Some(p) = &self.page else { return &[] };
        let segs = &p.rendered.srcmap.segments;
        let lo = segs.partition_point(|s| s.span.row < row);
        let hi = segs.partition_point(|s| s.span.row <= row);
        &segs[lo..hi]
    }

    /// Source bytes of the grapheme drawn at `g` on `row`, on grapheme
    /// boundaries. Graphemes of a segment map one to one onto its source
    /// graphemes; when the counts differ (escapes, entities, reflowed
    /// newlines) the index is scaled.
    fn grapheme_bytes(
        &self,
        row: usize,
        text: &str,
        gs: &[Grapheme],
        i: usize,
    ) -> Option<Range<usize>> {
        let p = self.page.as_ref()?;
        let g = &gs[i];
        let seg = self
            .row_segments(row)
            .iter()
            .find(|s| (s.span.col_start..s.span.col_end).contains(&g.col))?;
        let drawn: Vec<&Grapheme> = gs
            .iter()
            .filter(|h| (seg.span.col_start..seg.span.col_end).contains(&h.col))
            .collect();
        let k = drawn.iter().position(|h| h.col == g.col)?;
        let src = p.doc.source.get(seg.src.clone())?;
        let sg: Vec<(usize, &str)> = src.grapheme_indices(true).collect();
        if sg.is_empty() {
            return None;
        }
        let drawn_text: String = drawn.iter().map(|h| &text[h.text.clone()]).collect();
        let (a, b) = if sg.len() == drawn.len() || drawn_text == src {
            (k, k + 1)
        } else {
            let a = k * sg.len() / drawn.len();
            (a, ((k + 1) * sg.len() / drawn.len()).max(a + 1))
        };
        let a = a.min(sg.len() - 1);
        let b = b.min(sg.len()).max(a + 1);
        let start = seg.src.start + sg[a].0;
        let end = seg.src.start + sg[b - 1].0 + sg[b - 1].1.len();
        Some(start..end)
    }

    /// Charwise: source bytes from the first to the last drawn grapheme in
    /// `lo..=hi` (inclusive positions; `col` usize::MAX = end of row).
    fn char_text(&self, lo: Cursor, hi: Cursor) -> String {
        let Some(p) = &self.page else {
            return String::new();
        };
        let mut range: Option<Range<usize>> = None;
        for row in lo.row..=hi.row.min(self.last_row()) {
            let Some(line) = p.rendered.lines.get(row) else {
                break;
            };
            let text = line_text(line);
            let gs = graphemes(&text);
            let selected: Vec<usize> = (0..gs.len())
                .filter(|&i| {
                    let g = &gs[i];
                    !((row == lo.row && g.col + g.w <= lo.col) || (row == hi.row && g.col > hi.col))
                })
                .collect();
            // Drawn graphemes without source (prefixes, bullets) map to
            // nothing; take the first and last that do.
            let first = selected
                .iter()
                .find_map(|&i| self.grapheme_bytes(row, &text, &gs, i));
            let last = selected
                .iter()
                .rev()
                .find_map(|&i| self.grapheme_bytes(row, &text, &gs, i));
            for b in first.into_iter().chain(last) {
                {
                    range = Some(match range {
                        None => b,
                        Some(r) => r.start.min(b.start)..r.end.max(b.end),
                    });
                }
            }
        }
        range.map_or_else(String::new, |r| p.doc.source[r].to_string())
    }

    /// Linewise: the whole source lines rows `lo..=hi` were drawn from.
    fn lines_text(&self, lo: usize, hi: usize) -> String {
        let Some(p) = &self.page else {
            return String::new();
        };
        let src = &p.doc.source;
        let mut range: Option<Range<usize>> = None;
        for row in lo..=hi.min(self.last_row()) {
            let segs = self.row_segments(row);
            let (Some(a), Some(b)) = (
                segs.iter().map(|s| s.src.start).min(),
                segs.iter().map(|s| s.src.end).max(),
            ) else {
                continue;
            };
            range = Some(match range {
                None => a..b,
                Some(r) => r.start.min(a)..r.end.max(b),
            });
        }
        let Some(r) = range else {
            return String::new();
        };
        let start = src[..r.start].rfind('\n').map_or(0, |i| i + 1);
        let last = r.end.saturating_sub(1).max(r.start);
        let end = src[last..].find('\n').map_or(src.len(), |i| last + i + 1);
        let mut text = src[start..end].to_string();
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text
    }

    /// Blockwise: the rendered text of columns `c1..=c2` on each row.
    fn block_text(&self, rows: std::ops::RangeInclusive<usize>, c1: usize, c2: usize) -> String {
        let Some(p) = &self.page else {
            return String::new();
        };
        let out: Vec<String> = rows
            .filter(|&r| r < p.rendered.lines.len())
            .map(|r| {
                let text = line_text(&p.rendered.lines[r]);
                graphemes(&text)
                    .iter()
                    .filter(|g| g.col + g.w > c1 && g.col <= c2)
                    .map(|g| &text[g.text.clone()])
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect();
        out.join("\n")
    }

    /// Screen spans of the visual selection, for drawing.
    pub fn selection_spans(&self) -> Vec<ScreenSpan> {
        let (Some(s), Some(p)) = (self.selection(), &self.page) else {
            return Vec::new();
        };
        let (lo, hi) = ordered(s.anchor, s.cursor);
        let (c1, c2) = (
            s.anchor.col.min(s.cursor.col),
            s.anchor.col.max(s.cursor.col),
        );
        let mut out = Vec::new();
        for row in lo.row..=hi.row.min(self.last_row()) {
            let Some(line) = p.rendered.lines.get(row) else {
                break;
            };
            let gs = graphemes(&line_text(line));
            let keep = |g: &&Grapheme| match s.kind {
                VisualKind::Line => true,
                VisualKind::Block => g.col + g.w > c1 && g.col <= c2,
                VisualKind::Char => {
                    !((row == lo.row && g.col + g.w <= lo.col) || (row == hi.row && g.col > hi.col))
                }
            };
            let sel: Vec<&Grapheme> = gs.iter().filter(keep).collect();
            let span = match (sel.first(), sel.last()) {
                (Some(a), Some(b)) => ScreenSpan {
                    row,
                    col_start: a.col,
                    col_end: b.col + b.w,
                },
                // An empty row: one cell, so the selection shows.
                _ if s.kind != VisualKind::Block => ScreenSpan {
                    row,
                    col_start: 0,
                    col_end: 1,
                },
                _ => continue,
            };
            out.push(span);
        }
        out
    }
}
