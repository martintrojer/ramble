//! The `g?` help overlay (spec § Keymap): every binding that works right
//! now, grouped, scrollable and filterable.
//!
//! Dispatch stays in the `match`-based keymaps; [`BINDINGS`] describes
//! them. A unit test resolves every row through [`App::keymap`] in its
//! context, so a row that no longer matches a binding fails the build.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;

use super::keys::{Action, KeyResult};
use super::launch::{LeaderMatch, match_leader};
use super::{App, Focus, Mode};
use crate::config::SidebarSide;
use crate::notebook::Op;

/// Where a binding works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ctx {
    /// The content pane has focus.
    Normal,
    /// A sidebar pane has focus.
    Sidebar,
    /// Both of the above.
    Any,
    /// The picker is open.
    Picker,
    /// The help overlay is open.
    Help,
    /// A visual selection is active.
    Visual,
}

/// Help sections, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Group {
    General,
    Motions,
    Scrolling,
    Links,
    History,
    Search,
    Visual,
    Review,
    Sidebar,
    Pickers,
    Launchers,
    Commands,
    Help,
}

impl Group {
    const ALL: [Group; 13] = [
        Group::General,
        Group::Motions,
        Group::Scrolling,
        Group::Links,
        Group::History,
        Group::Search,
        Group::Visual,
        Group::Review,
        Group::Sidebar,
        Group::Pickers,
        Group::Launchers,
        Group::Commands,
        Group::Help,
    ];

    pub(super) fn title(self) -> &'static str {
        match self {
            Group::General => "General",
            Group::Motions => "Motions",
            Group::Scrolling => "Scrolling",
            Group::Links => "Links",
            Group::History => "History",
            Group::Search => "Search & marks",
            Group::Visual => "Visual & yank",
            Group::Review => "Review",
            Group::Sidebar => "Sidebar",
            Group::Pickers => "Pickers",
            Group::Launchers => "Launchers",
            Group::Commands => "Commands",
            Group::Help => "Help",
        }
    }
}

/// One help row. `keys` lists alternatives separated by `", "`; within one
/// alternative `<leader>` is the leader key, `C-x` is Ctrl+x, `{a-z}` is
/// any letter of the range, `{motion}` is any motion, and `Enter Esc Tab Down Up Left Right Home End
/// PgDn PgUp` name keys.
pub(crate) struct Binding {
    pub keys: &'static str,
    pub context: Ctx,
    pub group: Group,
    pub desc: &'static str,
    /// Whether the action can do anything right now.
    pub avail: fn(&App) -> bool,
}

fn always(_: &App) -> bool {
    true
}

fn side_left(a: &App) -> bool {
    a.sidebar_side() == SidebarSide::Left
}

fn side_right(a: &App) -> bool {
    a.sidebar_side() == SidebarSide::Right
}

fn sidebar_on_left(a: &App) -> bool {
    side_left(a) && a.can_focus_sidebar()
}

fn sidebar_on_right(a: &App) -> bool {
    side_right(a) && a.can_focus_sidebar()
}

const fn b(
    keys: &'static str,
    context: Ctx,
    group: Group,
    desc: &'static str,
    avail: fn(&App) -> bool,
) -> Binding {
    Binding {
        keys,
        context,
        group,
        desc,
        avail,
    }
}

use Ctx::{Any, Normal as N, Picker as P, Sidebar as S, Visual as V};
use Group as G;

