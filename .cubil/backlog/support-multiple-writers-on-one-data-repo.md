---
created: 2026-10-29
---

# Support multiple writers on one data repo

Several writers will share one data repo: Fran's Mac and every Hermes Agent (agent-provisioning), each with its own clone of husmo-data and its own Forgejo user pushing to `main`. husmo assumes a single machine today (`docs/ARCHITECTURE.md`, "Git mechanics"; `src/git_sync.rs`).

## What breaks with several writers

- **Push race.** Two writers save within seconds of each other. The second push is rejected (`SyncError::PushRejected`) and its commit stays local. Every later `sync_write` then fails in `pull` with "local and remote history have diverged", so that clone can't write again until someone repairs it by hand.
- **Stale reads.** Only mutating operations pull. `get`, `list` and the three searches read the local clone, so a writer never sees the others' Documents until it saves something itself.
- **Stale indexes.** The vector index and the full-text and tag caches are built at server startup. Documents that arrive with a pull stay out of search until the MCP server restarts.

## What to build

1. On a rejected push or a diverged pull, rebase the local commits onto `origin/<branch>` and retry the push, a bounded number of times. Report an error only when the rebase conflicts.
2. Decide how conflicts resolve. Documents are separate files named by UUID, so most saves never touch the same file. `relate` and `unrelate` rewrite the `related` list of two Documents, and two writers can edit the same list. A union merge of `related` (or a re-applied operation after a fresh pull) fits the symmetric edge model.
3. Pull before reads too, rate-limited (for example at most once a minute), or on a timer in the MCP server.
4. After any pull that brings new commits, refresh the indexes for the changed Documents (their sidecar embeddings are in the repo) without a restart.
5. Record who wrote what: commits already carry the clone's git identity. Check whether a Document's `author` should default to it.
6. Update `docs/ARCHITECTURE.md` ("Git mechanics") and the module comment of `src/git_sync.rs`, and add tests with two clones of one bare remote racing a save.

## Done when

Two clones of the same bare repo save, relate and delete concurrently without either getting stuck, and each one finds the other's Documents through search without a restart. A release ships it, and agent-provisioning's `25-tools.sh` picks it up on the next `update`.
