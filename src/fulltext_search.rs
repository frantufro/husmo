//! Full-text/keyword search, per `docs/ARCHITECTURE.md` ("Retrieval",
//! `search-fulltext`): "exact string matches semantic search can miss."
//!
//! This is a literal, case-insensitive substring search over a Document's
//! title and content — deliberately distinct from
//! [`crate::semantic_search`], which scores chunks by cosine similarity
//! between whole-token bag-of-words vectors (see [`crate::embed`]). A query
//! that's a substring of a larger token (e.g. `"cern"` inside
//! `"concerned"`) or that shares no vocabulary with a chunk it nonetheless
//! appears in verbatim is exactly the case semantic search can plausibly
//! miss and this module still finds.

use crate::document::Document;

/// One Document that matched a [`fulltext_search`] query.
#[derive(Debug, Clone, PartialEq)]
pub struct FullTextSearchHit {
    /// The matching Document.
    pub document: Document,
    /// How many times `query` occurs (case-insensitively) across the
    /// Document's title and content combined. Always at least 1.
    pub match_count: usize,
}

/// Searches `documents` for every Document whose title or content contains
/// `query` as a case-insensitive literal substring, most occurrences first.
/// Ties preserve `documents`' original relative order.
///
/// An empty (or all-whitespace) `query` matches nothing — there is no
/// meaningful literal substring to search for.
#[must_use]
pub fn fulltext_search(documents: &[Document], query: &str) -> Vec<FullTextSearchHit> {
    if query.trim().is_empty() {
        return Vec::new();
    }

    let mut hits: Vec<FullTextSearchHit> = documents
        .iter()
        .filter_map(|document| {
            let match_count =
                count_occurrences(&document.title, query) + count_occurrences(&document.content, query);
            (match_count > 0).then(|| FullTextSearchHit {
                document: document.clone(),
                match_count,
            })
        })
        .collect();

    hits.sort_by(|a, b| b.match_count.cmp(&a.match_count));
    hits
}

/// Counts non-overlapping, case-insensitive occurrences of `needle` in
/// `haystack`.
fn count_occurrences(haystack: &str, needle: &str) -> usize {
    haystack.to_lowercase().matches(&needle.to_lowercase()).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fulltext_search_finds_an_exact_substring_inside_a_larger_word() {
        // "cern" only ever appears here as part of "concerned". A
        // whole-token, bag-of-words semantic search (see
        // `crate::embed::tokenize`) treats "concerned" as one atomic token
        // and would need the literal token "cern" to appear on its own to
        // register any similarity at all, so it can plausibly miss this.
        // Full-text/keyword search matches it directly as a literal
        // substring.
        let matching = Document::new("Report", "Several reviewers were concerned about the plan.");
        let unrelated = Document::new("Unrelated", "Bake the sourdough loaf for forty minutes.");
        let documents = vec![matching.clone(), unrelated];

        let hits = fulltext_search(&documents, "cern");

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].document.id, matching.id);
    }

    #[test]
    fn fulltext_search_is_case_insensitive() {
        let doc = Document::new("Title", "Rust is great.");

        let hits = fulltext_search(std::slice::from_ref(&doc), "RUST");

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].document.id, doc.id);
    }

    #[test]
    fn fulltext_search_matches_against_the_title_too() {
        let doc = Document::new("A Rust Retrospective", "Some unrelated body text.");

        let hits = fulltext_search(std::slice::from_ref(&doc), "retrospective");

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].document.id, doc.id);
    }

    #[test]
    fn fulltext_search_returns_empty_when_no_document_matches() {
        let doc = Document::new("Title", "Some content.");

        let hits = fulltext_search(&[doc], "nonexistent-phrase");

        assert!(hits.is_empty());
    }

    #[test]
    fn fulltext_search_returns_empty_for_an_empty_query() {
        let doc = Document::new("Title", "Some content.");

        let hits = fulltext_search(&[doc], "   ");

        assert!(hits.is_empty());
    }

    #[test]
    fn fulltext_search_ranks_more_occurrences_first() {
        let few = Document::new("Few", "rust appears once here.");
        let many = Document::new("Many", "rust rust rust appears three times here.");
        let documents = vec![few.clone(), many.clone()];

        let hits = fulltext_search(&documents, "rust");

        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].document.id, many.id);
        assert_eq!(hits[0].match_count, 3);
        assert_eq!(hits[1].document.id, few.id);
        assert_eq!(hits[1].match_count, 1);
    }
}
