# ramble design

How ramble works. For what it does from the user's side, see the
[guide](guide.md); for every option, [config.default.toml](../config.default.toml).

## Principles

- **Read-only.** ramble never writes user files. Editing happens in
  `$EDITOR` through a launcher; ramble reloads on return.
- **The source map is the core.** The renderer maps every drawn cell back to
  the source bytes it came from. The cursor, search, link hints, mouse,
  yank, review comments and "open the editor at this line" all depend on it.
- **Notebook knowledge comes from mdroots.** Link resolution, broken links,
  backlinks, previews, search and tags come from the embedded
  [mdroots](https://github.com/martintrojer/mdroots) index, or from a
  configured language server. Knowledge ramble needs and mdroots lacks is
  added to mdroots, not to ramble.
- **Best effort without a backend.** With no backend (stdin, a broken
  server) ramble still renders, follows relative links, `#anchors` and
  code-path links, searches the page and walks the tree.
- **Threads and channels, no tokio.** mdroots is synchronous: one worker
  thread. Each language server gets a reader thread.

## Architecture

```
ramble
├── cli      args → tree root, start target, TTY detection, --print, --init-config
├── config   TOML over built-in defaults
├── doc      pulldown-cmark layout parse + mdroots::syntax → Document
├── render   Document + width + theme → RenderedPage { lines, srcmap }
├── app/mdroots_glue   worker thread owning mdroots::Workspaces
├── backend/lsp        optional stdio JSON-RPC client, zk adapter
├── notebook notebook operations → mdroots | zk adapter | LSP | local
├── nav      history of pages; back/forward
├── ui       ratatui widgets only, no logic
└── app      event loop: key | mouse | fs-watch | mdroots | lsp | resize | tick → state → draw
```

Rust stack: ratatui, crossterm, pulldown-cmark, mdroots, syntect, lsp-types,
notify, nucleo, ignore, debrief-review.

### cli

- **Tree root:** a file argument's folder, a folder argument itself; with no
  argument, the nearest VCS root above the working directory (`.git`, `.jj`,
  `.hg`, `.sl`), else `$HOME`.
- **Start target:** a file argument (stdin ignored); a folder argument (tree,
  no page); no argument with piped stdin (read the document from stdin); no
  argument on a terminal (tree from the working directory).
- **Print mode:** `--print`, or stdout is not a TTY. Render once as ANSI and
  exit; never page. With no document, exit 2 with a usage error.
- **Width in print mode:** `--width`, else `$COLUMNS`, else the terminal,
  else 80; capped at `render.max_width`. Interactive: terminal width minus
  the sidebar, capped at `render.max_width`.

### config

Serde structs with defaults; a user file overrides one field at a time. No
file is needed. `--init-config` writes the embedded `config.default.toml`
(fully commented) to `~/.config/ramble/config.toml` and refuses to
overwrite. A parse error prints its line and exits before the TUI starts.
Enumerations accept only their documented values; there are no aliases.

### doc

- No rendering dependency. Input is UTF-8; invalid bytes become U+FFFD with a
  status note. A NUL in the first 8 KiB means binary: a message, no render.
  Interactive stdin is the exception: it is decoded lossily with no note and
  no binary check (`--print` from stdin still gets the binary check).
- Two parses, both with `mdroots::syntax::markdown_options()` so ranges line
  up (a test checks every mdroots range falls on a layout boundary):
  - ramble's pulldown-cmark layout parse: blocks, inlines, code spans, math;
  - `mdroots::syntax::parse_with` (with `unfenced_frontmatter` off): links
    (wikilinks included, each with a full span and a visible-text span),
    headings with GitHub slugs, front matter.
- **Code-path links.** On each load the app turns inline code spans that name
  an existing regular file into links: relative, absolute or `~/` paths,
  optionally `:LINE[:COL]`; no whitespace, at most 512 bytes, not URLs.
  Relative paths are tried against the page's folder, its VCS root, then the
  tree root. A markdown target opens in ramble at LINE; anything else in the
  editor with `+LINE`. Backends never touch them.

### render

- Lays out a `Document` at one width into styled lines plus a `srcmap`: a
  two-way map between source byte ranges and `(row, cols)` spans. Undrawn
  bytes (link syntax, emphasis markers, fences) map to the nearest drawn
  position. Re-runs on resize. Print mode writes the same lines as ANSI.
- Prose reflows; code is highlighted with syntect (catppuccin-mocha), with
  common language aliases (`py3`, `shell`, `rust,ignore`, `{.python}`).
- **Math** (teal). `$…$`, or `$$…$$` inside a paragraph: one line via
  `latex-to-unicode`. `$$…$$` alone in a paragraph, or a `math` fence: a 2-D
  grid via `term-maths`, centred, falling back to one line when too wide.
  Output that looks wrong (blank, a leftover `\command`, unbalanced braces,
  `\begin{…}`) shows the trimmed source instead. `[render] math = false`
  shows the source.
- **Front matter** (YAML `---` or TOML `+++`) is metadata, never markdown.
  The first row is a dim `▸ front matter · N keys`; `za` or `Enter` on it
  expands to one row per top-level key in source order: keys padded (max 20
  columns), scalars as written, lists joined with `, `, maps as `{…}`, values
  clipped with `…`. Fold state is per page, kept across reloads. Parsing
  (in mdroots) is best effort: a real parser, then a line scan; if nothing
  is found the marker says `unparsed` and expands to the raw lines. Raw view
  shows the source; print mode omits it.
- **Raw view** (`gR`, `:Raw`): the source, one line per row, soft-wrapped by
  grapheme, highlighted as Markdown. Per page; kept across history, resize
  and reload; a followed link opens rendered. Everything works unchanged;
  links are their full source span. Status shows `RAW`.

### Backends

A page is served by the first `[[lsp.server]]` whose `root_markers` are found
above it, else by mdroots. Stdin pages have no backend. The status line
shows `mdroots ●` (root index), `mdroots ○` (single-file index), `zk ●`
(running), `zk ○` (starting) or `—`. Replies for another page, or an older
version of this one, are dropped. A request slower than 2 s shows a spinner.

**mdroots** (`app/mdroots_glue.rs`). One worker thread owns
`mdroots::Workspaces` and runs every call; the event loop drains replies.
Page requests are coalesced to the latest. A page whose root isn't open is
answered first from `Workspace::open_single`, then from the root's workspace.
For links the worker sets the page text as an overlay and reads
`document_links`, matched to ramble's links by exact range; broken-link
dimming comes only from a root workspace. mdroots' own watcher reports
changes to other notes; ramble's watcher reloads the page and asks again
with `refresh_paths`. The cache lives in the user cache dir
(`MDROOTS_CACHE_DIR` moves it); mdroots decides which roots get one. Only
marker and VCS roots count as complete; elsewhere notes fall back to a file
walk and other picker titles end in `(partial)`. ramble depends only on the
`mdroots` facade crate.

**Language servers** (`backend/lsp`, optional). A small stdio JSON-RPC
client, one process per (server, root), started lazily with the root as its
working directory. `kind` is `zk`, `marksman` or `generic` (standard
requests only).
- Messages: `initialize`, `initialized`, `didOpen` (`languageId =
  "markdown"`), `didClose` (on leaving a page; reload closes, then reopens),
  `documentLink`, `definition`, `hover`, `references`, `executeCommand`,
  `publishDiagnostics`, `shutdown`.
- zk gets `zk.index` with its root after `initialized`, because it indexes
  only at startup and on `didSave`, which ramble never sends.
- Positions: offers `utf-8`/`utf-16`; with no answer, `utf-32` for zk and
  `utf-16` otherwise; `position_encoding` overrides. One function converts.
- File URIs are canonicalized before comparison (`/private/tmp` vs `/tmp`).
- LSP link ranges are matched to local links by overlap, never drawn.
  Diagnostics covering a link dim it.
- A missing server is reported once; a crashed one shows `—` and its pages
  lose backend features. mdroots does not take over.

**Following a link** (`gd`, `Enter`, `C-]`): the target from mdroots or
`documentLink`, else (server) `definition`, else the local parse. URLs and
same-page anchors are always local. In the local parse a wikilink without an
extension gets `.md`, and so does a markdown link when only the `.md` file
exists. Markdown opens in ramble, URLs with `open`/`xdg-open`, other files in
the editor. An unresolvable link leaves history unchanged.

### Notebook operations

| Operation | mdroots | zk | marksman | no backend |
|---|---|---|---|---|
| notes | `notes()`, newest first | `zk.list` | file walk | file walk |
| search | `full_text`, one row per hit | `zk.list` + `match` | — | — |
| tags | `tags()` (case folded), then `notes_with_tag` | `zk.tag.list`, then `zk.list` + `tags` | — | — |
| backlinks | `backlinks(path)` | `references` at 0:0 | `references` at the first heading | — |
| links | local parse + mdroots targets | local parse + `documentLink` | local parse + `definition` | local parse |

An operation with no source is hidden from help and the command line. zk
commands pass the server root as the notebook path. Choosing a backlink
opens the note on its first link back to the origin page. mdroots search
matches every word case-insensitively, the last as a prefix; zk's own syntax
applies under zk. `K` shows mdroots' `preview(target, 12)` (title, front
matter, excerpt) or the server's hover.

