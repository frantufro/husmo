//! An in-process vector index over every Document's chunk embeddings, per
//! `docs/ARCHITECTURE.md` ("Retrieval"): "The assembled searchable vector
//! index ... is not committed. It's a local, disposable, gitignored cache
//! rebuilt in memory at server startup from the committed sidecar files."
//!
//! [`VectorIndex::build`] assembles the index from in-memory
//! [`DocumentEmbeddings`] (easy to drive from fixtures in tests);
//! [`build_from_dir`] is the startup path that first loads every committed
//! sidecar file from a directory (via [`crate::embeddings::load_all`]) and
//! builds the index from those.

use std::path::Path;

use crate::embed::{EmbedError, embed};
use crate::embeddings::{DocumentEmbeddings, EmbeddingsError};

/// One chunk's embedding, tagged with the Document it came from.
#[derive(Debug, Clone, PartialEq)]
struct IndexedChunk {
    document_id: String,
    chunk: String,
    vector: Vec<f32>,
}

/// An in-memory index over every chunk embedding across every Document.
///
/// Search here is a brute-force linear scan over every chunk's cosine
/// similarity to the query: simple, dependency-free, and fast enough for
/// the corpus sizes a single local git-backed document store holds. If
/// that stops being true, this is the seam to swap in an ANN structure
/// (e.g. `usearch` or `instant-distance`) without changing callers.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct VectorIndex {
    chunks: Vec<IndexedChunk>,
}

impl VectorIndex {
    /// Builds an index over every chunk in `embeddings`.
    #[must_use]
    pub fn build(embeddings: &[DocumentEmbeddings]) -> Self {
        let chunks = embeddings
            .iter()
            .flat_map(|document| {
                document
                    .chunks
                    .iter()
                    .map(move |chunk_embedding| IndexedChunk {
                        document_id: document.document_id.clone(),
                        chunk: chunk_embedding.chunk.clone(),
                        vector: chunk_embedding.vector.clone(),
                    })
            })
            .collect();
        VectorIndex { chunks }
    }

    /// Number of chunks currently indexed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    /// True when the index holds no chunks at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// Returns the `top_k` chunks in the index most similar to `query`, by
    /// cosine similarity, most similar first. Returns fewer than `top_k`
    /// if the index doesn't hold that many chunks.
    ///
    /// # Errors
    ///
    /// Returns an error if `query` can't be embedded (see
    /// [`crate::embed::embed`]).
    pub fn search(&self, query: &str, top_k: usize) -> Result<Vec<SearchHit>, EmbedError> {
        let query_vector = embed(query)?;
        let mut hits: Vec<SearchHit> = self
            .chunks
            .iter()
            .map(|indexed| SearchHit {
                document_id: indexed.document_id.clone(),
                chunk: indexed.chunk.clone(),
                score: cosine_similarity(&query_vector, &indexed.vector),
            })
            .collect();
        hits.sort_by(|a, b| b.score.total_cmp(&a.score));
        hits.truncate(top_k);
        Ok(hits)
    }
}

/// One chunk that matched a [`VectorIndex::search`] query.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    /// The id of the Document this chunk belongs to.
    pub document_id: String,
    /// The matching chunk's own text.
    pub chunk: String,
    /// Cosine similarity between the query and this chunk, in `[-1, 1]`.
    pub score: f32,
}

/// Builds a [`VectorIndex`] by loading every committed chunk-embedding
/// sidecar file directly inside `dir` (see
/// [`crate::embeddings::load_all`]) — the startup path described in
/// `docs/ARCHITECTURE.md` ("Retrieval").
///
/// # Errors
///
/// Returns an error if `dir` can't be listed or one of its sidecar files
/// can't be read or parsed.
pub fn build_from_dir(dir: &Path) -> Result<VectorIndex, EmbeddingsError> {
    let embeddings = crate::embeddings::load_all(dir)?;
    Ok(VectorIndex::build(&embeddings))
}

/// Cosine similarity between two equal-length vectors. Every vector
/// [`crate::embed::embed`] produces is already L2-normalized, so this
/// reduces to a plain dot product.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embeddings::ChunkEmbedding;

    fn fixture(document_id: &str, chunks: &[&str]) -> DocumentEmbeddings {
        DocumentEmbeddings {
            document_id: document_id.to_string(),
            chunks: chunks
                .iter()
                .map(|text| ChunkEmbedding {
                    chunk: text.to_string(),
                    vector: embed(text).expect("embed should succeed"),
                })
                .collect(),
        }
    }

    #[test]
    fn search_ranks_the_most_similar_chunk_first() {
        let embeddings = vec![
            fixture(
                "rust-doc",
                &["Rust is a systems programming language with strong static typing."],
            ),
            fixture(
                "baking-doc",
                &["Bake the sourdough loaf for forty minutes at high heat."],
            ),
        ];
        let index = VectorIndex::build(&embeddings);

        let hits = index
            .search("systems programming in a strongly typed language", 2)
            .expect("search should succeed");

        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].document_id, "rust-doc");
        assert_eq!(hits[1].document_id, "baking-doc");
        assert!(
            hits[0].score > hits[1].score,
            "expected the Rust chunk ({}) to score higher than the baking chunk ({})",
            hits[0].score,
            hits[1].score
        );
    }

    #[test]
    fn search_truncates_to_top_k() {
        let embeddings = vec![fixture("doc", &["alpha", "beta", "gamma"])];
        let index = VectorIndex::build(&embeddings);

        let hits = index.search("alpha", 1).expect("search should succeed");

        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn search_returns_fewer_than_top_k_when_the_index_holds_less() {
        let embeddings = vec![fixture("doc", &["only chunk"])];
        let index = VectorIndex::build(&embeddings);

        let hits = index
            .search("only chunk", 5)
            .expect("search should succeed");

        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn build_indexes_every_chunk_across_every_document() {
        let embeddings = vec![fixture("one", &["a", "b"]), fixture("two", &["c"])];

        let index = VectorIndex::build(&embeddings);

        assert_eq!(index.len(), 3);
        assert!(!index.is_empty());
    }

    #[test]
    fn build_from_dir_loads_committed_sidecar_fixture_files() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let rust_doc = crate::document::Document::new(
            "Rust",
            "Rust is a systems programming language with strong static typing.",
        );
        let baking_doc = crate::document::Document::new(
            "Bread",
            "Bake the sourdough loaf for forty minutes at high heat.",
        );
        crate::embeddings::write(
            dir.path(),
            &rust_doc.slug,
            &DocumentEmbeddings::build(&rust_doc).expect("build should succeed"),
        )
        .expect("write should succeed");
        crate::embeddings::write(
            dir.path(),
            &baking_doc.slug,
            &DocumentEmbeddings::build(&baking_doc).expect("build should succeed"),
        )
        .expect("write should succeed");

        let index = build_from_dir(dir.path()).expect("build_from_dir should succeed");
        let hits = index
            .search("systems programming in a strongly typed language", 1)
            .expect("search should succeed");

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].document_id, rust_doc.id);
    }
}
