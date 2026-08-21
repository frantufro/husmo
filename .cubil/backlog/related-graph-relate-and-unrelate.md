---
created: 2026-08-21
---

# Related graph: relate and unrelate

Implement relate/unrelate as a symmetric, untyped edge between two existing Documents, distinct from outgoing links. Every Document retrieval (get and search) must always list Related documents by reference (id/title) regardless of the expansion flag. TDD: relate is symmetric (A related to B implies B related to A), unrelate removes both directions, relating a nonexistent Document errors, retrieval always includes the Related list.
