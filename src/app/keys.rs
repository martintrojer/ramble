//! Key dispatch: [`App::keymap`] turns the pending key sequence into a
//! [`KeyResult`]; [`App::apply`] runs an [`Action`]. A new unit adds
//! `Action` variants, rows in the table for its mode, and one arm in
//! `apply` that calls into its own file.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use super::launch::{LeaderMatch, match_leader};
use super::sidebar::{self, SidebarAction};
use super::{App, Focus, Mode};

/// Cap on a typed count, so `n as isize` never wraps negative.
const MAX_COUNT: usize = 1_000_000;

/// Everything a key can do. `Option<usize>` is the typed count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    // Cursor motions.
    Left(Option<usize>),
    Right(Option<usize>),
    Down(Option<usize>),
    Up(Option<usize>),
    LineStart,
    LineEnd(Option<usize>),
    WordForward(Option<usize>),
    WordBackward(Option<usize>),
    WordEnd(Option<usize>),
    ParagraphForward(Option<usize>),
    ParagraphBackward(Option<usize>),
    /// `gg`: row `count` (1-based), else the first.
    GotoTop(Option<usize>),
    /// `G`: row `count` (1-based), else the last.
    GotoBottom(Option<usize>),
    /// `H`
    ScreenTop(Option<usize>),
    /// `L`
    ScreenBottom(Option<usize>),
    /// `M`
    ScreenMiddle,
    // Scrolling.
    HalfPageDown,
    HalfPageUp,
    PageDown,
    PageUp,
    LineDown,
    LineUp,
    /// `zz`
    CenterCursor,
    /// `zt`
    CursorToTop,
    /// `zb`
    CursorToBottom,
    // Links and history.
    Follow,
    OpenExternal,
    Back,
    Forward,
    // Search.
    /// `/` (true) or `?`: open the search prompt.
    SearchStart(bool),
    /// `n` (true) / `N`.
    SearchNext(bool, Option<usize>),
    /// `*` (true) / `#`.
    SearchWord(bool),
    /// `Esc` in normal mode: clear highlights.
    SearchClear,
    /// A character typed into the search prompt.
    SearchInput(char),
    SearchBackspace,
    SearchCommit,
    SearchCancel,
    // Marks and yank.
    SetMark(char),
    GotoMark(char),
    Yank,
    // Launchers.
    /// Run `config.launch[i]`.
    Launch(usize),
    /// `<leader>` followed by keys bound to nothing.
    NoMapping,
    // LSP.
    /// `K`: hover preview for the link under the cursor.
    Hover,
    /// `Esc` while the hover popup is open.
    HoverClose,
    // Sidebar.
    Sidebar(SidebarAction),
    // Link and heading motions, hints.
    /// `]l` (true) / `[l`.
    LinkMotion(bool, Option<usize>),
    /// `;` (true) / `,`.
    LinkRepeat(bool, Option<usize>),
    /// `]]` (true) / `[[`.
    HeadingMotion(bool, Option<usize>),
    /// `s`: show hint labels.
    HintStart,
    /// A letter typed in hint mode.
    HintInput(char),
    HintCancel,
    // Notebook pickers and the command line.
    OpenOp(crate::notebook::Op),
    Picker(super::PickerAction),
    Cmd(super::CmdAction),
}

/// What a key sequence means so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyResult {
    Action(Action),
    /// A prefix of a longer sequence: keep the keys and wait.
    Pending,
    /// A count digit (0-9) to append to the typed count.
    Count(usize),
    /// Not bound: drop the sequence and the count.
    None,
}

/// The character of a key pressed without Ctrl.
fn plain(key: &KeyEvent) -> Option<char> {
    match key.code {
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => Some(c),
        _ => None,
    }
}

