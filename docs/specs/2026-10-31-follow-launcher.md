# Follow launcher: `<leader>O`

Agreed with the user 2026-10-31. Calls marked *(my call)* were filled in
by the agent while writing the spec. They aren't user decisions; check them.
This is the companion to debrief's
`docs/specs/2026-10-31-follow-editor.md`, which owns the shared parts.

## Problem

`--follow` has to be chosen at startup, and the editor has to be opened
by hand in another pane. `<leader>o` suspends ramble, so the preview is
hidden while you edit. One key should open the editor beside ramble and
make ramble follow it.

## Decisions

1. **Shared crate** (agreed): `src/follow_sock.rs` moves to the
   `debrief-follow` crate in the debrief workspace. ramble depends on it
   from crates.io (0.6.5, debrief's release), as it does on
   `debrief-review`. The socket directory becomes `debrief-follow-<uid>/` and socket names gain the tool,
   `<scope>.ramble.<pid>.sock`. Everything else in
   `2026-10-22-follow-cursor.md` still holds.
2. **`follow = true` on a `[[launch]]`** (agreed): inside tmux, the argv
   runs in `tmux split-window -h -t $TMUX_PANE -c <cwd> -- …`
   (`debrief_follow::split_argv`), without suspending or reloading
   (the watcher sees saves). The listener starts if `--follow` didn't
   already start it. Outside tmux it's a plain suspend-and-run
   *(my call)*.
3. **Default** (agreed): a built-in `edit-follow` launcher on `<leader>O`
   with `["${editor}", "+${line}", "${file}"]` and `follow = true`, with
   the same target as `<leader>o`. `needs_vcs = false`. A user entry
   replaces a default whole, as ramble's launchers always have, so an
   `edit-follow` override without `follow = true` isn't a follow
   launcher *(my call; debrief merges fields instead)*.
4. **Listener on demand**: `App` keeps `Option<Listener>`
   (`follow_start`), started by `--follow` or by the first follow
   launch. The launch binds before it splits, so the editor's first
   `--goto` can't be missed, and clears the grace window the `O`
   keypress started.
   `follow_event` is unchanged.
5. **Hook**: the guide's nvim snippet becomes the one shared with debrief.
   It fires for every normal file buffer and calls `ramble --goto` or
   `debrief --goto`. ramble still ignores non-markdown paths.

## Testing

- The follow_sock tests move to `debrief-follow`. ramble keeps
  `tests/follow.rs` as a smoke test through the crate.
- Launch: `<leader>O` expands to the tmux argv given injected
  `$TMUX`/`$TMUX_PANE`, falls back without them, and `follow` parses.
- A follow launch starts the listener once. A second one doesn't rebind.
- `tui_e2e`: `<leader>O` in tmux opens a pane (the editor stubbed with
  `cat` or `sleep`), and then `ramble --goto` moves the view.

## Checklist

1. Depend on `debrief-follow` and delete `src/follow_sock.rs`. Fix the
   socket dir in tests and docs.
2. `follow` launcher option, the split effect, the default `<leader>O`,
   help and clue entries.
3. The listener on demand in `run`.
4. Guide: `<leader>O` in "Following your editor", the shared snippet,
   and the launcher table.
