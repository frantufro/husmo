//! Tag-filter search, per `docs/ARCHITECTURE.md` ("Retrieval",
//! `search-tag`): a distinct retrieval capability from full-text and
//! semantic search, filtering Documents by an exact `tags` membership
//! check rather than any matching of `title`/`content`.

use crate::document::Document;

/// Returns every Document in `documents` whose `tags` contains `tag`
/// exactly, preserving `documents`' original relative order.
///
/// A Document with zero tags never matches. A Document with multiple tags
/// matches as long as one of them equals `tag`.
#[must_use]
pub fn tag_search(documents: &[Document], tag: &str) -> Vec<Document> {
    documents
        .iter()
        .filter(|document| document.tags.iter().any(|candidate| candidate == tag))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_search_finds_a_document_with_exactly_that_tag() {
        let mut doc = Document::new("Title", "content");
        doc.tags = vec!["rust".to_string()];

        let hits = tag_search(&[doc.clone()], "rust");

        assert_eq!(hits, vec![doc]);
    }

    #[test]
    fn tag_search_excludes_a_document_with_zero_tags() {
        let doc = Document::new("Title", "content");
        assert!(doc.tags.is_empty());

        let hits = tag_search(&[doc], "rust");

        assert!(hits.is_empty());
    }

    #[test]
    fn tag_search_matches_a_document_with_multiple_tags_when_one_of_them_matches() {
        let mut doc = Document::new("Title", "content");
        doc.tags = vec!["cooking".to_string(), "rust".to_string(), "notes".to_string()];

        let hits = tag_search(&[doc.clone()], "rust");

        assert_eq!(hits, vec![doc]);
    }

    #[test]
    fn tag_search_excludes_a_document_with_multiple_tags_none_of_which_match() {
        let mut doc = Document::new("Title", "content");
        doc.tags = vec!["cooking".to_string(), "notes".to_string()];

        let hits = tag_search(&[doc], "rust");

        assert!(hits.is_empty());
    }

    #[test]
    fn tag_search_returns_every_matching_document_in_original_order() {
        let mut first = Document::new("First", "content");
        first.tags = vec!["rust".to_string()];
        let mut second = Document::new("Second", "content");
        second.tags = vec!["cooking".to_string()];
        let mut third = Document::new("Third", "content");
        third.tags = vec!["rust".to_string(), "notes".to_string()];
        let documents = vec![first.clone(), second, third.clone()];

        let hits = tag_search(&documents, "rust");

        assert_eq!(hits, vec![first, third]);
    }

    #[test]
    fn tag_search_does_not_partially_match_a_tag() {
        let mut doc = Document::new("Title", "content");
        doc.tags = vec!["rustacean".to_string()];

        let hits = tag_search(&[doc], "rust");

        assert!(hits.is_empty());
    }
}
