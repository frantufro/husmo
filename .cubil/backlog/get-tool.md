---
created: 2026-08-21
---

# get tool

Implement the get tool: accepts exactly one of id/slug/url as named optional parameters, server-side validated as exactly-one-of; returns the Document including its Related list by reference. TDD: lookup by each of the three identifiers resolves the same Document, zero or multiple identifiers supplied is a validation error.
