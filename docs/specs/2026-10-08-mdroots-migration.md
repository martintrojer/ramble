# Plan: ramble is mdroots-first

Status: plan, revised 2026-10-08 after the user's direction (Q1–Q5 below
agreed). No code work yet. Step S1 can start now. S2 onward wait for
`mdroots-core` and `mdroots-resolve`.
Related: mdroots design in `~/hacking/mdroots/docs/`, especially
`specs/library-and-editors.md`, `specs/api-and-scheduling.md` (embedder API),
`specs/roots-and-instances.md` (root markers),
`decisions/0007-in-process-only-single-writer.md` (no daemon), and the
code-donation note `research/ramble-code-donation.md`.

## Direction

ramble embeds the mdroots crate and is built around it. A third-party
language server (zk, marksman, anything else) is an optional backend, turned
on in config. Every decision optimises for mdroots:

- **Offload everything that is knowledge about markdown or a notebook to
  mdroots:** link, heading, tag and code-span ranges; front matter; link
  resolution (anchors, `file:` URLs, the extensionless `.md` rule, code-path
  links with `:LINE`); slugs; backlinks, notes, full-text search, tags,
  previews; and index freshness.
- **ramble keeps only what is about reading and the UI:** the renderer and
  its srcmap, the block/inline layout parse, the sidebar, the pickers' UI,
  motions, visual mode and yank, mouse, the key clue, help, launchers, the
  tuicr review markers and the event loop.
- **Less code in ramble is a goal in itself.** When a feature could live in
  either place, it lives in mdroots, and ramble keeps a thin call.

## Decisions

