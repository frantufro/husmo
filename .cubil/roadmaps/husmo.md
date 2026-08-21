# husmo

Local-first, git-backed link/document database with a Rust MCP server.

Full architecture context: docs/ARCHITECTURE.md
Domain glossary: CONTEXT.md
Key ADR: docs/adr/0001-local-first-no-external-services.md

Build order matters: this is a single evolving codebase and data model, not
independent features. Each task assumes prior tasks in the roadmap are done.
Every task should be implemented via TDD (red, green, refactor), reviewed,
and fixed before being marked done.

## Milestone: Foundation & Ingestion
- [ ] scaffold-husmo-rust-project
- [ ] document-model-and-file-i-o
- [ ] git-backed-persistence-wrapper
- [ ] url-ingestion-fetch-and-extraction
- [ ] image-handling-during-extraction
- [ ] local-file-ingestion-text-and-pdf
- [ ] pasted-text-ingestion

## Milestone: Retrieval & Relationships
- [ ] chunking-and-local-embeddings
- [ ] in-process-vector-index-and-semantic-search
- [ ] full-text-and-tag-search
- [ ] related-graph-relate-and-unrelate
- [ ] outgoing-link-archiving-flow

## Milestone: MCP Server & Polish
- [ ] mcp-server-scaffold-and-save-tool
- [ ] get-tool
- [ ] search-semantic-search-tag-search-fulltext-tools
- [ ] relate-unrelate-list-delete-tools
- [ ] end-to-end-smoke-test-and-readme-finalization
