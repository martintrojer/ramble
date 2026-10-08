# Plan: replace ramble's LSP client and link handling with the mdroots crate

Status: plan; blocked on mdroots (design phase, no code yet)
Date: 2026-10-08
Related: mdroots design in `~/hacking/mdroots/docs/`, especially
`specs/library-and-editors.md`, `specs/api-and-scheduling.md` (embedder API),
`decisions/0007-in-process-only-single-writer.md` (no daemon), and the
code-donation note `research/ramble-code-donation.md`.

## Why

ramble resolves links by asking whichever language server happens to be
installed (zk or marksman). It spawns one server per root, speaks JSON-RPC
itself, and loses search, tags and backlinks when no server is present. zk
also blocks for up to 30 s on `zk.index` before its features work, and its
reindex writes into the notebook's `.zk/notebook.db`.

mdroots is planned as a zero-config markdown index and language server, built
as a library first. Embedded as a crate, it gives ramble resolved links,
backlinks, search, tags and previews with no external binary. It understands
zk, Obsidian, marksman, org and plain relative-path styles, and never writes
user files. The code ramble keeps is the code that is about reading:
rendering, navigation and the UI.

## What changes

| ramble today | After | Lines (today) |
|---|---|---|
| `src/lsp/` (JSON-RPC client, framing, positions, URIs) | deleted; mdroots works in byte offsets and paths | ~810 |
| `src/app/lsp_glue.rs` (spawn, handshake, `zk.index`, didOpen, documentLink, diagnostics → broken links) | ~150-line `mdroots_glue.rs`: one `mdroots::Workspaces`, one worker thread, the existing event channel | 702 |
| `src/notebook.rs` (zk.list / zk.tag.list adapters, notes walk fallback, backlink labels) | thin calls: `notes`, `full_text`, `tags`, `notes_with_tag`, `backlinks` | 276 |
| `src/nav.rs::resolve` (anchors, URLs, `file:`, `.md` suffix) | mdroots resolution per link. ramble keeps anchor and URL dispatch (open browser, jump to heading) | ~40 of 175 |
| `src/app/codepath.rs` (inline code naming a file, `:LINE`) | mdroots code-mention links, with extra search dirs and `:LINE` (to be donated, see below) | 181 |
| `[[lsp.server]]` config, `ServerSpec`, encoding overrides | deleted. One optional `[mdroots]` table (write-cache on/off) | ~80 |
| `src/doc.rs` block/inline model for rendering | **kept**. It is the renderer's input. Its link and front-matter collection come from mdroots instead | — |
| `src/app/watch.rs` (live reload of the open page) | **kept**. It also calls `client.touched(path)` so the index catches up | — |
| sidebar tree walk, nucleo pickers, render, review, marks, motions | **unchanged** | — |

Net: about 1.8–2k lines removed and about 200 added, plus no runtime
dependency on zk or marksman.

## Target design

```text
ramble UI loop ── mpsc ──▶ worker thread ── mdroots::Workspaces (in-process; SQLite cache shared with other editors)
```

- **mdroots runs in-process** (mdroots ADR 0007: no daemon). ramble is a read-only
  peer: it reads the shared index and keeps the page it shows in an in-memory
  overlay. When ramble is the only mdroots process on a root, it holds the
  flock and is the writer. Nothing in ramble depends on which role it has.
- **A worker thread** owns `Workspaces`. It runs blocking calls and sends
  results back as events on the existing channel that `pump_lsp` polls
  today. Stale replies are still dropped by `(page_id<<32)|version`, and a
  `Cancel` token also stops the work. Pushed events (`LinksChanged`,
  `Freshness`, `Progress`) come from `subscribe()`.
- **The status bar** shows `mdroots ●` with the root mode (walk, git-index,
  lazy, memory) instead of `zk ●`.
- **Byte offsets everywhere.** ramble's link ranges and mdroots' ranges are
  the same kind of value, so `position.rs` and the per-server encoding guesswork
  (the zk utf-32 quirk) go away.
- **Read-only.** ramble never writes user files, and neither does mdroots. With
  `--no-index`, or when browsing someone else's tree, ramble sets
  `write_cache(false)` (in-memory index).

### API ramble needs (to request from mdroots)

Taken from the ramble study (2026-10-08). Items marked *gap* are not yet in
the mdroots library spec.