| # | Question | Decision | Why |
|---|---|---|---|
| Q1 | ramble's own `LinkSource` trait, or mdroots' types directly? | **mdroots' types directly** (`mdroots::Workspaces`, `mdroots_syntax::Link`, `Frontmatter`, …). The optional third-party LSP backend translates LSP replies into those types. | No adapter on the main path. The translation cost lands on the rarely used path. |
| Q2 | Does mdroots own block structure too? | **No, only the semantic elements:** links, headings, tags, code spans, front matter. ramble keeps a slim pulldown-cmark layout parse (paragraphs, lists, tables, code blocks, inline styles) for the renderer. | Block layout is presentation, the srcmap depends on it, and mdroots' other users don't need it. Both use pulldown-cmark 0.13 with `mdroots_syntax::markdown_options()`, so ranges line up. A test checks this. |
| Q3 | Start before `mdroots-core` exists? | **Yes.** A path dependency on `~/hacking/mdroots/crates/mdroots-syntax` now, a git or crates.io dependency once mdroots publishes. The syntax switch (S1) goes first. | It deletes `frontmatter.rs` and the link and heading collection in `doc.rs` immediately. |
| Q4 | Does this build write mdroots' missing API? | **No.** mdroots designs its own API from the needs listed here. ramble adopts each call as it lands. The exception is code ramble donates (front matter fixes, code paths, `nav::resolve` rules): it is ported into mdroots by the same author, with its tests, before ramble deletes its copy. | mdroots has its own specs and review. |
| Q5 | Cache when browsing a plain folder? | **Write the SQLite cache only for notebook or repo roots** (mdroots' root markers: `.zk`, `.git`, `.obsidian`, `.marksman.toml`, …). Elsewhere, `write_cache(false)` gives an in-memory index. | `ramble ~/Downloads` leaves nothing behind, and a notebook gets the cache's speed. |

## What ramble deletes, and what replaces it

| ramble today (main `e24c3b88`) | After | Lines |
|---|---|---|
| `src/frontmatter.rs` (YAML/TOML display parsing) | `mdroots_syntax::Frontmatter` / `Value`. ramble keeps only the fold UI (marker row, `za`, table layout) | −1053 |
| `src/doc.rs` link, heading, slug, code-span and front-matter collection, `from_bytes` | `mdroots_syntax::parse_bytes` → `Document::{links, headings, tags, frontmatter}`. ramble's `doc.rs` keeps only blocks and inlines for layout | about −500 of 1044 |
| `src/nav.rs::resolve` (anchors, schemes, `file:`, percent-decoding, extensionless `.md`) | mdroots resolution: `DocLink.target`, `anchor`, `line`. ramble keeps only the dispatch (open page, jump to heading, open browser, open editor) | about −120 of 175 |
| `src/app/codepath.rs` (inline code naming a file, `:LINE`) | mdroots code-mention links. ramble passes its search dirs (page dir, VCS root, tree root) per call | about −150 of 181 |
| `src/lsp/` (JSON-RPC client, framing, positions, URIs) | moves behind the optional backend (S4); not on the main path | 810 moved; later reduced |
| `src/app/lsp_glue.rs` (spawn, handshake, `zk.index`, didOpen, documentLink, diagnostics) | `mdroots_glue.rs` (~150 lines): one `Workspaces`, one worker thread, the existing event channel. The LSP glue shrinks into the optional backend | −702, +150 |
| `src/notebook.rs` (zk.list / zk.tag.list adapters, notes walk fallback, backlink labels) | thin calls: `notes`, `full_text`, `tags`, `notes_with_tag`, `backlinks` | about −220 of 276 |
| `[[lsp.server]]` config default list | default: mdroots, no config. `[[lsp.server]]` remains only to turn on a third-party backend | about −40 |
| `fake-lsp` test binary, `tests/lsp.rs`, `lsp_zk_e2e.rs`, `marksman_e2e.rs` | kept only as tests of the optional backend (S4) | moved |

Estimate: about −3,000 lines on the main path and +200 for the mdroots glue,
with no runtime dependency on zk or marksman. The optional backend keeps
roughly 1,000 lines of LSP code until there is a reason to shrink or drop it.

Kept unchanged: `render.rs`, sidebar, pickers' UI, motions, visual, mouse,
clue, help, launchers, review (tuicr), watch (it also calls
`touched(path)`), marks, config for everything else.

## Target design

```text
ramble UI loop ── mpsc ──▶ worker thread ── mdroots::Workspaces (in-process; SQLite cache per root, shared with other editors)
                                         └─ optional: third-party LSP backend, translating replies into mdroots types
```

- **In-process** (mdroots ADR 0007). ramble is a read-only peer. When it is
  the only mdroots process on a root it holds the flock and writes the cache.
  Nothing in ramble depends on which role it has.
- **The worker thread** owns `Workspaces`. It runs blocking calls and sends
  results back on the existing event channel. Stale replies are dropped by
  `(page_id<<32)|version`, and a `Cancel` token stops abandoned work. Pushed
  events (`LinksChanged`, `Freshness`, `Progress`) come from `subscribe()`.
- **Byte offsets everywhere.** No position encodings on the main path. The
  zk utf-32 quirk is the optional backend's problem.
- **Status bar:** `mdroots ●` with the root mode (walk, git-index, lazy,
  memory). With a third-party backend, its name, as today.
- **Read-only.** Neither ramble nor mdroots writes user files. The cache is
  written per Q5.
- **No tokio, ever** (ramble spec Q22). mdroots' core is synchronous.

## API ramble needs from mdroots

ramble states its needs; mdroots designs the calls (Q4). *gap* means not in
the mdroots library spec yet.

| ramble feature | Needed from mdroots | Status |
|---|---|---|
| render-time links, headings, tags, front matter | `mdroots_syntax::parse_bytes(bytes, opts) -> Document` with `links()`, `headings()`, `tags()`, `frontmatter()`; `markdown_options()` | **exists** in `mdroots-syntax`; front matter needs ramble's last fixes first (S0) |
| link targets and broken-link dimming for the page | `document_links(path, text?) -> Vec<DocLink{range, text_range, kind, context, target, anchor, line, status}>` | gap (the spec has single-link `resolve`) |
| `gd` / `Enter` | `DocLink.target` + `anchor` + `line` | gap (same call) |
| `K` preview | `preview(target, max_lines) -> {title, summary, frontmatter, excerpt}` | gap |
| backlinks picker | `backlinks(path) -> Vec<{from, from_title, range, line, in_code}>` | partial (title, line and `in_code` missing) |
| notes picker | `notes(root) -> Vec<{path, title, tags, modified}>`, unranked (ramble filters with nucleo) | gap (the spec has ranked `search_notes`) |
| full-text search | `full_text(root, q, n) -> Vec<{path, line, snippet}>` | partial (snippet and line missing) |
| tags, notes by tag | `tags(root)`, `notes_with_tag(root, tag)` | partial (`notes_with_tag` missing) |
| code-path links | code-mention links with `:LINE[:COL]` and caller-supplied search dirs | gap; ramble donates the code (S0) |
| live reload | `touched(path)`; `Event::LinksChanged{path}` | gap |
| status label | `backend()`, `Event::Freshness`, `Event::BackendLost` | gap |
| cache policy (Q5) | root-marker detection exposed (`is_project_root(path)` or the root mode), `write_cache(bool)` per root | partial |

## Steps

Each step ships on its own and keeps every test green. Rendering snapshots
(`tests/snapshots`) must not change in any step; a diff means ranges moved.

**S0. Donate first (mdroots side, same author).** Bring mdroots up to the
code ramble will delete:

- Front matter: port ramble main's `src/frontmatter.rs` fixes newer than the
  `donated/ramble-frontmatter/` snapshot (taken at `5f6e7ed2`). These are
  block scalars (`|`/`>`) whose lines look like keys or items; linear-time
  parsing for large blocks; nested list items taken from the parser; and
  `- |` block-scalar list items. Bring their tests with them. Refresh the
  snapshot to `01e1ef6d`.
- The empty-block and BOM handling from ramble `doc.rs`, if not already
  covered.
- Code paths (`codepath::resolve`, `strip_position`, `locate`, `clean`) and
  the `nav::resolve` rules (extensionless `.md`, `file:` forms,
  percent-decoding) into `mdroots-resolve` once it exists.

**S1. Syntax from mdroots (can start now).**

- Add a path dependency on `mdroots-syntax`.
- `doc.rs` keeps blocks and inlines. Links, headings (with slugs), tags, code
  spans and front matter come from `mdroots_syntax::parse_bytes`. Both
  parses use `markdown_options()`.
- Delete `src/frontmatter.rs`. The fold UI reads `mdroots_syntax::Frontmatter`.
- Tests:
  - every mdroots link and heading range falls on a ramble inline or block
    boundary;
  - the 37 `tests/doc.rs` cases and the front-matter tests either pass
    against the new source or move to mdroots, with ramble keeping thin
    integration tests;
  - snapshots unchanged.

**S2. mdroots as the backend (needs `mdroots-core`, `mdroots-resolve`).**

- Add `mdroots_glue.rs` with the worker thread. Links and broken-link
  dimming come from `document_links`. `gd` uses mdroots' target. `K` uses
  `preview`. The pickers use `notes`, `full_text`, `tags`, `backlinks`. Live
  reload calls `touched`. The status bar shows mdroots' state.
- Apply the Q5 cache policy.
- Delete `nav::resolve` (keep the dispatch), `codepath.rs` (keep the search
  dir policy), and the notebook adapters.
- Tests: the `tests/fixtures/zk` notebook gives the expected targets,
  backlinks and broken links from mdroots. Expected values are written in
  the test, not taken from zk. A new test asserts that a session never
  writes inside the browsed tree (tree mtimes before and after), and that a
  plain folder leaves no cache file.

**S3. Default without config.** With no `[[lsp.server]]` configured, ramble
uses mdroots. Starting ramble no longer looks for zk or marksman.

**S4. The third-party LSP backend (optional, config).** Move `src/lsp/` and
the remains of `lsp_glue.rs` into `src/backend/lsp/`. It runs only when
`[[lsp.server]]` is configured, and translates documentLink, hover,
references, and zk commands into mdroots' types. The LSP e2e tests stay as
tests of this backend. It is kept indefinitely (settled, below).

## Risks

| Risk | Mitigation |
|---|---|
| mdroots API not ready, or shaped differently | S1 needs only `mdroots-syntax`, which exists. ramble adopts each call as it lands; until then the current LSP glue keeps working for the index features. |
| ramble regresses where its code had fixes mdroots lacks | S0 ports the fixes with their tests before ramble deletes anything. Snapshot and integration tests catch range drift. |
| Two parses per page (ramble layout + mdroots semantics) | Both run at ~1 GB/s; a 50 KB page costs <0.1 ms each. mdroots could expose `parse_with(events)` later to share one event stream. |
| Path dependency on a sibling checkout | Local only. mdroots is published first and ramble switches to that dependency before any push that includes S1 (settled, below). |
| Binary size and build time (bundled SQLite, S2) | Accepted; SQLite adds ~1.5 MB and one C compile. |

## Settled after review

1. **Push ordering (user: yes).** mdroots is published (git or crates.io)
   before ramble pushes anything that includes S1. ramble then switches the
   path dependency to the published one. No vendoring. S1 can be built and
   reviewed locally on the path dependency, but stays unpushed until then.
   Pushing mdroots is still the user's call.
2. **The third-party LSP backend (user: keep it).** S4's backend stays
   supported with no planned removal. Its e2e tests remain part of the gate.

## Open questions

None.
