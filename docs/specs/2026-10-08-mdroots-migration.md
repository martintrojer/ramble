# ramble on mdroots

Status: current design. Steps S0–S4 are in place; using `Workspace::goto`
for `gd` is an optional cleanup.

ramble embeds [mdroots](https://github.com/martintrojer/mdroots), a markdown
notebook index, as its built-in backend. A third-party language server
([zk](https://github.com/zk-org/zk),
[marksman](https://github.com/artempyanykh/marksman) or any other) is an
optional backend, turned on per root in config. With no `[[lsp.server]]`
configured (the default) ramble runs no external tool.

## Split of work

- **mdroots knows markdown and the notebook:** link, heading and front
  matter ranges; slugs; link resolution and broken links; backlinks, notes,
  full-text search, tags, previews; index freshness and the cache.
- **ramble knows reading and the UI:** the block/inline layout parse, the
  renderer and its srcmap, the sidebar, the pickers' UI, motions, visual
  mode and yank, mouse, the key clue, help, launchers, review comments and
  the event loop.
- When something could live in either, it lives in mdroots, and ramble
  keeps a thin call.

## Decisions

| # | Question | Decision | Why |
|---|---|---|---|
| Q1 | ramble's own backend trait, or mdroots' types directly? | **mdroots' types directly** (`mdroots::{Workspaces, Workspace, DocLink, Backlink, NoteSummary, Hit}`, `mdroots::syntax::{Document, Link, Frontmatter}`). | No adapter on the main path. |
| Q2 | Does mdroots own block structure too? | **No, only the semantic elements:** links, headings and front matter. ramble keeps a slim pulldown-cmark layout parse (paragraphs, lists, tables, code blocks, inline styles, code spans) for the renderer. | Block layout is presentation and the srcmap depends on it. Both parses use pulldown-cmark 0.13 with `mdroots::syntax::markdown_options()`, so ranges line up; a test checks this. |
| Q3 | Dependency | **`mdroots` from crates.io** (`mdroots = "0.2.4"`; the facade re-exports the parser as `mdroots::syntax`). Never the internal `mdroots-*` crates: the facade pins them and they have no stability promise. For work on both repos at once, a local `[patch.crates-io]`, never committed. | mdroots is 0.x: read its CHANGELOG on upgrades. |
| Q4 | Who designs mdroots' API? | **mdroots.** ramble states its needs (below); mdroots designs the calls in its own specs and review, and ramble adopts each as it lands. | mdroots has other users. |
| Q5 | Cache when browsing a plain folder? | **mdroots decides; ramble passes no policy.** mdroots writes a per-root cache only for a discovered root (marker, VCS or accepted loose root), never for a single-file workspace, and runs denied locations (home, `~/Downloads`, `/tmp`, ...) single-file in memory. | mdroots already has the root rules and the denylist. |
| Q6 | The third-party LSP backend | **Kept, with no planned removal.** It runs only for a configured `[[lsp.server]]` whose root marker is found above the page. Its end-to-end tests stay in the gate. | Some readers want the server their editor uses. |

## Design

```text
ramble UI loop ── mpsc ──▶ worker thread ── mdroots::Workspaces (in-process; SQLite cache per root)
              └─ optional: LSP client (src/backend/lsp, src/app/lsp_glue.rs) for pages under a configured server's root
```

### Syntax (`src/doc.rs`, `src/render.rs`)

- One `mdroots::syntax::parse_with` per page, with `unfenced_frontmatter`
  off (an unfenced header is prose to ramble). Headings (with slugs; an
  explicit `{#id}` is kept as written) and prose links come from it, in
  ramble's `Heading` and `Link` shapes. Reference definitions, links in
  code and images are dropped.
- The layout parse (pulldown-cmark with `markdown_options()`) gives blocks,
  inlines and the inline code spans that can become code-path links.
- Front matter is `mdroots::syntax::Frontmatter` (`fields()`, `parsed()`,
  `inner()`, `FieldValue::display`); ramble keeps only the fold UI (see
  `2026-10-08-front-matter.md`).
- `tests/doc.rs` (`mdroots_links_and_headings_fall_on_layout_boundaries`)
  checks that every mdroots link and heading range falls on a layout
  boundary; rendering snapshots pin the rest.

### The backend glue (`src/app/mdroots_glue.rs`)

- **One worker thread** owns `Workspaces` and runs every mdroots call. The
  UI drains its replies each loop iteration (`App::pump_mdroots`). Page
  requests carry the tag `(page_id << 32) | version`; a reply for another
  page or an older version is dropped. Queued page requests are coalesced
  to the latest; picker and preview requests are answered in order.
  Dropping the app cancels the shared `mdroots::Cancel`.
- **Which backend serves a page.** A page with a path that no configured
  server selects goes to mdroots. Stdin pages get no backend.
- **Open without blocking.** For a page whose root has no ready workspace,
  the worker answers from `Workspace::open_single` first, then opens the
  root with `Workspaces::for_path` and answers again.
- **Page links.** The worker sets ramble's text as the overlay, reads
  `document_links`, and clears the overlay. Each `DocLink` is matched to a
  ramble link by exact range; its target and anchor drive `gd`, and a
  `Broken` status dims the link. Dimming comes only from a root workspace
  that is not single-file, since a one-page index calls a `[[Title]]` it
  cannot see broken.
- **Following** (`gd`, `Enter`). A link with an mdroots target opens it at
  the anchor. URLs, same-page anchors and links mdroots did not report
  (e.g. in an unfenced header) are resolved by `nav::resolve`.
- **`K` preview** asks `preview(target, 12)` on the target's own root
  workspace, with its own seq, so it never makes the page's reply stale.
  The popup shows the title, the front matter except the title, and the
  excerpt.
- **Pickers.** Notes (`notes()`, newest first by `modified`), notes with a
  tag (`notes_with_tag`), search (`full_text`, at most 200 hits, one row
  per hit at its line), tags (`tags()`, merged case-insensitively) and
  backlinks (`backlinks(path)`) are answered from the page's root
  workspace. Only marker and VCS roots are complete. For any other root
  (loose, lazy, single file) the notes picker walks the tree root and the
  other pickers' titles end in `(partial)`.
- **Live reload.** ramble's own watcher (`src/app/watch.rs`) reloads the
  page; the reload asks mdroots again with the page in `refresh_paths`.
  Other notes changing (a new note a `[[Title]]` link names) come from
  mdroots' watcher: the worker opens with `Options::watch` and subscribes
  to each root workspace it answers from. mdroots watches only roots it
  reconciles in a database on a local disk; elsewhere such changes show
  after `C-l` or reopening the page.
- **Status line.** `mdroots ●` from the root's workspace, `mdroots ○` from
  the single-file one, `—` before the first answer or when nothing serves
  the page, and the usual spinner after a slow request. A page under a
  configured server shows that server's label instead (`zk ●`); the label
  never names both.
- **Read-only.** Neither ramble nor mdroots writes user files; only the
  mdroots cache dir is written (Q5).
- **No tokio** (ramble spec Q22). mdroots is synchronous.

### Cache

The cache is mdroots' per-user cache dir. `MDROOTS_CACHE_DIR` (the same
variable as the mdroots CLI) points it elsewhere. Tests use
`MdrootsOptions::memory()` (`IndexMode::Memory`) or
`MdrootsOptions::cache_dir(tempdir)`, never the user's cache;
`tests/mdroots.rs` asserts that a session writes nothing inside the browsed
tree and that a plain folder caches only in the given dir.

## What ramble needs from mdroots

| ramble feature | mdroots call | Status |
|---|---|---|
| links, headings, front matter for layout | `mdroots::syntax::parse_with`, `markdown_options()` | used |
| link targets, broken-link dimming | `Workspace::set_overlay`, `document_links` | used |
| `K` preview | `Workspace::preview(target, max_lines)` | used |
| notes, notes by tag | `Workspace::notes()` (`NoteSummary.modified`), `notes_with_tag` | used |
| full-text search | `Workspace::full_text(q, limit, cancel)` | used |
| tags | `Workspace::tags()` | used |
| backlinks | `Workspace::backlinks(path)` | used |
| live reload | `Workspace::refresh_paths`, `Options::watch` + `subscribe` | used |
| fast first page | `Workspace::open_single` + `Workspaces::for_path`, `Workspaces::get` | used |
| `gd` targets with heading ranges | `Workspace::goto(path, offset)` | available, not used (S4) |
| code-path links | `LinkKind::CodeMention` with `Options::code_dirs` | available, not used (ramble resolves code paths itself) |

## The LSP backend module (S4)

- The client, the zk adapter and the LSP-only picker helpers live in
  `src/backend/lsp/`; `ramble::lsp` stays as a re-export. The App glue
  stays in `src/app/lsp_glue.rs`, where it reaches the app's private
  state. Its replies (documentLink, hover, references, zk commands) are
  not translated into mdroots' types.
- Code-path links stay in ramble: `src/app/codepath.rs` resolves them on
  every page (against the page's dir, its VCS root and the tree root; any
  existing file; `~/` from the environment). stdin and LSP pages need them
  too, and mdroots' `CodeMention` search differs, so mdroots' code mentions
  are not used.
- Optional cleanup: take `gd` targets from `Workspace::goto` and keep only
  the dispatch in `nav` (open page, jump to heading, open browser, open
  editor).

## Risks

| Risk | Mitigation |
|---|---|
| mdroots API changes (0.x) | Depend only on the facade; read its CHANGELOG on upgrades. |
| Range drift between the two parses | Both use `markdown_options()`; the boundary test and the rendering snapshots catch drift. |
| Two parses per page | Both run at about 1 GB/s; a 50 KB page costs under 0.1 ms each. |
| A slow first index blocks the worker | One worker, no per-call cancellation: a big root's first `for_path` delays later requests (pickers show loading). The first page is still served from `open_single`. |
| Binary size and build time (bundled SQLite) | Accepted; about 1.5 MB and one C compile. |
| Tests writing the real cache | Tests use memory mode or a temp cache dir (see Cache). |
