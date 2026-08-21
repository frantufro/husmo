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
- Embeddings are generated **locally, in-process** (e.g. via `candle`
  running a small local model). No network calls, no external embeddings
  API. This is a deliberate project value — see ADR 0001, and state it in
  the README.
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

## Explicit non-goals (for now)

- No headless browser / JS rendering for fetched pages.
- No external vector database, no external embeddings API.
- No persistent daemon / auto-start.
- No multi-machine conflict resolution beyond pull-before-write.
- No typed/directional relationships (Related is plain and symmetric).
- No automatic/recursive link-following (one level deep, on request only).
