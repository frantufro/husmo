# husmo

A local-first, git-backed document/link database for an AI agent to save and
retrieve content, with a Rust MCP server as its interface.

## Quick start

Three steps: install the binary, create the data repo, connect your agent.
The full design is in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) and the domain
glossary in [`CONTEXT.md`](CONTEXT.md).

### 1. Install the binary

```sh
curl -sSL https://raw.githubusercontent.com/frantufro/husmo/main/install.sh | sh
```

Homebrew (macOS and Linux) works too:

```sh
brew install frantufro/tap/husmo
```

To build from source, clone the repo and run `cargo install --path .`.

The script installs to `~/.local/bin`. Add `export PATH="$HOME/.local/bin:$PATH"`
to your shell config if that directory is missing from your `PATH`.

### 2. Create the data repo

Your Documents live in a git repo of their own (see "Repo split" below). Create
an empty one on your git host, then clone it with husmo:

```sh
cd wherever/you/want/the/data/repo
husmo init --repo git@github.com:you/husmo-data.git
```

`husmo init` clones the repo into the current directory and writes the config
file that points husmo at it. Remotes over SSH authenticate with your
ssh-agent first, then with `~/.ssh/id_ed25519`, `id_ecdsa` or `id_rsa`. A
remote on a non-standard port needs an `ssh://` URL that carries the port,
for example `ssh://git@host:2222/you/husmo-data.git`.

### 3. Connect your agent

husmo is an MCP server: `husmo` with no arguments speaks MCP over stdio. Pick
your agent. Each block registers the server and, where the agent supports it,
adds the skill that teaches the agent when to save a page and how to triage
the outgoing links `save` reports.

<details>
<summary><b>Claude Code</b></summary>

```
/plugin marketplace add frantufro/claude-plugins
/plugin install husmo@frantufro-plugins
```

The plugin carries the skill and an MCP declaration that runs `husmo` from
your `PATH`. Restart Claude Code.

The first `save` downloads about 90MB of embedding-model weights. If it times
out, raise Claude Code's MCP timeouts and restart:

```sh
export MCP_TIMEOUT=120000
export MCP_TOOL_TIMEOUT=120000
```

</details>

<details>
<summary><b>OpenCode</b></summary>

```sh
npx @frantufro/husmo install
```

One command installs the skill (`~/.config/opencode/skills/`), registers the
MCP server in `~/.config/opencode/opencode.json`, and downloads the binary
when `husmo` is missing from your `PATH`, so it also covers step 1:

```
  ✓ skill   installed  ~/.config/opencode/skills/husmo-document-management
  ✓ binary  downloaded husmo 0.1.4 to ~/.local/bin/husmo
  ✓ mcp     registered husmo in  ~/.config/opencode/opencode.json
  → data    no config at ~/.config/husmo/config.toml
    create a data repo before using husmo's tools:
      cd wherever/you/want/the/data/repo
      husmo init --repo git@github.com:you/husmo-data.git

  restart OpenCode to pick up the skill and the husmo tools.
```

Restart OpenCode afterwards.

For one project only, run `npx @frantufro/husmo install --project`. It writes
`./.opencode/skills/` and `./opencode.json`, both safe to commit.

Other flags: `--force` overwrites without asking, `--skip-existing` keeps what
is there, `--no-binary` skips the download, and `--dry-run` reports every
action and writes nothing. Re-running the installer is safe.

To register the server by hand (OpenCode 1.18 and later):

```sh
opencode mcp add husmo -- husmo
```

or add this to the `mcp` block of `opencode.json`:

```jsonc
"husmo": { "type": "local", "command": ["husmo"], "timeout": 120000 }
```

The `timeout` of 120 seconds covers the first `save`, which downloads the
embedding-model weights. Verify with `opencode mcp list`; a `husmo connected`
row means the server runs.

</details>

<details>
<summary><b>Codex</b></summary>

Register the server:

```sh
codex mcp add husmo -- husmo
```

Raise the tool timeout for the first `save`, which downloads about 90MB of
embedding-model weights. Edit `~/.codex/config.toml` (or a trusted project's
`.codex/config.toml`):

