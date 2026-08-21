---
created: 2026-08-21
---

# Scaffold husmo Rust project

Set up the Cargo project (package name `husmo`), module layout, and a config file loader that locates and parses a config pointing at the data repo path. Write the README stating the local-first/self-contained value proposition explicitly (no external services, no external APIs, works offline) per docs/adr/0001-local-first-no-external-services.md. See docs/ARCHITECTURE.md for full context. TDD: start with a test for config loading (missing file, malformed file, valid file) before writing the loader.
