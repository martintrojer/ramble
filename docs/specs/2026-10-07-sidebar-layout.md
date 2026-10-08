# Sidebar: visibility, modes, sizing, clipping

Status: approved design (user answers Q1–Q6, plus the directory-start rule).

## Problem

The sidebar mixes two questions in one setting, wastes space, and draws
text badly.

- Showing it and choosing what it shows are the same control. `<leader>e`
  cycles off → files → outline → split, and `sidebar.default` takes `off`
  alongside the modes. Hiding the sidebar and getting back to the outline
  takes three presses.
- Its width is a fixed `sidebar.width = 30`. A short outline leaves most of
  the column empty, and a deep tree with long names doesn't fit.
- Rows that don't fit are cut off at the border by ratatui, mid-character
  and with no sign that anything is missing. The `◂` current-heading mark
  and the `●` review mark fall off with them.
- It only hides when the page would get fewer than 10 columns. On a
  60-column terminal that leaves a 30-column sidebar beside a 29-column
  page.

## Decisions

### D1. Showing is separate from the mode

| Key | Action |
|---|---|
| `<leader>e` | show or hide the sidebar |
| `<leader>E` | cycle the mode: outline → files → split → outline |

This matches `<leader>e` in nvim-tree, neo-tree and LazyVim. `<leader>E`
works while the sidebar is hidden: it changes the mode and shows the
sidebar, so the key always has a visible effect.

`:Sidebar files|outline|split` sets the mode and shows the sidebar.
`:Sidebar toggle`, `:Sidebar show` and `:Sidebar hide` control visibility.
`:Sidebar off` is kept as an alias of `hide`.

### D2. Config

```toml
[sidebar]
show = true          # shown at start (see D3 and D6 for the exceptions)
default = "auto"     # auto | files | outline | split
reading = "outline"  # what auto shows while a page is loaded
width = "auto"       # "auto", or a number of columns
min_width = 16
max_width = 48       # also capped at 35% of the terminal width
auto_hide_below = 80 # terminal columns; 0 disables
split_ratio = 0.5
show_all = false
```

Older configs keep working without a warning. `default = "off"` means
`show = false` with `default = "auto"`, and a numeric `width` keeps its old
meaning.

`SidebarMode` loses `Off`. The parse accepts `"off"` and maps it as above.
Code that matched on `Off` now asks whether the sidebar is shown (D3).

### D3. Visibility state

Visibility is one global flag, not part of each history entry. History
still remembers the mode per page (`nav.rs` `Entry.sidebar`), but restoring
an entry never shows or hides the sidebar.

Whether the sidebar is drawn is decided in this order:

1. **No page loaded** (a directory argument, or no argument, which starts
   in the current directory): the file tree is always drawn and has focus.
   This overrides `show = false`, auto-hide and a narrow terminal. With no
   page, the tree is the only thing on screen. (User rule.)
2. **The user pressed `<leader>e` or `:Sidebar ...` while the terminal was
   narrower than `auto_hide_below`**: their choice holds until the next
   resize.
3. **The terminal is narrower than `auto_hide_below`**: hidden.
4. **Otherwise**: the user's flag, which starts at `sidebar.show`.

When the terminal grows past the threshold, the sidebar comes back unless
the user hid it themselves (step 4 then gives false). A resize clears the
step-2 override.

The existing guard stays as a last resort: if what's left is narrower than
`MIN_CONTENT` (10), don't draw the sidebar. That can only happen when
`auto_hide_below` is 0 or very small.

When opening the first page replaces the directory view, the rules move
from step 1 to steps 2–4, so the sidebar may hide at that point
(`show = false` or a narrow terminal). Focus moves to the content then.

### D4. Directory start focuses the tree

With no page at start, focus begins on the files pane (`Focus::Files`) with
the first row selected, and `j`/`k`/`Enter` work immediately. Today focus
starts on the content, which shows only the placeholder message. Opening a
file from the tree moves focus to the content, as it does now.

### D5. Auto width

Width means the sidebar columns, not counting the 1-column border. It is
computed in a pure function `sidebar_width(inputs) -> u16` that the UI
code doesn't depend on, so it can be tested with plain values.

Inputs:
- `need`: the display width of each row in each pane shown, measured as it
  would be drawn: indent + icon + name + markers. The files pane counts
  visible tree rows (expanded folders only). The outline pane counts every
  heading at its indent, plus the 1-column marker gutter (D8). Rows with a review mark
  add ` ● N`. Each pane title is a row too. Width is measured with
  `unicode-width`.
- `cols`: the terminal width.
- `page_cap`: `render.max_width`.

Rule:
1. Sort `need` and take the 90th percentile, rounded up to the next row.
   With fewer than 10 rows, take the maximum. One long name then doesn't
   stretch the sidebar, while a short list fits exactly.
2. Add 1 column of right padding.
3. Clamp to `[min_width, min(max_width, 35% of cols)]`.
4. Give it spare room: if the page area left over (`cols - width - 1`) is
   wider than `page_cap`, the extra columns don't help the page. The
   sidebar can grow into them up to `max_width`, and the page keeps its
   full `page_cap`. The sidebar never takes columns that would push the
   page below `page_cap`, beyond what step 3 already allows.

