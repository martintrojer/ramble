//! The `:` command line (spec § Keymap notes): `:e <path>`, `:q`,
//! `:Notes`, `:Search <query>`, `:Tags`, `:Backlinks`, `:Links`,
//! `:Launch <name>`, `:Sidebar <off|files|outline|split>`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::keys::{Action, KeyResult};
use super::{App, Mode};
use crate::config::SidebarMode;
use crate::notebook::Op;

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
            "e" | "edit" if arg.is_empty() => self.set_status(":e needs a path"),
            "e" | "edit" => {
                let path = self.link_dir().join(arg);
                self.open_path_at(&path, None);
            }
            "Notes" => self.open_op(Op::Notes, None),
            "Search" => self.open_op(Op::Search, Some(arg)),
            "Tags" => self.open_op(Op::Tags, None),
            "Backlinks" => self.open_op(Op::Backlinks, None),
            "Links" => self.open_op(Op::Links, None),
            "Launch" if arg.is_empty() => self.set_status(":Launch needs a name"),
            "Launch" => self.launch(arg),
            "Sidebar" => match parse_sidebar(arg) {
                Some(m) => self.set_sidebar_mode(m),
                None => self.set_status(":Sidebar off|files|outline|split"),
            },
            _ => self.set_status(format!("Not a command: {cmd}")),
        }
    }
}

fn parse_sidebar(s: &str) -> Option<SidebarMode> {
    Some(match s {
        "off" => SidebarMode::Off,
        "files" => SidebarMode::Files,
        "outline" => SidebarMode::Outline,
        "split" => SidebarMode::Split,
        _ => return None,
    })
}
