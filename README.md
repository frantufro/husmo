# husmo

A local-first, git-backed document/link database for an AI agent to save and
retrieve content, with a Rust MCP server as its interface.

## Local-first: no external services

husmo is fully local and self-contained. It works offline, has no per-save
API cost, and never sends saved content to a third party. Saved Documents
may include private or work content, so this is a deliberate project value,
not an incidental implementation choice — see
[`docs/adr/0001-local-first-no-external-services.md`](docs/adr/0001-local-first-no-external-services.md).

Concretely:

- **No external embeddings API.** Embeddings are generated locally,
  in-process, via `candle` running a small pre-trained sentence-embedding
  model (`all-MiniLM-L6-v2`). Its weights are fetched once, on first use,
  and cached locally after that — no network call happens on any
  individual save or search.
- **No external vector database.** The searchable index is assembled
  in-process from committed sidecar files, not stored in a separate service.
- **No headless browser or JS rendering** for fetched pages — plain HTTP
  fetch plus readability-style extraction.
- **No persistent daemon.** The MCP server runs over stdio, spawned per
  session by the MCP client.
- **Files as source of truth.** Every Document is a human-readable Markdown
  file with YAML frontmatter, committed to a git-tracked data repo separate
  from this one — diffable and recoverable without the tool.

## Repo split

- **This repo** (`husmo`): Rust source, the MCP server, docs. No user data
  lives here.
- **Data repo**: the actual Documents, git-tracked, at a path supplied via
  config (see below). husmo never hardcodes a data repo location.

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for the full design and
[`CONTEXT.md`](CONTEXT.md) for the domain glossary.

## Configuration

husmo reads a TOML config file that points at the data repo:

```toml
data_repo_path = "/path/to/your/husmo-data"
```

It locates that file by checking, in order:

1. `HUSMO_CONFIG` — an explicit path to the config file.
2. `$XDG_CONFIG_HOME/husmo/config.toml`.
3. `$HOME/.config/husmo/config.toml`.

## Status

Early scaffolding. The MCP server's tool surface (`save`, `get`,
`search-semantic`, `search-tag`, `search-fulltext`, `relate`, `unrelate`,
`list`, `delete`) is not implemented yet — see `docs/ARCHITECTURE.md` for
what's planned.

## Development

```sh
cargo test
cargo clippy -- -D warnings -W clippy::pedantic
cargo fmt
```
