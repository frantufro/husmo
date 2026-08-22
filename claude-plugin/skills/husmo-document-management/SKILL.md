---
name: husmo-document-management
description: >
  Save and retrieve Documents (URLs, pasted text, local files) through the
  husmo MCP server, including triaging outgoing links discovered during a
  save. Covers save, get, search-semantic, search-tag, search-fulltext,
  relate, unrelate, list, delete, and MCP resources. TRIGGER when: user
  mentions husmo, saving a link/URL/document for later, archiving outgoing
  links from a saved page, relating saved documents, or searching a personal
  document/link database.
allowed-tools: [Bash]
---

# husmo Document Management

husmo is a local-first, git-backed document/link database. Every Document
(a URL fetch, pasted text, or a local file) is saved as a Markdown file with
YAML frontmatter in a separate git-tracked data repo — human-readable,
diffable, recoverable without the tool. Retrieval and mutation happen
through the husmo MCP server's tools, plus a read-only resources interface.
Full design: `docs/ARCHITECTURE.md` and `CONTEXT.md` in the husmo repo.

## Prerequisites

husmo needs a data repo before it has anywhere to save Documents. If saving
or searching fails because none is configured, bootstrap one:

```bash
husmo init --repo git@github.com:you/husmo-data.git
```

`--repo` is optional; omitted, `husmo init` prompts for the URL
interactively. This clones the repo into the current directory and writes
the config file pointing at it — a one-time setup step, not something to
repeat per session. The husmo MCP server itself must already be registered
with the client (stdio transport, spawned per session) for the tools below
to be available at all.

## Tool Surface

All Document mutation and retrieval goes through these MCP tools (the
client's own namespacing applies — call them by their plain names below):

| Tool | Purpose |
|------|---------|
| `save` | Save a URL, pasted text, or local file as a Document. |
| `get` | Look up one Document by exactly one of `id` / `slug` / `url`. |
| `search-semantic` | Meaning-based search over chunked content, RAG-style. |
| `search-tag` | Exact tag-membership filter. |
| `search-fulltext` | Literal, case-insensitive substring search. |
| `list` | List every Document. |
| `relate` / `unrelate` | Declare/remove a symmetric Related edge between two Documents. |
| `delete` | Delete a Document (still recoverable from git history). |

Every result that includes a Document lists its Related documents by
reference (`id`/`title`) — their content is only pulled in when a search
call explicitly sets `expand_related`.

### `save`

Exactly one of `url`, `path`, or `content` is required; `content` also
requires `title`. Optional `tags`. Re-saving a `url` that's already saved
overwrites that Document in place instead of creating a duplicate — this is
deliberate, not a bug to work around.

`save` returns the saved Document plus `outgoing_links`: hyperlinks found in
the content, reported as data only. **They are never followed
automatically** — see "Outgoing Link Triage" below for what to do with them.

### Choosing a search tool

The three search tools answer different questions — don't default to one
for everything:

- **`search-semantic`** — "find Documents about X" when you don't know the
  exact wording. Ranked by meaning, most similar first.
- **`search-fulltext`** — "find the Document that contains this exact
  phrase." Semantic search can miss an exact string it should have matched.
- **`search-tag`** — "find every Document tagged Y." Neither other tool
  looks at tags at all.

### `relate` / `unrelate`

A Related edge is a **deliberate, symmetric, untyped** connection declared
between two existing Documents by `id`. It is unrelated to whether one
Document's content happens to link to the other's URL — that's a separate
concept (an outgoing link). Calling `relate` on an edge that already exists,
or `unrelate` on one that doesn't, is a no-op, not an error.

## Outgoing Link Triage

When `save` returns a non-empty `outgoing_links`, don't archive any of them
automatically — ask the user which ones are worth keeping:

1. Present the discovered links (title/URL) from `save`'s response.
2. Ask which ones to archive. Skip this prompt only if the user already
   said up front which links they want (e.g. "save this and archive
   anything about X").
3. For each link the user picks, call `save` again with `url` set to that
   link's URL — this runs the exact same fetch → extract → store pipeline a
   top-level save would, so no separate "archive" tool is needed.
4. This is one level deep only: don't inspect the newly-archived Document's
   own `outgoing_links` and re-offer to archive *those* without being asked
   again. Recursive crawling is a deliberate non-goal.

Archiving a link and declaring it Related are **separate, independent
steps**. Archiving never implies Related — if the user wants the newly
archived Document connected to the one it came from, call `relate`
explicitly as its own step; don't assume they want it.

## MCP Resources

Alongside the tools above, husmo also exposes Documents as MCP resources
(`resources/list` / `resources/read`) for a human to `@`-attach a Document
directly in clients that support it. This is a second interface over the
same data, not a replacement — use the tools above when *you* (the agent)
are deciding what to retrieve; resources are for a human who already knows
which Document they want. A resource's URI is keyed by the Document's
`slug`; `resources/read` returns the same raw Markdown-with-frontmatter
shape as `get`, with Related listed by reference only.

## Important Notes

- **Fully local.** No external embeddings API, no external vector database,
  no headless browser for fetched pages. The only network access, ever, is
  fetching a URL you asked it to save, and a one-time embedding-model
  download on first use — not per save or per search.
- **Every mutation is committed and pushed.** `save`, `delete`, `relate`,
  and `unrelate` each pull, write, commit, and push the data repo
  automatically. There's no separate manual commit step, and nothing is
  truly unrecoverable — `delete` just makes a Document absent from current
  state, not absent from git history.
- **One Document, many sources.** URLs, pasted text, and local files (text,
  PDF) are all just "a Document" — there's no separate "link" type to
  reason about differently.
