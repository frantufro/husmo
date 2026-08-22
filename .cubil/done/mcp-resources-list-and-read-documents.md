---
created: 2026-08-21
---

# MCP resources: list and read Documents

Expose Documents as MCP resources (resources/list + resources/read) alongside the existing list/get tools, per docs/adr/0002-mcp-resources-alongside-tools-for-document-browsing.md. Resource URI keyed by slug (retitling can break an old @ reference — accepted, no alias tracking). resources/read returns the same shape as get: raw Markdown+frontmatter, Related Documents listed by reference only, never inlined. Resource name = title, description = summary when present else a content snippet. resources/list is paginated via cursor from the start.