/// Every key binding of the normal, sidebar, picker and help keymaps.
pub(crate) static BINDINGS: &[Binding] = &[
    b("g?", Any, G::General, "show this help", always),
    b(":", Any, G::General, "command line", always),
    b("q", Any, G::General, "quit", always),
    b("ZZ", N, G::General, "quit", always),
    b(
        "C-l",
        Any,
        G::General,
        "refresh: re-read file and tree, redraw",
        always,
    ),
    b(
        "gR",
        N,
        G::General,
        "toggle the raw source view",
        App::has_page,
    ),
    // Motions.
    b("h, Left", N, G::Motions, "left", App::has_page),
    b("l, Right", N, G::Motions, "right", App::has_page),
    b("j, Down", N, G::Motions, "down", App::has_page),
    b("k, Up", N, G::Motions, "up", App::has_page),
    b("w", N, G::Motions, "next word", App::has_page),
    b("b", N, G::Motions, "previous word", App::has_page),
    b("e", N, G::Motions, "end of word", App::has_page),
    b("0, Home", N, G::Motions, "start of line", App::has_page),
    b("$, End", N, G::Motions, "end of line", App::has_page),
    b("}", N, G::Motions, "next paragraph", App::has_page),
    b("{", N, G::Motions, "previous paragraph", App::has_page),
    b("gg", N, G::Motions, "first line (or line N)", App::has_page),
    b("G", N, G::Motions, "last line (or line N)", App::has_page),
    b("H", N, G::Motions, "top of screen", App::has_page),
    b("M", N, G::Motions, "middle of screen", App::has_page),
    b("L", N, G::Motions, "bottom of screen", App::has_page),
    b(
        "{1-9}",
        N,
        G::Motions,
        "count for the next motion",
        App::has_page,
    ),
    // Scrolling.
    b("C-d", N, G::Scrolling, "half page down", App::has_page),
    b("C-u", N, G::Scrolling, "half page up", App::has_page),
    b("C-f, PgDn", N, G::Scrolling, "page down", App::has_page),
    b("C-b, PgUp", N, G::Scrolling, "page up", App::has_page),
    b("C-e", N, G::Scrolling, "scroll a line down", App::has_page),
    b("C-y", N, G::Scrolling, "scroll a line up", App::has_page),
    b(
        "zz",
        N,
        G::Scrolling,
        "cursor line to the middle",
        App::has_page,
    ),
    b(
        "zt",
        N,
        G::Scrolling,
        "cursor line to the top",
        App::has_page,
    ),
    b(
        "zb",
        N,
        G::Scrolling,
        "cursor line to the bottom",
        App::has_page,
    ),
    // Links.
    b(
        "gd, Enter, C-]",
        N,
        G::Links,
        "follow the link",
        App::has_page,
    ),
    b("gx", N, G::Links, "open the URL externally", App::has_page),
    b("]l", N, G::Links, "next link", App::has_page),
    b("[l", N, G::Links, "previous link", App::has_page),
    b(";", N, G::Links, "repeat the link motion", App::has_page),
    b(",", N, G::Links, "repeat it backwards", App::has_page),
    b(
        "s",
        N,
        G::Links,
        "hint jump to a visible link",
        App::has_page,
    ),
    b("]]", N, G::Links, "next heading", App::has_page),
    b("[[", N, G::Links, "previous heading", App::has_page),
    b("K", N, G::Links, "hover preview", App::can_hover),
    // History.
    b("C-o, C-t", N, G::History, "back", App::can_back),
    b("C-i, Tab", N, G::History, "forward", App::can_forward),
    // Search and marks.
    b("/", N, G::Search, "search forward", App::has_page),
    b("?", N, G::Search, "search backward", App::has_page),
    b("n", N, G::Search, "next match", App::can_search_next),
    b("N", N, G::Search, "previous match", App::can_search_next),
    b("*", N, G::Search, "search word forward", App::has_page),
    b("#", N, G::Search, "search word backward", App::has_page),
    b("Esc", N, G::Search, "clear highlights, close hover", always),
    b("m{a-z}", N, G::Search, "set a mark", App::has_page),
    b("'{a-z}", N, G::Search, "jump to a mark", App::has_page),
    // Visual mode and the yank operator.
    b("v", N, G::Visual, "select characters", App::has_page),
    b("V", N, G::Visual, "select lines", App::has_page),
    b("C-v", N, G::Visual, "select a block", App::has_page),
    b("gv", N, G::Visual, "reselect the last selection", |a| {
        a.has_page() && a.visual.has_last()
    }),
    b(
        "y{motion}",
        N,
        G::Visual,
        "yank the source a motion covers",
        App::has_page,
    ),
    b("yy, Y", N, G::Visual, "yank source lines", App::has_page),
    b("yf", N, G::Visual, "yank the absolute file path", always),
    b(
        "yF",
        N,
        G::Visual,
        "yank the path relative to the root",
        always,
    ),
    b(
        "yu",
        N,
        G::Visual,
        "yank link target or file path",
        App::can_yank,
    ),
    b("o", V, G::Visual, "in visual: swap the ends", always),
    b("y", V, G::Visual, "in visual: yank the selection", always),
    b("Y", V, G::Visual, "in visual: yank whole lines", always),
    b("v, V, C-v", V, G::Visual, "in visual: switch kind", always),
    b("Esc", V, G::Visual, "in visual: cancel", always),
    // Review markers.
    b(
        "]r",
        N,
        G::Review,
        "next review comment",
        App::can_review_jump,
    ),
    b(
        "[r",
        N,
        G::Review,
        "previous review comment",
        App::can_review_jump,
    ),
    // Sidebar.
    b(
        "<leader>e",
        Any,
        G::Sidebar,
        "show or hide the sidebar",
        always,
    ),
    b(
        "<leader>E",
        Any,
        G::Sidebar,
        "cycle sidebar: outline, files, split",
        always,
    ),
    // `C-w h` / `C-w l` follow the screen (D9): one row pair per side.
    b("C-w h", Any, G::Sidebar, "to the sidebar", sidebar_on_left),
    b("C-w l", S, G::Sidebar, "to the content", side_left),
    b("C-w l", Any, G::Sidebar, "to the sidebar", sidebar_on_right),
    b("C-w h", S, G::Sidebar, "to the content", side_right),
    b(
        "C-w w",
        Any,
        G::Sidebar,
        "next pane",
        App::can_focus_sidebar,
    ),
    b(
        "C-w W",
        Any,
        G::Sidebar,
        "previous pane",
        App::can_focus_sidebar,
    ),
    b(
        "C-w j",
        Any,
        G::Sidebar,
        "pane below (split)",
        App::can_focus_sidebar,
    ),
    b(
        "C-w k",
        Any,
        G::Sidebar,
        "pane above (split)",
        App::can_focus_sidebar,
    ),
    b(
        "C-w p",
        Any,
        G::Sidebar,
        "last pane",
        App::can_focus_sidebar,
    ),
    b("j, Down", S, G::Sidebar, "down", always),
    b("k, Up", S, G::Sidebar, "up", always),
    b("gg", S, G::Sidebar, "first row", always),
    b("G", S, G::Sidebar, "last row", always),
    b("h, Left", S, G::Sidebar, "collapse / parent", always),
    b("l, Right", S, G::Sidebar, "expand", always),
    b("o, Enter", S, G::Sidebar, "open / jump to heading", always),
    b("/", S, G::Sidebar, "filter the pane", always),
    b("Esc", S, G::Sidebar, "clear the filter", always),
    // Pickers.
    b("<leader>zf", Any, G::Pickers, "notes", |a| {
        a.can_open(Op::Notes)
    }),
    b("<leader>zs", Any, G::Pickers, "search notes", |a| {
        a.can_open(Op::Search)
    }),
    b("<leader>zz", Any, G::Pickers, "tags", |a| {
        a.can_open(Op::Tags)
    }),
    b("<leader>zb", Any, G::Pickers, "backlinks", |a| {
        a.can_open(Op::Backlinks)
    }),
    b("<leader>zl", Any, G::Pickers, "links", |a| {
        a.can_open(Op::Links)
    }),
    b("grr", N, G::Pickers, "backlinks", |a| {
        a.can_open(Op::Backlinks)
    }),
    b(
        "C-n, Down",
        P,
        G::Pickers,
        "in a picker: next item",
        App::can_pick,
    ),
    b(
        "C-p, Up",
        P,
        G::Pickers,
        "in a picker: previous item",
        App::can_pick,
    ),
    b("Enter", P, G::Pickers, "in a picker: open", App::can_pick),
    b("Esc", P, G::Pickers, "in a picker: close", App::can_pick),
    // The help overlay itself.
    b("j, Down", Ctx::Help, G::Help, "scroll down", always),
    b("k, Up", Ctx::Help, G::Help, "scroll up", always),
    b("C-d", Ctx::Help, G::Help, "half page down", always),
    b("C-u", Ctx::Help, G::Help, "half page up", always),
    b("gg", Ctx::Help, G::Help, "top", always),
    b("G", Ctx::Help, G::Help, "bottom", always),
    b(
        "/",
        Ctx::Help,
        G::Help,
        "filter (Enter keeps, Esc clears)",
        always,
    ),
    b("q, Esc, g?", Ctx::Help, G::Help, "close", always),
];

