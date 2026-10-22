# AGENTS.md

## Gates

- **Fast** (while iterating): `cargo nextest run` (~7 s warm), or `cargo nextest run --test <file>` / `-E 'test(<name>)'` for one area.
- **Slow** (before closing work, and once per merged batch), as CI runs it: `cargo fmt --all --check && cargo clippy --all-targets -- -D warnings && cargo test`. ~20 s warm.
- **Targeted slow**: `cargo nextest run --test tui_e2e` (real binary in tmux; skipped without tmux) and `--test lsp_zk_e2e` (skipped without `zk`). A skip is not a pass: say so in the gate record.
- **Budget**: slow gate 5 min. Over budget is a trigger for test upkeep, not a stop.

Record each gate run in the task note (`GATE: commit tree command scope`, `RESULT: exit duration counts skips`, `CONDITIONS: host, git, load, cache`).

Test notes:
- A test whose tree walk climbs above its fixture creates its tempdir under `env!("CARGO_TARGET_TMPDIR")`: macOS `$TMPDIR` can hold thousands of entries (`tests/sidebar.rs` `going_up_stops_at_the_filesystem_root`).
- Fixed waits in `tests/watch.rs`, `tests/app.rs` and `tests/mdroots.rs` are negative windows (proving a late reply is dropped or nothing fires); keep them and state the timing contract when adding one. Positive waits poll with a bound.
- ramble runs no git in its tests.