| ramble feature | mdroots call | Status |
|---|---|---|
| link targets and broken-link dimming for the page | `document_links(path, text?) -> Vec<DocLink{range, text_range, kind, context, target, anchor, line, status}>` | gap (spec has single-link `resolve`) |
| `gd` / `Enter` on a link | `DocLink.target` (+ anchor, line) from the call above | gap (same) |
| `K` preview | `preview(target, max_lines) -> {title, summary, frontmatter, excerpt}` | gap |
| backlinks picker | `backlinks(path) -> Vec<{from, from_title, range, line, in_code}>` | partial (title, line and `in_code` missing) |
| notes picker | `notes(root) -> Vec<{path, title, tags, modified}>`, unranked so nucleo filters | gap (spec has ranked `search_notes`) |
| full-text search | `full_text(root, q, n) -> Vec<{path, line, snippet}>` | partial (snippet and line missing) |
| tags, notes by tag | `tags(root)`, `notes_with_tag(root, tag)` | partial (`notes_with_tag` missing) |
| code-path links (`` `src/main.rs:12` ``) | code-mention links with `:LINE` and extra search dirs (page dir, VCS root, tree root) | gap; ramble can donate the code |
| live reload | `touched(path)`; `Event::LinksChanged{path}` | gap |
| status label | `backend()`, `Event::Freshness`, `Event::BackendLost` | gap |

Also needed: no tokio, ever (ramble spec Q22); mdroots' core is synchronous.
SQLite is bundled; `write_cache(false)` gives an in-memory index.

## Phases

Each phase ships on its own and keeps every existing test green.

0. **Prepare (now, no mdroots needed).**
   - Put a `LinkSource` trait in front of `lsp_glue`: `links(page)`,
     `preview`, `backlinks`, `notes`, `search`, `tags`. The current LSP code
     becomes the first implementation. All later phases then swap the backend
     behind the trait, not the UI.
   - Move `codepath::resolve`/`strip_position`/`locate`/`clean` and
     `doc.rs::slugify`/`unique_slug` into a small module with no `App` dependency,
     ready to donate (see the mdroots note).
1. **mdroots alongside LSP (feature flag `mdroots`).**
   - `MdrootsSource` implements `LinkSource` over `mdroots::Workspaces`.
   - Selection: `[mdroots] enable = true` in config, else LSP as today.
   - Differential test: for the fixtures in `tests/fixtures/zk`, assert that
     both backends return the same targets, backlinks and broken links.
     Unexplained differences become mdroots issues.
2. **mdroots default; LSP becomes the fallback.** When mdroots can't open a root
   (rare: e.g. a virtual FS in lazy mode), fall back to an LSP server if one
   is configured.
3. **Link data from mdroots, not doc.rs.** `doc.rs::parse` stops collecting links
   and front matter. `Document.links` is filled from `document_links`. The
   front-matter fold (spec 2026-10-08) reads mdroots' parsed frontmatter, which
   covers YAML, TOML, org `#+KEY` and Logseq `key::`, instead of drawing raw lines.
   - Block and inline rendering stays on ramble's pulldown-cmark pass. Both
     use pulldown-cmark 0.13 with the same options (wikilinks, YAML metadata,
     math), so byte ranges line up. A test asserts that each mdroots link
     range falls on a ramble inline boundary.
4. **Delete the LSP client.** Remove `src/lsp/`, `lsp_glue.rs`, the zk adapters,
   `[[lsp.server]]`, `fake-lsp`, and the LSP e2e tests (`lsp.rs`,
   `lsp_zk_e2e.rs`, `marksman_e2e.rs`). Keep a generic "external LSP" backend
   only if a real use for it shows up.

## Tests

- Phase 1's differential suite is the migration's main safety net.
- The existing `tests/doc.rs` (37), `tests/nav.rs` (8) and `tests/codepath.rs`
  (22) keep passing until their logic moves. Then the cases move with the code
  into mdroots, and ramble keeps thin integration tests.
- Snapshot tests (`tests/snapshots`) must not change, since rendering is
  untouched. Any diff means link ranges moved.
- A new test asserts that ramble with mdroots never writes inside the browsed tree
  (compare tree mtimes before and after a session).

## Risks

| Risk | Mitigation |
|---|---|
| mdroots API not ready or different from the sketch | Phase 0's `LinkSource` trait isolates ramble; the LSP backend keeps working |
| Link semantics differ from zk's (e.g. partial-href matching) | the differential suite; mdroots' ladder includes zk-style partial matching as a last step |
| Two parses of each page (ramble render + mdroots links) | both run at ~1 GB/s; a 50 KB page costs <0.1 ms each. Phase 3 can share one event stream if needed (mdroots exposing `parse_with(events)`) |
| Binary size and build time (bundled SQLite) | accepted; SQLite adds ~1.5 MB and one C compile |

## Open questions

1. Should ramble keep a generic external-LSP backend after phase 4 (for
   servers other than zk and marksman)?
2. Should the notes picker use mdroots' ranked search, or ramble's nucleo over an
   unranked list (current behaviour)?
3. Code-path search dirs: the page dir, VCS root and tree root are ramble policy. Pass
   them per call, or let mdroots infer them (VCS root it already knows)?
