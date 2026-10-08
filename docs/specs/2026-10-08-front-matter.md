# Front matter in the normal view

Status: approved design (user answered Q1–Q6 "agreed to all", plus: parse
as generously as possible; other tools can police the format).

## Problem

Normal view drops front matter entirely (`src/doc.rs:235` records
`Document::front_matter`; nothing draws it). Only raw view (`gR`) shows it,
so a reader can't tell a page has metadata, let alone what it says.

## Behaviour

- **Folded marker (default).** When a page has front matter, the first
  rendered row is a dim marker:
  `▸ front matter · 5 keys` (or `· 1 key`).
- **Toggle.** `za` (vim fold toggle), or `Enter` with the cursor on the
  marker row, expands or folds it. Once mouse support is on main, a
  double-click on the marker toggles it too. The state is per page, kept
  across reloads of the same page, and not remembered between sessions.
  Folded is the default for every newly opened page.
- **Expanded.** The marker becomes `▾ front matter`, followed by one row
  per top-level key, in source order:
  - Keys are dim and padded to the widest key (capped at 20 columns).
  - Values are in normal text, clipped with `…` at the content width.
  - **Scalars** are shown as written in the source, so quotes, dates and
    numbers are not reformatted.
  - **Lists** are joined with `, ` (`tags  rust, tui`).
  - **Nested maps** are shown as `{…}`.
  - **Empty values** are shown as an empty value.
  - A blank row follows the block.
- **Raw view** (`gR`) is unchanged: the source, front matter included.
- **Print mode** (`--print`) is unchanged: front matter is left out.
- **TOML front matter** (`+++` … `+++`) is supported as well as YAML
  (`---` … `---`). Turn on pulldown-cmark's
  `ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS`. Parse it with the existing
  `toml` crate.
- **No special keys.** `title`, `tags` and the rest are all shown the
  same way.

## Parsing: generous, best effort

The goal is to show whatever data can be extracted. ramble is not a
linter. Every step falls back to the next one rather than failing.

1. **Real parser.** YAML uses a YAML library (see Dependencies) and TOML
   uses `toml`. If the result is a mapping, use its top-level entries.
   For the displayed text, take each value's source text where it can be
   recovered (step 2's line scan gives it). Use the parser's value only
   for its shape: list, map or scalar.
2. **Line scan, used when the parser fails or returns something other
   than a mapping.** Walk the block line by line:
   - A line `key: value` or `key = value` at the smallest indentation in
     the block starts an entry. `key` is any run without `:` or `=`,
     with quotes trimmed.
   - Lines that follow with more indentation belong to the current entry:
     - `- item` lines become list items;
     - lines starting with `key:` make the entry a nested map;
     - anything else is joined to the value with a space (folded
       continuation).
   - Inline lists `[a, b]` are split on commas, outside quotes.
   - Surrounding quotes are stripped from values.
   - Lines that start no entry and continue none (stray text, comments
     `#`) are skipped.
3. **Nothing extracted.** If neither step finds any key, the marker reads
   `▸ front matter · unparsed`. Expanding it shows the raw lines, dimmed.

The page body always renders, whatever the front matter contains.

## Implementation

- `src/frontmatter.rs` (new):
  - `pub struct FrontMatter { pub entries: Vec<(String, FmValue)>, pub parsed: bool }`
  - `pub enum FmValue { Scalar(String), List(Vec<String>), Map }`
  - `pub fn parse(src: &str, kind: FmKind) -> FrontMatter`, where
    `FmKind` is `Yaml` or `Toml`. It is pure and has unit tests.
- `src/doc.rs`: record the kind next to the range. Enable the
  pluses-delimited option at `doc.rs:445`. Strip the delimiters before
  parsing.
- `src/render.rs`: emit the marker row, and the entry rows when expanded,
  at the top of the rendered page. These rows map to the front-matter
  byte range in the srcmap, so the cursor, visual selection and yank
  (which copies the source) all work on them. Rendering takes an
  `expanded: bool` input.
- `src/app`: a per-page `fm_expanded: bool`, the `za` binding
  (`z` is already a prefix, for `zz`/`zt`/`zb`), and `Enter` on the
  marker row (checked before link-follow). Toggling re-renders through
  the existing layout path. Add a help `BINDINGS` row for `za`; the clue
  box picks it up automatically.
- Dependencies: one YAML crate. Use `saphyr` if it builds cleanly on the
  current toolchain, otherwise `serde_yaml_ng`. Record which one in the
  commit message.
- README: one line under Reading. Spec `2026-10-07-ramble.md`: note that
  front matter now has a folded marker.

## Testing

- Unit tests for `frontmatter::parse`:
  - valid YAML: scalars, lists (block and inline), a nested map, quoted
    values, a date;
  - broken YAML (bad indentation, a tab, an unclosed quote, a duplicate
    key) still yields entries through the line scan;
  - TOML, including broken TOML falling back to `key = value` lines;
  - an empty block;
  - only comments or stray text gives `parsed = false` and no entries.
- Render (TestBackend or rendered rows):
  - The folded marker appears with the key count.
  - Expanded shows rows in source order, with keys padded and values
    clipped.
  - The unparsed case shows the raw lines.
  - A page without front matter has no marker.
  - `--print` output is unchanged (an existing test, or a new one).
- App:
  - `za` toggles.
  - `Enter` on the marker toggles; `Enter` on a link still follows it.
  - The state survives a reload and resets on a new page.
  - The cursor and yank on an entry row copy source text.
  - The help row resolves (existing test).

## Out of scope

- Editing front matter.
- Treating specific keys specially.
- Showing nested maps in full.
- Front matter in `--print`.
- JSON front matter.
