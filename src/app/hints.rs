//! Link and heading motions (`]l [l ; , ]] [[`) and hint labels (`s`).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::keys::{Action, KeyResult};
use super::{App, Mode};
use crate::render::ScreenSpan;

/// Hint label letters, home row first.
pub const HINT_ALPHABET: &str = "asdfghjklqwertyuiopzxcvbnm";

/// Link-motion and hint state.
#[derive(Debug, Clone, Default)]
pub(super) struct Hints {
    /// Direction of the last `]l` (true) / `[l`; `None` before any.
    last: Option<bool>,
    /// Labels shown in hint mode, with the span each sits on.
    labels: Vec<(String, ScreenSpan)>,
    /// Label letters typed so far.
    typed: String,
}

/// Labels for `n` links: single letters up to 26, else two letters each
/// so no label is a prefix of another. At most 26 * 26.
pub fn hint_labels(n: usize) -> Vec<String> {
    let abc: Vec<char> = HINT_ALPHABET.chars().collect();
    if n <= abc.len() {
        return abc[..n].iter().map(|c| c.to_string()).collect();
    }
    (0..n.min(abc.len() * abc.len()))
        .map(|k| [abc[k / abc.len()], abc[k % abc.len()]].iter().collect())
        .collect()
}

/// Keys in hint mode: letters type a label, anything else cancels.
pub(super) fn hint_keymap(keys: &[KeyEvent]) -> KeyResult {
    let Some(key) = keys.last() else {
        return KeyResult::None;
    };
    KeyResult::Action(match key.code {
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => Action::HintInput(c),
        _ => Action::HintCancel,
    })
}

impl App {
    /// Start of each link's drawn text, in screen order. Links with no
    /// drawn text are skipped.
    fn link_starts(&self) -> Vec<ScreenSpan> {
        let Some(p) = self.page.as_ref() else {
            return Vec::new();
        };
        let map = &p.rendered.srcmap;
        let mut v: Vec<ScreenSpan> = p
            .doc
            .links
            .iter()
            .filter_map(|l| map.spans_for(l.text_range.clone()).into_iter().min())
            .collect();
        v.sort();
        v
    }

    /// `]l` (forward) / `[l`, `n` times; remembers the direction for `;`.
    pub(super) fn link_motion(&mut self, forward: bool, n: usize) {
        self.hints.last = Some(forward);
        self.link_step(forward, n);
    }

    /// `;` (same direction) / `,` (opposite) after a link motion.
    pub(super) fn link_repeat(&mut self, same: bool, n: usize) {
        match self.hints.last {
            Some(fwd) => self.link_step(fwd == same, n),
            None => self.set_status("No previous link motion"),
        }
    }

    fn link_step(&mut self, forward: bool, n: usize) {
        let starts = self.link_starts();
        let mut here = (self.cursor.row, self.cursor.col);
        let mut moved = false;
        for _ in 0..n {
            let pos = |s: &ScreenSpan| (s.row, s.col_start);
            let next = if forward {
                starts.iter().find(|s| pos(s) > here)
            } else {
                starts.iter().rev().find(|s| pos(s) < here)
            };
            let Some(s) = next else { break };
            here = pos(s);
            moved = true;
        }
        if !moved {
            self.set_status("No more links");
            return;
        }
        self.cursor.row = here.0.min(self.last_row());
        self.set_col(here.1);
    }

    /// `]]` (forward) / `[[`, `n` times: cursor to a heading's row.
    pub(super) fn heading_motion(&mut self, forward: bool, n: usize) {
        let Some(p) = self.page.as_ref() else {
            return;
        };
        let map = &p.rendered.srcmap;
        let mut rows: Vec<usize> = p
            .doc
            .headings
            .iter()
            .filter_map(|h| map.row_for(h.range.start))
            .collect();
        rows.sort();
        rows.dedup();
        let mut row = self.cursor.row;
        let mut moved = false;
        for _ in 0..n {
            let next = if forward {
                rows.iter().find(|&&r| r > row)
            } else {
                rows.iter().rev().find(|&&r| r < row)
            };
            let Some(&r) = next else { break };
            row = r;
            moved = true;
        }
        if moved {
            self.goto_row(row);
        } else {
            self.set_status("No more headings");
        }
    }

    /// `s`: label every visible link and enter hint mode.
    pub(super) fn hint_start(&mut self) {
        let width = self.render_width_cols();
        let (top, bottom) = (self.scroll, self.scroll + self.viewport_height());
        let mut spans: Vec<ScreenSpan> = match self.page.as_ref() {
            None => Vec::new(),
            Some(p) => p
                .doc
                .links
                .iter()
                .filter_map(|l| {
                    p.rendered
                        .srcmap
                        .spans_for(l.text_range.clone())
                        .into_iter()
                        .filter(|s| (top..bottom).contains(&s.row) && s.col_start < width)
                        .min()
                })
                .collect(),
        };
        spans.sort();
        if spans.is_empty() {
            self.set_status("No links");
            return;
        }
        self.hints.labels = hint_labels(spans.len()).into_iter().zip(spans).collect();
        self.hints.typed.clear();
        self.mode = Mode::Hint;
    }

    /// A letter typed in hint mode: follow on a full label, wait on a
    /// prefix, cancel otherwise.
    pub(super) fn hint_input(&mut self, c: char) {
        self.hints.typed.push(c);
        let typed = self.hints.typed.as_str();
        if let Some(&(_, span)) = self.hints.labels.iter().find(|(l, _)| l == typed) {
            self.hint_cancel();
            self.cursor.row = span.row.min(self.last_row());
            self.set_col(span.col_start);
            self.follow();
        } else if !self.hints.labels.iter().any(|(l, _)| l.starts_with(typed)) {
            self.hint_cancel();
            self.set_status("No such hint");
        }
    }

    /// `Esc` (or any non-letter) in hint mode.
    pub(super) fn hint_cancel(&mut self) {
        self.mode = Mode::Normal;
        self.hints.labels.clear();
        self.hints.typed.clear();
    }

    /// Hint labels still matching the typed prefix: the untyped rest of
    /// each label and the span it sits on. Empty outside hint mode.
    pub fn hints(&self) -> Vec<(&str, ScreenSpan)> {
        if self.mode != Mode::Hint {
            return Vec::new();
        }
        let typed = self.hints.typed.as_str();
        self.hints
            .labels
            .iter()
            .filter_map(|(l, s)| l.strip_prefix(typed).map(|rest| (rest, *s)))
            .collect()
    }

    /// Columns of the content area.
    fn render_width_cols(&self) -> usize {
        self.size.0.saturating_sub(self.sidebar_cols()) as usize
    }
}