### nav and history

Each entry stores the path, cursor, scroll, sidebar mode and raw flag.
Following a link from mid-history drops the forward entries. `C-o` / `C-t`
go back, `C-i` / `Tab` forward. Stdin pages stay in memory so `C-o` returns
to them.

### Stdin

Titled `[stdin]`; relative links resolve against the working directory; no
backend, no `<leader>o`, no live reload, no comments, no `yf`. Keys are read
from `/dev/tty`.

## Keys

`<leader>` defaults to space. Only `<leader>` and launcher keys are
configurable. The `g?` overlay lists the bindings available right now; its
rows come from `help::BINDINGS`, which a test checks against the keymap, so
the overlay is the reference and this section only covers behaviour.

- **Motions and scrolling:** vim's, with counts. `f`/`t` are not provided;
  `;`/`,` repeat link motions (`]l`/`[l`). `s` + label jumps to a visible
  link.
- **Visual mode and yank.** `y` is an operator (`yL` yanks to the screen
  bottom, so the link target is `yu`). Charwise yanks copy the source bytes
  from the first selected drawn grapheme to the last, dropping hidden markup
  at the edges. Linewise yanks copy the whole source lines the rows came
  from. Blockwise is the one yank of rendered text. Copies go out over
  OSC 52.
- **Command line:** `:e [path]`, `:q[!]`, `:Notes`, `:Search <q>`, `:Tags`,
  `:Backlinks`, `:Links`, `:Launch <name>`,
  `:Sidebar files|outline|split|toggle|show|hide|left|right`, `:Raw`,
  `:Refresh`. `R` / `:e` re-read the page (keeping the cursor) and every
  folder of the tree read so far, then repaint.
