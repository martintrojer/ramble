# Plan: ramble is mdroots-first

Status: plan, revised 2026-10-09 against **mdroots 0.2.0 on crates.io**. No
ramble code work yet. S1 and S2 can start now; S0 has two front matter fixes
left to port (below).
Related: the mdroots repo (`~/hacking/mdroots`, published as `mdroots` and
`mdroots-cli`): `docs/specs/library.md` (public API, embedder additions),
`docs/specs/roots.md` (root discovery and markers), `docs/DECISIONS.md` D3
(in-process, one writer per root, no daemon), `docs/ROADMAP.md` (what mdroots
still lacks), and `docs/research/ramble.md` (code ported from ramble, what
ramble deletes).

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
| Q1 | ramble's own `LinkSource` trait, or mdroots' types directly? | **mdroots' types directly** (`mdroots::{Workspaces, Workspace, DocLink, Backlink, NoteSummary, Preview, Hit}`, `mdroots::syntax::{Document, Link, Frontmatter}`). The optional third-party LSP backend translates LSP replies into those types. | No adapter on the main path. The translation cost lands on the rarely used path. |
| Q2 | Does mdroots own block structure too? | **No, only the semantic elements:** links, headings, tags, code spans, front matter. ramble keeps a slim pulldown-cmark layout parse (paragraphs, lists, tables, code blocks, inline styles) for the renderer. | Block layout is presentation, the srcmap depends on it, and mdroots' other users don't need it. Both use pulldown-cmark 0.13 with `mdroots::syntax::markdown_options()`, so ranges line up. A test checks this. |
| Q3 | Dependency | **`mdroots = "0.2"` from crates.io** (the facade re-exports the parser as `mdroots::syntax`). Depend only on `mdroots`, never on the internal `mdroots-*` crates: they are pinned by the facade and have no stability promise. For work on both repos at once, a local `[patch.crates-io]` pointing at `~/hacking/mdroots/crates/mdroots`, never committed. | Published, so the old "path dependency, unpushed" rule no longer applies. |
| Q4 | Does this build write mdroots' missing API? | **No.** mdroots designs its own API from the needs listed here (they are in its `ROADMAP.md`). ramble adopts each call as it lands. The exception is code ramble donates (front matter fixes, code-path search dirs): it is ported into mdroots by the same author, with its tests, before ramble deletes its copy. | mdroots has its own specs and review. |
| Q5 | Cache when browsing a plain folder? | **mdroots decides; ramble passes no policy.** `Workspace::open_for` writes the per-root cache only for a discovered root (marker, VCS or accepted loose root) and never for single-file or `open_at` workspaces. For `ramble ~/Downloads` and other denied locations (home, `~/Downloads`, `/tmp`, ...) mdroots runs single-file in memory. A ramble flag can force `IndexMode::Memory`. | mdroots already has the root rules and the denylist; duplicating them in ramble would drift. |

## What ramble deletes, and what replaces it

