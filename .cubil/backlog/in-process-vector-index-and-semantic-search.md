---
created: 2026-08-21
---

# In-process vector index and semantic search

Build an in-process vector index (e.g. usearch or instant-distance) in memory at startup by loading all committed per-Document embedding sidecar files -- this index itself is never committed to git (gitignored, disposable). Implement the semantic-search capability over it, with an opt-in flag to expand results into the full content of a hit's Related documents. TDD: index build from fixture sidecar files, top-k semantic search correctness on fixture embeddings, expansion flag behavior.