/// Named keys of the `keys` notation, as whole alternatives.
pub(super) const NAMED: [(&str, KeyCode); 11] = [
    ("Enter", KeyCode::Enter),
    ("Esc", KeyCode::Esc),
    ("Tab", KeyCode::Tab),
    ("Down", KeyCode::Down),
    ("Up", KeyCode::Up),
    ("Left", KeyCode::Left),
    ("Right", KeyCode::Right),
    ("Home", KeyCode::Home),
    ("End", KeyCode::End),
    ("PgDn", KeyCode::PageDown),
    ("PgUp", KeyCode::PageUp),
];

/// One token of a row's key alternative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Tok<'a> {
    Key(KeyEvent),
    /// `{a-z}`, `{motion}`, `{1-9}`, braces included.
    Placeholder(&'a str),
}

/// One alternative of a row's `keys` as tokens.
pub(super) fn tokens(alt: &str, leader: char) -> Vec<Tok<'_>> {
    let ev = |c| Tok::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    if let Some(&(_, code)) = NAMED.iter().find(|(n, _)| *n == alt) {
        return vec![Tok::Key(KeyEvent::new(code, KeyModifiers::NONE))];
    }
    let mut out = Vec::new();
    let mut rest = alt;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("<leader>") {
            out.push(ev(leader));
            rest = r;
        } else if let Some(r) = rest.strip_prefix("C-")
            && let Some(c) = r.chars().next()
        {
            out.push(Tok::Key(KeyEvent::new(
                KeyCode::Char(c),
                KeyModifiers::CONTROL,
            )));
            rest = r[c.len_utf8()..].trim_start();
        } else if rest.starts_with('{') && rest.len() > 2 {
            let end = rest.find('}').expect("closing }") + 1;
            out.push(Tok::Placeholder(&rest[..end]));
            rest = &rest[end..];
        } else {
            let c = rest.chars().next().unwrap();
            out.push(ev(c));
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// A key that stands for a placeholder: `w` for `{motion}`, else the
/// first character of the range (`{a-z}` → `a`).
pub(super) fn placeholder_key(p: &str) -> KeyEvent {
    let c = match p {
        "{motion}" => 'w',
        _ => p[1..].chars().next().unwrap_or('a'),
    };
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
}

/// A key as the help shows it: `Space` for a space leader, `C-w`, `Esc`.
pub(super) fn show_key(key: &KeyEvent, leader: char) -> String {
    if let Some(&(name, _)) = NAMED.iter().find(|(_, c)| *c == key.code) {
        return name.to_string();
    }
    match key.code {
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) => format!("C-{c}"),
        KeyCode::Char(' ') if leader == ' ' => "Space".to_string(),
        KeyCode::Char(c) => c.to_string(),
        other => format!("{other:?}"),
    }
}

