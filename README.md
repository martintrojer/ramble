# ramble

**Read your notes in the terminal, and follow them wherever they link.**

ramble is a fast, read-only markdown reader for the terminal, with vim keys
and a browser's links. It renders pages properly, follows wikilinks and
relative links with `gd`, comes back with `C-o`, and knows your whole
notebook: backlinks, full-text search and tags with no setup.

![ramble reading its own README, with the file tree and outline in the sidebar](docs/images/ramble.png)

## Why it's good

- **Links are links.** Pagers like `glow` and `bat` print a link as text.
  ramble follows it, keeps history, and answers "what links here?".
- **It understands your notebook.** Links resolve through the built-in
  [mdroots](https://github.com/martintrojer/mdroots) index, which reads zk,
  Obsidian, marksman and relative-path styles. Nothing to install, and
  nothing is written into your notes.
- **It renders well.** Reflowed prose, tables, highlighted code, GitHub
  alerts, LaTeX drawn as Unicode, folded front matter.
- **Your fingers already know it.** Vim motions, search, marks, visual mode
  and `y` (copying the markdown source, over OSC 52, so it works through
  SSH and tmux). Press `g?` for every key and pause mid-sequence for hints.
- **It never touches your files.** `<leader>o` opens `$EDITOR` at the line
  you're on, and ramble reloads when you're back.
- **It reviews.** `cc` leaves a comment on a line; `<leader>rr` sends the
  batch to a person or an agent.

## What it isn't

- **An editor.** It hands off to yours.
- **A pager.** `--print` renders once to stdout, and that's it.
- **An Obsidian replacement.** No images, diagrams, graph view or note
  creation.

## Install

Prebuilt binaries for macOS (Apple silicon) and Linux (x86_64) are on the
[releases page](https://github.com/martintrojer/ramble/releases), or:

```sh
cargo install --path . --bin ramble
```

## Use

```sh
ramble notes/            # browse a folder
ramble notes/index.md    # read a file
cat README.md | ramble   # read stdin
```

No config needed. `ramble --init-config` writes every option, commented out.

## More

- [Guide](docs/guide.md): features, keys, sidebar, comments, tmux, launchers
- [Default config](config.default.toml): every option, with its default
- [Design](docs/design.md): how it works and why

Young and moving fast. MIT licensed.
