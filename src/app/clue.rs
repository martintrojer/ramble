//! The key clue (spec `2026-10-07-key-clue.md`): after a pause in a
//! pending key sequence, a box lists the keys that can come next. The rows
//! come from `help::BINDINGS` and the launchers, so they never drift from
//! the keymap.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::help::{BINDINGS, Ctx, Group, Tok, placeholder_key, show_key, tokens};
use super::keys::{Action, KeyResult};
use super::{App, Mode, VisualAction};

/// How long a sequence must pause before the clue opens.
pub const CLUE_DELAY: Duration = Duration::from_millis(400);

/// Timing of the clue box, owned by [`App`].
#[derive(Debug, Clone, Default)]
pub(super) struct ClueState {
    /// When the current sequence started.
    since: Option<Instant>,
    /// The pause has passed; stays set while the sequence grows.
    shown: bool,
}

/// One line of the clue box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClueRow {
    pub key: String,
    /// The binding's description, or `+label` for a deeper prefix.
    pub desc: String,
    /// More keys follow this one.
    pub group: bool,
}

/// A key sequence that may follow the typed prefix.
struct Cand {
    toks: Vec<Tok<'static>>,
    desc: String,
    group: Group,
    /// Leading keys the keymap check skips (the synthetic `y` of the
    /// operator).
    skip: usize,
}

/// Rows sharing one next key.
struct Next {
    key: String,
    /// 0 plain character, 1 named or control key, 2 placeholder.
    class: u8,
    order: String,
    leaf: Option<String>,
    /// Group and identity of each longer sequence.
    deeper: Vec<(Group, String)>,
}

fn same_key(a: &KeyEvent, b: &KeyEvent) -> bool {
    let ctrl = |k: &KeyEvent| k.modifiers.contains(KeyModifiers::CONTROL);
    a.code == b.code && ctrl(a) == ctrl(b)
}

fn is_y(t: &Tok) -> bool {
    matches!(t, Tok::Key(k) if k.code == KeyCode::Char('y') && !k.modifiers.contains(KeyModifiers::CONTROL))
}

impl App {
    /// A sequence is under way: keys are pending in a mode with
    /// multi-key sequences, or `y` waits for its motion.
    fn clue_active(&self) -> bool {
        self.mode == Mode::OpPending
            || (!self.pending.is_empty() && matches!(self.mode, Mode::Normal | Mode::Visual(_)))
    }

    /// Called after every key: start timing a new sequence, or forget the
    /// one that ended.
    pub(super) fn clue_sync(&mut self) {
        if !self.clue_active() {
            self.clue = ClueState::default();
        } else if self.clue.since.is_none() {
            self.clue.since = Some(Instant::now());
        }
    }

    /// Called from [`App::tick`]: open the box once the pause is long enough.
    pub(super) fn clue_tick(&mut self, now: Instant) {
        if !self.clue_active() {
            self.clue = ClueState::default();
        } else if let Some(since) = self.clue.since
            && now.saturating_duration_since(since) >= CLUE_DELAY
        {
            self.clue.shown = true;
        }
    }

    /// Whether the clue box is drawn now.
    pub fn clue_visible(&self) -> bool {
        self.config.keys.clue
            && self.clue.shown
            && self.clue_active()
            && !self.clue_rows().is_empty()
    }

    /// The typed keys, as the box title (`Space z`, `C-w`, `y g`). A count
    /// is left out.
    pub fn clue_title(&self) -> String {
        let leader = self.config.keys.leader;
        let mut keys: Vec<String> = Vec::new();
        if self.mode == Mode::OpPending {
            keys.push("y".into());
        }
        keys.extend(self.pending.iter().map(|k| show_key(k, leader)));
        keys.join(" ")
    }

    /// The keys that can follow the pending sequence right now.
    pub fn clue_rows(&self) -> Vec<ClueRow> {
        if !self.clue_active() {
            return Vec::new();
        }
        self.clue_rows_for(&self.pending, true)
    }

