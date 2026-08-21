//! Pasted/typed text ingestion, per `docs/ARCHITECTURE.md` ("Content
//! extraction"): "Pasted/typed text: no fetch, no `canonical_url`, Document
//! created directly from supplied content." Unlike URL ingestion
//! (`crate::fetch` + `crate::extract`) or local file ingestion
//! (`crate::local_file`), there is no extraction step here — the supplied
//! text becomes the Document's Markdown body exactly as given.

use crate::document::Document;

/// Creates a Document directly from pasted/typed `content`, titled `title`.
/// No fetch and no extraction happen: `content` becomes the Document's
/// Markdown body unchanged, and `canonical_url` is left unset, since there
/// is no URL this Document was sourced from.
#[must_use]
pub fn ingest(title: impl Into<String>, content: impl Into<String>) -> Document {
    Document::new(title, content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingest_creates_a_document_with_no_canonical_url() {
        let doc = ingest("My Pasted Note", "Some pasted content.");

        assert_eq!(doc.canonical_url, None);
    }

    #[test]
    fn ingest_preserves_content_exactly_as_supplied() {
        // Deliberately irregular whitespace/blank lines that a readability
        // extraction pass (crate::extract's `normalize`) would collapse —
        // pasted text must survive untouched, with no such normalization
        // applied.
        let content = "Line one.\n\n\n   Line two, with   odd spacing.  \n";

        let doc = ingest("My Pasted Note", content);

        assert_eq!(doc.content, content);
    }

    #[test]
    fn ingest_sets_the_title_and_derives_a_slug_from_it() {
        let doc = ingest("My Pasted Note", "content");

        assert_eq!(doc.title, "My Pasted Note");
        assert_eq!(doc.slug, "my-pasted-note");
    }
}
