# husmo — architecture reference

This is the implementation context distilled from the project's requirements
interview. `CONTEXT.md` stays a pure domain glossary; this file carries the
technical decisions. See also `docs/adr/0001-local-first-no-external-services.md`.

## Repo split

Two repositories:
- **App repo** (this one, `husmo`): Rust source, the MCP server crate, docs.
  No user data lives here.
- **Data repo**: the actual Documents (Markdown + images + embedding sidecar
  files), git-tracked. Its path is supplied via a config file — the app
  should never hardcode a data repo location.

## Storage model

- **Files-as-source-of-truth**: every Document is a Markdown file with YAML
  frontmatter, committed to the data repo. Human-readable, diffable,
  recoverable without the tool.
- **Document** is the one unified concept for anything saved — a URL fetch,
  pasted/typed text, or a local file (PDF, etc.). No separate "Link" type.
- Fields: stable internal `id` (frontmatter, never changes), `slug`
  (filename, derived from title, human-browsable, collision-handled),
  `canonical_url` (optional — set only when sourced from a URL), `title`,
  `tags` (list of free-form strings), `saved_at`, `summary` (optional),
  `author` (optional), Markdown `content`, `related` (list of other
  Document ids).
- A `canonical_url` identifies at most one Document. Re-saving the same URL
  **overwrites** that Document's content in place — git history carries the
  diff. It does not create a second Document.
- Identity resolution (used by `get` and internally) accepts exactly one of
  `id` / `slug` / `url` and resolves to the same Document.

## Bootstrapping a data repo: `husmo init`

A CLI subcommand (not an MCP tool) that bootstraps the data repo on a new
machine/folder:
- Prompts interactively for the data repo's git URL (also accepts it via a
  `--repo <url>` flag for scripted/non-interactive use).
- Clones it into the current directory.
- Writes/updates the config file (see "Repo split" above) to point at the
  cloned path, so the app has a working data repo location immediately
  after `init` completes — no separate manual config-editing step.

## Content extraction

- URL fetch: plain HTTP via `reqwest` — **no headless browser, no JS
  rendering**. Then readability-style extraction to Markdown.
- Outgoing hyperlinks found in the content are preserved as links in the
  Markdown (not stripped).
- Images are downloaded (actual bytes, not just referenced) and stored as
  local files alongside the Document; the Markdown is rewritten to point at
  the local copies.
- **Outgoing links** discovered during extraction are returned by `save` as
  data, not auto-followed. A caller can later archive one as its own
  Document — one level deep per save, no automatic recursive crawling. (The
  "ask the human which links look worth archiving" behavior is a Skill
  layered on top of this codebase, not something this server implements
  itself — the server just needs to report discovered links honestly.)
- Local file ingestion uses an extensible per-file-type extractor. Start
  with plain text and PDF; more formats slot in behind the same interface
  later.
- Pasted/typed text: no fetch, no `canonical_url`, Document created
  directly from supplied content.

## Related (distinct from outgoing links)

- A **Related** edge is deliberate, symmetric, and untyped — declared
  explicitly via `relate`/`unrelate` between any two existing Documents. It
  is unrelated to whether one Document's content links to the other's URL.
- `get` and every search result always list a Document's Related documents
  by reference (id/title). Their content is only pulled into a result when
  a search call explicitly opts into expansion.

## Retrieval

Four distinct capabilities — do not collapse into one fuzzy "search":
1. **Semantic search** — over chunked embeddings, RAG-style.
2. **Full-text/keyword search** — exact string matches semantic search can miss.
3. **Tag-filter search**.
4. **Exact lookup** by `id`/`slug`/`url` — this is the `get` tool.

- Documents are split into chunks (paragraph/section-sized) before
  embedding — not embedded as one whole-document vector. Better recall
  against long content.
