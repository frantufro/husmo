# Contributing

Thanks for your interest in improving `husmo`. The project is small enough
that a drive-by fix is welcome; the bar is green tests and clean lint.

## Ground rules

Before opening a PR, make sure these succeed:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
```

`[lints.clippy]` in `Cargo.toml` already turns the pedantic set on for every
build and denies bare `#[allow]` in favour of `#[expect]`, so the clippy line
above only has to promote warnings to errors.

Changes under `bin/`, `claude-plugin/skills/` or `tests/node/` also need the
installer suite:

```bash
npm ci
npm test
```

It drives `bin/install.mjs` as a subprocess against a throwaway `HOME` and
`XDG_CONFIG_HOME`, with the GitHub API and the release download served by a
fixture HTTP server on localhost, so it runs offline.

`.github/workflows/ci.yml` runs all of it on every pull request and on every
push to `main`. The Rust suite runs on Linux and macOS, and the installer
suite runs on node 18, 20 and 24 — 18 is the floor `package.json` declares.

### The pinned lint toolchain

The `lint` job pins rustc to a specific version. The pedantic set grows with
each release, so a floating toolchain would turn an untouched branch red on
rustc's schedule. To reproduce CI exactly:

```bash
rustup toolchain install 1.98.0 --component rustfmt --component clippy
rustup run 1.98.0 cargo fmt --check
rustup run 1.98.0 cargo clippy --all-targets -- -D warnings
```

Dependabot is told to leave that pin alone, because the action tags version
branches ahead of the Rust releases they name. Move it by hand when a new
stable is out, in the same commit as any fixes its new lints require.

## Project layout

Every module carries a `//!` doc comment pointing at the section of
`docs/ARCHITECTURE.md` it implements; that file is the place to start.

```
src/
├── main.rs            CLI definition (clap) and command dispatch
├── lib.rs             Module tree and the crate-level overview
├── config.rs          Locate and parse the config that points at the data repo
├── init.rs            `husmo init`: bootstrap a data repo
├── document.rs        The Document model shared by every other module
├── store.rs           Read and write Markdown+frontmatter files on disk
├── git_sync.rs        The pull -> write -> commit -> push wrapper
├── mcp_server.rs      The MCP server over stdio, and its tool surface
├── resources.rs       Pure logic behind `resources/list` and `resources/read`
├── save.rs            Ingestion dispatch for the `save` tool
├── url_ingest.rs pasted_text.rs local_file.rs   The three ingestion paths
├── fetch.rs extract.rs images.rs   Turn a fetched page into Markdown
├── chunk.rs embed.rs embeddings.rs vector_index.rs   The embedding pipeline
├── semantic_search.rs fulltext_search.rs tag_search.rs   Retrieval
├── related.rs         The symmetric Related graph
├── archive.rs delete.rs   Archiving an outgoing link, and removal
└── test_support.rs    Fixtures shared across the modules' `#[cfg(test)]` suites
```

Unit tests are co-located in each module. `tests/end_to_end_smoke_test.rs`
drives one full save -> discover -> archive -> relate -> get -> search ->
delete flow against a temporary data repo, and `tests/node/` holds the
installer suite.

Decisions that a future reader would find surprising live in `docs/adr/`, and
`CONTEXT.md` is the glossary for the capitalized domain terms the code and
docs use.

## Making a release

Run the script. It takes an exact version or a `patch` / `minor` / `major`
bump:

```bash
script/release patch
```

It writes the new version to `Cargo.toml`, `Cargo.lock`, `package.json`,
`claude-plugin/.claude-plugin/plugin.json` and the console sample in
`README.md`, runs both test suites, and commits on a `release/vX.Y.Z` branch.
It refuses to run off `main`, on a dirty tree, when `main` and `origin/main`
have diverged, or when the tag already exists. `script/release --check`
reports whether the five files already agree.

Then open the PR, merge it, and push the tag:

```bash
gh pr create --fill
gh pr merge --squash --delete-branch
git checkout main && git pull
git tag vX.Y.Z && git push origin vX.Y.Z
```

Pushing the tag is what publishes, which is why the script stops before it.

`release.yml` opens with a `verify` job that asserts the tag and all five
version files read the same version, then runs the installer suite. Every
later job depends on it, so a version left behind stops the release before a
single artifact is built. Once `verify` passes, the workflow builds binaries
for macOS aarch64 and Linux x86_64/aarch64, publishes the GitHub release,
updates the Homebrew tap formula, and publishes `@frantufro/husmo` to npm.

The npm publish uses OIDC trusted publishing, so the repository holds no npm
credential. The trust configuration lives at
<https://www.npmjs.com/package/@frantufro/husmo/access> and names organization
`frantufro`, repository `husmo`, workflow `release.yml`, with the environment
field left empty.

The Claude Code plugin is distributed through the marketplace at
<https://github.com/frantufro/claude-plugins>, whose `marketplace.json` pins a
version per plugin. Bump husmo's entry there after a release, otherwise the
marketplace keeps serving the previous tag.
