//! The Related graph: relate/unrelate a deliberate, symmetric, untyped edge
//! between two existing Documents, per `docs/ARCHITECTURE.md` ("Related")
//! and `CONTEXT.md`. Distinct from outgoing hyperlinks discovered in
//! content, which are a separate concept (see `docs/ARCHITECTURE.md`,
//! "Content extraction").

use std::path::Path;

use crate::document::Document;
use crate::store::{self, StoreError};

/// An error encountered while relating or unrelating two Documents.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RelateError {
    /// No Document in the store matched the given id.
    #[error("no Document found with id {0:?}")]
    NotFound(String),
    /// `id_a` and `id_b` were the same id; a Document cannot be related to
    /// itself.
    #[error("cannot relate a Document to itself (id {0:?})")]
    SameDocument(String),
    /// Reading or writing a Document in the store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Declares a symmetric Related edge between the Documents identified by
/// `id_a` and `id_b`.
///
/// # Errors
///
/// Returns [`RelateError::SameDocument`] if `id_a` and `id_b` are the same
/// id, [`RelateError::NotFound`] if `id_a` or `id_b` doesn't match a
/// Document in `dir`, or [`RelateError::Store`] if reading or writing
/// fails.
pub fn relate(dir: &Path, id_a: &str, id_b: &str) -> Result<(), RelateError> {
    if id_a == id_b {
        return Err(RelateError::SameDocument(id_a.to_string()));
    }

    let documents = store::load_all(dir)?;
    let mut doc_a = find_by_id(&documents, id_a)?;
    let mut doc_b = find_by_id(&documents, id_b)?;

    add_related(&mut doc_a, id_b);
    add_related(&mut doc_b, id_a);

    store::write(dir, &doc_a)?;
    store::write(dir, &doc_b)?;
    Ok(())
}

/// Removes the symmetric Related edge between the Documents identified by
/// `id_a` and `id_b`, if one exists.
///
/// # Errors
///
/// Returns [`RelateError::SameDocument`] if `id_a` and `id_b` are the same
/// id, [`RelateError::NotFound`] if `id_a` or `id_b` doesn't match a
/// Document in `dir`, or [`RelateError::Store`] if reading or writing
/// fails.
pub fn unrelate(dir: &Path, id_a: &str, id_b: &str) -> Result<(), RelateError> {
    if id_a == id_b {
        return Err(RelateError::SameDocument(id_a.to_string()));
    }

    let documents = store::load_all(dir)?;
    let mut doc_a = find_by_id(&documents, id_a)?;
    let mut doc_b = find_by_id(&documents, id_b)?;

    doc_a.related.retain(|id| id != id_b);
    doc_b.related.retain(|id| id != id_a);

    store::write(dir, &doc_a)?;
    store::write(dir, &doc_b)?;
    Ok(())
}

/// Adds `related_id` to `document.related` unless it's already present.
fn add_related(document: &mut Document, related_id: &str) {
    if !document.related.iter().any(|id| id == related_id) {
        document.related.push(related_id.to_string());
    }
}

