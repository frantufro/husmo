---
created: 2026-08-21
---

# Chunking and local embeddings

Implement paragraph/section-based chunking of Document content and local, in-process embedding generation (e.g. via candle) with zero network calls -- this is a stated project value, not an optimization. Store each Document's chunk embeddings as small per-Document sidecar files (not one shared blob). TDD: chunking boundaries on fixture content, embedding determinism/shape, sidecar file format round-trip.
