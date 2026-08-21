---
created: 2026-08-21
---

# Local file ingestion (text and PDF)

Implement an extensible per-file-type extraction pipeline dispatched by file type. Support plain text and PDF for now; the interface must allow adding more formats later without changing callers. TDD: text file ingestion, PDF text extraction ingestion, unsupported type produces a clear error.