- **Picker:** one nucleo fuzzy overlay for every list. Search prompts for a
  query, sends it once, then filters the results.
- **Help overlay** (`g?`): grouped by section, scrollable and filterable;
  hides rows that can't do anything now (no `n` before a search, no `]r`
  without comments). Bare `?` stays backward search.
- **Key clue.** After the first key of a sequence (`g`, `z`, `[`, `]`, `m`,
  `'`, `C-w`, `<leader>`, launcher prefixes, `y`), if nothing follows within
  400 ms, a box at the bottom right lists the next keys, from the same rows
  as `g?`. It never takes keys; it updates at once as the sequence grows and
  closes when it ends. Several continuations under one key show as `+label`.
  Overflow wraps into columns, then `… N more`. A test fails if a pending
  prefix has no rows. `[keys] clue = false` turns it off.
- **Pane navigation:** `C-h/j/k/l` are `C-w h/j/k/l`; at an edge inside tmux
  they run `tmux select-pane`. `[keys] pane_nav = false` unbinds them.

## Sidebar

Three modes: **files** (tree from the root, `.gitignore` respected, lazy;
non-markdown files dimmed or hidden by `show_all`), **outline** (headings of
the page), **split** (files above outline, `split_ratio`). `<leader>E`
cycles them; `<leader>e` shows or hides. Hiding is not a mode.

- **Mode:** `default = "auto"` shows files until a page is loaded, then
  `reading` (outline or split). Picking a mode by hand stops that switching.
  History restores each entry's mode but never visibility.
- **Visibility** (`show = always | never | auto`), decided in order:
  1. no page loaded: the tree is drawn and focused, whatever the settings;
  2. the user toggled it while the terminal was narrow: that holds until
     the next resize;
  3. terminal narrower than `auto_hide_below`: hidden;
  4. otherwise the user's flag, which starts from `show`.
  If the page would get fewer than 10 columns, it isn't drawn.
- **Peek** (`show = "auto"`). Hidden while reading. `C-w` into it,
  `<leader>E` or `:Sidebar <mode>` open an unpinned peek with focus inside.
  Opening a file or heading from it, `C-w` back to the text, clicking the
  text or `Esc` (after clearing any filter) close it; nothing else does.
  `<leader>e` pins and unpins. A peek goes beside the page only when the page
  keeps its full `render.max_width`; otherwise it is drawn over the page's
  edge without relaying out the text.
- **Width** (`width = "auto"`): the 90th-percentile row width (the max under
  10 rows) plus 1, clamped to `min_width..min(max_width, 35% of cols)`, and
  allowed to grow into columns the page can't use. Recomputed on page, mode,
  show, resize and refresh; while browsing it only widens. A number is used
  as given.
- **Clipping** by display width and grapheme: headings at the end with `…`,
  file names in the middle keeping the extension. The current heading and
  the open file are marked `▎` in a left gutter that clipping can't reach;
  review marks `● N` drop to `●` before they clip. Deep trees stop indenting
  so names keep 8 columns.
- **Side:** `side = left | right` (or `:Sidebar left|right`). `C-w h/l`
  follow screen position.
