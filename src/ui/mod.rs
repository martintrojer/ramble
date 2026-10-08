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
pub(crate) mod sidebar;
mod status;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};

use crate::app::{App, Layout as Hits};

/// Draw one full screen: sidebar (when shown), content view, status line.
/// Records where everything went in the app's layout (for the mouse).
pub fn draw(frame: &mut Frame, app: &App) {
    let [main, status] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());
    let [side, content] =
        Layout::horizontal([Constraint::Length(app.sidebar_cols()), Constraint::Min(0)])
            .areas(main);
    let mut hits = Hits::default();
    if side.width > 0 {
        hits.sidebar = Some(side);
        (hits.files, hits.outline) = sidebar::draw(frame, app, side);
    }
    let (gutter_area, text_area) = gutter::split(app, content);
    gutter::draw(frame, app, gutter_area);
    content::draw(frame, app, text_area);
    hits.gutter = (gutter_area.width > 0).then_some(gutter_area);
    hits.text = (app.page().is_some() && text_area.width > 0).then_some(text_area);
    hits.banner = app.banner().is_some();
    hits.hover = hover::draw(frame, app, content);
    hints::draw(frame, app, content);
    status::draw(frame, app, status);
    hits.status = Some(status);
    sidebar::draw_prompt(frame, app, status);
    hits.clue = clue::draw(frame, app, main);
    hits.picker = picker::draw(frame, app, main);
    hits.help = help::draw(frame, app, main);
    picker::draw_cmdline(frame, app, status);
    app.set_layout(hits);
}
