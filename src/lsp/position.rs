//! Byte offset <-> LSP position conversion, per negotiated encoding (spec R4).
//!
//! Every conversion between source byte offsets and LSP positions goes
//! through these functions. Lines are split on `'\n'` only: a trailing `'\r'`
//! is part of the line text, as LSP counts it.

use lsp_types::Position;

/// The unit an LSP `character` offset counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Encoding {
    /// UTF-8 code units (bytes).
    Utf8,
    /// UTF-16 code units (the LSP default).
    Utf16,
    /// Unicode code points (zk 0.15.6 uses these without advertising it).
    Utf32,
}

impl Encoding {
    /// Parses an LSP `PositionEncodingKind` string.
    pub fn from_lsp(kind: &str) -> Option<Encoding> {
        match kind {
            "utf-8" => Some(Encoding::Utf8),
            "utf-16" => Some(Encoding::Utf16),
            "utf-32" => Some(Encoding::Utf32),
            _ => None,
        }
    }

    fn width(self, c: char) -> u32 {
        match self {
            Encoding::Utf8 => c.len_utf8() as u32,
            Encoding::Utf16 => c.len_utf16() as u32,
            Encoding::Utf32 => 1,
        }
    }
}

/// Byte offset in `line_text` of `character` units. An offset inside a code
/// point (or between surrogates) rounds down to that code point's start; an
/// offset past the end clamps to `line_text.len()`.
pub fn to_byte(line_text: &str, character: u32, enc: Encoding) -> usize {
    let mut units = 0u32;
    for (i, c) in line_text.char_indices() {
        let next = units + enc.width(c);
        if next > character {
            return i;
        }
        units = next;
    }
    line_text.len()
}

/// Units before `byte` in `line_text`. A byte inside a code point rounds down
/// to the code point's start; a byte past the end clamps to the line length.
pub fn from_byte(line_text: &str, byte: usize, enc: Encoding) -> u32 {
    let b = floor_char_boundary(line_text, byte);
    line_text[..b].chars().map(|c| enc.width(c)).sum()
}

/// Byte offset in `source` of `pos`. A line past the end of `source` gives
/// `source.len()`.
pub fn position_to_byte(source: &str, pos: Position, enc: Encoding) -> usize {
    let mut start = 0usize;
    for _ in 0..pos.line {
        match source[start..].find('\n') {
            Some(i) => start += i + 1,
            None => return source.len(),
        }
    }
    let end = source[start..]
        .find('\n')
        .map_or(source.len(), |i| start + i);
    start + to_byte(&source[start..end], pos.character, enc)
}

/// LSP position of `byte` in `source` (clamped, rounded down to a char start).
pub fn byte_to_position(source: &str, byte: usize, enc: Encoding) -> Position {
    let b = floor_char_boundary(source, byte);
    let before = &source[..b];
    let line = before.matches('\n').count() as u32;
    let start = before.rfind('\n').map_or(0, |i| i + 1);
    Position::new(line, from_byte(&source[start..b], b - start, enc))
}

fn floor_char_boundary(s: &str, byte: usize) -> usize {
    let mut b = byte.min(s.len());
    while !s.is_char_boundary(b) {
        b -= 1;
    }
    b
}
