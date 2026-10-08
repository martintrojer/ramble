//! ratatui widgets only, no logic. `draw` lays out the screen and calls one
//! widget per area; later units add a file and one call here.

pub mod clip;
mod clue;
mod content;
mod gutter;
mod help;
mod hints;
mod hover;
mod picker;
mod sidebar;
mod status;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};

use crate::app::App;

/// Draw one full screen: sidebar (when shown), content view, status line.
pub fn draw(frame: &mut Frame, app: &App) {
    let [main, status] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());
    let [side, content] =
        Layout::horizontal([Constraint::Length(app.sidebar_cols()), Constraint::Min(0)])
            .areas(main);
    if side.width > 0 {
        sidebar::draw(frame, app, side);
    }
    let (gutter_area, text_area) = gutter::split(app, content);
    gutter::draw(frame, app, gutter_area);
    content::draw(frame, app, text_area);
    hover::draw(frame, app, content);
    hints::draw(frame, app, content);
    status::draw(frame, app, status);
    sidebar::draw_prompt(frame, app, status);
    clue::draw(frame, app, main);
    picker::draw(frame, app, main);
    help::draw(frame, app, main);
    picker::draw_cmdline(frame, app, status);
}