    /// Rows after `pending` in the current mode and focus; `avail` false
    /// keeps rows whose action can't do anything right now.
    fn clue_rows_for(&self, pending: &[KeyEvent], avail: bool) -> Vec<ClueRow> {
        let op = self.mode == Mode::OpPending;
        let check = op || matches!(self.mode, Mode::Visual(_));
        let mut cands = self.clue_candidates(avail);
        let mut matched: Vec<(Cand, usize)> = Vec::new();
        let y = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        let op_prefix: Vec<KeyEvent> = std::iter::once(y).chain(pending.iter().copied()).collect();
        for mut c in cands.drain(..) {
            let prefix: &[KeyEvent] = if op && c.toks.first().is_some_and(is_y) {
                c.skip = 1;
                &op_prefix
            } else if op && pending.is_empty() {
                continue;
            } else {
                pending
            };
            let starts = c.toks.len() > prefix.len()
                && prefix.iter().zip(&c.toks).all(|(k, t)| match t {
                    Tok::Key(e) => same_key(k, e),
                    Tok::Placeholder(_) => true,
                });
            if !starts {
                continue;
            }
            if check {
                let keys: Vec<KeyEvent> = c.toks[c.skip..]
                    .iter()
                    .map(|t| match t {
                        Tok::Key(k) => *k,
                        Tok::Placeholder(p) => placeholder_key(p),
                    })
                    .collect();
                let ok = match self.keymap(&keys) {
                    KeyResult::Pending => true,
                    KeyResult::Action(Action::NoMapping)
                    | KeyResult::Action(Action::Visual(VisualAction::OpCancel)) => false,
                    KeyResult::Action(_) => true,
                    KeyResult::Count(_) | KeyResult::None => false,
                };
                if !ok {
                    continue;
                }
            }
            let at = prefix.len();
            matched.push((c, at));
        }
        group_rows(matched, self.config.keys.leader)
    }

    /// Every binding sequence that applies with the current focus.
    fn clue_candidates(&self, avail: bool) -> Vec<Cand> {
        let leader = self.config.keys.leader;
        let mut out = Vec::new();
        for b in BINDINGS {
            if matches!(b.context, Ctx::Help | Ctx::Picker | Ctx::Comment)
                || !self.ctx_shown(b.context)
            {
                continue;
            }
            if (avail && !(b.avail)(self)) || !self.leader_free(b.keys) {
                continue;
            }
            for alt in b.keys.split(", ") {
                let toks = tokens(alt, leader);
                if toks.contains(&Tok::Placeholder("{1-9}")) {
                    continue;
                }
                out.push(Cand {
                    toks,
                    desc: b.desc.to_string(),
                    group: b.group,
                    skip: 0,
                });
            }
        }
        let has_vcs = self.has_vcs_root();
        let ev = |c| Tok::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        for (seq, i) in &self.leader_bindings {
            let l = &self.config.launch[*i];
            let mut desc = l.name.clone();
            if l.needs_vcs && !has_vcs {
                desc.push_str(" (needs a repo)");
            }
            out.push(Cand {
                toks: std::iter::once(leader)
                    .chain(seq.iter().copied())
                    .map(ev)
                    .collect(),
                desc,
                group: Group::Launchers,
                skip: 0,
            });
        }
        out
    }
}

