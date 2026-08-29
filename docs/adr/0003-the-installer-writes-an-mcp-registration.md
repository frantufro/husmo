# The installer writes an MCP Registration into the Host's config

husmo's tool surface is MCP, so the Skill this project ships is inert on its own: it tells a
model how to triage Outgoing Links returned by `save`, and `save` exists for a Host only once
that Host's config carries an entry under `mcp` pointing at the Binary. cubil's equivalent
installer copies a `SKILL.md` and downloads a binary, and that is a complete installation there
because its Skill instructs the agent to shell out to a command on `PATH`. Here the same two
steps leave a user with a Skill describing tools their Host never loaded. So
`npx @frantufro/husmo install` does three things: it places the Skill, it writes a
**Registration**, and it fetches the Binary when `husmo` is absent from `PATH`.

Bootstrapping a Data Repo stays outside that list. `husmo init` clones a repository into the
current directory and writes husmo's own config file, and choosing where a personal archive of
possibly-private Documents lives is a decision worth making deliberately, at a prompt the user
went looking for. The Installer detects the three states — no config file, a config whose
`data_repo_path` has no `.git`, or a working Data Repo — and reports the matching next step. It exits 0 in all three, because its own three artifacts did land.

## Why the Installer edits the config itself

OpenCode ships `opencode mcp add`. Through 1.15 it was a pure wizard — location, server name,
transport type, and command all came from interactive selects, with no flags to supply them.
1.18 added a non-interactive form, so `opencode mcp add husmo -- husmo` now writes a working
local entry. Three gaps keep the Installer writing the file itself. The non-interactive form
always targets the global config, leaving project Scope with no equivalent. It has no `timeout`
option, and husmo's first call needs a raised one (below). It places no Skill, so setting husmo
up would still take two commands. Versions before 1.18 remain in wide use and support none of
it. The Installer performs the same edit directly, and mirrors OpenCode's own behaviour in three
respects.

It resolves the target file the way OpenCode does. Global Scope takes the first existing of
`opencode.json` or `opencode.jsonc` under the Host's user config directory, defaulting to the
former; project Scope also considers `.opencode/opencode.json` and `.opencode/opencode.jsonc`
beneath the repository root.

It writes through `jsonc-parser`'s `modify` and `applyEdits`, the library and the call pair
OpenCode itself uses, at `["mcp", "husmo"]` with two-space indentation. This is a deliberate
departure from the zero-dependency shape of the sibling cubil package. The target is a file
users fill with provider credentials and, by OpenCode's own resolution order, may legitimately
be JSONC with comments in it. `JSON.parse` followed by `JSON.stringify` would reformat the
whole file wholesale and throw outright on the commented variant, and hand-rolling a JSONC
editor is how that file gets corrupted. One dependency, itself dependency-free, buys a patch
that is byte-identical to what `opencode mcp add` would have produced.

It leaves the `environment` block out. OpenCode's MCP module spawns local servers with
`env: {...process.env, ...config.environment}`, so husmo receives the full environment of a
Host that is itself launched from a terminal. `XDG_CONFIG_HOME`, `HF_HOME`, and `SSH_AUTH_SOCK`
all arrive intact, and pinning any of them into the Registration would freeze a value that is
correct today. `SSH_AUTH_SOCK` in particular is a per-login socket path on macOS, and a
Registration carrying one would point at a dead socket by the following week.

## What the Registration contains

The Installer writes `type`, `command`, and `timeout`, and omits everything else. Omitting
`enabled` matches what `opencode mcp add` produces.

`command` varies by Scope, and this is the part that would look inconsistent without the
reason. Global Scope writes the absolute path the Installer resolved for the Binary, re-resolved
on every run so a later `brew install` is picked up by re-running the Installer; the file it
lands in is machine-specific and nobody shares it, and the Installer's own download target of
`~/.local/bin` is frequently missing from `PATH`. Project Scope writes `["husmo"]`, because
`./opencode.json` is a file teams commit, and an absolute path under one developer's home
directory is broken for everyone else who checks it out.

`timeout` is set to 120000. OpenCode's default is 30000, and husmo's first `save` or
`search-semantic` fetches `config.json`, `tokenizer.json`, and roughly 90MB of
`model.safetensors` for `all-MiniLM-L6-v2` and then runs first inference, all inside one tool
call. Thirty seconds is a plausible miss on a slow connection, and the resulting timeout would
land on a user whose installation is entirely correct. The raised ceiling costs nothing once
the model is cached, since a timeout only applies to a request that hangs.

## Naming

The npm package is `@frantufro/husmo` and its executable is `husmo-install`.

The scope is deliberate. npm's typosquat filter runs at publish time and rejected the bare name
`cubil` for similarity to `cuid` and `util`, both at edit distance 2; `husmo` sits at the same
distance from `husky`. Scoped names are exempt from that filter, and the bare name `skulk` is
already taken by an unrelated package, so all three projects in this family read the same way.

The executable name is load bearing, more so here than for cubil. A project Registration is
literally `"command": ["husmo"]`, resolved through `PATH` by a spawn with `shell: false`. An npm
package exposing an executable named `husmo` would place this Node installer exactly where the
Host expects the MCP Server, and the Host would spawn the Installer and wait for an MCP
handshake that never comes. A distinct executable name keeps `husmo` on `PATH` meaning the
Binary, and `npx @frantufro/husmo install` still works, because npm runs a package's single
declared executable whatever that executable is called.

## Consequences

- The Installer's `--force` replaces the whole value at `["mcp", "husmo"]`, so any `cwd` or
  `environment` a user added by hand is discarded. Conflicts therefore print the existing and
  proposed Registrations in full before asking, and a non-TTY run with neither `--force` nor
  `--skip-existing` fails with exit 1, leaving the config untouched. `--dry-run`
  renders every action, writing nothing.
- A project-Scope install on a machine with `XDG_CONFIG_HOME` set is weaker than a global one.
  The Registration carries no `HUSMO_CONFIG`, so the MCP Server resolves its config through the
  chain in `config.rs`, which is correct while the Host's environment reaches it. The Installer
  prints the `environment` block to add by hand for anyone who wants it pinned.
- Claude Code reaches the same place by a different route. The Installer stays scoped to
  OpenCode, and `claude-plugin/` gains its own MCP server declaration alongside `plugin.json`,
  so installing the plugin delivers the Skill and the Registration together. That declaration
  names the Binary as `husmo` through `PATH`, for the same reason project Scope does: the
  plugin is distributed, and the Binary lives outside it. Claude Code controls MCP timeouts
  through `MCP_TIMEOUT` and `MCP_TOOL_TIMEOUT`, so the raised ceiling above is documented in
  the README for that Host.
- The Installer never upgrades a Binary it finds on `PATH`, and husmo has no `update`
  subcommand to point at. It infers the origin from the resolved path — a Homebrew Cellar prefix
  or anything else — and prints `brew upgrade husmo` or the `install.sh` line accordingly.
