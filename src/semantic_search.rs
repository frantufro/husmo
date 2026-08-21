//! Document-level semantic search, per `docs/ARCHITECTURE.md` ("Retrieval",
//! "MCP server" `search-semantic`): finds the Documents whose content best
//! matches a query's meaning, built on top of [`crate::vector_index`]'s
//! chunk-level search. Each Document contributes at most one hit, scored
//! by its single best-matching chunk.
//!
//! A hit's Related documents (see `docs/ARCHITECTURE.md`, "Related") are
//! always visible by reference through `hit.document.related` (ids); their
//! full content is only pulled into `hit.expanded_related` when the caller
//! opts into expansion, per "Retrieval": "a search call explicitly opts
//! into expansion."

use std::collections::HashSet;

use crate::document::Document;
use crate::embed::EmbedError;
use crate::vector_index::VectorIndex;

/// One Document that matched a [`semantic_search`] query.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticSearchHit {
    /// The matching Document. Its own `related` field always lists the
    /// ids of the Documents it's Related to, whether or not expansion was
    /// requested.
    pub document: Document,
    /// This Document's best-matching chunk's cosine similarity to the
    /// query, in `[-1, 1]`.
    pub score: f32,
    /// The text of the chunk that produced `score`.
    pub matched_chunk: String,
    /// The full content of every Document `document` is Related to,
    /// resolved from the candidate pool passed to [`semantic_search`].
    /// Populated only when that call's `expand_related` was `true`;
    /// empty otherwise, even if `document.related` is non-empty.
    pub expanded_related: Vec<Document>,
}

/// Searches `index` for the `top_k` Documents (drawn from `documents`)
/// whose content best matches `query`, most similar first.
///
/// `documents` is both the pool a hit's id resolves against and the pool
/// `expand_related` resolves each hit's Related ids against — typically
/// every Document currently in the data repo.
///
/// When `expand_related` is `true`, each hit's `expanded_related` holds
/// the full content of every Document referenced in its `document.related`
/// that's present in `documents`; when `false`, `expanded_related` is
/// always empty.
///
/// # Errors
///
/// Returns an error if `query` can't be embedded (see
/// [`crate::vector_index::VectorIndex::search`]).
pub fn semantic_search(
    index: &VectorIndex,
    documents: &[Document],
    query: &str,
    top_k: usize,
    expand_related: bool,
) -> Result<Vec<SemanticSearchHit>, EmbedError> {
    let mut seen_documents = HashSet::new();
    let mut hits = Vec::new();

    // `index.search` sorts every chunk by descending score, so the first
    // time a document_id appears in that order is necessarily its
    // best-scoring chunk, and dedup-by-first-occurrence yields documents
    // already in descending order of their own best score.
    for chunk_hit in index.search(query, index.len())? {
        if hits.len() >= top_k {
            break;
        }
        if !seen_documents.insert(chunk_hit.document_id.clone()) {
            continue;
        }
        let Some(document) = documents.iter().find(|doc| doc.id == chunk_hit.document_id) else {
            continue;
        };
        let expanded_related = if expand_related {
            resolve_related(document, documents)
        } else {
            Vec::new()
        };
        hits.push(SemanticSearchHit {
            document: document.clone(),
            score: chunk_hit.score,
            matched_chunk: chunk_hit.chunk,
            expanded_related,
        });
    }

    Ok(hits)
}

