# Follow the editor's cursor

Agreed with the user 2026-10-22. Calls marked *(my call)* were filled in
by the agent while writing the spec. They aren't user decisions; check them.

## Problem

ramble is used as a live markdown preview in a tmux split beside nvim. It
re-renders on save but stays where it's scrolled, so an edit far from the
preview's position stays out of sight until you scroll there. Editors
don't share their cursor position, so ramble has to be told it.

## Approach

The editor sends its position to ramble. A ramble started with `--follow`
listens on a Unix socket. Editor hooks (an nvim Lua snippet, or the
`ramble --goto` client for anything else) write `<line> <path>` to every
ramble socket. A ramble moves when the message names markdown, unless you
used ramble yourself in the last 2 s.

Rejected:
- **ramble polling nvim** (`nvim --remote-expr`): nvim-only, and finding
  the right `$NVIM` socket across tmux panes is fragile.
- **a state file ramble watches**: watcher debounce lag, and the file is
  rewritten on every cursor move.
- **mdroots / LSP**: LSP has no "cursor moved" notification; mdroots
  indexes notes and doesn't carry editor events.

## Decisions

1. **Opt-in**: `ramble --follow [PATH]`. Without it, no socket and no
   behaviour change. A ramble you're using to browse in another window is
   never moved.
2. **Discovery**: each following ramble binds
   `<dir>/ramble-<uid>/<scope>.<pid>.sock` (scope: decision 4), where
   `<dir>` is `$XDG_RUNTIME_DIR`, else `$TMPDIR`, else `/tmp`. The directory is created mode 0700, and
   it's refused if it exists but is not ours or is group/world accessible
   *(my call: the uid avoids one user owning a shared `/tmp/ramble`)*.
   A drop guard removes the socket on normal exit and on panic. There are
   no signal handlers and no new dependency, so a pane closed with SIGHUP
   leaves its socket behind *(my call)*. Senders check the directory the
   same way (else another user could pre-create it and collect paths),
   write only to sockets in their scope there, and delete a socket that
   gives `ECONNREFUSED`. A listener gives each client 500 ms and 64 KiB
   in total, so a client holding its connection open can't stall others.
   A bind
   failure under `--follow` is a status message ("follow off: …"), like
   "live reload off", not a startup error *(my call)*.
3. **Message**: one UTF-8 line per update, `<line> <absolute path>\n`.
   `line` is 1-based and the path is the rest of the line, so spaces and
   colons work. A connection may carry several lines, and the last one
   wins. Bad lines are ignored silently *(my call)*. Stream sockets, since
   nvim's `vim.uv` speaks them *(my call)*.
4. **Scope by tmux window** (agreed): `<scope>` is
   `t<server pid>w<window id digits>`, where the server pid is the second
   field of `$TMUX` and the window id comes from
   `tmux display-message -p -t $TMUX_PANE '#{window_id}'`, run once at
   startup. Outside tmux, or with `$TMUX_PANE` unset, the scope is `any`. A sender inside tmux works out its own
   scope the same way and writes to its scope's sockets plus `any.*`. A
   sender outside tmux writes to every socket. If the tmux lookup fails,
   both sides use `any` *(my call)*. The scope is fixed at startup, so a
   pane moved to another window (`move-pane`/`break-pane`) keeps its old
   scope until restarted *(my call: accepted)*. The server pid is in the
   scope so that two tmux servers can't collide *(my call)*.
5. **Another file**: if the path is markdown (`nav::is_markdown`) and not
   the current page, ramble opens it as a followed link (history push, so
   `C-o` returns) through `open_link_file`, then lands per decision 7.
   Non-markdown, missing, unreadable and binary files are ignored without
   a status message *(my call: silent, because an editor hook fires
   constantly)*. Paths are compared with the existing `same_file`. Updates that arrive while following is
   paused (decision 6) are dropped.
6. **Grace period**: any key, mouse event (except bare pointer motion) or
   paste in ramble starts a 2 s window, and updates arriving in it are
   dropped, not queued. Following is also paused while the mode is not
   `Normal` (search, filter, picker, command, hint, help, visual,
   op-pending, comment) or a key sequence or count is half typed. Sidebar focus
   doesn't pause it *(my call)*. The 2 s is a constant, with no config key
   until it's needed.