When it is recomputed:
- On a page change, a mode change, showing the sidebar, a resize, and a
  tree refresh (`C-l`).
- Expanding a folder or changing a filter may widen the sidebar right
  away, but never narrows it. It narrows at the next page change, so it
  doesn't shift while you browse the tree.

A number in `width` turns all of this off. It is still clamped so the page
keeps `MIN_CONTENT`.

Every width change goes through the existing re-layout path
(`resize(cols, rows)`). That path is the source of the srcmap, the hint
labels and the review gutter, so they stay correct.

### D6. Narrow auto-hide

Covered by D3, steps 2–3. At startup the same rule applies: if the
terminal is narrower than `auto_hide_below` and a page is loaded, the
sidebar starts hidden. The current "window too narrow" status message is
dropped for auto-hide. It is kept only for the `MIN_CONTENT` last resort,
where the user asked for something that can't be drawn.

### D7. Clipping

Each row is laid out at the pane width. Markers are drawn first, then the
name gets whatever room is left.

- **Headings**: clip at the end with `…` (`Installing on mac…`).
- **File and folder names**: clip in the middle with `…` and keep the end,
  extension included (`2026-10-07-ram…-spec.md`). A name keeps at least
  its last 6 columns, or the whole extension if that is longer.
- **Markers**: the current marker sits in the left gutter (D8), so clipping
  never touches it. ` ● N` is never clipped. When there is no room
  for the count, ` ● N` drops to ` ●`, as `review_mark` already does.
- **Deep nesting**: when the indent would leave fewer than 8 columns for
  the name, stop indenting at that depth, so the name keeps at least 8
  columns. Below a depth limit the tree loses some of its visual
  structure, but the names stay readable.
- All clipping is by display width and grapheme. `unicode-width` measures
  and `unicode-segmentation` splits (both are already in the dependency
  tree via ratatui; add them as direct dependencies if needed), so CJK
  and emoji never split.

The clip helpers (`clip_end(&str, width)`, `clip_middle(&str, width)`) are
pure functions in `src/ui/clip.rs`, with their own tests.

### D8. Current marker on the left (user nit)

The current heading (outline) and the open file (files tree) are marked in a
1-column gutter at the left edge of each pane, before the indent, so a long
name can never push the marker out of view. Both panes always reserve this
gutter, so rows don't shift when the marker moves. The glyph is `▎` in
peach, replacing the trailing ` ◂`. The files pane marks the row whose path
is the current page's file. The selection highlight is unchanged.

## Out of scope

- Resizing the sidebar with the mouse or keys (`C-w <`/`C-w >`). That can
  come later. The `width` config covers a fixed preference.
- A sidebar on the right.
- Remembering visibility between sessions.

## Testing

- `sidebar_width`: unit tests over plain inputs.
  - A short outline gives `min_width`.
  - One 120-column name among 30 short ones is ignored at p90.
  - Fewer than 10 rows gives the maximum.
  - The clamps work: 35% of a 100-column terminal is 35.
  - A wide terminal with `page_cap` 100 grows the sidebar.
  - A fixed `width` is used as given.
- Visibility: App tests in `tests/sidebar.rs`, covering each D3 step.
  - No page plus `show = false` plus a 60-column terminal still draws the
    tree with focus.
  - A narrow terminal hides it; widening shows it again.
  - The user hides it, then the terminal widens: it stays hidden.
  - `<leader>e` while narrow shows it, and the next resize hides it.
  - History restore never changes visibility.
- Keys: `<leader>e` and `<leader>E`, plus `<leader>E` while hidden. The
  help rows are updated, and `representative_bindings_have_rows` covers
  `<leader>E`.
- Clip helpers: ASCII, CJK and emoji. Widths 0, 1 and the exact fit.
  Middle clipping keeps the extension.
- Rendering (TestBackend):
  - A long heading ends in `…` and the current-heading marker is still drawn
    in the left gutter.
  - A long file name keeps `.md`.
  - A deep tree caps its indent.
  - A `●` row keeps its mark.
- Config: `show`, `width = "auto"`, a numeric `width`, `min_width` and
  `max_width`, `auto_hide_below`. Legacy `default = "off"` maps to
  `show = false`.

## Implementation checklist

1. config: `show`, `width: Auto | Fixed(u16)`, `min_width`, `max_width`,
   `auto_hide_below`. Drop `SidebarMode::Off` and parse legacy `"off"`.
   Update `config.default.toml` and the README config section.
2. app/sidebar: a `shown` flag plus a narrow override, a `visible()` that
   applies the D3 order, `Toggle` and `Cycle` actions, `<leader>e` and
   `<leader>E` in the keymap, `:Sidebar` arguments. Directory start
   focuses the files pane.
3. Width: a pure `sidebar_width()`, row measurement, and the recompute
   hooks (page, mode, show, resize, refresh). The widen-only rule within
   one page.
4. ui: `src/ui/clip.rs` and its use in `draw_files` and `draw_outline`,
   with the indent cap.
5. Help rows, the spec note in `2026-10-07-ramble.md` § Sidebar, the
   README key list, and `:Sidebar` in the COMMANDS table.

## Open questions

None blocking. The numbers (16, 48, 35%, 80, p90, 8-column floor) are
starting values. Tune them after using it.
