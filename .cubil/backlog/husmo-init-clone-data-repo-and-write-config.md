---
created: 2026-08-21
---

# husmo init: clone data repo and write config

Add a CLI subcommand `husmo init` (not an MCP tool) that: prompts interactively for the data repo's git URL (also accepts --repo <url> for non-interactive/scripted use), clones it into the current directory, and writes/updates the config file (from the scaffold task) to point at the cloned path -- so the app has a working data repo location immediately after init completes. See docs/ARCHITECTURE.md, 'Bootstrapping a data repo: husmo init'. TDD: init with a fake/local git repo fixture clones it correctly and the resulting config file resolves to the right path; init with --repo skips the interactive prompt; re-running init against an existing folder is handled sanely (clear error or no-op, not silent data loss).
