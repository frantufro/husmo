---
created: 2026-08-21
---

# Git-backed persistence wrapper

Implement the git pull -> write -> commit -> push wrapper (via git2) that will wrap every mutating operation on the data repo, per docs/ARCHITECTURE.md. Single machine for now; pull-before-write is a safety habit, not full conflict resolution. TDD against a throwaway local git repo fixture: commit produced on write, pull attempted before write, push attempted after commit.
