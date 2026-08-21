---
created: 2026-08-21
---

# relate, unrelate, list, delete tools

Implement the remaining MCP tools: relate, unrelate, list (browse all Documents), and delete (goes through the same git pull/commit/push cycle as save; recoverable via git history). TDD: delete removes the Document from current state but a prior git commit still contains it; list returns all Documents; relate/unrelate wire directly to the task-11 implementation.
