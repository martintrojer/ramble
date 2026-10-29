# ramble guide

What ramble does and how to drive it. Inside ramble, `g?` lists every key
that works right now (tested against the real keymap, so it can't drift).
Every config option, with its default, is in
[config.default.toml](../config.default.toml).

## Starting

```sh
ramble notes/index.md        # read a file
ramble --line 42 notes/a.md  # open with the cursor on source line 42
ramble --follow notes/a.md   # preview beside your editor, following its cursor
ramble notes/                # browse a folder
cat README.md | ramble       # read stdin (keys come from the terminal)
ramble --print doc.md        # render once to stdout
ramble doc.md | less -R      # print mode is automatic when stdout is a pipe
ramble --init-config         # write ~/.config/ramble/config.toml
```

## Reading

- CommonMark and GFM: tables, task lists, footnotes, GitHub alerts. Prose
  reflows to the terminal width, capped at `[render] max_width`.
- Code blocks are highlighted (catppuccin-mocha), including the names people
  actually write: `py3`, `shell`, `rust,ignore`, `{.python}`.
- LaTeX as Unicode: `$E = mc^2$` reads as E = mc², `$$\frac{a+b}{c}$$` is a
  stacked fraction. `[render] math = false` shows the source.
- A scrollbar shows on a long list's right edge for a second after it
  scrolls; `[render] scrollbars = false` turns it off.
- Front matter folds to `▸ front matter · 5 keys`; `za` or `Enter` expands it.
- `gR` toggles the highlighted raw source; every motion still works.
- Live reload keeps your place. `R` (or `:e`) re-reads the page and the tree.

## Moving

- Vim motions with counts: `hjkl w b e 0 $ { } gg G H M L`, `C-d C-u C-f C-b`,
  `zz zt zb`.
- `/ ? n N * #` search the page; `m{a-z}` / `'{a-z}` set and jump to marks.
- `]l` / `[l` next and previous link, `;` / `,` repeat, `]]` / `[[` headings.
- `s` labels every visible link; type a label to jump.

## Links

- `gd`, `Enter` or `C-]` follows the link: markdown opens in ramble,
  `#anchors` jump, URLs open in the browser, PDFs, images, audio and video
  open with `open` / `xdg-open`, and other files open in your editor.
- Inline code naming a real file is a link: `` `src/main.rs:12` `` opens it at
  line 12.
- `C-o` / `Tab` go back and forward; each page keeps its cursor, scroll and
  sidebar.
- `K` previews the target. Broken links are dimmed.
- Pickers: notes `<leader>zf`, links on this page `<leader>zl`, backlinks
  `<leader>zb` / `grr`, full-text search `<leader>zs`, tags `<leader>zz`.

Links, backlinks, search and tags come from the built-in
[mdroots](https://github.com/martintrojer/mdroots) index, which understands
zk, Obsidian, marksman and relative-path links. Its cache lives in your user
cache dir (`MDROOTS_CACHE_DIR` moves it), never in your notes. To use a
language server instead (zk, marksman, or any as `generic`), add an
`[[lsp.server]]` entry; it serves the pages under its root markers.

## Copying

- `v`, `V`, `C-v`, `o`, `gv`, as in vim.
- `y` is vim's operator: `yy`, `Y`, `3yy`, `yw`, `y}`, `yG`, `y'a`. It copies
  the markdown source, so a pasted link is still a link.
- `yf` / `yF` copy the absolute / relative path; `yu` the link under the
  cursor.
- Copies use OSC 52, so they work over SSH and in tmux.

## Mouse

Click places the cursor or picks a row; double-click follows a link, opens a
file or copies a word; triple-click copies the line; drag selects and copies;
the wheel scrolls whatever is under the pointer. Shift+drag still gives the
terminal's own selection. `[mouse] enabled = false` hands the mouse back.

## Sidebar

- A file tree (respects `.gitignore`, lazy, so `$HOME` is fine), the page
  outline, or both.
- `<leader>e` shows or hides it, `<leader>E` cycles outline / files / split.
- `C-h/j/k/l` (or `C-w h/j/k/l`) move between panes; in a pane, `j/k`,
  `C-d/C-u` and `gg/G` move.
- In the tree, `-` moves the root up and `.` makes the selected folder the
  root.
- `[sidebar] show`: `always`, `never`, or `auto`, a peek that hides while you
  read and shows while you use it. `side = "right"` moves it.
- It sizes itself to its rows and hides below 80 columns.

## Panes and tmux

`C-h/j/k/l` move between ramble's panes and, at the edge inside tmux, on to
the next tmux pane, like
[vim-tmux-navigator](https://github.com/christoomey/vim-tmux-navigator).
`[keys] pane_nav = false` unbinds them. vim-tmux-navigator only forwards
these keys to panes it thinks run vim; add ramble to its pattern:

```tmux
set -g @vim_navigator_pattern '(\S+/)?g?\.?(view|l?n?vim?x?|fzf|debrief|ramble)(diff)?(-wrapped)?'
```

## Following your editor

Run ramble as a live preview next to your editor:

```sh
tmux split-window -h ramble --follow notes/a.md
```

It re-renders on save, and with `--follow` it also moves to where your
editor's cursor is:

- The cursor moves to that line. The view scrolls only when the line is
  off screen, and then centres it.
- When the editor moves to another markdown file, ramble opens it (`C-o`
  goes back). Other files are ignored.
- Using ramble yourself (a key, a click or scroll, a paste) pauses
  following for 2 seconds, and following waits while a prompt, picker,
  comment box, visual mode or half-typed key sequence is open.
- Inside tmux, ramble follows only editors in its own tmux window, so two
  editor + preview pairs don't drag each other around.

The editor tells ramble with `ramble --goto FILE:LINE`. It finds the
following rambles (sockets in `$XDG_RUNTIME_DIR/ramble-<uid>/`, else
`$TMPDIR`, else `/tmp`), picks those in its own tmux window, and exits 0
even when none is listening. It takes a few milliseconds, so an editor can
run it on each cursor move. Linux and macOS only.

For nvim, put this in your config (e.g. `~/.config/nvim/plugin/ramble.lua`):

```lua
if vim.fn.executable("ramble") == 0 then
	return
end

local last, timer

local function send()
	local path = vim.api.nvim_buf_get_name(0)
	if path == "" then
		return
	end
	local target = path .. ":" .. vim.api.nvim_win_get_cursor(0)[1]
	if target == last then
		return
	end
	last = target
	vim.system({ "ramble", "--goto", target })
end

vim.api.nvim_create_autocmd({ "CursorMoved", "CursorMovedI", "BufEnter" }, {
	group = vim.api.nvim_create_augroup("ramble_follow", {}),
	pattern = { "*.md", "*.markdown", "*.mdown", "*.mkd" },
	callback = function()
		timer = timer or vim.uv.new_timer()
		timer:stop()
		timer:start(50, 0, vim.schedule_wrap(send))
	end,
})
```

## Comments

Review notes on the lines you're reading, handed back to whoever wrote them
(a person, or an agent via `[send] command`).

- `cc` comments on the cursor line, visual `c` on the selection; works in
  raw view too.
- The box opens next to the lines without covering them. Enter saves,
  `C-j` / `Alt-Enter` adds a line, `C-e` moves it into `$EDITOR`, Esc
  cancels.
- Tab / S-Tab set a kind (issue, suggestion, question, nit; see
  `[review] kinds`).
- `●` marks commented lines and files; `]r` / `[r` jump between them; `K`
  shows the comment.
- `<leader>rl` lists the batch (Enter jumps, `C-d` removes, `C-e` edits).
- `<leader>rr` sends it as markdown to `[send] command`, or copies it, then
  clears it. Quitting with unsent comments warns once.

Comments live in [debrief](https://github.com/martintrojer/debrief)'s review
batch in your cache dir, never in the repo, and comments from debrief or
another ramble show up within a second.

## Launchers

Keys that hand off to another program and reload on return. `<leader>o`
opens `$VISUAL` / `$EDITOR` at the cursor line. Add your own:

```toml
[[launch]]
name = "lazygit"
key = "<leader>g"
command = ["lazygit"]
needs_vcs = true
```

## Theme

`[theme] name` picks a built-in scheme (`catppuccin-mocha`, the default,
`catppuccin-macchiato`, `catppuccin-frappe`, `catppuccin-latte`,
`solarized-dark`, `solarized-light`, `gruvbox-dark`, `gruvbox-light`) or
a path to a base16 or base24 scheme file. `[theme.colors]` replaces single
colours of whatever `name` selected, `syntax` picks the code-highlighting
theme, and `fill = true` paints the theme background, for a light scheme
on a dark terminal:

```toml
[theme]
name = "gruvbox-dark"
fill = true

[theme.colors]
cursor_bg = "#585b70"
```

The keys are in [config.default.toml](../config.default.toml); the full
rules are in [the theming spec](specs/2026-10-21-theming.md).

## Help

- `g?` lists every key that works right now.
- Pause after the first key of a sequence (`g`, `<leader>`, `C-w`, `y`) and a
  small box lists what can come next.
