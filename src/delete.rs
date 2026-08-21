//! Deleting a Document, per `docs/ARCHITECTURE.md` ("MCP server", `delete`):
//! "goes through the same git pull/commit/push cycle; nothing is truly
//! unrecoverable since it's still in git history." This module is the pure
//! business logic behind that tool: given an identifier that resolves to an
//! existing Document, it removes the Document's own `{slug}.md` file and
//! its chunk-embeddings sidecar file (if any) from a data repo directory.
//! Wrapping the removal in the git pull/commit/push cycle (see
//! `crate::git_sync`) is left to the caller — the MCP server layer — the
//! same way `crate::related` and `crate::save` leave it to theirs.
//!
//! Deleting only removes the Document from the data repo's *current*
//! state: the commit that removes it still has the prior commit (which
//! still has the file) as its parent, so every past version of the
//! Document remains recoverable from git history.

use std::path::Path;

use crate::document::Document;
use crate::embeddings::{self, EmbeddingsError};
use crate::store::{self, Identifier, ResolveError, StoreError};

/// An error encountered while deleting a Document.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DeleteError {
    /// No Document in the store matched the given identifier.
    #[error("no Document found matching {0:?}")]
    NotFound(Identifier),
    /// Resolving the identifier, or removing the Document's own file,
    /// failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Removing the Document's chunk-embeddings sidecar file failed.
    #[error(transparent)]
    Embeddings(#[from] EmbeddingsError),
}

impl From<ResolveError> for DeleteError {
    fn from(error: ResolveError) -> Self {
        match error {
            ResolveError::NotFound(identifier) => DeleteError::NotFound(identifier),
            ResolveError::Store(source) => DeleteError::Store(source),
        }
    }
}

/// Deletes the Document `identifier` resolves to from `dir`: removes its
/// own `{slug}.md` file and its chunk-embeddings sidecar file (if one
/// exists), and returns the Document as it stood just before removal.
///
/// # Errors
///
/// Returns [`DeleteError::NotFound`] if `identifier` doesn't resolve to a
/// Document in `dir`, or [`DeleteError::Store`]/[`DeleteError::Embeddings`]
/// if removing its files fails.
pub fn delete(dir: &Path, identifier: &Identifier) -> Result<Document, DeleteError> {
    let document = store::resolve(dir, identifier)?;
    store::remove(dir, &document.slug)?;
    embeddings::remove(dir, &document.slug)?;
    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_removes_the_documents_file_and_returns_it() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let doc = Document::new("My Title", "content\n");
        store::write(dir.path(), &doc).expect("write should succeed");

        let deleted = crate::delete::delete(dir.path(), &Identifier::Id(doc.id.clone()))
            .expect("delete should succeed");

        assert_eq!(deleted, doc);
        let result = store::resolve(dir.path(), &Identifier::Id(doc.id.clone()));
        assert!(
            matches!(result, Err(ResolveError::NotFound(_))),
            "the Document should no longer resolve after being deleted"
        );
    }

    #[test]
    fn delete_removes_the_documents_embeddings_sidecar_file() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let doc = Document::new("My Title", "content\n");
        store::write(dir.path(), &doc).expect("write should succeed");
        let embeddings =
            crate::embeddings::DocumentEmbeddings::build(&doc).expect("build should succeed");
        let sidecar_path =
            crate::embeddings::write(dir.path(), &doc.slug, &embeddings).expect("write should succeed");
        assert!(sidecar_path.is_file());

        crate::delete::delete(dir.path(), &Identifier::Id(doc.id.clone())).expect("delete should succeed");

        assert!(!sidecar_path.is_file());
    }

    #[test]
    fn delete_succeeds_when_the_document_has_no_embeddings_sidecar_file() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let doc = Document::new("My Title", "content\n");
        store::write(dir.path(), &doc).expect("write should succeed");

        let result = crate::delete::delete(dir.path(), &Identifier::Id(doc.id.clone()));

        assert!(result.is_ok());
    }

    #[test]
    fn delete_errors_on_an_unknown_identifier() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");

        let result = crate::delete::delete(dir.path(), &Identifier::Id("nonexistent-id".to_string()));

        assert!(matches!(result, Err(crate::delete::DeleteError::NotFound(_))));
    }
}
