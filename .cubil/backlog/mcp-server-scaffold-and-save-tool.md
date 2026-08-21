---
created: 2026-08-21
---

# MCP server scaffold and save tool

Scaffold the MCP server over stdio transport (spawned per session, no persistent daemon). Implement the save tool: dispatches across URL/pasted-text/local-file ingestion, chunks and embeds the result, runs the git pull/commit/push cycle, and returns the saved Document plus any discovered outgoing links. TDD: save via each of the three input kinds end-to-end, re-saving the same canonical_url overwrites in place rather than duplicating.