7. **Landing**: if the line's first rendered row is already visible, only
   the cursor moves. Otherwise the view centres on it. A line past the end
   goes to the last row (same as `goto_source_line`).
8. **Client**: `ramble --goto FILE:LINE` canonicalises FILE, sends to
   the sockets in its scope (decisions 2 and 4) and exits 0, even when no
   ramble is listening *(my call: so hooks never error)*. It splits on the **last**
   `:`. It's a separate mode in `cli::plan` and needs no TTY.
9. **nvim**: a Lua snippet in the guide. On `CursorMoved`, `CursorMovedI`
   and `BufEnter` in markdown buffers, debounced to about 50 ms, it runs
   `ramble --goto <abs path>:<line>` with `vim.system` (async), skipping
   unchanged positions. Revised 2026-10-29 (agreed): first draft wrote to
   the sockets from Lua. `--goto` is the only client, so discovery, scope,
   directory checks and stale-socket cleanup live in tested Rust only
   (measured: 3 ms per call, 5 ms in tmux). A proper plugin is out of scope.
10. **Platforms**: Linux and macOS (`cfg(unix)`, std `UnixListener`, no
   new dependency). Elsewhere `--follow` and `--goto` fail with a clear
   message. On macOS, `$XDG_RUNTIME_DIR` is usually unset, so `$TMPDIR`
   (per-user) is used. Socket paths stay well under macOS's 104-byte
   limit.

## Architecture

- `src/follow_sock.rs` *(name my call)*: socket dir, `bind` → listener
  plus cleanup guard, `scope()`, `parse_line`, and `send_all(line, path)`
  (used by `--goto`). Pure functions are unit-tested.
- Listener thread: accepts connections, reads lines, sends
  `AppEvent::Follow { path, line }` through `App::sender`. It's started
  in `run::event_loop` next to `start_watcher` when `--follow` is set.
- `App`: `follow_quiet_until: Option<Instant>` is set in `handle_key`,
  `mouse` and `paste`. `follow_event(path, line)` applies decisions 5–7.
  Time comes from `self.now` / the event, so tests can control it.
- `cli`: `--follow` (bool, with the usual PATH) and `--goto FILE:LINE`
  (conflicts with everything else). `--follow` stores its flag in
  `StartOptions`.

## Known limits

- Line numbers come from the editor's buffer, while ramble renders the
  saved file. With unsaved edits, the preview lands on the matching line
  number of the old text until you save. That's acceptable for a preview
  that updates on save.
- No reverse sync (ramble → editor beyond `<leader>o`), no column, no
  heading anchors. The line format leaves room to add them.

## Testing

- Unit: `parse_line` (spaces, colons, bad input), `--goto` arg split,
  socket dir choice, scope from `$TMUX` + window id and the
  sender's socket selection, the grace window and mode gating, landing
  (visible/centred), another-file open plus history, non-markdown
  ignored.
- Integration (`tests/follow.rs`): bind in a short-named tempdir under
  `/tmp` (`CARGO_TARGET_TMPDIR` can exceed the 104-byte socket path limit), `send_all`, and wait with a bound
  for the event. Also stale socket removal. The grace-window test uses
  injected `Instant`s, not sleeps.
- `tui_e2e`: one case, ramble `--follow` in tmux, `ramble --goto`, and
  check the screen moved. It's skipped without tmux, as now.

## Checklist

1. `follow_sock` module: dir, bind/cleanup, parse, send_all + tests.
2. `AppEvent::Follow`, grace state, `follow_event` + tests.
3. CLI `--follow` / `--goto`, wired in `main` and `run` + `tests/cli.rs`.
4. Integration and e2e tests.
5. Docs: guide section "Following your editor" with the nvim snippet and
   a `--goto` example. README "Why it's good" bullet (preview beside your
   editor) and the `## Use` line switched to `--follow`. Design note.
