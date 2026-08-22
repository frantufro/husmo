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

## Setup

husmo needs a data repo (see "Repo split" above) before its MCP server has
anywhere to save Documents. `husmo init` bootstraps one:

```sh
cd wherever/you/want/the/data/repo
husmo init --repo git@github.com:you/husmo-data.git
```

`--repo` is optional; omitted, `husmo init` prompts for the URL
interactively instead, for a scripted/non-interactive setup. `init`:

1. Clones the given git URL into the current directory. Refuses to run if
   that directory already exists and isn't empty, so it never clobbers
   anything already there.
2. Writes/updates the config file below to point `data_repo_path` at the
   clone, so the MCP server has a working data repo location the moment
   `init` finishes — no separate manual config-editing step.

An empty git repo works too (`git init --bare` it somewhere first, or create
one on your git host and skip pushing anything to it before cloning);
`init` doesn't require the repo to already contain Documents.

Once a data repo is configured, add husmo as an MCP server to whichever MCP
client you use, pointed at the built `husmo` binary run with no arguments
(`husmo` — not `husmo init`). It speaks the standard MCP stdio transport, so
no extra wiring beyond what the client already does for any other stdio MCP
server is needed. The first `save` or `search-semantic` call downloads the
local embedding model's weights (see "Local-first" below) and caches them;
every call after that, including in future runs, needs no network access
for embeddings.

## Configuration

husmo reads a TOML config file that points at the data repo:

```toml
data_repo_path = "/path/to/your/husmo-data"
```

`husmo init` writes this file for you (see "Setup" above); editing it by
hand is only needed to point at a different data repo later. husmo locates
the file by checking, in order:

1. `HUSMO_CONFIG` — an explicit path to the config file.
2. `$XDG_CONFIG_HOME/husmo/config.toml`.
3. `$HOME/.config/husmo/config.toml`.

## Status

The full tool surface from `docs/ARCHITECTURE.md` is implemented: `save`,
`get`, `search-semantic`, `search-tag`, `search-fulltext`, `relate`,
`unrelate`, `list`, and `delete`, plus MCP resources (`resources/list` /
`resources/read`) for browsing Documents directly. An end-to-end test
(`tests/end_to_end_smoke_test.rs`) exercises the full story across them:
saving a URL, discovering and archiving an outgoing link, relating the two
Documents, retrieving the Related list, finding a Document by semantic
search, and deleting it.

Archiving a discovered outgoing link (`husmo::archive::archive_outgoing_link`)
has no MCP tool of its own yet — deciding which discovered links are worth
archiving is left to a Skill layered on top of this server, per
`docs/ARCHITECTURE.md` ("Content extraction").

## Development

```sh
cargo test
cargo clippy -- -D warnings -W clippy::pedantic
cargo fmt
```
