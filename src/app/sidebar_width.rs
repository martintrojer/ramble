//! The auto sidebar width (docs/design.md § Sidebar): a
//! pure function over row widths, plus the row measurement it is fed.

use unicode_width::UnicodeWidthStr;

/// Columns the content keeps at least before the sidebar is dropped.
pub(crate) const MIN_CONTENT: u16 = 10;

/// The current-row marker gutter at the left edge of each pane.
pub(crate) const GUTTER_COLS: usize = 1;
/// Columns of a tree row before the name: the folder arrow or file
/// spacer (`▸ `).
pub(crate) const ICON_COLS: usize = 2;

/// The sidebar width in columns, not counting its border, for rows that
/// need `need` columns each, on a terminal `cols` wide whose page is
/// capped at `page_cap` columns (0 with no page).
///
/// 1. The 90th percentile of `need` (the maximum under 10 rows), so one
///    long name doesn't stretch it while a short list fits exactly.
/// 2. Plus 1 column of right padding.
/// 3. Clamped to `[min, min(max, 35% of cols)]`.
/// 4. The upper bound rises to `max` while the page still gets its full
///    `page_cap`: columns past the cap don't help the page.
///
/// Last, the page keeps `MIN_CONTENT` columns when that leaves at least
/// `min`; below that the caller drops the sidebar.
pub(crate) fn sidebar_width(need: &[u16], cols: u16, page_cap: u16, min: u16, max: u16) -> u16 {
    let mut sorted = need.to_vec();
    sorted.sort_unstable();
    let fit = match sorted.len() {
        0 => 0,
        n if n < 10 => sorted[n - 1],
        // Rounded up to the next row: of 30 rows, the 27th.
        n => sorted[(n * 9).div_ceil(10) - 1],
    };
    let want = fit.saturating_add(1);
    let pct = (u32::from(cols) * 35 / 100) as u16;
    let spare = cols.saturating_sub(1).saturating_sub(page_cap);
    let hi = pct.min(max).max(spare.min(max));
    let guard = cols.saturating_sub(1 + MIN_CONTENT);
    want.min(hi).max(min).min(guard.max(min))
}

/// Display width of `s`, saturated to `u16`.
pub(crate) fn cols(s: &str) -> u16 {
    s.width().min(u16::MAX as usize) as u16
}

/// What a tree row needs: gutter, indent, icon, name and review mark.
pub(crate) fn tree_row_need(depth: usize, name: &str, mark: &str) -> u16 {
    let n = GUTTER_COLS + 2 * depth + ICON_COLS + name.width() + mark.width();
    n.min(u16::MAX as usize) as u16
}

/// What an outline row needs: gutter, indent and text.
pub(crate) fn outline_row_need(level: u8, text: &str) -> u16 {
    let n = GUTTER_COLS + 2 * usize::from(level.saturating_sub(1)) + text.width();
    n.min(u16::MAX as usize) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u16 = 16;
    const MAX: u16 = 48;

    #[test]
    fn short_outline_gives_min_width() {
        assert_eq!(sidebar_width(&[7, 5, 9], 120, 100, MIN, MAX), MIN);
        assert_eq!(sidebar_width(&[], 120, 100, MIN, MAX), MIN);
    }

    #[test]
    fn one_long_name_is_ignored_at_p90() {
        let mut need = vec![20; 30];
        need.push(120);
        assert_eq!(sidebar_width(&need, 120, 100, MIN, MAX), 21);
    }

    #[test]
    fn p90_rounds_up_to_the_next_row() {
        // 30 rows: the 27th smallest, so the top 3 are ignored.
        let need: Vec<u16> = (1..=30).collect();
        assert_eq!(sidebar_width(&need, 200, 100, 1, 100), 28);
        // 11 rows: ceil(9.9) = the 10th.
        let need: Vec<u16> = (1..=11).collect();
        assert_eq!(sidebar_width(&need, 200, 100, 1, 100), 11);
    }

    #[test]
    fn fewer_than_ten_rows_gives_the_maximum() {
        assert_eq!(sidebar_width(&[10, 30, 12], 120, 100, MIN, MAX), 31);
    }

    #[test]
    fn clamps_to_35_percent_of_the_terminal() {
        assert_eq!(sidebar_width(&[60], 100, 100, MIN, MAX), 35);
        assert_eq!(sidebar_width(&[60], 400, 100, MIN, MAX), MAX);
        assert_eq!(sidebar_width(&[1], 400, 100, MIN, MAX), MIN);
    }

    #[test]
    fn wide_terminal_grows_into_room_the_page_cannot_use() {
        // 35% of 120 is 42, but a page capped at 70 leaves 49 spare
        // columns: the sidebar takes what it needs up to max_width.
        assert_eq!(sidebar_width(&[60], 120, 70, MIN, MAX), MAX);
        assert_eq!(sidebar_width(&[44], 120, 70, MIN, MAX), 45);
        // With page_cap 100 the same terminal has 19 spare: 35% holds.
        assert_eq!(sidebar_width(&[60], 120, 100, MIN, MAX), 42);
        // Growing never pushes the page below its cap.
        let w = sidebar_width(&[60], 130, 85, MIN, MAX);
        assert_eq!(w, 45, "35% of 130");
        let w = sidebar_width(&[60], 140, 85, MIN, MAX);
        assert_eq!(w, 54.min(MAX), "140 - 1 - 85 = 54 spare");
        assert!(140 - w > 85, "the page keeps its cap");
    }

    #[test]
    fn page_keeps_min_content_unless_min_width_cannot_fit() {
        // 35% of 30 is 10 -> min 16; the page keeps 30 - 17 = 13.
        assert_eq!(sidebar_width(&[40], 30, 100, MIN, MAX), MIN);
        // A min_width of 4 on 12 columns: the guard leaves 1, so 4.
        assert_eq!(sidebar_width(&[40], 12, 100, 4, MAX), 4);
        // No page (cap 0) still keeps MIN_CONTENT columns.
        assert_eq!(sidebar_width(&[40], 40, 0, MIN, MAX), 29);
    }

    #[test]
    fn row_needs() {
        assert_eq!(tree_row_need(0, "a.md", ""), 7);
        assert_eq!(tree_row_need(2, "a.md", " ● 3"), 15);
        assert_eq!(tree_row_need(0, "日本.md", ""), 10);
        assert_eq!(outline_row_need(1, "Title"), 6);
        assert_eq!(outline_row_need(3, "Deep"), 9);
    }
}
