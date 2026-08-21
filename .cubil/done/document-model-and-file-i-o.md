---
created: 2026-08-21
---

# Document model and file I/O

Define the Document struct per docs/ARCHITECTURE.md (id, slug, canonical_url, title, tags, saved_at, summary, author, content, related). Implement stable id generation, slug derivation from title with collision handling, and Markdown+frontmatter serialization/deserialization. Implement resolving a Document by exactly one of id/slug/url. TDD: round-trip save/load tests, slug collision test, ambiguous/zero-identifier resolution test.
