//! Pure logic backing the MCP `resources/list` and `resources/read`
//! interface, per `docs/adr/0002-mcp-resources-alongside-tools-for-document-browsing.md`:
//! the Document<->resource-URI mapping, the description shown in a listing,
//! and cursor-based pagination. Kept separate from `crate::mcp_server`,
//! which wires this onto the wire types the `rmcp` crate expects — the same
//! split `crate::tag_search`/`crate::fulltext_search` keep from their own
//! tools.

use crate::document::Document;

/// The URI scheme prefix used for a Document exposed as an MCP resource. A
/// resource's URI is `document://{slug}`, per `docs/ARCHITECTURE.md` ("MCP
/// resources": "A resource's URI identifies a Document by `slug`").
const URI_SCHEME_PREFIX: &str = "document://";

/// Builds the resource URI for the Document whose slug is `slug`.
#[must_use]
pub fn resource_uri(slug: &str) -> String {
    format!("{URI_SCHEME_PREFIX}{slug}")
}

/// Extracts the slug from a resource URI built by [`resource_uri`]. Returns
/// `None` if `uri` doesn't use the expected `document://` scheme.
#[must_use]
pub fn slug_from_uri(uri: &str) -> Option<&str> {
    uri.strip_prefix(URI_SCHEME_PREFIX)
}

/// The maximum number of characters from a Document's content used as the
/// fallback resource description when it has no `summary`.
const SNIPPET_MAX_CHARS: usize = 200;

/// The description shown for `document` in a `resources/list` result: its
/// `summary` when present, otherwise a short snippet of its `content`, per
/// `docs/adr/0002-mcp-resources-alongside-tools-for-document-browsing.md`
/// ("A resource's ... `description` is `summary` when present, falling
/// back to a short `content` snippet when it isn't").
#[must_use]
pub fn resource_description(document: &Document) -> String {
    match &document.summary {
        Some(summary) => summary.clone(),
        None => content_snippet(&document.content),
    }
}

/// Takes the first `SNIPPET_MAX_CHARS` characters of `content`, trimmed of
/// surrounding whitespace, appending an ellipsis if that cut it short.
fn content_snippet(content: &str) -> String {
    let trimmed = content.trim();
    let mut snippet: String = trimmed.chars().take(SNIPPET_MAX_CHARS).collect();
    if trimmed.chars().count() > SNIPPET_MAX_CHARS {
        snippet.push('…');
    }
    snippet
}

/// One page of Documents for a `resources/list` call.
#[derive(Debug, Clone, PartialEq)]
pub struct ResourcePage {
    /// This page's Documents, sorted by slug.
    pub documents: Vec<Document>,
    /// The cursor to pass back for the next page. `None` once the listing
    /// is exhausted.
    pub next_cursor: Option<String>,
}

/// The default number of Documents returned per `resources/list` page.
pub const DEFAULT_PAGE_SIZE: usize = 50;

/// Paginates `documents` for `resources/list`, per
/// `docs/adr/0002-mcp-resources-alongside-tools-for-document-browsing.md`:
/// "`resources/list` is paginated from day one, even though today's scale
/// doesn't need it."
///
/// Sorts `documents` by slug for a stable, deterministic order across pages,
/// then returns the `page_size` Documents whose slug sorts after `cursor`
/// (all of them, from the start, when `cursor` is `None`). A `cursor`
/// naming a slug no longer present (e.g. that Document was deleted between
/// calls) degrades gracefully: pagination simply resumes after where that
/// slug would have sorted, rather than erroring.
#[must_use]
pub fn paginate(mut documents: Vec<Document>, cursor: Option<&str>, page_size: usize) -> ResourcePage {
    documents.sort_by(|a, b| a.slug.cmp(&b.slug));
    let start = match cursor {
        Some(cursor) => documents.partition_point(|doc| doc.slug.as_str() <= cursor),
        None => 0,
    };
    let end = documents.len().min(start + page_size);
    let next_cursor = if end < documents.len() {
        Some(documents[end - 1].slug.clone())
    } else {
        None
    };
    ResourcePage {
        documents: documents[start..end].to_vec(),
        next_cursor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;

    #[test]
    fn resource_uri_and_slug_from_uri_round_trip() {
        let uri = resource_uri("my-title");

        assert_eq!(slug_from_uri(&uri), Some("my-title"));
    }

    #[test]
    fn slug_from_uri_rejects_a_uri_using_a_different_scheme() {
        assert_eq!(slug_from_uri("https://example.com/my-title"), None);
    }

    #[test]
    fn resource_description_uses_the_summary_when_present() {
        let mut document = Document::new("Title", "Some content that would otherwise be used.");
        document.summary = Some("A short summary.".to_string());

        assert_eq!(resource_description(&document), "A short summary.");
    }

    #[test]
    fn resource_description_falls_back_to_a_content_snippet_when_no_summary() {
        let document = Document::new("Title", "Some short content.");

        assert_eq!(resource_description(&document), "Some short content.");
    }

    #[test]
    fn resource_description_truncates_a_long_content_snippet() {
        let long_content = "a".repeat(SNIPPET_MAX_CHARS + 50);
        let document = Document::new("Title", long_content);

        let description = resource_description(&document);

        assert_eq!(description.chars().count(), SNIPPET_MAX_CHARS + 1);
        assert!(description.ends_with('…'));
    }

    fn doc_with_slug(slug: &str) -> Document {
        let mut document = Document::new(slug, "content");
        document.slug = slug.to_string();
        document
    }

    #[test]
    fn paginate_returns_everything_in_one_page_when_it_all_fits() {
        let documents = vec![doc_with_slug("b"), doc_with_slug("a")];

        let page = paginate(documents, None, 10);

        let slugs: Vec<&str> = page.documents.iter().map(|doc| doc.slug.as_str()).collect();
        assert_eq!(slugs, vec!["a", "b"]);
        assert_eq!(page.next_cursor, None);
    }

    #[test]
    fn paginate_splits_into_pages_by_sorted_slug_and_resumes_from_the_cursor() {
        let documents = vec![
            doc_with_slug("c"),
            doc_with_slug("a"),
            doc_with_slug("b"),
        ];

        let first = paginate(documents.clone(), None, 2);
        let first_slugs: Vec<&str> = first.documents.iter().map(|doc| doc.slug.as_str()).collect();
        assert_eq!(first_slugs, vec!["a", "b"]);
        assert_eq!(first.next_cursor, Some("b".to_string()));

        let second = paginate(documents, first.next_cursor.as_deref(), 2);
        let second_slugs: Vec<&str> = second.documents.iter().map(|doc| doc.slug.as_str()).collect();
        assert_eq!(second_slugs, vec!["c"]);
        assert_eq!(second.next_cursor, None);
    }

    #[test]
    fn paginate_degrades_gracefully_when_the_cursors_document_was_deleted() {
        // "b" was the last item of a previous page but no longer exists.
        let documents = vec![doc_with_slug("a"), doc_with_slug("c")];

        let page = paginate(documents, Some("b"), 10);

        let slugs: Vec<&str> = page.documents.iter().map(|doc| doc.slug.as_str()).collect();
        assert_eq!(slugs, vec!["c"]);
    }
}
