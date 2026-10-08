//! Clipping text to a display width with `…`, by grapheme, so CJK and
//! emoji never split (docs/specs/2026-10-07-sidebar-layout.md D7).

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const ELLIPSIS: &str = "…";

/// The longest prefix of whole graphemes of `s` at most `width` columns.
fn head(s: &str, width: usize) -> &str {
    let mut used = 0;
    let mut end = 0;
    for (i, g) in s.grapheme_indices(true) {
        used += g.width();
        if used > width {
            break;
        }
        end = i + g.len();
    }
    &s[..end]
}

/// The longest suffix of whole graphemes of `s` at most `width` columns.
fn tail(s: &str, width: usize) -> &str {
    let mut used = 0;
    let mut start = s.len();
    for (i, g) in s.grapheme_indices(true).rev() {
        used += g.width();
        if used > width {
            break;
        }
        start = i;
    }
    &s[start..]
}

/// `s` in at most `width` columns, cut at the end with `…` when it does
/// not fit: `Installing on mac…`.
pub fn clip_end(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    format!("{}{ELLIPSIS}", head(s, width - 1))
}

/// `s` in at most `width` columns, cut at the start with `…` when it
/// does not fit, keeping the end: `…/notes/sub` (D10).
pub fn clip_start(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    format!("{ELLIPSIS}{}", tail(s, width - 1))
}

/// `s` in at most `width` columns, cut in the middle with `…` when it does
/// not fit, keeping the end: `2026-10-07-ram…-spec.md`. The end keeps at
/// least its last 6 columns, or the whole extension if that is longer, and
/// up to half the room.
pub fn clip_middle(s: &str, width: usize) -> String {
    if s.width() <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let room = width - 1;
    let ext = match s.rfind('.') {
        Some(i) if i > 0 => s[i..].width(),
        _ => 0,
    };
    let keep = ext.max(6).max(room / 2).min(room);
    let end = tail(s, keep);
    let start = head(s, room - end.width());
    format!("{start}{ELLIPSIS}{end}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> usize {
        s.width()
    }

    #[test]
    fn end_ascii() {
        assert_eq!(clip_end("Installing on macOS", 18), "Installing on mac…");
        assert_eq!(clip_end("short", 10), "short");
        assert_eq!(clip_end("exact", 5), "exact", "exact fit is not clipped");
        assert_eq!(clip_end("exact", 4), "exa…");
    }

    #[test]
    fn start_keeps_the_end() {
        assert_eq!(clip_start("~/notes/sub", 8), "…tes/sub");
        assert_eq!(clip_start("~/notes/sub", 11), "~/notes/sub");
        assert_eq!(clip_start("abc", 1), "…");
        assert_eq!(clip_start("abc", 0), "");
        let c = clip_start("日本語の見出し", 4);
        assert_eq!(c, "…し");
        assert!(w(&c) <= 4);
    }

    #[test]
    fn end_tiny_widths() {
        assert_eq!(clip_end("abc", 0), "");
        assert_eq!(clip_end("abc", 1), "…");
        assert_eq!(clip_end("", 0), "");
    }

    #[test]
    fn end_cjk_and_emoji_never_split() {
        // Each CJK char is 2 columns: 3 columns of room holds one, not 1.5.
        let c = clip_end("日本語の見出し", 4);
        assert_eq!(c, "日…");
        assert!(w(&c) <= 4);
        // A family emoji is one grapheme of several code points.
        let fam = "👨‍👩‍👧";
        let c = clip_end(&format!("{fam}{fam}{fam}"), 5);
        assert_eq!(c, format!("{fam}{fam}…"));
        assert!(w(&c) <= 5);
    }

    #[test]
    fn middle_keeps_the_extension() {
        let c = clip_middle("2026-10-07-ramble-spec.md", 16);
        assert!(c.ends_with("spec.md"), "{c}");
        assert!(c.starts_with("2026-10"), "{c}");
        assert!(c.contains('…'));
        assert_eq!(w(&c), 16);
        // A long extension is kept whole.
        let c = clip_middle("report-final.markdown", 12);
        assert!(c.ends_with(".markdown"), "{c}");
        assert_eq!(w(&c), 12);
    }

    #[test]
    fn middle_keeps_six_columns_without_an_extension() {
        let c = clip_middle("abcdefghijklmnopqrstuvwxyz", 10);
        assert!(c.ends_with("uvwxyz"), "{c}");
        assert_eq!(w(&c), 10);
        // A dotfile's leading dot is not an extension.
        let c = clip_middle(".abcdefghijklmnop", 8);
        assert!(c.ends_with("klmnop"), "{c}");
    }

    #[test]
    fn middle_tiny_widths_and_exact_fit() {
        assert_eq!(clip_middle("abc.md", 0), "");
        assert_eq!(clip_middle("abc.md", 1), "…");
        assert_eq!(clip_middle("abc.md", 6), "abc.md");
        let c = clip_middle("abcdef.md", 4);
        assert_eq!(c, "….md");
    }

    #[test]
    fn middle_cjk_and_emoji_never_split() {
        let c = clip_middle("日本語のファイル名.md", 11);
        assert!(w(&c) <= 11, "{c}");
        assert!(c.ends_with(".md"), "{c}");
        let fam = "👨‍👩‍👧";
        let s = format!("{fam}{fam}{fam}{fam}{fam}");
        let c = clip_middle(&s, 7);
        assert!(w(&c) <= 7);
        assert!(c.replace('…', "").replace(fam, "").is_empty(), "{c}");
    }
}
