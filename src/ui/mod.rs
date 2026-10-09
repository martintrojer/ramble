//! ratatui widgets only, no logic. `draw` lays out the screen and calls one
//! widget per area; later units add a file and one call here.

pub mod clip;
mod clue;
mod comment_box;
mod content;
mod gutter;
mod help;
mod hints;
mod hover;
mod picker;
pub(crate) mod sidebar;
mod status;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::widgets::Clear;

use crate::app::{App, Layout as Hits};
use crate::config::SidebarSide;

/// Draw one full screen: sidebar (when shown, on its side, beside the
/// content or over it), content view, status line.
/// Records where everything went in the app's layout (for the mouse).
pub fn draw(frame: &mut Frame, app: &App) {
    let [main, status] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());
    let cols = app.sidebar_cols();
    let (side, content) = match app.sidebar_side() {
        SidebarSide::Left => {
            let [s, c] =
                Layout::horizontal([Constraint::Length(cols), Constraint::Min(0)]).areas(main);
            (s, c)
        }
        SidebarSide::Right => {
            let [c, s] =
                Layout::horizontal([Constraint::Min(0), Constraint::Length(cols)]).areas(main);
            (s, c)
        }
    };
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
    // A peek without room beside the page is drawn over its sidebar-side
    // edge, above the text, gutter, hover and hints (D11).
    if app.sidebar_overlay() {
        let w = app.sidebar_drawn_cols().min(main.width);
        let x = match app.sidebar_side() {
            SidebarSide::Left => main.x,
            SidebarSide::Right => main.right() - w,
        };
        let area = Rect {
            x,
            width: w,
            ..main
        };
        if w > 0 {
            frame.render_widget(Clear, area);
            hits.sidebar = Some(area);
            (hits.files, hits.outline) = sidebar::draw(frame, app, area);
        }
    }
    // The comment box goes over an overlay sidebar: it is what you type in.
    comment_box::draw(frame, app, content);
    status::draw(frame, app, status);
    hits.status = Some(status);
    sidebar::draw_prompt(frame, app, status);
    hits.clue = clue::draw(frame, app, main);
    hits.picker = picker::draw(frame, app, main);
    hits.help = help::draw(frame, app, main);
    picker::draw_cmdline(frame, app, status);
    app.set_layout(hits);
}
