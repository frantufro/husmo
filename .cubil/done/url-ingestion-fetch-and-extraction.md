---
created: 2026-08-21
---

# URL ingestion: fetch and extraction

Implement URL fetch via reqwest (plain HTTP, no headless browser/JS rendering) and readability-style extraction to Markdown. Preserve outgoing hyperlinks as links (do not strip). Return the set of discovered outgoing links as data (do not auto-follow). TDD against fixture HTML: extraction quality, link preservation, outgoing-link collection.
