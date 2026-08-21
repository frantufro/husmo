# Expose Documents as MCP resources, alongside the existing tools

husmo needs two distinct retrieval paths: an agent deciding mid-task to browse or fetch a
Document as part of its own reasoning, and a human composing a prompt who already knows which
Document they want and would rather attach it directly than describe it to the agent. MCP
models these as different capabilities — tools are actions an agent invokes, resources are
browsable content a client attaches, typically via a human-driven `@`-mention flow — and not
every MCP client exposes resources to the model autonomously the way Claude Code does.

We're keeping the planned `list` and `get` tools exactly as designed, and adding
`resources/list` + `resources/read` as a second, additional interface over the same Store code,
rather than replacing either tool or having one interface delegate to the other. This keeps
agent-autonomous retrieval working on any MCP client regardless of its resource support, while
giving humans in resource-aware clients a direct `@`-attach path.

Two decisions within this are also hard to reverse once a client depends on them:

- **Resource URIs identify a Document by `slug`, not `id`.** `slug` is human-derived from
  `title` and can change if the title is edited later (see `document.rs`), so an old `@`
  reference can go stale after a retitle. We accept that: this is a personal, single-user
  archive, discovery happens through the client's fuzzy picker (matched against `name`/
  `description`, not the raw URI), and building alias tracking to keep every historical slug
  resolvable forever is real ongoing complexity for a rare, self-healing inconvenience.
- **`resources/list` is paginated from day one**, even though today's scale doesn't need it.
  Retrofitting a cursor later means every client that already assumed "one full, unpaginated
  list" needs to change that assumption — cheap to build in now, disruptive to add after the
  fact.

## Consequences

- `resources/read` returns the same shape as `get`: the raw on-disk Markdown-with-frontmatter
  (`text/markdown`), Related Documents listed by reference only, never inlined. This keeps
  retrieval semantics identical regardless of which interface fetched the Document.
- A resource's `name` is the Document's `title`; its `description` is `summary` when present,
  falling back to a short `content` snippet when it isn't — `summary` stays optional, as
  already documented in `docs/ARCHITECTURE.md`.
