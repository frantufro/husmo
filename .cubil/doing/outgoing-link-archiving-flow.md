---
created: 2026-08-21
---

# Outgoing-link archiving flow

Implement archiving a previously-discovered outgoing link as its own Document, one level deep per save with no automatic recursion. This is distinct from Related (see task on Related graph). TDD: archiving a discovered outgoing link creates a new Document with correct canonical_url, archiving does not itself trigger further link discovery beyond that new Document's own extraction.
