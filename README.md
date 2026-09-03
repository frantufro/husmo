# husmo

A local-first, git-backed document/link database for an AI agent to save and
retrieve content, with a Rust MCP server as its interface.

## Quick start

On OpenCode, one command installs the skill, registers the MCP server, and
fetches the binary if you don't have it:

```sh
npx @frantufro/husmo install
```

Then create the data repo your Documents will live in:

```sh
cd wherever/you/want/the/data/repo
husmo init --repo git@github.com:you/husmo-data.git
```

Restart OpenCode and ask it to save a page. The details, the project-scoped
variant, and the by-hand equivalent are in
["Use husmo from OpenCode"](#use-husmo-from-opencode) below.

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

## Install

```bash
curl -sSL https://raw.githubusercontent.com/frantufro/husmo/main/install.sh | sh
```

Or via Homebrew (macOS and Linux):

```bash
brew install frantufro/tap/husmo
```

Or build and install from source:

```bash
git clone https://github.com/frantufro/husmo.git
cd husmo
cargo install --path .
```

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

Once a data repo is configured, point an MCP client at the `husmo` binary run
with no arguments. `husmo init` bootstraps the data repo; bare `husmo` is the
MCP server. It speaks the standard MCP stdio transport, so any client that
handles stdio servers can reach it. See "Use husmo from OpenCode" and "Use
husmo from Claude Code" below for the two hosts husmo packages itself for.

The first `save` or `search-semantic` call downloads the local embedding
model's weights (see "Local-first" above) and caches them. Every call after
that, including in future runs, works without network access for embeddings.

## Use husmo from OpenCode

```sh
npx @frantufro/husmo install
```

That installs the three pieces OpenCode needs and reports on a fourth:

```
  ✓ skill   installed  ~/.config/opencode/skills/husmo-document-management
  ✓ binary  downloaded husmo 0.1.2 to ~/.local/bin/husmo
  ✓ mcp     registered husmo in  ~/.config/opencode/opencode.json
  → data    no config at ~/.config/husmo/config.toml
    create a data repo before using husmo's tools:
      cd wherever/you/want/the/data/repo
      husmo init --repo git@github.com:you/husmo-data.git

  restart OpenCode to pick up the skill and the husmo tools.
```

| Piece | Where it lands | What it does |
| --- | --- | --- |
| Skill | `~/.config/opencode/skills/husmo-document-management/` | teaches the agent when to save a page and how to triage the outgoing links `save` reports |
| MCP registration | the `mcp` block of `~/.config/opencode/opencode.json` | makes OpenCode spawn the binary, which is what puts husmo's tools in front of the model |
| Binary | `~/.local/bin/husmo`, from the latest GitHub release | skipped when `husmo` is already on `PATH` (Homebrew, cargo, `install.sh`) |
| Data repo | wherever you choose | created by `husmo init`; the installer reports whether one is configured and stops there |

The skill and the registration are separate pieces. The skill is instructions
for the model; the registration is what makes OpenCode start the server at all.
The installer places both, which is why one command covers the setup.

Re-running the installer is safe. Anything already in place and unchanged is
reported as `up to date` and left alone, so it doubles as a way to repair a
half-finished setup.

### Verify

Restart OpenCode, then:

```sh
opencode mcp list
```

```
┌  MCP Servers
│
●  ✓ husmo connected
│      /opt/homebrew/bin/husmo
│
└  1 server(s)
```

To exercise the whole path — skill, server, and data repo — ask the agent to
save a page:

> save https://example.com/some-article and tell me what links it points at

### Install into one project

```sh
npx @frantufro/husmo install --project
```

This writes `./.opencode/skills/husmo-document-management/` and a `husmo` entry
in `./opencode.json`, both of which OpenCode picks up for that directory only.
Both are safe to commit: the project registration runs `husmo` through `PATH`,
so it stays correct for every teammate who has husmo installed.

### Options

| Flag | Effect |
| --- | --- |
| `--project` | install into the current project; the default is the user config directory |
| `--force` | overwrite an existing skill or registration without asking |
| `--skip-existing` | keep whatever is already there and exit 0 |
| `--no-binary` | install the skill and registration only |
| `--dry-run` | report every action, write nothing |

On a terminal, a skill or registration that differs from what the installer
would write prompts you before anything is overwritten. In a script or CI,
where there is nobody to ask, the same situation fails with exit 1 and leaves
the file untouched; pass `--force` or `--skip-existing` to say which you meant.

### What the registration looks like

```jsonc
{
  "mcp": {
    "husmo": {
      "type": "local",
      "command": ["/Users/you/.local/bin/husmo"],
      "timeout": 120000
    }
  }
}
```

The installer edits `opencode.json` in place through the same JSONC library
OpenCode itself uses, so comments, formatting, and the provider credentials
that usually live in that file all survive untouched. An existing
`opencode.jsonc` is used when there is one.

`timeout` is raised from OpenCode's default of 30 seconds because the first
`save` or `search-semantic` downloads roughly 90MB of embedding-model weights
inside a single tool call. Later calls use the cache and return quickly.

### Setting it up by hand

The installer performs three ordinary steps, each doable by hand:

1. Install the binary (see "Install" above) and run `husmo init`.
2. Copy [`claude-plugin/skills/husmo-document-management/`](claude-plugin/skills/husmo-document-management)
   into `~/.config/opencode/skills/`.
3. Add the `mcp` block shown above to `~/.config/opencode/opencode.json`,
   with `command` pointing at your `husmo`.

OpenCode 1.18 and later can write step 3 for you:

```sh
opencode mcp add husmo -- husmo
```

That form always targets the global config and sets no timeout, so adjust the
file afterwards if you want either changed.

### Troubleshooting

**husmo's tools never appear.** OpenCode reads its config at startup, so
restart it. Then run `opencode mcp list`: a missing `husmo` row means the
registration landed in a config file OpenCode isn't reading — check
`opencode debug paths` against the path the installer printed.

**`husmo connected` but every tool call fails.** The server is running and the
data repo is missing. Run `husmo init --repo <url>` in the directory you want
the Documents to live in, then restart OpenCode.

**The server shows as failed with a spawn error.** The registration names a
binary that has moved. Re-run `npx @frantufro/husmo install --force` to point
it at the current one.

**The first save takes a long time or times out.** It is fetching roughly
90MB of embedding-model weights inside that one call. Raise `timeout` in the
registration and try again; once the weights are cached, every later call
returns quickly.

**`~/.local/bin` is not on your PATH.** The installer says so when it downloads
there. Add `export PATH="$HOME/.local/bin:$PATH"` to your shell config; the
global registration uses the absolute path, so OpenCode works either way.

## Use husmo from Claude Code

Install the binary and run `husmo init` first (see "Install" and "Setup"
above), then add the plugin:

```
/plugin marketplace add frantufro/claude-plugins
/plugin install husmo@frantufro-plugins
```

The plugin carries `claude-plugin/` from this repository: the same skill the
OpenCode installer copies, plus an MCP server declaration that runs `husmo`
through `PATH`. Restart Claude Code and husmo's tools are available.

Claude Code sets MCP timeouts through environment variables. If the first
`save` times out while the embedding weights download, raise them and restart:

```sh
export MCP_TIMEOUT=120000
export MCP_TOOL_TIMEOUT=120000
```

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

## Contributing

Build it with `cargo build`, run the suite with `cargo test`.
[CONTRIBUTING.md](CONTRIBUTING.md) covers the lint setup, the project layout
and how a release is cut.

## License

MIT
