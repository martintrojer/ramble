//! Auto-hiding vertical scrollbars: ratatui's [`Scrollbar`], styled like
//! pi's (the thumb alone: no arrows, no track), on the right edge of a
//! long list, shown for [`SHOW_FOR`] after its offset moves. The event
//! loop redraws at least every 100 ms, so a bar hides on its own.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};

use crate::app::App;

/// How long a bar stays after its list last scrolled.
pub const SHOW_FOR: Duration = Duration::from_millis(1000);

/// One list's last draw: the frame, its offset and length, and when it
/// last scrolled.
#[derive(Clone, Copy)]
struct Seen {
    frame: u64,
    pos: usize,
    total: usize,
    moved: Option<Instant>,
}

/// Per list (by key), what the last frames drew. Drawing takes `&App`,
/// hence the cell.
#[derive(Default)]
pub struct Scrollbars(RefCell<(u64, HashMap<&'static str, Seen>)>);

impl Scrollbars {
    /// Start a frame: a list not drawn in the previous one starts fresh.
    pub fn next_frame(&self) {
        self.0.borrow_mut().0 += 1;
    }

    /// Record list `key` at offset `pos` of `total` rows; whether its bar
    /// shows at `now`. Only a move of a list drawn last frame with the
    /// same length is a scroll: a list's first frame (an overlay opening,
    /// a pane coming back) and a change of content (another commit, a
    /// filter, a reload) are not.
    pub fn visible(&self, key: &'static str, pos: usize, total: usize, now: Instant) -> bool {
        let (frame, map) = &mut *self.0.borrow_mut();
        let frame = *frame;
        let prev = map.insert(
            key,
            Seen {
                frame,
                pos,
                total,
                moved: None,
            },
        );
        let moved = match prev {
            Some(p) if p.frame + 1 >= frame && p.total == total => {
                if p.pos != pos {
                    Some(now)
                } else {
                    p.moved
                }
            }
            _ => None,
        };
        map.get_mut(key).expect("just inserted").moved = moved;
        moved.is_some_and(|t| now.duration_since(t) < SHOW_FOR)
    }
}

/// Draw list `key`'s bar on the right column of `area` when
/// `[render] scrollbars` is on, `total` rows overflow it and it scrolled
/// lately; `pos` is the first row shown.
pub(super) fn draw(
    frame: &mut Frame,
    app: &App,
    key: &'static str,
    area: Rect,
    total: usize,
    pos: usize,
) {
    if !app.config().render.scrollbars {
        return;
    }
    let shown = app.scrollbars().visible(key, pos, total, Instant::now());
    let h = area.height as usize;
    if !shown || total <= h || area.width == 0 {
        return;
    }
    // Positions run 0..=max offset, so the thumb reaches the bottom.
    let mut state = ScrollbarState::new(total - h + 1)
        .position(pos)
        .viewport_content_length(h);
    // pi's look: the thumb alone, no arrows or track.
    let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .end_symbol(None)
        .track_symbol(None)
        .thumb_symbol("┃")
        .thumb_style(Style::new().fg(app.theme().palette.text));
    frame.render_stateful_widget(bar, area, &mut state);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bar_shows_after_a_scroll_and_hides_after_show_for() {
        let bars = Scrollbars::default();
        let t = Instant::now();
        let frame = |pos, at| {
            bars.next_frame();
            bars.visible("l", pos, 50, at)
        };
        assert!(!frame(0, t), "first sight is not a scroll");
        assert!(!frame(0, t + SHOW_FOR), "no move");
        let t1 = t + SHOW_FOR;
        assert!(frame(3, t1), "scrolled");
        assert!(frame(3, t1 + SHOW_FOR / 2), "still fresh");
        assert!(!frame(3, t1 + SHOW_FOR), "expired");
    }

    #[test]
    fn reappearing_or_new_content_is_not_a_scroll() {
        let bars = Scrollbars::default();
        let t = Instant::now();
        bars.next_frame();
        assert!(!bars.visible("l", 7, 50, t));
        // Not drawn for a frame (an overlay closed), back at the top.
        bars.next_frame();
        bars.next_frame();
        assert!(!bars.visible("l", 0, 50, t), "reopened");
        // Drawn every frame, but the content changed under the offset.
        bars.next_frame();
        assert!(!bars.visible("l", 4, 60, t), "new content");
        bars.next_frame();
        assert!(bars.visible("l", 5, 60, t), "then a scroll");
        bars.next_frame();
        assert!(!bars.visible("other", 9, 60, t), "lists are separate");
    }
}