/// Resolves every id in `document.related` to its full Document, looking
/// it up in `documents`. A related id with no matching Document in
/// `documents` is silently skipped.
fn resolve_related(document: &Document, documents: &[Document]) -> Vec<Document> {
    document
        .related
        .iter()
        .filter_map(|related_id| documents.iter().find(|candidate| &candidate.id == related_id))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embeddings::DocumentEmbeddings;

    #[test]
    fn semantic_search_returns_one_hit_per_document_scored_by_its_best_chunk() {
        let rust_doc = Document::new(
            "Rust",
            "Intro paragraph.\n\nRust is a systems programming language with strong static typing.",
        );
        let baking_doc =
            Document::new("Bread", "Bake the sourdough loaf for forty minutes at high heat.");
        let documents = vec![rust_doc.clone(), baking_doc.clone()];
        let index = VectorIndex::build(&[
            DocumentEmbeddings::build(&rust_doc).expect("build should succeed"),
            DocumentEmbeddings::build(&baking_doc).expect("build should succeed"),
        ]);

        let hits = semantic_search(
            &index,
            &documents,
            "systems programming in a strongly typed language",
            2,
            false,
        )
        .expect("semantic_search should succeed");

        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].document.id, rust_doc.id);
        assert_eq!(
            hits[0].matched_chunk,
            "Rust is a systems programming language with strong static typing."
        );
        assert_eq!(hits[1].document.id, baking_doc.id);
    }

    #[test]
    fn semantic_search_respects_top_k_at_the_document_level() {
        // Many chunks from one document should not crowd out a second
        // document's single best chunk.
        let rust_doc = Document::new(
            "Rust",
            "Rust is great.\n\nRust is fast.\n\nRust is safe.\n\nRust is fun.",
        );
        let baking_doc = Document::new("Bread", "Sourdough baking is a slow art.");
        let documents = vec![rust_doc.clone(), baking_doc.clone()];
        let index = VectorIndex::build(&[
            DocumentEmbeddings::build(&rust_doc).expect("build should succeed"),
            DocumentEmbeddings::build(&baking_doc).expect("build should succeed"),
        ]);

        let hits = semantic_search(&index, &documents, "rust", 2, false)
            .expect("semantic_search should succeed");

        assert_eq!(hits.len(), 2);
        let document_ids: HashSet<_> = hits.iter().map(|hit| hit.document.id.clone()).collect();
        assert_eq!(
            document_ids,
            HashSet::from([rust_doc.id.clone(), baking_doc.id.clone()])
        );
    }

    #[test]
    fn expand_related_false_leaves_expanded_related_empty() {
        let mut main_doc = Document::new("Main", "Rust systems programming content.");
        let related_doc = Document::new("Related", "More Rust content, Related to Main.");
        main_doc.related = vec![related_doc.id.clone()];
        let documents = vec![main_doc.clone(), related_doc.clone()];
        let index = VectorIndex::build(&[
            DocumentEmbeddings::build(&main_doc).expect("build should succeed"),
            DocumentEmbeddings::build(&related_doc).expect("build should succeed"),
        ]);

        let hits = semantic_search(&index, &documents, "rust systems programming", 1, false)
            .expect("semantic_search should succeed");

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].document.id, main_doc.id);
        assert_eq!(hits[0].document.related, vec![related_doc.id.clone()]);
        assert!(hits[0].expanded_related.is_empty());
    }

    #[test]
    fn expand_related_true_pulls_in_the_full_content_of_related_documents() {
        let mut main_doc = Document::new("Main", "Rust systems programming content.");
        let related_doc = Document::new("Related", "More Rust content, Related to Main.");
        main_doc.related = vec![related_doc.id.clone()];
        let documents = vec![main_doc.clone(), related_doc.clone()];
        let index = VectorIndex::build(&[
            DocumentEmbeddings::build(&main_doc).expect("build should succeed"),
            DocumentEmbeddings::build(&related_doc).expect("build should succeed"),
        ]);

        let hits = semantic_search(&index, &documents, "rust systems programming", 1, true)
            .expect("semantic_search should succeed");

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].expanded_related, vec![related_doc]);
    }

    #[test]
    fn expand_related_skips_a_related_id_missing_from_the_document_pool() {
        let mut main_doc = Document::new("Main", "Rust systems programming content.");
        main_doc.related = vec!["missing-id".to_string()];
        let documents = vec![main_doc.clone()];
        let index =
            VectorIndex::build(&[DocumentEmbeddings::build(&main_doc).expect("build should succeed")]);

        let hits = semantic_search(&index, &documents, "rust systems programming", 1, true)
            .expect("semantic_search should succeed");

        assert_eq!(hits.len(), 1);
        assert!(hits[0].expanded_related.is_empty());
    }
}
