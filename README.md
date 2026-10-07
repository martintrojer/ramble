# ramble

**Read your notes in the terminal, and follow them wherever they link.**

ramble is a fast, read-only markdown reader for the terminal. It renders a
page properly (reflowed prose, tables, syntax-highlighted code, callouts), lets
you follow any link with `gd`, and comes back with `C-o`, like a browser with
vim keys. Point it at a [zk](https://github.com/zk-org/zk) notebook and it
talks to zk's language server, so wikilinks, backlinks, hover previews,
full-text search and tags all work, without opening an editor.

![ramble reading its own README, with the file tree and outline in the sidebar](docs/images/ramble.png)

## Why

Reading markdown in a terminal usually means one of two compromises:

- **A pager** (`glow`, `mdcat`, `bat`). Pretty output, but a link is just
  text. You can't follow it, you can't go back, and you can't ask "what links
  here?".
- **Your editor.** Everything works, but you're in an editor: modes,
  buffers, a cursor that can change the file, and a setup step every time you
  just want to read.

ramble is the reading half of the editor, done properly. It never writes your
files. When you do want to edit, `<leader>o` opens `$EDITOR` at the line you're
looking at (`$VISUAL`, then `$EDITOR`), and ramble reloads the page when you come back.

Other readers resolve links with their own guesses. ramble asks the language
server your editor already uses. If zk can resolve a wikilink in nvim, ramble
can follow it.

## What you get

**Reading**
- CommonMark and GFM: headings, lists, task lists, tables, footnotes, GitHub
  alerts, block quotes. Prose is reflowed to the terminal width (capped at a
  readable maximum).
- Syntax-highlighted code blocks (catppuccin-mocha), including the language
  names people actually write (`py3`, `shell`, `jsx`, `rust,ignore`, `{.python}`).
- YAML front matter is treated as metadata, not drawn as a heading.
- `gR` toggles the raw markdown source, highlighted, with every motion still
  working.

**Moving around**
- A real cursor and vim motions: `hjkl w b e 0 $ { } gg G H M L`, counts,
  `C-d C-u C-f C-b`, `zz zt zb`.
- `/` `?` `n` `N` `*` `#` search within the page; `m{a-z}` / `'{a-z}` marks.
- `]l` / `[l` jump between links, `;` / `,` repeat, `]]` / `[[` jump between
  headings, and `s` puts a hint label on every visible link so you can jump to
  one in two keystrokes.

**Following links**
- `gd`, `Enter` or `C-]` follows the link under the cursor: markdown files
  open in ramble, `#anchors` jump to the heading, URLs open in your browser,
  anything else opens in your editor.
- Inline code that names a real file is a link too: `` `src/main.rs:12` ``
  opens that file at line 12.
- `C-o` / `Tab` go back and forward. Each page remembers its cursor, scroll
  position and sidebar layout.

**The sidebar**
- A file tree (respects `.gitignore`, walks lazily, so pointing it at `$HOME`
  is fine), the current page's outline, or both stacked.
- On `auto` it shows the tree until you open something, then the outline.
  `<leader>e` cycles modes, `C-w h/l/j/k` moves between panes.

**Pickers**
- Notes (`<leader>zf`) and the links on this page (`<leader>zl`) work
  anywhere, with or without a language server.

**With a language server** (zk first-class, marksman supported)
- Wikilinks resolved by your notebook, not by filename guessing.
- `K` previews the link target without leaving the page.
- Broken links are dimmed as you read.
- Backlinks to this page (`<leader>zb` or `grr`).
- With zk: full-text search (`<leader>zs`) and tags (`<leader>zz`), straight
  from your notebook's index.
- No server installed? Everything that doesn't need one keeps working, and
  ramble tells you once at startup instead of showing an error on every page.

**Around the edges**
- Live reload when the file changes on disk.
- Launchers: configurable keys that hand off to another program and come
  back. `<leader>o` opens your editor at the cursor line. `<leader>rr` opens
  the page in [tuicr](https://github.com/agavra/tuicr) for review.
- Review markers: comments from a live tuicr session show up as `●` in the
  file tree and in a gutter beside the lines they're on. `]r` / `[r` jump
  between them.
- The window or tmux pane title shows the file you're reading.
- `g?` lists every key that works right now. The list is tested against the
  real keymap, so it can't drift.

## Install

```sh
cargo install --path .
```

This also installs a small `fake-lsp` helper used by the test suite; you can
delete it. Optional, for notebook features: [zk](https://github.com/zk-org/zk) or
[marksman](https://github.com/artempyanykh/marksman) on your `PATH`.

## Use

```sh
ramble notes/index.md        # read a file
ramble notes/                # browse a folder
cat README.md | ramble       # read stdin (keys come from the terminal)
ramble --print doc.md        # render once to stdout, like a pager would
ramble doc.md | less -R      # same, automatically, when stdout is a pipe
```

Inside, press `g?` for the full list of keys.

## Configure

ramble works with no config file. To customise it:

```sh
ramble --init-config         # writes ~/.config/ramble/config.toml
```

The file lists every option with its default, commented out. The ones you're
most likely to change:

```toml
[render]
max_width = 100              # cap the reading width

[sidebar]
default = "auto"             # auto | off | files | outline | split
reading = "outline"          # what auto shows while you read: outline | split

[keys]
leader = " "

[[lsp.server]]               # replaces the default server list
kind = "zk"
command = ["zk", "lsp"]
root_markers = [".zk"]

[[launch]]                   # add your own hand-offs
name = "lazygit"
key = "<leader>g"
command = ["lazygit"]
needs_vcs = true
```

## How it works

ramble parses markdown with pulldown-cmark and lays it out itself, keeping a
map from every drawn cell back to the source bytes it came from. That map is
what makes the cursor, search, link hints, review markers and "open the editor
at this line" all line up with the file on disk.

Notebook knowledge comes from the language server, over a small stdio
JSON-RPC client that never blocks the UI: the page renders immediately, and
link targets and broken-link marks arrive when the server answers.

The full design, including the decisions behind it and what the review panel
changed, is in [docs/specs/2026-10-07-ramble.md](docs/specs/2026-10-07-ramble.md).

## Status

Young and moving fast. It's built to replace a glow-plus-nvim reading setup,
and its end-to-end tests run against real zk and tuicr when they're
installed. Expect sharp edges in the corners
the tests don't reach yet.

## License

MIT
