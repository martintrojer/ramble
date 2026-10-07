//! ratatui widgets only, no logic. `draw` lays out the screen and calls one
//! widget per area; later units add a file and one call here.

mod content;
mod hover;
mod status;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};

use crate::app::App;

/// Draw one full screen: content view plus status line.
pub fn draw(frame: &mut Frame, app: &App) {
    let [content, status] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());
    content::draw(frame, app, content);
    hover::draw(frame, app, content);
    status::draw(frame, app, status);
}