- **Root:** `-` (or `h` on a top-level row) moves the tree root up, `.` makes
  the selected folder the root. The title shows it (`Files ~/notes/sub`).
  This is only a view: history, `yF` and backends keep the original root.
- **Focus:** `C-w h/l/w/W/p`; in split mode `C-w j/k` pick the outline or
  files pane. The tree follows the open file.

## Mouse

On by default; `[mouse] enabled = false` leaves it to the terminal. Capture
is released around launchers. `ui::draw` records a layout of rects each
frame; hits test popups first, then panes.

- **Click:** text places the cursor (not a jump; leaves visual mode); a
  sidebar row focuses and selects it, and its `▸` toggles the folder; a
  picker item is selected; a click outside an open popup closes it.
- **Double click** (same cell, 400 ms): follow a link, open a file or
  heading, open a picker item, or select and copy a word. **Triple click:**
  copy the line.
- **Drag** in text: charwise selection, scrolling at the edges, copied on
  release; visual mode stays.
- **Wheel:** scrolls the pane under the pointer (3 rows; picker 1) without
  moving focus.
- Ignored: right and middle buttons, plain moves, border drags, horizontal
  wheel.

## Review comments

Comment mode follows debrief's spec (`docs/specs/2026-10-09-debrief.md` §2
in debrief). Comments go to the debrief-review batch for the page's repo
root in the user cache dir, never into the repo.

- `cc` comments the cursor line, visual `c` the selection's source lines
  (raw view too). A multi-line box opens below the lines, or above when it
  doesn't fit, never over them. Enter saves, `C-j`/`Alt-Enter` adds a line,
  Esc cancels, `C-e` moves the text into `$EDITOR`; PageUp/PageDown and the
  wheel scroll the page behind it. Tab/S-Tab cycle the kind through
  `[review] kinds`.
- The batch is re-read every second. Commented lines get a gutter `●`,
  files a `● N` in the tree; `]r`/`[r` jump. On a commented row the status
  line shows the comment's first line and `K` shows all of them.
- `<leader>rl` lists the batch (diff comments from debrief included): Enter
  jumps, `C-d` removes, `C-e` edits. `<leader>rr` exports it through
  debrief-review's `handback::send` (`[send] command` on stdin with
  `DEBRIEF_ROOT`, else the clipboard), then clears it; a failed hand-back
  keeps it. Quitting with unsent comments warns once.

## Launchers

A launcher binds a key to a command. Running one suspends the TUI, runs the
command in the restored terminal, waits, then resumes and reloads the page.
The default is the editor:

```toml
[[launch]]
name = "edit"
key = "<leader>o"
command = ["${editor}", "+${line}", "${file}"]
```

Variables, substituted per argument with no shell: `${file}`, `${line}`,
`${dir}` (the page's folder, or the working directory for stdin),
`${vcs_root}`, `${editor}` (`$VISUAL`, `$EDITOR`, `vi`). The working
directory is `${vcs_root}`, else `${dir}`. A launcher whose variable has no
value, or with `needs_vcs` outside a repo, is unavailable with a
status message. User entries replace defaults by name; `disabled = true`
removes one. `:Launch <name>` runs one by name.

## Terminal

- **Title:** `ramble — <page>` (`[stdin]`, or `ramble` with no page), set
  only on change, saved and restored with XTWINOPS, resent after a launcher.
  Under tmux it becomes `#{pane_title}`. Print mode never sets it.
- **Status line:** path, backend state, position, history depth, and a dim
  `g? help` hint that is dropped first when space is short.
- **Errors:** a deleted file shows a banner and keeps its content; an
  unresolvable link shows a message.

## Testing

- **doc and render:** insta golden snapshots at several widths (non-ASCII,
  emoji, CJK) and property tests that every link's visible text is drawn,
  every drawn cell maps back into its block, and source → screen → source
  round-trips.
- **app:** state-machine tests drive `App` with events and an injected
  clock, no terminal: history, sidebar, pickers, stale replies, reloads,
  launchers with recording scripts, mouse after a `TestBackend` draw.
- **Backends:** `tests/mdroots.rs` uses temp caches and asserts nothing is
  written in the browsed tree. `tests/lsp.rs` drives a scripted fake server
  (`tests/support/fake_lsp.rs`). zk and marksman end-to-end tests run the
  real servers when installed and print a skip otherwise.
- **Edges:** print mode against the insta snapshot
  `tests/snapshots/cli__print_golden_at_width_80.snap`, a real `notify`
  watcher, CLI precedence, config overrides and errors, and tmux end-to-end
  tests.

## Out of scope

Editing, note creation and renaming; splits and tabs; images and diagrams;
remote URLs as documents; more than one server per file; workspace symbols;
full key remapping.
