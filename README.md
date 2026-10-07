# ramble

A read-only TUI markdown reader that follows links between notes, backed by
markdown language servers ([zk](https://github.com/zk-org/zk) first,
marksman second).

## Status

Early development. The design lives in
[docs/specs/2026-10-07-ramble.md](docs/specs/2026-10-07-ramble.md).

- Renders CommonMark and GFM, reflowed to the terminal width
- Follows links with browser-style history
- Vim bindings throughout

| Mode | Command |
|---|---|
| Interactive | `ramble notes/index.md` |
| Print | `ramble --print README.md` |

```sh
cargo run -- --print README.md
```