/// Keys while the help overlay is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpAction {
    Open,
    Close,
    Down,
    Up,
    HalfDown,
    HalfUp,
    Top,
    Bottom,
    FilterStart,
    FilterInput(char),
    FilterBackspace,
    /// Enter while typing: keep the filter.
    FilterCommit,
    /// Esc while typing, or with a kept filter: clear it.
    FilterClear,
}

/// Help overlay state owned by [`App`].
#[derive(Debug, Clone, Default)]
pub(super) struct HelpState {
    scroll: usize,
    filter: String,
    /// Typing the filter.
    filtering: bool,
}

/// One drawn help line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelpLine {
    Group(&'static str),
    Item {
        keys: String,
        desc: String,
        /// Listed but unusable right now (drawn dimmed).
        dim: bool,
    },
}

/// What the UI draws for the help overlay.
#[derive(Debug, Clone)]
pub struct HelpView<'a> {
    pub lines: Vec<HelpLine>,
    pub scroll: usize,
    pub filter: &'a str,
    pub filtering: bool,
}

/// The overlay's rectangle inside `area` (the screen above the status
/// line). Never larger than `area`.
pub fn help_rect(area: Rect) -> Rect {
    let width = area.width.saturating_sub(4).clamp(1, 72).min(area.width);
    let height = area.height.saturating_sub(2).clamp(1, 30).min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}

/// List rows inside a help rect: borders and the filter line excluded.
pub fn help_list_rows(rect: Rect) -> usize {
    rect.height.saturating_sub(3) as usize
}