- Embeddings are generated **locally, in-process, via a real trained
  model** (`candle` + a small pre-trained sentence-embedding model, e.g.
  `all-MiniLM-L6-v2`). This is a deliberate project value — see ADR 0001
  — and state it in the README. The constraint is zero network calls
  *per operation* (no save/search ever calls out to an API); fetching the
  model's weights once, at setup/first-run, and caching them locally
  after that, is within that constraint and does not violate it.
- An earlier revision of this file described a deterministic
  feature-hashing bag-of-words placeholder in place of the trained model,
  reasoning that even a one-time weights download broke "zero network
  calls." That reasoning was rejected — that implementation is being
  replaced with the real model it should have used from the start. The
  sidecar file format is agnostic to which function produced the vectors,
  so this is a drop-in replacement, not a format change.
- Each Document's chunk embeddings are committed to the data repo as small
  per-Document sidecar files — not one giant shared blob.
- The assembled searchable vector index (e.g. `usearch` or a pure-Rust HNSW
  crate like `instant-distance`) is **not committed**. It's a local,
  disposable, gitignored cache rebuilt in memory at server startup from the
  committed sidecar files. No external vector DB service (no Qdrant) — ADR 0001.
- Full-text and tag search can similarly use a disposable local cache
  (e.g. SQLite FTS5) rebuilt at startup, not committed to git.

## Git mechanics (data repo, at runtime)

Every mutating operation (`save`, `delete`, `relate`, `unrelate`) does, in
order: **git pull → write files → git commit → git push**, automatically.
No manual commit step. Single machine for now — the pull-first step is a
forward-looking safety habit, not a full multi-machine conflict-resolution
system.

This is distinct from the app repo's own normal development git history —
don't conflate the two.

## MCP server

- Transport: **stdio, spawned per session** by the MCP client. No
  persistent background daemon, no auto-start requirement, for now.
- Tool surface (plain names — rely on the MCP client's own namespacing,
  don't manually prefix):
  - `save` — URL, pasted text, or local file path; dispatches to the right
    ingestion path; chunks + embeds; runs the git pull/commit/push cycle;
    returns the saved Document plus any outgoing links discovered.
  - `get` — exactly one of `id` / `slug` / `url` (named optional params,
    server validates exactly one is set); includes Related list.
  - `search-semantic` — with an opt-in flag to expand into Related
    documents' content.
  - `search-tag`
  - `search-fulltext`
  - `relate` / `unrelate`
  - `list`
  - `delete` — goes through the same git pull/commit/push cycle; nothing is
    truly unrecoverable since it's still in git history.

## MCP resources

Alongside the tool surface above, husmo exposes Documents as MCP resources
(`resources/list` + `resources/read`) — a second, additional interface, not
a replacement for `list`/`get`. Tools serve an agent deciding mid-task to
retrieve something; resources serve a human directly `@`-attaching a
Document they already know they want, in clients that support it (e.g.
Claude Code). Both interfaces share the same underlying Store code. See
`docs/adr/0002-mcp-resources-alongside-tools-for-document-browsing.md`.

- A resource's URI identifies a Document by `slug` (human-derived,
  fuzzy-searchable via the client's picker). Retitling a Document changes
  its slug and can break an old `@` reference — accepted, not guarded
  against with alias tracking.
- `resources/read` returns the same shape as `get`: raw on-disk
  Markdown-with-frontmatter (`text/markdown`), Related Documents listed by
  reference only, never inlined.
- A resource's `name` is the Document's `title`; `description` is
  `summary` when present, else a short `content` snippet.
- `resources/list` is paginated from the start (cursor-based), even though
  current scale doesn't require it.

## Explicit non-goals (for now)

- No headless browser / JS rendering for fetched pages.
- No external vector database, no external embeddings API.
- No persistent daemon / auto-start.
- No multi-machine conflict resolution beyond pull-before-write.
- No typed/directional relationships (Related is plain and symmetric).
- No automatic/recursive link-following (one level deep, on request only).
