---
created: 2026-08-21
---

# Replace hash-based embeddings with a real local sentence-embedding model

Supersedes the embedding-generation part of chunking-and-local-embeddings. The implementer there shipped a deterministic feature-hashing bag-of-words vector (src/embed.rs), reasoning that even a one-time model-weights download violated 'zero network calls' per ADR 0001. That reasoning is rejected: a one-time local model fetch at setup/first-run is fine, the constraint is zero network calls per save/search operation. Replace embed::embed's implementation with real inference via candle plus a small pre-trained sentence-embedding model (e.g. all-MiniLM-L6-v2), with weights fetched once and cached locally (not re-fetched on every run). Update EMBEDDING_DIM and any code/tests that assumed the old 256-dim hash vectors. Update the vector-index and semantic-search code from in-process-vector-index-and-semantic-search to work against the new vectors (the sidecar file format itself does not need to change). TDD: the model loads and produces embeddings where semantically similar sentences (paraphrases/synonyms) score higher cosine similarity than unrelated ones -- a property the hash-based version could not satisfy -- plus determinism and shape tests. See docs/ARCHITECTURE.md 'Retrieval' section for the corrected policy.
