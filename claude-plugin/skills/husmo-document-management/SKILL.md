---
name: husmo-document-management
description: >
  Triage outgoing links discovered when husmo saves a URL. Use this skill
  whenever the husmo MCP server's `save` tool returns outgoing links, to
  decide with the user which ones are worth archiving. TRIGGER when: a
  husmo `save` result includes outgoing links, or the user mentions
  archiving/following links found on a saved page.
---

# husmo Document Management

husmo is a local-first, git-backed document/link database: it saves URLs,
pasted text, and local files as Documents, and lets you search, tag, and
relate them later. Its MCP tools (`save`, `get`, `search-*`, `relate`,
`unrelate`, `list`, `delete`) are self-descriptive through MCP itself — no
need to duplicate their usage here. The one behavior that needs explicit
guidance is what to do with outgoing links.

## Outgoing Link Triage

`save` returns any outgoing links discovered in the saved page's content,
as data only. **Never archive any of them automatically.**

1. Present the discovered links (title/URL) to the user.
2. Ask which ones are worth archiving. Skip the question only if the user
   already said up front which links they want (e.g. "save this and
   archive anything about X").
3. For each link the user picks, call `save` again with `url` set to that
   link's URL — no separate "archive" tool exists; a plain `save` on the
   link runs the same pipeline.
4. Stop there. Don't inspect the newly-archived Document's own outgoing
   links and re-offer to archive *those* without being asked again —
   archiving is one level deep, never recursive.

Archiving a link and declaring it Related are separate, independent steps.
Archiving never implies Related — if the user wants the newly archived
Document connected to the one it came from, that's a distinct `relate`
call they need to ask for, not something to assume.
