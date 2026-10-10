# Sidebar: visibility, modes, sizing, clipping

Status: approved design (user answers Q1–Q6, plus the directory-start rule; D8–D11 added later on request).

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
There is no `:Sidebar off`; use `:Sidebar hide` (no aliases, D11).

### D2. Config

```toml
[sidebar]
show = "always"      # always | never | auto (D11)
default = "auto"     # auto | files | outline | split
reading = "outline"  # what auto shows while a page is loaded
width = "auto"       # "auto", or a number of columns
min_width = 16
max_width = 48       # also capped at 35% of the terminal width
auto_hide_below = 80 # terminal columns; 0 disables
split_ratio = 0.5
show_all = false
```

`SidebarMode` has no `Off`; whether the sidebar is drawn is `show` (D11).
No old spellings are accepted: `default = "off"`, `show = true|false` and
`:Sidebar off` are config or command errors (superseded by D11, which drops
the compatibility shims).

### D3. Visibility state

Visibility is one global flag, not part of each history entry. History
still remembers the mode per page (`nav.rs` `Entry.sidebar`), but restoring
an entry never shows or hides the sidebar.

Whether the sidebar is drawn is decided in this order:

1. **No page loaded** (a directory argument, or no argument, which starts
   in the current directory): the file tree is always drawn and has focus.
   This overrides `show = "never"`, auto-hide and a narrow terminal. With no
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
(`show = "never"` or a narrow terminal). Focus moves to the content then.

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
  tree refresh (`R`).
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

### D9. Side: left or right (user request)

- Config `[sidebar] side = "left" | "right"`, default `"left"`. Also
  `:Sidebar left` and `:Sidebar right` at runtime. There is no key for it.
- On the right, the sidebar is drawn at the right edge, and its 1-column
  border moves to the sidebar's left edge. Inside each pane nothing is
  mirrored: the `▎` marker, the indent, the folder arrows and the text
  stay left-aligned.
- `C-w h` and `C-w l` follow the screen position, as vim windows do. On
  the right, `C-w l` goes to the sidebar and `C-w h` back to the content.
  `C-w w`, `W`, `j`, `k` and `p` are unchanged.
- The help rows and the clue box describe these keys by direction. The
  rows are context-dependent: the "to the sidebar" key is `h` or `l`
  depending on the side.
- Mouse hit-testing uses the recorded rects, so it follows automatically.
  Its tests cover the right side too.
- Width, auto-hide and clipping are unchanged.

### D10. Moving the tree root up and down (user request)

- `-` in the files pane goes up one level: the tree is re-rooted at the
  parent of the current root. `h` on a top-level row also goes up; that
  is a row with no parent inside the tree, where `h` does nothing today.
- `.` on a folder row makes that folder the root.
- After going up, the old root is expanded and selected, so you can see
  where you came from. The `expanded` set (absolute paths) is kept, so
  folders below keep their state. The filter is cleared when the root
  changes.
- The pane title shows the root, clipped from the left with `clip_end`'s
  mirror (`…/notes/sub`), with `~` for `$HOME`: `Files ~/notes/sub`.
  There is no `..` row.
- Going up stops at `/` with the status "At the filesystem root".
- If the open file is outside the root (`Tree::outside`), the tree shows
  and selects it once the new root contains it.
- Root changes are only a browsing view. They are not added to `C-o`
  history and are not remembered between sessions. They don't change
  `App::tree_root` uses outside the tree: tuicr review discovery, the
  LSP notebook root and `yF` all keep the original root. Keep the
  browsing root in `Tree`, not in `App::tree_root`.
- Width is recomputed after a root change (the tree refresh hook).
- Mouse needs nothing new: it hit-tests recorded rows.
- `.` and `-` get help `BINDINGS` rows in the sidebar context. The clue
  box needs nothing.

### D11. `show = always | never | auto` (peek); no compatibility shims (user request)

`sidebar.show` takes exactly one of three values. There are no aliases:
`true`, `false` and the old `default = "off"` are config errors, with the
usual line number. `SidebarShow` has its own `Deserialize`, so any other
`show` value fails with `sidebar.show: expected "always", "never" or
"auto", got true`, which names the key and the values. `SidebarMode` has
no `Off`, so `default = "off"` gets serde's unknown-variant error, which
lists the four modes.

| `show` | Meaning |
|---|---|
| `always` (default) | Shown, as D3 steps 1–4 describe with the user flag starting at shown. |
| `never` | Hidden unless toggled (`<leader>e`), as D3 with the flag starting at hidden. |
| `auto` | Peek: shown while you're using it, hidden while you read. |

State in `auto`: a **pin** (the user flag) and a **peek** (an unpinned
showing). Visible = no page (D3 step 1) || pinned || peeked. Narrow only
forces an overlay; the D3 step-2 narrow override is never set. A file or
stdin start begins hidden and unpinned.