| ramble today (main `cf0dcb8`) | After | Lines |
|---|---|---|
| `src/frontmatter.rs` (YAML/TOML display parsing; deleted in S1) | `mdroots::syntax::Frontmatter` (`fields()`, `parsed()`, `inner()`, `FieldValue::display`). ramble keeps only the fold UI (marker row, `za`, table layout) | −1053 |
| `src/doc.rs` link, heading, slug, code-span and front-matter collection, `from_bytes` | `mdroots::syntax::parse_bytes` → `Document::{links, headings, tags, frontmatter}`. ramble's `doc.rs` keeps only blocks and inlines for layout | about −500 of 1044 |
| `src/nav.rs::resolve` (anchors, schemes, `file:`, percent-decoding, extensionless `.md`) | `Workspace::document_links` (`DocLink.target`, `anchor`, `line`, `status`) and `Workspace::goto(path, offset)` (target plus heading range). ramble keeps only the dispatch (open page, jump to heading, open browser, open editor) | about −120 of 175 |
| `src/app/codepath.rs` (inline code naming a file, `:LINE`) | mdroots code-mention links (`LinkKind::CodeMention`, `DocLink.line`). Gap: extra search dirs (page dir, VCS root, tree root) in `Options`; until then mdroots resolves code mentions against the root only | about −150 of 181 |
| `src/lsp/` (JSON-RPC client, framing, positions, URIs) | moves behind the optional backend (S4); not on the main path | 810 moved; later reduced |
| `src/app/lsp_glue.rs` (spawn, handshake, `zk.index`, didOpen, documentLink, diagnostics) | `mdroots_glue.rs` (~150 lines): one `Workspaces`, one worker thread, the existing event channel. The LSP glue shrinks into the optional backend | −702, +150 |
| `src/notebook.rs` (zk.list / zk.tag.list adapters, notes walk fallback, backlink labels) | thin calls: `notes`, `full_text`, `tags`, `backlinks`; notes by tag filtered from `notes()` until `notes_with_tag` exists | about −220 of 276 |
| `[[lsp.server]]` config default list | default: mdroots, no config. `[[lsp.server]]` remains only to turn on a third-party backend | about −40 |
| `fake-lsp` test binary, `tests/lsp.rs`, `lsp_zk_e2e.rs`, `marksman_e2e.rs` | kept only as tests of the optional backend (S4) | moved |

Estimate: about −3,000 lines on the main path and +200 for the mdroots glue,
with no runtime dependency on zk or marksman. The optional backend keeps
roughly 1,000 lines of LSP code until there is a reason to shrink or drop it.

Kept unchanged: `render.rs`, sidebar, pickers' UI, motions, visual, mouse,
clue, help, launchers, review (tuicr), watch (see live reload below),
marks, config for everything else.

## Target design

```text
ramble UI loop ── mpsc ──▶ worker thread ── mdroots::Workspaces (in-process; SQLite cache per root, shared with other editors)
                                         └─ optional: third-party LSP backend, translating replies into mdroots types
```

- **In-process** (mdroots D3). When ramble is the only mdroots process on a
  root it holds the root's flock and writes the cache (`Workspace::role()` is
  `Reconciler`); otherwise it is a peer that never writes. Nothing in ramble
  depends on which role it has.
- **The worker thread** owns `Workspaces`. It runs blocking calls and sends
  results back on the existing event channel. Stale replies are dropped by
  `(page_id<<32)|version`, and a `Cancel` token stops abandoned work.
- **Open without blocking.** For the first page of a root, the worker serves
  `Workspace::open_single(page)` at once (no discovery, no cache) and calls
  `Workspaces::for_path(page)` in the background, then swaps; this is what
  `mdroots lsp` does. `Workspaces::get(page)` checks for a ready workspace
  without opening one.
- **Live reload.** ramble keeps its own watcher (`src/app/watch.rs`, for
  re-rendering). On a change it calls `Workspace::refresh_paths(&[path])` so
  links and backlinks follow. Alternatively, `Options::watch(true)` lets
  mdroots watch the root itself and `Workspace::subscribe()` delivers the
  changed paths; pick one, not both. (`subscribe` sends paths only; typed
  events are an mdroots gap.)
- **Byte offsets everywhere.** No position encodings on the main path. The
  zk utf-32 quirk is the optional backend's problem.
- **Status line:** the footer names the backend in the same slot and format
  as today's `zk ●` / `marksman ●` label (`App::lsp_label`,
  `src/app/lsp_glue.rs`):
  - `mdroots ●` when the page is served by its root's workspace (the
    background open finished; `Workspace::freshness()` then says `Fresh`, or
    `Lazy` for a lazy root);
  - `mdroots ○` while the background open runs (the page is served by its
    `open_single` workspace);
  - the same spinner after a slow request as today;
  - `—` only when no backend serves the page.

  With a third-party backend configured, its own name shows (`zk ●`,
  `marksman ●`) exactly as now. The label never names both. The root mode
  (`Workspace::root()`: mode, reason) goes in `g?` help or a later
  `:Status` command, so the footer stays as short as today. A test asserts
  the footer shows `mdroots ●` for a page in `tests/fixtures/zk` with no
  `[[lsp.server]]` configured.