impl App {
    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        self.pending.push(key);
        let keys = std::mem::take(&mut self.pending);
        match self.keymap(&keys) {
            KeyResult::Count(d) => {
                let n = self.count.unwrap_or(0).saturating_mul(10).saturating_add(d);
                self.count = Some(n.min(MAX_COUNT));
                return;
            }
            KeyResult::Pending => self.pending = keys,
            KeyResult::Action(a) => {
                self.count = None;
                self.apply(a);
            }
            KeyResult::None => self.count = None,
        }
        self.keep_visible();
    }

    /// Meaning of the key sequence `keys` in the current mode, given the
    /// typed count.
    pub fn keymap(&self, keys: &[KeyEvent]) -> KeyResult {
        match self.mode {
            Mode::Normal if self.focus() != Focus::Content => self.sidebar_keymap(keys),
            Mode::Normal => self.normal_keymap(keys),
            Mode::Search => search_keymap(keys),
            Mode::Filter => sidebar::filter_keymap(keys),
            Mode::Picker => super::picker::picker_keymap(keys),
            Mode::Command => super::cmdline::cmdline_keymap(keys),
            Mode::Hint => super::hints::hint_keymap(keys),
        }
    }

    /// `<leader>` sequences, resolved against the launcher keys.
    pub(super) fn leader_keymap(&self, keys: &[KeyEvent]) -> Option<KeyResult> {
        let leader = self.config.keys.leader;
        let (first, rest) = keys.split_first()?;
        if plain(first) != Some(leader) {
            return None;
        }
        let Some(typed) = rest.iter().map(plain).collect::<Option<Vec<char>>>() else {
            return Some(KeyResult::Action(Action::NoMapping));
        };
        Some(match match_leader(&self.leader_bindings, &typed) {
            LeaderMatch::Launch(i) => KeyResult::Action(Action::Launch(i)),
            LeaderMatch::Pending => KeyResult::Pending,
            // Built-in, unless a launcher took the key.
            LeaderMatch::NoMapping if typed == ['e'] => {
                KeyResult::Action(Action::Sidebar(SidebarAction::Cycle))
            }
            LeaderMatch::NoMapping => {
                super::picker::leader_op(&typed).unwrap_or(KeyResult::Action(Action::NoMapping))
            }
        })
    }

    fn normal_keymap(&self, keys: &[KeyEvent]) -> KeyResult {
        use Action as A;
        let count = self.count;
        if let Some(r) = self.leader_keymap(keys) {
            return r;
        }
        if let Some(r) = sidebar::window_keymap(keys) {
            return r;
        }
        if let Some(r) = super::picker::normal_keys(keys) {
            return r;
        }
        let [key] = keys else {
            let (Some(prefix), Some(c)) =
                (keys.first().and_then(plain), keys.get(1).and_then(plain))
            else {
                return KeyResult::None;
            };
            return match (prefix, c) {
                ('m', 'a'..='z') => KeyResult::Action(A::SetMark(c)),
                ('\'', 'a'..='z') => KeyResult::Action(A::GotoMark(c)),
                ('g', 'g') => KeyResult::Action(A::GotoTop(count)),
                ('g', 'd') => KeyResult::Action(A::Follow),
                ('g', 'x') => KeyResult::Action(A::OpenExternal),
                ('z', 'z') => KeyResult::Action(A::CenterCursor),
                ('z', 't') => KeyResult::Action(A::CursorToTop),
                ('z', 'b') => KeyResult::Action(A::CursorToBottom),
                ('Z', 'Z') => KeyResult::Action(A::Quit),
                (']', 'l') => KeyResult::Action(A::LinkMotion(true, count)),
                ('[', 'l') => KeyResult::Action(A::LinkMotion(false, count)),
                (']', ']') => KeyResult::Action(A::HeadingMotion(true, count)),
                ('[', '[') => KeyResult::Action(A::HeadingMotion(false, count)),
                _ => KeyResult::None,
            };
        };
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            let action = match key.code {
                // C-] arrives as C-5 from crossterm.
                KeyCode::Char('5' | ']') => A::Follow,
                KeyCode::Char('o' | 't') => A::Back,
                KeyCode::Char('i') => A::Forward,
                KeyCode::Char('d') => A::HalfPageDown,
                KeyCode::Char('u') => A::HalfPageUp,
                KeyCode::Char('f') => A::PageDown,
                KeyCode::Char('b') => A::PageUp,
                KeyCode::Char('e') => A::LineDown,
                KeyCode::Char('y') => A::LineUp,
                _ => return KeyResult::None,
            };
            return KeyResult::Action(action);
        }
        let action = match key.code {
            KeyCode::Char(c @ '0'..='9') if c != '0' || count.is_some() => {
                return KeyResult::Count(c as usize - '0' as usize);
            }
            KeyCode::Char('g' | 'z' | 'Z' | 'm' | '\'') => return KeyResult::Pending,
            KeyCode::Char('[' | ']') => return KeyResult::Pending,
            KeyCode::Char(';') => A::LinkRepeat(true, count),
            KeyCode::Char(',') => A::LinkRepeat(false, count),
            KeyCode::Char('s') => A::HintStart,
            KeyCode::Char('/') => A::SearchStart(true),
            KeyCode::Char('?') => A::SearchStart(false),
            KeyCode::Char('n') => A::SearchNext(true, count),
            KeyCode::Char('N') => A::SearchNext(false, count),
            KeyCode::Char('*') => A::SearchWord(true),
            KeyCode::Char('#') => A::SearchWord(false),
            KeyCode::Esc if self.hover_popup().is_some() => A::HoverClose,
            KeyCode::Esc => A::SearchClear,
            KeyCode::Char('K') => A::Hover,
            KeyCode::Char('y') => A::Yank,
            KeyCode::Enter => A::Follow,
            KeyCode::Tab => A::Forward,
            KeyCode::Char('q') => A::Quit,
            KeyCode::Char('h') | KeyCode::Left => A::Left(count),
            KeyCode::Char('l') | KeyCode::Right => A::Right(count),
            KeyCode::Char('j') | KeyCode::Down => A::Down(count),
            KeyCode::Char('k') | KeyCode::Up => A::Up(count),
            KeyCode::Char('0') | KeyCode::Home => A::LineStart,
            KeyCode::Char('$') | KeyCode::End => A::LineEnd(count),
            KeyCode::Char('w') => A::WordForward(count),
            KeyCode::Char('b') => A::WordBackward(count),
            KeyCode::Char('e') => A::WordEnd(count),
            KeyCode::Char('}') => A::ParagraphForward(count),
            KeyCode::Char('{') => A::ParagraphBackward(count),
            KeyCode::Char('G') => A::GotoBottom(count),
            KeyCode::Char('H') => A::ScreenTop(count),
            KeyCode::Char('L') => A::ScreenBottom(count),
            KeyCode::Char('M') => A::ScreenMiddle,
            KeyCode::PageDown => A::PageDown,
            KeyCode::PageUp => A::PageUp,
            _ => return KeyResult::None,
        };
        KeyResult::Action(action)
    }

    /// Run one action. The caller keeps the cursor visible afterwards.
    pub fn apply(&mut self, action: Action) {
        use Action as A;
        let n = |count: Option<usize>| count.unwrap_or(1).max(1);
        match action {
            A::Quit => self.quit = true,
            A::Left(c) => self.horizontal(-(n(c) as isize)),
            A::Right(c) => self.horizontal(n(c) as isize),
            A::Down(c) => self.vertical(n(c) as isize),
            A::Up(c) => self.vertical(-(n(c) as isize)),
            A::LineStart => self.set_col(0),
            A::LineEnd(c) => self.line_end(n(c)),
            A::WordForward(c) => self.repeat(n(c), Self::word_forward),
            A::WordBackward(c) => self.repeat(n(c), Self::word_backward),
            A::WordEnd(c) => self.repeat(n(c), Self::word_end),
            A::ParagraphForward(c) => self.repeat(n(c), Self::paragraph_forward),
            A::ParagraphBackward(c) => self.repeat(n(c), Self::paragraph_backward),
            A::GotoTop(c) => self.goto_row(c.map_or(0, |c| c - 1)),
            A::GotoBottom(c) => self.goto_row(c.map_or(self.last_row(), |c| c - 1)),
            A::ScreenTop(c) => self.screen_top(n(c)),
            A::ScreenBottom(c) => self.screen_bottom(n(c)),
            A::ScreenMiddle => self.screen_middle(),
            A::HalfPageDown => self.half_page(true),
            A::HalfPageUp => self.half_page(false),
            A::PageDown => self.page_down(),
            A::PageUp => self.page_up(),
            A::LineDown => self.line_down(),
            A::LineUp => self.line_up(),
            A::CenterCursor => self.center_cursor(),
            A::CursorToTop => self.cursor_to_top(),
            A::CursorToBottom => self.cursor_to_bottom(),
            A::Follow => self.follow(),
            A::OpenExternal => self.open_external(),
            A::Back => self.back(),
            A::Forward => self.forward(),
            A::SearchStart(fwd) => self.search_start(fwd),
            A::SearchNext(same, c) => self.search_next(same, n(c)),
            A::SearchWord(fwd) => self.search_word(fwd),
            A::SearchClear => self.search_clear(),
            A::SearchInput(c) => self.search_edit(Some(c)),
            A::SearchBackspace => self.search_edit(None),
            A::SearchCommit => self.search_commit(),
            A::SearchCancel => self.search_cancel(),
            A::SetMark(c) => self.set_mark(c),
            A::GotoMark(c) => self.goto_mark(c),
            A::Yank => self.yank(),
            A::Launch(i) => self.launch_index(i),
            A::NoMapping => self.set_status("No mapping"),
            A::Hover => self.hover(),
            A::HoverClose => self.hover_close(),
            A::Sidebar(a) => self.sidebar_action(a),
            A::LinkMotion(fwd, c) => self.link_motion(fwd, n(c)),
            A::LinkRepeat(same, c) => self.link_repeat(same, n(c)),
            A::HeadingMotion(fwd, c) => self.heading_motion(fwd, n(c)),
            A::HintStart => self.hint_start(),
            A::HintInput(c) => self.hint_input(c),
            A::HintCancel => self.hint_cancel(),
            A::OpenOp(op) => self.open_op(op, None),
            A::Picker(a) => self.picker_action(a),
            A::Cmd(a) => self.cmd_action(a),
        }
    }
}

/// Keys in the search prompt. Typed text, not commands: no counts or
/// multi-key sequences.
fn search_keymap(keys: &[KeyEvent]) -> KeyResult {
    let Some(key) = keys.last() else {
        return KeyResult::None;
    };
    let action = match key.code {
        KeyCode::Enter => Action::SearchCommit,
        KeyCode::Esc => Action::SearchCancel,
        KeyCode::Backspace => Action::SearchBackspace,
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            Action::SearchInput(c)
        }
        _ => return KeyResult::None,
    };
    KeyResult::Action(action)
}