Rules in `auto`:

- **Peek opens** (from hidden, focus then moves into the sidebar):
  - `C-w h`/`C-w l` (whichever points at the sidebar per D9), `C-w w`,
    `C-w W`, `C-w p` when its previous pane is a sidebar pane, and `C-w j`/
    `C-w k` in split mode;
  - `<leader>E` and `:Sidebar files|outline|split`: they peek (not pin)
    and focus the mode's first pane.
- **Peek closes** (focus goes to the content), on user focus changes only:
  - a markdown file opened from the tree (`Enter`, `o`, double-click). The
    peek closes before the page is laid out; if the file can't be shown
    (binary, unreadable) the peek comes back with the tree focused;
  - a heading jumped to from the outline (`Enter`, `o`, double-click);
  - `C-w` back to the content (`C-w h/l`, `w`, `W`, `p` landing on it);
  - a click in the text, even with focus already on the content;
  - `Esc` in a sidebar pane with no filter on that pane. The first `Esc`
    clears the filter, as now; the second closes. On a pin, or with
    `always`/`never`, `Esc` does nothing new.
- **The peek stays:** `Enter` on a folder (it toggles) or on a non-markdown
  file (the editor), mode changes that move focus off a pane that went
  away, following a link (`gd`), `C-o`/`Tab` history, live reload, pickers
  and help. The close is never hooked into `set_focus`, `set_page` or
  reload.
- **First page after a directory start:** the sidebar hides in
  `sidebar_page_loading` (D3), before the first render, so the page is
  laid out at its reading width. Focus moves to the content.
- **`<leader>e`:** on a hidden sidebar it shows and pins it (it behaves as
  `always`); on a pin it unpins and hides; on an unpinned peek it hides and
  stays unpinned. `:Sidebar show` pins, `:Sidebar hide` unpins and hides
  (a peek included), `:Sidebar toggle` is `<leader>e`.
- **Overlay or beside:**
  - A peek goes beside the page iff `w + 1 <= cols - review_gutter -
    render.max_width` and the terminal isn't narrow, with `w` the width
    refitted for the panes shown (refit first, then decide). Otherwise it
    is drawn over the page's sidebar-side edge, with its border, and the
    page is not laid out again, so the text doesn't move.
  - The choice is made when the peek opens and again on a terminal resize,
    not on widen-only refits. On the left, beside shifts the text right by
    `w + 1` (accepted).
  - A pin lays out beside the page, except below `auto_hide_below`, where
    it is an overlay too.
  - `sidebar_cols()` stays the layout columns and is 0 for an overlay.
    `sidebar_drawn_cols()` is what is drawn, and focus, `sidebar_fit` and
    the help `avail` checks use it. An overlay is `min(w, cols - 1)` plus
    its border; `MIN_CONTENT` and the `min_width` drop don't apply.
  - Relayout: opening or closing re-lays out the page (`resize`) only when
    `sidebar_cols()` changed, so an overlay never does.
  - The overlay draws after the content, gutter, hover popup and hints,
    clearing its rect first. The cursor and hint labels under it are
    hidden (accepted).
- **Mouse:** hit-testing uses the recorded rects, and the overlay rect is
  recorded as the sidebar. The sidebar is tested before the hover popup,
  the gutter and the text, so the overlay sits on top. Clicks in a peeked
  sidebar work as now. There is no hot edge: nothing opens a hidden
  sidebar by mouse.
- **Help and clue:** the `C-w` rows are also available in `auto` while a
  peek could open. `<leader>e` has two rows: "show or hide the sidebar
  (pin in auto)" when `show = auto`, "show or hide the sidebar" otherwise.

Code and docs that carried the old spellings are cleaned up with this
change: the raw `Off` enum and its mapping in `config.rs`, the
`:Sidebar off` arm in `cmdline.rs`, tests that use `show = false|true` or
`default = "off"` (they switch to `SidebarShow::Never|Always`, or
`show = "never"` in TOML), `config.default.toml`, the README and
`2026-10-07-ramble.md` § Sidebar.

## Out of scope

- Resizing the sidebar with the mouse or keys (`C-w <`/`C-w >`). That can
  come later. The `width` config covers a fixed preference.
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
  - No page plus `show = "never"` plus a 60-column terminal still draws the
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
  `max_width`, `auto_hide_below`. `show = always|never|auto` (D11); the old
  `show = true|false` and `default = "off"` are rejected.

## Implementation checklist

1. config: `show`, `width: Auto | Fixed(u16)`, `min_width`, `max_width`,
   `auto_hide_below`. No `SidebarMode::Off` and no legacy spellings (D11).
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