/// Finds the one Document in `documents` whose `id` is `id`.
fn find_by_id(documents: &[Document], id: &str) -> Result<Document, RelateError> {
    documents
        .iter()
        .find(|doc| doc.id == id)
        .cloned()
        .ok_or_else(|| RelateError::NotFound(id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reloads the Document with `id` from `dir`, for asserting on state
    /// written by `relate`/`unrelate`.
    fn reload(dir: &Path, id: &str) -> Document {
        let documents = store::load_all(dir).expect("load_all should succeed");
        find_by_id(&documents, id).expect("document should still exist")
    }

    #[test]
    fn relate_is_symmetric() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let doc_a = Document::new("A", "content a");
        let doc_b = Document::new("B", "content b");
        store::write(dir.path(), &doc_a).expect("write should succeed");
        store::write(dir.path(), &doc_b).expect("write should succeed");

        relate(dir.path(), &doc_a.id, &doc_b.id).expect("relate should succeed");

        let reloaded_a = reload(dir.path(), &doc_a.id);
        let reloaded_b = reload(dir.path(), &doc_b.id);
        assert_eq!(reloaded_a.related, vec![doc_b.id.clone()]);
        assert_eq!(reloaded_b.related, vec![doc_a.id.clone()]);
    }

    #[test]
    fn relate_does_not_duplicate_an_existing_edge() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let doc_a = Document::new("A", "content a");
        let doc_b = Document::new("B", "content b");
        store::write(dir.path(), &doc_a).expect("write should succeed");
        store::write(dir.path(), &doc_b).expect("write should succeed");

        relate(dir.path(), &doc_a.id, &doc_b.id).expect("relate should succeed");
        relate(dir.path(), &doc_a.id, &doc_b.id).expect("relating again should succeed");

        let reloaded_a = reload(dir.path(), &doc_a.id);
        assert_eq!(reloaded_a.related, vec![doc_b.id.clone()]);
    }

    #[test]
    fn unrelate_removes_both_directions() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let doc_a = Document::new("A", "content a");
        let doc_b = Document::new("B", "content b");
        store::write(dir.path(), &doc_a).expect("write should succeed");
        store::write(dir.path(), &doc_b).expect("write should succeed");
        relate(dir.path(), &doc_a.id, &doc_b.id).expect("relate should succeed");

        unrelate(dir.path(), &doc_a.id, &doc_b.id).expect("unrelate should succeed");

        let reloaded_a = reload(dir.path(), &doc_a.id);
        let reloaded_b = reload(dir.path(), &doc_b.id);
        assert!(reloaded_a.related.is_empty());
        assert!(reloaded_b.related.is_empty());
    }

    #[test]
    fn unrelate_is_a_no_op_when_no_edge_exists() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let doc_a = Document::new("A", "content a");
        let doc_b = Document::new("B", "content b");
        store::write(dir.path(), &doc_a).expect("write should succeed");
        store::write(dir.path(), &doc_b).expect("write should succeed");

        unrelate(dir.path(), &doc_a.id, &doc_b.id).expect("unrelate should succeed");

        let reloaded_a = reload(dir.path(), &doc_a.id);
        assert!(reloaded_a.related.is_empty());
    }

    #[test]
    fn relate_errors_on_a_nonexistent_document() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let doc_a = Document::new("A", "content a");
        store::write(dir.path(), &doc_a).expect("write should succeed");

        let result = relate(dir.path(), &doc_a.id, "nonexistent-id");

        assert!(matches!(result, Err(RelateError::NotFound(id)) if id == "nonexistent-id"));
        let reloaded_a = reload(dir.path(), &doc_a.id);
        assert!(
            reloaded_a.related.is_empty(),
            "doc_a should not be written to when the other id doesn't resolve"
        );
    }

    #[test]
    fn unrelate_errors_on_a_nonexistent_document() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let doc_a = Document::new("A", "content a");
        store::write(dir.path(), &doc_a).expect("write should succeed");

        let result = unrelate(dir.path(), &doc_a.id, "nonexistent-id");

        assert!(matches!(result, Err(RelateError::NotFound(id)) if id == "nonexistent-id"));
    }

    #[test]
    fn relate_errors_when_id_a_and_id_b_are_the_same() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let doc_a = Document::new("A", "content a");
        store::write(dir.path(), &doc_a).expect("write should succeed");

        let result = relate(dir.path(), &doc_a.id, &doc_a.id);

        assert!(matches!(result, Err(RelateError::SameDocument(id)) if id == doc_a.id));
        let reloaded_a = reload(dir.path(), &doc_a.id);
        assert!(
            reloaded_a.related.is_empty(),
            "doc_a should not be related to itself"
        );
    }

    #[test]
    fn unrelate_errors_when_id_a_and_id_b_are_the_same() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let doc_a = Document::new("A", "content a");
        store::write(dir.path(), &doc_a).expect("write should succeed");

        let result = unrelate(dir.path(), &doc_a.id, &doc_a.id);

        assert!(matches!(result, Err(RelateError::SameDocument(id)) if id == doc_a.id));
    }

    #[test]
    fn retrieval_always_lists_related_documents_by_reference() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let doc_a = Document::new("A", "content a");
        let doc_b = Document::new("B", "content b");
        store::write(dir.path(), &doc_a).expect("write should succeed");
        store::write(dir.path(), &doc_b).expect("write should succeed");
        relate(dir.path(), &doc_a.id, &doc_b.id).expect("relate should succeed");

        // Every retrieval path clones the Document as stored, so its
        // `related` list is visible regardless of any expansion flag —
        // resolving by id/slug/url is the simplest case to demonstrate.
        let resolved = store::resolve(dir.path(), &store::Identifier::Id(doc_a.id.clone()))
            .expect("resolve should succeed");
        assert_eq!(resolved.related, vec![doc_b.id.clone()]);
    }
}