/// Group matched sequences by the key after the prefix (at index `at`).
fn group_rows(matched: Vec<(Cand, usize)>, leader: char) -> Vec<ClueRow> {
    let mut nexts: Vec<Next> = Vec::new();
    for (c, at) in matched {
        let (key, class, order) = match c.toks[at] {
            Tok::Placeholder(p) => (p.to_string(), 2, p.to_string()),
            Tok::Key(k) => {
                let plain = matches!(k.code, KeyCode::Char(_))
                    && !k.modifiers.contains(KeyModifiers::CONTROL);
                let order = match k.code {
                    KeyCode::Char(ch) if plain => ch.to_string(),
                    _ => show_key(&k, leader),
                };
                (show_key(&k, leader), if plain { 0 } else { 1 }, order)
            }
        };
        let i = match nexts.iter().position(|n| n.key == key) {
            Some(i) => i,
            None => {
                nexts.push(Next {
                    key,
                    class,
                    order,
                    leaf: None,
                    deeper: Vec::new(),
                });
                nexts.len() - 1
            }
        };
        let n = &mut nexts[i];
        if c.toks.len() == at + 1 {
            n.leaf.get_or_insert(c.desc);
        } else {
            let id = format!("{:?}", c.toks);
            if !n.deeper.iter().any(|(_, d)| *d == id) {
                n.deeper.push((c.group, id));
            }
        }
    }
    nexts.sort_by(|a, b| (a.class, &a.order).cmp(&(b.class, &b.order)));
    nexts
        .into_iter()
        .map(|n| match n.leaf {
            Some(desc) => ClueRow {
                key: n.key,
                desc,
                group: false,
            },
            None => {
                let g = n.deeper[0].0;
                let desc = if n.deeper.iter().all(|(h, _)| *h == g) {
                    format!("+{}", g.title().to_lowercase())
                } else {
                    format!("+{} keys", n.deeper.len())
                };
                ClueRow {
                    key: n.key,
                    desc,
                    group: true,
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Focus, StartOptions, StartTarget};
    use crate::config::{Config, SidebarMode};

    fn app(dir: &std::path::Path) -> App {
        let path = dir.join("a.md");
        std::fs::write(&path, "# A\n\nSee [b](b.md).\n").unwrap();
        let mut config = Config::default();
        config.sidebar.default = SidebarMode::Files;
        config.lsp.server = vec![];
        let opts = StartOptions {
            target: StartTarget::File(path),
            tree_root: dir.to_path_buf(),
            config,
            review_cache: None,
        };
        App::new(opts, (100, 30)).unwrap()
    }

    fn ch(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn shown_exactly_at_the_delay() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = app(dir.path());
        a.handle_key(ch('g'));
        let t0 = Instant::now();
        a.clue.since = Some(t0);
        a.tick(t0 + Duration::from_millis(399));
        assert!(!a.clue_visible(), "not yet at 399 ms");
        a.tick(t0 + CLUE_DELAY);
        assert!(a.clue_visible(), "shown at 400 ms");
    }

    /// Every key, as the coverage test tries it.
    fn all_keys() -> Vec<KeyEvent> {
        let mut out: Vec<KeyEvent> = (' '..='~').map(ch).collect();
        out.extend(('a'..='z').map(|c| KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)));
        out.extend(
            super::super::help::NAMED
                .iter()
                .map(|(_, code)| KeyEvent::new(*code, KeyModifiers::NONE)),
        );
        out.push(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        out
    }

    /// Every prefix a keymap answers `Pending` for has clue rows, so a new
    /// prefix without help rows fails the build.
    #[test]
    fn every_pending_prefix_has_clue_rows() {
        let dir = tempfile::tempdir().unwrap();
        let normal = app(dir.path());
        let mut sidebar = app(dir.path());
        sidebar.sidebar_action(super::super::sidebar::SidebarAction::FocusSidebar);
        assert_eq!(sidebar.focus(), Focus::Files);
        let keys = all_keys();
        let leader = normal.config.keys.leader;
        let mut checked = 0;
        for a in [&normal, &sidebar] {
            let mut stack: Vec<Vec<KeyEvent>> = keys.iter().map(|k| vec![*k]).collect();
            while let Some(seq) = stack.pop() {
                if a.keymap(&seq) != KeyResult::Pending {
                    continue;
                }
                checked += 1;
                assert!(
                    !a.clue_rows_for(&seq, false).is_empty(),
                    "no clue rows after {:?} (focus {:?})",
                    seq.iter().map(|k| show_key(k, leader)).collect::<Vec<_>>(),
                    a.focus()
                );
                let depth = if seq[0] == ch(leader) { 3 } else { 2 };
                if seq.len() < depth {
                    for k in &keys {
                        let mut s = seq.clone();
                        s.push(*k);
                        stack.push(s);
                    }
                }
            }
        }
        // g z Z [ ] m ' C-w Space, Space z, Space r, gr; sidebar g C-w Space …
        assert!(checked >= 15, "only {checked} prefixes checked");
    }
}
