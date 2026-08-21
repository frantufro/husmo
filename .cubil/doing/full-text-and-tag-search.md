---
created: 2026-08-21
---

# Full-text and tag search

Implement full-text/keyword search and tag-filter search as distinct capabilities from semantic search (per docs/ARCHITECTURE.md, these must not collapse into one fuzzy search). A local disposable cache (e.g. SQLite FTS5) rebuilt at startup is fine; it must not be committed to git. TDD: exact string match found by full-text search but plausibly missed by semantic search framing, tag filter correctness including documents with zero/multiple tags.
