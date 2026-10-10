# ramble guide

What ramble does and how to drive it. Inside ramble, `g?` lists every key
that works right now (tested against the real keymap, so it can't drift).
Every config option, with its default, is in
[config.default.toml](../config.default.toml).

## Starting

```sh
ramble notes/index.md        # read a file
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
  `#anchors` jump, URLs open in the browser, other files open in your editor.
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

## Help

- `g?` lists every key that works right now.
- Pause after the first key of a sequence (`g`, `<leader>`, `C-w`, `y`) and a
  small box lists what can come next.
