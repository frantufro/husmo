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
- [✓] scaffold-husmo-rust-project — Scaffold husmo Rust project
- [✓] document-model-and-file-i-o — Document model and file I/O
- [✓] husmo-init-clone-data-repo-and-write-config — husmo init: clone data repo and write config
- [✓] git-backed-persistence-wrapper — Git-backed persistence wrapper
- [✓] url-ingestion-fetch-and-extraction — URL ingestion: fetch and extraction
- [✓] image-handling-during-extraction — Image handling during extraction
- [✓] local-file-ingestion-text-and-pdf — Local file ingestion (text and PDF)
- [✓] pasted-text-ingestion — Pasted-text ingestion

## Milestone: Retrieval & Relationships
- [✓] chunking-and-local-embeddings — Chunking and local embeddings
- [✓] in-process-vector-index-and-semantic-search — In-process vector index and semantic search
- [✓] full-text-and-tag-search — Full-text and tag search
- [✓] related-graph-relate-and-unrelate — Related graph: relate and unrelate
- [✓] outgoing-link-archiving-flow — Outgoing-link archiving flow
- [✓] replace-hash-based-embeddings-with-a-real-local-sentence-embedding-model — Replace hash-based embeddings with a real local sentence-embedding model

## Milestone: MCP Server & Polish
- [✓] mcp-server-scaffold-and-save-tool — MCP server scaffold and save tool
- [✓] get-tool — get tool
- [✓] search-semantic-search-tag-search-fulltext-tools — search-semantic, search-tag, search-fulltext tools
- [✓] relate-unrelate-list-delete-tools — relate, unrelate, list, delete tools
- [✓] mcp-resources-list-and-read-documents — MCP resources: list and read Documents
- [✓] end-to-end-smoke-test-and-readme-finalization — End-to-end smoke test and README finalization
