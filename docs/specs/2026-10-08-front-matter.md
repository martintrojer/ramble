# Front matter in the normal view

Status: current design.

## Problem

A reader should see that a page has metadata, and what it says, without
switching to raw view (`gR`). Front matter is never drawn as markdown: it
is metadata, not headings or rules.

## Behaviour

- **Folded marker (default).** When a page has front matter, the first
  rendered row is a dim marker:
  `▸ front matter · 5 keys` (or `· 1 key`).
- **Toggle.** `za` (vim fold toggle), or `Enter` with the cursor on the
  marker row, expands or folds it. The state is per page, kept
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
  (`---` … `---`). `mdroots::syntax::markdown_options()` turns on
  pulldown-cmark's metadata blocks for both.
- **No special keys.** `title`, `tags` and the rest are all shown the
  same way.

## Parsing: generous, best effort

The goal is to show whatever data can be extracted. ramble is not a
linter. Every step falls back to the next one rather than failing. The
parsing is done by
[mdroots](https://github.com/martintrojer/mdroots)' `mdroots::syntax`;
these are the rules it follows for YAML and TOML blocks.

1. **Real parser.** YAML uses a YAML parser and TOML a TOML parser. If
   the result is a mapping, use its top-level entries. For the displayed
   text, take each value's source text where it can be recovered (step
   2's line scan gives it). Use the parser's value only for its shape:
   list, map or scalar.
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

- Parsing lives in mdroots (`mdroots::syntax`), not in ramble. Its
  `Frontmatter` gives the block's range and format (`Yaml` or `Toml`),
  `fields()` (key, `FieldValue`, the entry's source lines), `parsed()`
  and `inner()` (the bytes between the fences); `FieldValue::display` is
  the value as drawn.
- `src/doc.rs`: one `mdroots::syntax::parse_with` per page, with
  `unfenced_frontmatter` off (an unfenced header is prose to ramble).
  `Document` keeps the block's range, its kind (`FmKind::Yaml` or `Toml`)
  and the `Frontmatter` itself (`Document::fm`). A blank block is masked
  out of the pulldown-cmark layout parse, which emits no metadata block
  for one.
- `src/render.rs`: emit the marker row, and the entry rows when expanded,
  at the top of the rendered page. These rows map to the front-matter
  byte range in the srcmap, so the cursor, visual selection and yank
  (which copies the source) all work on them. Rendering takes an
  `expanded: bool` input.
- `src/app`: a per-page `fm_expanded: bool`, the `za` binding
  (`z` is already a prefix, for `zz`/`zt`/`zb`), and `Enter` on the
  marker row (checked before link-follow), in `src/app/fold.rs`.
  Toggling re-renders through the layout path. A help `BINDINGS` row
  lists `za`; the clue box picks it up.
- Dependencies: none of ramble's own; YAML and TOML parsing come with
  mdroots.

## Testing

- Parser tests live in mdroots (the `mdroots-syntax` crate,
  `tests/frontmatter.rs`):
  - valid YAML: scalars, lists (block and inline), a nested map, quoted
    values, a date;
  - broken YAML (bad indentation, a tab, an unclosed quote, a duplicate
    key) still yields entries through the line scan;
  - TOML, including broken TOML falling back to `key = value` lines;
  - an empty block;
  - only comments or stray text gives `parsed = false` and no entries.
- ramble's `tests/frontmatter.rs` keeps the UI tests. Render (rendered
  rows):
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