- **Read-only.** Neither ramble nor mdroots writes user files. Only the
  mdroots cache dir is written (Q5); tests set `Options::cache_dir` to a
  temp dir or use `IndexMode::Memory`, never the real cache.
- **No tokio, ever** (ramble spec Q22). mdroots is synchronous.

## API ramble needs from mdroots

ramble states its needs; mdroots designs the calls (Q4). Status at mdroots
0.2.0.

| ramble feature | mdroots 0.2.0 | Status |
|---|---|---|
| render-time links, headings, tags, front matter | `mdroots::syntax::parse_bytes(bytes, opts) -> Option<Document>` with `links()`, `headings()`, `tags()`, `frontmatter()`; `markdown_options()` | **exists**; two front matter fixes still to port (S0) |
| link targets and broken-link dimming for the page | `Workspace::document_links(path) -> Vec<DocLink { range, text_range, kind, context, target, anchor, line, status }>`; current text set with `set_overlay` | **exists** |
| `gd` / `Enter` | `Workspace::goto(path, offset) -> Option<Goto { targets, heading, line }>` | **exists** |
| `K` preview | `Workspace::preview(target, max_lines) -> Preview { title, frontmatter, excerpt }` | **exists**; `summary` is an mdroots gap (ramble can show the excerpt) |
| backlinks picker | `Workspace::backlinks(path) -> Vec<Backlink { from, from_title, range, line, in_code }>` | **exists** |
| notes picker | `Workspace::notes() -> Vec<NoteSummary { path, title, tags }>`, unranked (ramble filters with nucleo); `search_notes` if ranked is wanted | **exists**; `modified` is an mdroots gap (ramble can `stat`) |
| full-text search | `Workspace::full_text(q, limit, cancel) -> Vec<Hit { path, line, snippet }>` (the index for a cache writer, a scan otherwise) | **exists** |
| tags, notes by tag | `Workspace::tags() -> Vec<(tag, count)>`; filter `notes()` by `NoteSummary.tags` | **exists**; `notes_with_tag` is an mdroots gap (convenience only) |
| code-path links | `DocLink { kind: CodeMention, target, line }` | **partial**: caller-supplied search dirs (page dir, VCS root, tree root) are an mdroots gap; ramble donates `code_path_dirs` |
| live reload | `Workspace::refresh_paths(paths, cancel)`, or `Options::watch(true)` + `subscribe()` | **exists** (paths only; typed events are a gap) |
| status label | `Workspace::freshness()`, `role()`, `root()`; `Workspaces::get` | **exists** (derive the label as in Target design) |
| cache policy (Q5) | built into `open_for`; `IndexMode::Memory` to force no cache | **exists** |
| open a page fast in a big root | `Workspace::open_single` + background `Workspaces::for_path` | **exists** |

## Steps

Each step ships on its own and keeps every test green. Rendering snapshots
(`tests/snapshots`) must not change in any step; a diff means ranges moved.

**S0. Donate first (mdroots side, same author).** Mostly done in mdroots
0.1–0.2 (`docs/research/ramble.md` in mdroots lists what was ported, with
ramble commits). Left:

- Front matter: two of ramble's fixes since the snapshot mdroots ported
  (`5f6e7ed2`) are not in mdroots 0.2.0, checked against the 0.2.0 parser:
  - nested list items should take the parser's items
    (`l:\n  - a\n  - - b\n    - c` gives `["a", "b, c"]` in ramble,
    `["a", "- b", "c"]` in mdroots; ramble `065bdf7`);
  - a block-scalar list item is its text (`- |` followed by indented lines
    gives the text in ramble, the literal `|` in mdroots; ramble `01e1ef6`).
  Port them with their tests (`nested_list_items_take_the_parser_items`,
  `block_scalar_list_items_are_their_text`). The block-scalar-as-text fix
  (`04f915b`) and linear parsing (`d5ff6fb`, 20k keys in about 0.6 s) already
  hold in mdroots.
- Code-path search dirs (`code_path_dirs`) as an `Options` field in mdroots.

**S1. Syntax from mdroots (can start now).**

- Add `mdroots = "0.2"`.
- `doc.rs` keeps blocks and inlines. Links, headings (with slugs), tags, code
  spans and front matter come from `mdroots::syntax::parse_bytes`. Both
  parses use `markdown_options()`.
- Delete `src/frontmatter.rs` once S0's two fixes are in a released mdroots.
  The fold UI reads `mdroots::syntax::Frontmatter`.
- Tests:
  - every mdroots link and heading range falls on a ramble inline or block
    boundary;
  - the 37 `tests/doc.rs` cases and the front-matter tests either pass
    against the new source or move to mdroots, with ramble keeping thin
    integration tests;
  - snapshots unchanged.

**S2. mdroots as the backend (can start now).**

- Add `mdroots_glue.rs` with the worker thread. Links and broken-link
  dimming come from `document_links`. `gd` uses `goto`. `K` uses `preview`.
  The pickers use `notes`, `full_text`, `tags`, `backlinks`. Live reload
  calls `refresh_paths`. The status line shows `mdroots ●` / `mdroots ○`
  (see Target design).
- `K` asks the worker for `preview(target, 12)` on the target's root
  workspace (its own request kind and seq; dropped only on page change,
  like the LSP hover). `gd` already prefers mdroots' targets; `nav::resolve`
  stays the gate for URLs and same-page anchors.
- Delete `nav::resolve` (keep the dispatch) and the notebook adapters
  (deferred: S3/S4 own deleting `nav::resolve` and `codepath.rs`'s pure
  half; the zk adapters stay for the LSP backend).
  `codepath.rs` goes once mdroots takes caller search dirs; until then keep
  `code_path_dirs` and the fallback.
- Tests: the `tests/fixtures/zk` notebook gives the expected targets,
  backlinks and broken links from mdroots. Expected values are written in
  the test, not taken from zk. A new test asserts that a session never
  writes inside the browsed tree (tree mtimes before and after), and that a
  plain folder leaves no cache file (with `Options::cache_dir` pointed at a
  temp dir).

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
| mdroots API shaped differently from ramble's needs | Every need above exists in 0.2.0 except the listed gaps, each with a workaround. mdroots is 0.x: pin `mdroots = "0.2"` and read its CHANGELOG on upgrades. |
| ramble regresses where its code had fixes mdroots lacks | S0 ports the two missing front matter fixes with their tests before ramble deletes `frontmatter.rs`. Snapshot and integration tests catch range drift. |
| Two parses per page (ramble layout + mdroots semantics) | Both run at ~1 GB/s; a 50 KB page costs <0.1 ms each. mdroots could expose `parse_with(events)` later to share one event stream. |
| Binary size and build time (bundled SQLite) | Accepted; SQLite adds ~1.5 MB and one C compile. |
| ramble's tests writing the real mdroots cache | Tests pass `Options::cache_dir(tempdir)` or `IndexMode::Memory`; a test asserts the user cache dir is untouched. |

## Settled after review

1. **Publishing (done).** mdroots is on crates.io (0.1.0, 0.2.0); ramble
   depends on the published crate. No vendoring.
2. **The third-party LSP backend (user: keep it).** S4's backend stays
   supported with no planned removal. Its e2e tests remain part of the gate.

## Open questions

None.
