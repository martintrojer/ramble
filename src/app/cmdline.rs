//! The `:` command line (spec § Keymap notes): `:e <path>`, `:q`,
//! `:Notes`, `:Search <query>`, `:Tags`, `:Backlinks`, `:Links`,
//! `:Launch <name>`, `:Sidebar <files|outline|split|toggle|show|hide>`
//! (`off` = `hide`), `:Sidebar left|right` (the screen edge), `:Raw`, and
//! `:e` / `:Refresh` without an argument (refresh, as `C-l`).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::keys::{Action, KeyResult};
use super::sidebar::SidebarAction;
use super::{App, Mode};
use crate::config::{SidebarMode, SidebarSide};
use crate::notebook::Op;

/// The `:` commands and what they do, for the help overlay. Keep in step
/// with [`App::execute`].
pub(crate) const COMMANDS: &[(&str, &str)] = &[
    (":e <path>", "open a file (relative to this one)"),
    (":e", "re-read this file and the tree (as C-l)"),
    (":Refresh", "re-read this file and the tree (as C-l)"),
    (":q", "quit"),
    (":Notes", "notes picker"),
    (":Search <query>", "search notes"),
    (":Tags", "tags picker"),
    (":Backlinks", "backlinks picker"),
    (":Links", "links picker"),
    (":Launch <name>", "run a launcher by name"),
    (
        ":Sidebar files|outline|split",
        "set the sidebar mode and show it",
    ),
    (
        ":Sidebar toggle|show|hide",
        "show or hide the sidebar (off = hide)",
    ),
    (":Sidebar left|right", "put the sidebar on that side"),
    (":Raw", "toggle the raw source view"),
];

/// Keys while typing a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmdAction {
    Start,
    Input(char),
    Backspace,
    Run,
    Cancel,
}

pub(super) fn cmdline_keymap(keys: &[KeyEvent]) -> KeyResult {
    let Some(key) = keys.last() else {
        return KeyResult::None;
    };
    let a = match key.code {
        KeyCode::Enter => CmdAction::Run,
        KeyCode::Esc => CmdAction::Cancel,
        KeyCode::Backspace => CmdAction::Backspace,
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => CmdAction::Input(c),
        _ => return KeyResult::None,
    };
    KeyResult::Action(Action::Cmd(a))
}

impl App {
    /// The command being typed (`:text`).
    pub fn cmdline_prompt(&self) -> Option<String> {
        self.cmdline.as_ref().map(|c| format!(":{c}"))
    }

    pub(super) fn cmd_action(&mut self, a: CmdAction) {
        match a {
            CmdAction::Start => {
                self.cmdline = Some(String::new());
                self.mode = Mode::Command;
            }
            CmdAction::Input(c) => {
                if let Some(s) = &mut self.cmdline {
                    s.push(c);
                }
            }
            CmdAction::Backspace => match &mut self.cmdline {
                // Backspace on an empty line leaves it, as in vim.
                Some(s) if s.is_empty() => self.cmd_end(),
                Some(s) => {
                    s.pop();
                }
                None => {}
            },
            CmdAction::Cancel => self.cmd_end(),
            CmdAction::Run => {
                let line = self.cmdline.take().unwrap_or_default();
                self.cmd_end();
                self.execute(&line);
            }
        }
    }

    fn cmd_end(&mut self) {
        self.cmdline = None;
        if self.mode == Mode::Command {
            self.mode = Mode::Normal;
        }
    }

    /// Run one command line (without the `:`).
    pub fn execute(&mut self, line: &str) {
        let line = line.trim();
        let (cmd, arg) = match line.split_once(char::is_whitespace) {
            Some((c, a)) => (c, a.trim()),
            None => (line, ""),
        };
        match cmd {
            "" => {}
            "q" | "q!" | "qa" | "quit" => self.quit = true,
            "e" | "edit" | "Refresh" if arg.is_empty() => self.refresh(),
            "e" | "edit" => {
                let path = self.link_dir().join(arg);
                self.open_path_at(&path, None);
            }
            "Notes" => self.open_op(Op::Notes, None),
            "Search" => self.open_op(Op::Search, Some(arg)),
            "Tags" => self.open_op(Op::Tags, None),
            "Backlinks" => self.open_op(Op::Backlinks, None),
            "Links" => self.open_op(Op::Links, None),
            "Raw" => self.toggle_raw(),
            "Launch" if arg.is_empty() => self.set_status(":Launch needs a name"),
            "Launch" => self.launch(arg),
            "Sidebar" => match parse_sidebar(arg) {
                Some(SidebarCmd::Mode(m)) => self.pick_sidebar_mode(m),
                Some(SidebarCmd::Toggle) => self.sidebar_action(SidebarAction::Toggle),
                Some(SidebarCmd::Show(show)) => self.show_sidebar(show),
                Some(SidebarCmd::Side(side)) => self.set_sidebar_side(side),
                None => self.set_status(":Sidebar files|outline|split|toggle|show|hide|left|right"),
            },
            _ => self.set_status(format!("Not a command: {cmd}")),
        }
    }
}

enum SidebarCmd {
    Mode(SidebarMode),
    Toggle,
    Show(bool),
    Side(SidebarSide),
}

fn parse_sidebar(s: &str) -> Option<SidebarCmd> {
    use SidebarCmd as C;
    Some(match s {
        "files" => C::Mode(SidebarMode::Files),
        "outline" => C::Mode(SidebarMode::Outline),
        "split" => C::Mode(SidebarMode::Split),
        "toggle" => C::Toggle,
        "show" => C::Show(true),
        "hide" | "off" => C::Show(false),
        "left" => C::Side(SidebarSide::Left),
        "right" => C::Side(SidebarSide::Right),
        _ => return None,
    })
}