/// `<leader>` shown as `Space ` when the leader is a space.
fn show_leader(keys: &str, leader: char) -> String {
    let l = if leader == ' ' {
        "Space ".to_string()
    } else {
        leader.to_string()
    };
    keys.replace("<leader>", &l)
}

fn plain(key: &KeyEvent) -> Option<char> {
    match key.code {
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => Some(c),
        _ => None,
    }
}

/// Keys in the help overlay.
pub(super) fn help_keymap(app: &App, keys: &[KeyEvent]) -> KeyResult {
    use HelpAction as H;
    let act = |a| KeyResult::Action(Action::Help(a));
    let Some(key) = keys.last() else {
        return KeyResult::None;
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if app.help.filtering {
        return match key.code {
            KeyCode::Enter => act(H::FilterCommit),
            KeyCode::Esc => act(H::FilterClear),
            KeyCode::Backspace => act(H::FilterBackspace),
            KeyCode::Char(c) if !ctrl => act(H::FilterInput(c)),
            _ => KeyResult::None,
        };
    }
    if let [first, second] = keys
        && plain(first) == Some('g')
    {
        return match plain(second) {
            Some('g') => act(H::Top),
            Some('?') => act(H::Close),
            _ => KeyResult::None,
        };
    }
    if keys.len() != 1 {
        return KeyResult::None;
    }
    match key.code {
        KeyCode::Char('d') if ctrl => act(H::HalfDown),
        KeyCode::Char('u') if ctrl => act(H::HalfUp),
        _ if ctrl => KeyResult::None,
        KeyCode::Char('j') | KeyCode::Down => act(H::Down),
        KeyCode::Char('k') | KeyCode::Up => act(H::Up),
        KeyCode::Char('g') => KeyResult::Pending,
        KeyCode::Char('G') => act(H::Bottom),
        KeyCode::Char('/') => act(H::FilterStart),
        KeyCode::Char('q') => act(H::Close),
        KeyCode::Esc if app.hover_popup().is_some() => KeyResult::Action(Action::HoverClose),
        KeyCode::Esc if !app.help.filter.is_empty() => act(H::FilterClear),
        KeyCode::Esc => act(H::Close),
        _ => KeyResult::None,
    }
}

impl App {
    pub(crate) fn has_page(&self) -> bool {
        self.page.is_some()
    }

    /// Some notebook operation is available, so the picker can open.
    pub(crate) fn can_pick(&self) -> bool {
        !self.available_ops().is_empty()
    }

    /// The `<leader>` sequence in `keys` reaches its built-in binding: no
    /// launcher key takes or extends it, and none sits on a prefix of it
    /// (dispatch launches as soon as the typed prefix matches).
    pub(super) fn leader_free(&self, keys: &str) -> bool {
        let Some(rest) = keys.strip_prefix("<leader>") else {
            return true;
        };
        let typed: Vec<char> = rest.chars().collect();
        (1..=typed.len()).all(|n| {
            !matches!(
                match_leader(&self.leader_bindings, &typed[..n]),
                LeaderMatch::Launch(_)
            )
        }) && match_leader(&self.leader_bindings, &typed) == LeaderMatch::NoMapping
    }

    /// Whether a row of context `ctx` applies with the current focus.
    pub(super) fn ctx_shown(&self, ctx: Ctx) -> bool {
        let content = self.focus() == Focus::Content;
        match ctx {
            Ctx::Normal => content,
            Ctx::Sidebar => !content,
            Ctx::Any | Ctx::Help => true,
            Ctx::Visual => content,
            Ctx::Picker => self.can_pick(),
        }
    }

    /// Every help line for the current state, before the filter.
    pub fn help_lines(&self) -> Vec<HelpLine> {
        let leader = self.config.keys.leader;
        let has_vcs = self.has_vcs_root();
        let mut out = Vec::new();
        for g in Group::ALL {
            let mut items: Vec<HelpLine> = BINDINGS
                .iter()
                .filter(|b| b.group == g && self.ctx_shown(b.context))
                .filter(|b| (b.avail)(self) && self.leader_free(b.keys))
                .map(|b| HelpLine::Item {
                    keys: show_leader(b.keys, leader),
                    desc: b.desc.to_string(),
                    dim: false,
                })
                .collect();
            match g {
                Group::Launchers => {
                    for (seq, i) in &self.leader_bindings {
                        let l = &self.config.launch[*i];
                        let keys =
                            show_leader(&format!("<leader>{}", String::from_iter(seq)), leader);
                        let dim = l.needs_vcs && !has_vcs;
                        let desc = if dim {
                            format!("{} (needs a repo)", l.name)
                        } else {
                            l.name.clone()
                        };
                        items.push(HelpLine::Item { keys, desc, dim });
                    }
                }
                Group::Commands => {
                    for (cmd, desc) in super::cmdline::COMMANDS {
                        let op = Op::ALL.into_iter().find(|op| {
                            cmd.strip_prefix(':')
                                .is_some_and(|c| c.split(' ').next() == Some(op.name()))
                        });
                        let launch = cmd.starts_with(":Launch");
                        if op.is_some_and(|op| !self.can_open(op))
                            || (launch && self.config.launch.is_empty())
                        {
                            continue;
                        }
                        items.push(HelpLine::Item {
                            keys: cmd.to_string(),
                            desc: desc.to_string(),
                            dim: false,
                        });
                    }
                    for l in self.config.launch.iter().filter(|l| l.key.is_none()) {
                        let dim = l.needs_vcs && !has_vcs;
                        let mut desc = format!("run launcher {}", l.name);
                        if dim {
                            desc.push_str(" (needs a repo)");
                        }
                        items.push(HelpLine::Item {
                            keys: format!(":Launch {}", l.name),
                            desc,
                            dim,
                        });
                    }
                }
                _ => {}
            }
            if !items.is_empty() {
                out.push(HelpLine::Group(g.title()));
                out.extend(items);
            }
        }
        out
    }

    /// Help lines matching the filter (case-insensitive, over keys and
    /// description); a group heading stays when any of its rows match.
    fn help_filtered(&self) -> Vec<HelpLine> {
        let f = self.help.filter.to_lowercase();
        if f.is_empty() {
            return self.help_lines();
        }
        let mut out = Vec::new();
        let mut group = None;
        for line in self.help_lines() {
            match &line {
                HelpLine::Group(_) => group = Some(line),
                HelpLine::Item { keys, desc, .. } => {
                    if keys.to_lowercase().contains(&f) || desc.to_lowercase().contains(&f) {
                        if let Some(g) = group.take() {
                            out.push(g);
                        }
                        out.push(line);
                    }
                }
            }
        }
        out
    }

    /// The open help overlay, for drawing and tests.
    pub fn help_view(&self) -> Option<HelpView<'_>> {
        if self.mode != Mode::Help {
            return None;
        }
        let lines = self.help_filtered();
        let scroll = self.help.scroll.min(self.help_max_scroll(lines.len()));
        Some(HelpView {
            lines,
            scroll,
            filter: &self.help.filter,
            filtering: self.help.filtering,
        })
    }

    fn help_page(&self) -> usize {
        let area = Rect::new(0, 0, self.size.0, self.size.1.saturating_sub(1));
        help_list_rows(help_rect(area))
    }

    fn help_max_scroll(&self, lines: usize) -> usize {
        lines.saturating_sub(self.help_page())
    }

    pub(super) fn help_action(&mut self, a: HelpAction) {
        use HelpAction as H;
        let max = self.help_max_scroll(self.help_filtered().len());
        let half = (self.help_page() / 2).max(1);
        let s = self.help.scroll.min(max);
        match a {
            H::Open => {
                self.help = HelpState::default();
                self.count = None;
                self.pending.clear();
                self.mode = Mode::Help;
            }
            H::Close => {
                self.help = HelpState::default();
                if self.mode == Mode::Help {
                    self.mode = Mode::Normal;
                }
            }
            H::Down => self.help.scroll = (s + 1).min(max),
            H::Up => self.help.scroll = s.saturating_sub(1),
            H::HalfDown => self.help.scroll = (s + half).min(max),
            H::HalfUp => self.help.scroll = s.saturating_sub(half),
            H::Top => self.help.scroll = 0,
            H::Bottom => self.help.scroll = max,
            H::FilterStart => self.help.filtering = true,
            H::FilterInput(c) => {
                self.help.filter.push(c);
                self.help.scroll = 0;
            }
            H::FilterBackspace => {
                if self.help.filter.pop().is_none() {
                    self.help.filtering = false;
                }
                self.help.scroll = 0;
            }
            H::FilterCommit => self.help.filtering = false,
            H::FilterClear => {
                self.help.filter.clear();
                self.help.filtering = false;
                self.help.scroll = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::VisualAction;
    use crate::app::{StartOptions, StartTarget};
    use crate::config::{Config, SidebarMode};

    /// One alternative of a row's `keys` as key events (a placeholder
    /// becomes a representative key).
    fn parse(alt: &str, leader: char) -> Vec<KeyEvent> {
        tokens(alt, leader)
            .into_iter()
            .map(|t| match t {
                Tok::Key(k) => k,
                Tok::Placeholder(p) => placeholder_key(p),
            })
            .collect()
    }

    fn alts(keys: &str) -> Vec<&str> {
        keys.split(", ").collect()
    }

    fn app(dir: &std::path::Path) -> App {
        app_on(dir, SidebarSide::Left)
    }

    fn app_on(dir: &std::path::Path, side: SidebarSide) -> App {
        let path = dir.join("a.md");
        std::fs::write(&path, "# A\n\nSee [b](b.md).\n").unwrap();
        let mut config = Config::default();
        config.sidebar.side = side;
        config.sidebar.default = SidebarMode::Files;
        config.lsp.server = vec![];
        config.launch.clear();
        let opts = StartOptions {
            target: StartTarget::File(path),
            tree_root: dir.to_path_buf(),
            config,
        };
        App::new(opts, (80, 24)).unwrap()
    }

    /// The app put into each context a row can be tried in.
    fn contexts(dir: &std::path::Path, ctx: Ctx) -> Vec<App> {
        let normal = || app(dir);
        let sidebar = || {
            let mut a = app(dir);
            a.sidebar_action(super::super::sidebar::SidebarAction::FocusSidebar);
            assert_eq!(a.focus(), Focus::Files);
            a
        };
        match ctx {
            Ctx::Normal => vec![normal()],
            Ctx::Sidebar => vec![sidebar()],
            Ctx::Any => vec![normal(), sidebar()],
            Ctx::Picker => {
                let mut a = app(dir);
                a.open_op(Op::Notes, None);
                assert_eq!(a.mode(), Mode::Picker);
                vec![a]
            }
            Ctx::Help => {
                let mut a = app(dir);
                a.help_action(HelpAction::Open);
                vec![a]
            }
            Ctx::Visual => {
                let mut a = app(dir);
                a.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
                assert!(matches!(a.mode(), Mode::Visual(_)));
                vec![a]
            }
        }
    }

    #[test]
    fn every_row_resolves_in_its_context_and_has_a_description() {
        let dir = tempfile::tempdir().unwrap();
        for b in BINDINGS {
            assert!(!b.desc.trim().is_empty(), "{} has no description", b.keys);
            for alt in alts(b.keys) {
                for mut app in contexts(dir.path(), b.context) {
                    let all = parse(alt, app.config.keys.leader);
                    // An operator key (`y`) is an action that enters
                    // operator-pending mode; the rest resolves there.
                    let (keys, op) = match all.split_first() {
                        Some((first, rest))
                            if !rest.is_empty()
                                && app.keymap(std::slice::from_ref(first))
                                    == KeyResult::Action(Action::Visual(
                                        VisualAction::OpStart(None),
                                    )) =>
                        {
                            app.handle_key(*first);
                            (rest.to_vec(), true)
                        }
                        _ => (all.clone(), false),
                    };
                    assert!(!op || app.mode() == Mode::OpPending);
                    let r = app.keymap(&keys);
                    assert!(
                        matches!(r, KeyResult::Action(_) | KeyResult::Count(_)),
                        "{alt:?} ({:?}) resolved to {r:?}",
                        b.context
                    );
                }
            }
        }
    }

    /// On either side, every `C-w` row shown resolves, and the
    /// direction rows run the action their description names (D9).
    #[test]
    fn ctrl_w_rows_resolve_and_match_the_side() {
        use super::super::sidebar::SidebarAction as SA;
        let dir = tempfile::tempdir().unwrap();
        for side in [SidebarSide::Left, SidebarSide::Right] {
            let (to_side, to_content) = match side {
                SidebarSide::Left => ("C-w h", "C-w l"),
                SidebarSide::Right => ("C-w l", "C-w h"),
            };
            let mut normal = app_on(dir.path(), side);
            let mut sidebar = app_on(dir.path(), side);
            sidebar.sidebar_action(SA::FocusSidebar);
            assert_eq!(sidebar.focus(), Focus::Files);
            for a in [&mut normal, &mut sidebar] {
                let mut shown = Vec::new();
                for b in BINDINGS.iter().filter(|b| b.keys.starts_with("C-w")) {
                    if !a.ctx_shown(b.context) || !(b.avail)(a) {
                        continue;
                    }
                    shown.push((b.keys, b.desc));
                    let r = a.keymap(&parse(b.keys, a.config.keys.leader));
                    let want = match b.desc {
                        "to the sidebar" => Some(SA::FocusSidebar),
                        "to the content" => Some(SA::FocusContent),
                        _ => None,
                    };
                    match want {
                        Some(w) => assert_eq!(
                            r,
                            KeyResult::Action(Action::Sidebar(w)),
                            "{side:?} {}: {}",
                            b.keys,
                            b.desc
                        ),
                        None => assert!(matches!(r, KeyResult::Action(_)), "{}", b.keys),
                    }
                }
                assert!(shown.contains(&(to_side, "to the sidebar")), "{side:?}");
                let content_row = shown.contains(&(to_content, "to the content"));
                assert_eq!(content_row, a.focus() != Focus::Content, "{side:?}");
                assert!(
                    !shown.contains(&(to_content, "to the sidebar"))
                        && !shown.contains(&(to_side, "to the content")),
                    "{side:?}: the other side's rows are hidden: {shown:?}"
                );
            }
        }
    }

    /// Keys bound in keys.rs / sidebar.rs / picker.rs each have a row.
    #[test]
    fn representative_bindings_have_rows() {
        let has = |key: &str, ctxs: &[Ctx]| {
            BINDINGS
                .iter()
                .any(|b| ctxs.contains(&b.context) && alts(b.keys).contains(&key))
        };
        let normal = [
            "h",
            "j",
            "k",
            "l",
            "w",
            "b",
            "e",
            "0",
            "$",
            "{",
            "}",
            "gg",
            "G",
            "H",
            "M",
            "L",
            "C-d",
            "C-u",
            "C-f",
            "C-b",
            "C-e",
            "C-y",
            "zz",
            "zt",
            "zb",
            "gd",
            "Enter",
            "C-]",
            "gx",
            "]l",
            "[l",
            ";",
            ",",
            "]]",
            "[[",
            "s",
            "K",
            "y{motion}",
            "C-o",
            "C-t",
            "C-i",
            "Tab",
            "/",
            "?",
            "n",
            "N",
            "*",
            "#",
            "Esc",
            "m{a-z}",
            "'{a-z}",
            "]r",
            "[r",
            ":",
            "grr",
            "ZZ",
            "gR",
            "PgDn",
            "PgUp",
            "Home",
            "End",
            "v",
            "V",
            "C-v",
            "gv",
            "yy",
            "Y",
            "yf",
            "yF",
            "yu",
        ];
        for k in normal {
            assert!(has(k, &[Ctx::Normal, Ctx::Any]), "no normal row for {k}");
        }
        let any = [
            ":",
            "q",
            "g?",
            "C-l",
            "<leader>e",
            "<leader>E",
            "C-w h",
            "C-w w",
            "C-w W",
            "C-w j",
            "C-w k",
            "C-w p",
            "<leader>zf",
            "<leader>zs",
            "<leader>zz",
            "<leader>zb",
            "<leader>zl",
        ];
        for k in any {
            assert!(has(k, &[Ctx::Any]), "no row for {k} in both panes");
        }
        let side = [
            "j", "k", "gg", "G", "h", "l", "o", "Enter", "/", "Esc", "C-w l",
        ];
        for k in side {
            assert!(has(k, &[Ctx::Sidebar]), "no sidebar row for {k}");
        }
        for k in ["o", "y", "Y", "v", "V", "C-v", "Esc"] {
            assert!(has(k, &[Ctx::Visual]), "no visual row for {k}");
        }
        for k in ["C-n", "C-p", "Enter", "Esc"] {
            assert!(has(k, &[Ctx::Picker]), "no picker row for {k}");
        }
        for k in ["j", "k", "C-d", "C-u", "gg", "G", "/", "q", "Esc", "g?"] {
            assert!(has(k, &[Ctx::Help]), "no help row for {k}");
        }
    }
}