```toml
[mcp_servers.husmo]
command = "husmo"
tool_timeout_sec = 120
```

Add the skill. Codex reads user skills from `~/.agents/skills/`:

```sh
mkdir -p ~/.agents/skills/husmo-document-management
curl -sSL https://raw.githubusercontent.com/frantufro/husmo/main/claude-plugin/skills/husmo-document-management/SKILL.md \
  -o ~/.agents/skills/husmo-document-management/SKILL.md
```

Restart Codex and check `codex mcp list`.

</details>

<details>
<summary><b>Kilo Code</b></summary>

Add husmo under the `mcp` key of `~/.config/kilo/kilo.json` (or `kilo.json` /
`.kilo/kilo.json` in a project). Keep any entries already in the file:

```json
{
  "mcp": {
    "husmo": {
      "type": "local",
      "command": ["husmo"],
      "enabled": true
    }
  }
}
```

In the VS Code extension you can also open Settings → MCP → Add Server and
choose Local (stdio) with the command `husmo`.

Add the skill. Kilo loads skills from `~/.agents/skills/` by default:

```sh
mkdir -p ~/.agents/skills/husmo-document-management
curl -sSL https://raw.githubusercontent.com/frantufro/husmo/main/claude-plugin/skills/husmo-document-management/SKILL.md \
  -o ~/.agents/skills/husmo-document-management/SKILL.md
```

Restart Kilo and check `kilo mcp list`.

</details>

<details>
<summary><b>Pi</b></summary>

Pi has MCP support built in. Register the server:

```sh
pi mcp add husmo -- husmo
```

Add `--local` to register it for the current project only (the project file
is `.pi/mcp.json`, read once you trust the project). The user-level file is
`~/.pi/agent/mcp.json`, and the entry looks like:

```json
{ "mcpServers": { "husmo": { "command": "husmo" } } }
```

Add the skill. Pi reads skills from `~/.agents/skills/` as well as
`~/.pi/agent/skills/`:

```sh
mkdir -p ~/.agents/skills/husmo-document-management
curl -sSL https://raw.githubusercontent.com/frantufro/husmo/main/claude-plugin/skills/husmo-document-management/SKILL.md \
  -o ~/.agents/skills/husmo-document-management/SKILL.md
```

Run `/reload` in Pi (or restart it) and check `pi mcp list`.

</details>

#### Check that it works

Ask your agent to save a page:

> save https://example.com/some-article and tell me what links it points at

The first `save` or `search-semantic` call downloads the local embedding
model's weights (see "Local-first" below) and caches them. Every call after
that works without network access for embeddings.

#### Troubleshooting

**husmo's tools never appear.** Agents read their MCP config at startup, so
restart the agent, then list its MCP servers with the command in your agent's
block above. A missing `husmo` row means the registration landed in a config
file the agent is not reading.

**The server connects and every tool call fails.** The data repo is missing.
Run `husmo init --repo <url>` (step 2), then restart the agent.

**The server fails with a spawn error.** The registration names a binary that
has moved or is missing from the agent's `PATH`. Use the absolute path from
`which husmo` as the command.

**The first save is slow or times out.** It is fetching about 90MB of
model weights in that one call. Raise the timeout as shown in your agent's
block and try again.

**`husmo init` fails with an SSH authentication error.** The message names the
key files tried. Load a passphrase-protected key into an ssh-agent, or use a
passphrase-less key in `~/.ssh`.

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

## What `husmo init` does

1. Clones the given git URL into the current directory. It refuses to run if
   that directory already exists and is not empty, so nothing already there
   gets overwritten.
2. Writes the config file (see "Configuration" below) with `data_repo_path`
   set to the clone, so the MCP server has a data repo the moment `init`
   finishes.

An empty git repo works. Create one on your git host and clone it before
pushing anything to it. `--repo` is optional: without it, `husmo init` asks for
the URL interactively.

## Configuration

husmo reads a TOML config file that points at the data repo:

```toml
data_repo_path = "/path/to/your/husmo-data"
```

`husmo init` writes this file for you (see "What `husmo init` does" above); editing it by
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
