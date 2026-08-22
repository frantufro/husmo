//! File I/O for Documents: reading and writing the Markdown+frontmatter
//! files that make up the data repo (see `docs/ARCHITECTURE.md`, "Storage
//! model"), and resolving a Document by exactly one of id/slug/url (see
//! "Identity resolution"). The git pull/commit/push cycle that wraps
//! mutating operations at runtime is a separate concern, layered on top of
//! this module.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::document::{Document, DocumentParseError};

/// File extension used for Document files in the data repo.
const EXTENSION: &str = "md";

/// An error encountered while reading or writing Documents on disk.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// A file could not be read.
    #[error("failed to read {}: {source}", path.display())]
    Read {
        /// The path that was read.
        path: PathBuf,
        /// The underlying I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// A file could not be written.
    #[error("failed to write {}: {source}", path.display())]
    Write {
        /// The path that was written.
        path: PathBuf,
        /// The underlying I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// A file could not be removed.
    #[error("failed to remove {}: {source}", path.display())]
    Remove {
        /// The path that was removed.
        path: PathBuf,
        /// The underlying I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// A directory could not be listed.
    #[error("failed to list directory {}: {source}", path.display())]
    ListDir {
        /// The directory that was listed.
        path: PathBuf,
        /// The underlying I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// A Document file's contents didn't parse.
    #[error("document at {} is malformed: {source}", path.display())]
    Malformed {
        /// The path that was parsed.
        path: PathBuf,
        /// The underlying parse failure.
        #[source]
        source: DocumentParseError,
    },
    /// A slug wasn't safe to use as a single filename component (a path
    /// separator, or a special component like `.`/`..`) — see
    /// [`is_bare_filename_component`].
    #[error("{slug:?} is not a valid slug: it must be a single filename component")]
    InvalidSlug {
        /// The slug that was rejected.
        slug: String,
    },
}

/// Writes `doc` to `dir` as `{slug}.md`, overwriting any existing file with
/// that name. Returns the path written to.
///
/// # Errors
///
/// Returns [`StoreError::InvalidSlug`] if `doc.slug` isn't a single bare
/// filename component (see [`is_bare_filename_component`]), or
/// [`StoreError::Write`] if the file can't be written.
pub fn write(dir: &Path, doc: &Document) -> Result<PathBuf, StoreError> {
    if !is_bare_filename_component(&doc.slug) {
        return Err(StoreError::InvalidSlug {
            slug: doc.slug.clone(),
        });
    }

    let path = dir.join(format!("{}.{EXTENSION}", doc.slug));
    std::fs::write(&path, doc.to_markdown()).map_err(|source| StoreError::Write {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

/// Removes `slug`'s Document file (`{slug}.md`) from `dir`.
///
/// # Errors
///
/// Returns [`StoreError::InvalidSlug`] if `slug` isn't a single bare
/// filename component (see [`is_bare_filename_component`]), or
/// [`StoreError::Remove`] if the file can't be removed, including if it
/// doesn't exist.
pub fn remove(dir: &Path, slug: &str) -> Result<(), StoreError> {
    if !is_bare_filename_component(slug) {
        return Err(StoreError::InvalidSlug {
            slug: slug.to_string(),
        });
    }

    let path = dir.join(format!("{slug}.{EXTENSION}"));
    std::fs::remove_file(&path).map_err(|source| StoreError::Remove { path, source })
}

/// Reads and parses the Document file at `path`.
///
/// # Errors
///
/// Returns [`StoreError::Read`] if the file can't be read, or
/// [`StoreError::Malformed`] if its contents aren't a valid Document.
pub fn read(path: &Path) -> Result<Document, StoreError> {
    let contents = std::fs::read_to_string(path).map_err(|source| StoreError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    Document::from_markdown(&contents).map_err(|source| StoreError::Malformed {
        path: path.to_path_buf(),
        source,
    })
}

/// Loads every Document file (`*.md`) directly inside `dir`.
///
/// # Errors
///
/// Returns [`StoreError::ListDir`] if `dir` can't be listed, or any error
/// [`read`] can return for one of its entries.
pub fn load_all(dir: &Path) -> Result<Vec<Document>, StoreError> {
    document_paths(dir)?
        .into_iter()
        .map(|path| read(&path))
        .collect()
}

/// Collects the slugs already in use in `dir`, derived from the file stems
/// of its `*.md` files. Used with [`crate::document::dedupe_slug`] to pick a
/// collision-free slug before writing a new Document.
///
/// # Errors
///
/// Returns [`StoreError::ListDir`] if `dir` can't be listed.
pub fn existing_slugs(dir: &Path) -> Result<HashSet<String>, StoreError> {
    Ok(document_paths(dir)?
        .into_iter()
        .filter_map(|path| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .collect())
}

/// Lists the paths of every `*.md` file directly inside `dir`.
fn document_paths(dir: &Path) -> Result<Vec<PathBuf>, StoreError> {
    let entries = std::fs::read_dir(dir).map_err(|source| StoreError::ListDir {
        path: dir.to_path_buf(),
        source,
    })?;

    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| StoreError::ListDir {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some(EXTENSION) {
            paths.push(path);
        }
    }
    Ok(paths)
}

/// Exactly one of a Document's three identifying references — the shape
/// `get` and other lookups accept, per `docs/ARCHITECTURE.md` ("Identity
/// resolution").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Identifier {
    /// The Document's stable internal id.
    Id(String),
    /// The Document's slug.
    Slug(String),
    /// The Document's canonical URL.
    Url(String),
}

/// An error encountered while validating that exactly one of id/slug/url
/// was supplied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum IdentifierError {
    /// None of id/slug/url were supplied.
    #[error("no identifier supplied: exactly one of id, slug, or url is required")]
    None,
    /// More than one of id/slug/url were supplied.
    #[error(
        "ambiguous identifier: exactly one of id, slug, or url is required, but more than one was supplied"
    )]
    Ambiguous,
}

/// Validates that exactly one of `id`/`slug`/`url` is set and returns it as
/// an [`Identifier`].
///
/// # Errors
///
/// Returns [`IdentifierError::None`] if all three are `None`, or
/// [`IdentifierError::Ambiguous`] if more than one is `Some`.
pub fn identifier(
    id: Option<String>,
    slug: Option<String>,
    url: Option<String>,
) -> Result<Identifier, IdentifierError> {
    match (id, slug, url) {
        (Some(id), None, None) => Ok(Identifier::Id(id)),
        (None, Some(slug), None) => Ok(Identifier::Slug(slug)),
        (None, None, Some(url)) => Ok(Identifier::Url(url)),
        (None, None, None) => Err(IdentifierError::None),
        _ => Err(IdentifierError::Ambiguous),
    }
}

/// An error encountered while resolving an [`Identifier`] to a Document.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ResolveError {
    /// No Document in `dir` matched the given identifier.
    #[error("no Document found matching {0:?}")]
    NotFound(Identifier),
    /// Reading or parsing Documents in `dir` failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Reports whether `s` is safe to use as a single filename component: no
/// path separators, and not a special component like `.` or `..`. Guards
/// [`resolve`]'s slug lookup against a slug that would otherwise let a
/// caller escape `dir` (e.g. `../../etc/passwd`) or resolve to a directory
/// entry (e.g. `.`).
fn is_bare_filename_component(s: &str) -> bool {
    Path::new(s).file_name() == Some(OsStr::new(s))
}

/// Resolves `identifier` to the one Document in `dir` it refers to.
///
/// # Errors
///
/// Returns [`ResolveError::NotFound`] if no Document matches, or
/// [`ResolveError::Store`] if `dir` can't be read.
pub fn resolve(dir: &Path, identifier: &Identifier) -> Result<Document, ResolveError> {
    match identifier {
        Identifier::Slug(slug) => {
            if !is_bare_filename_component(slug) {
                return Err(ResolveError::NotFound(identifier.clone()));
            }
            let path = dir.join(format!("{slug}.{EXTENSION}"));
            if path.is_file() {
                Ok(read(&path)?)
            } else {
                Err(ResolveError::NotFound(identifier.clone()))
            }
        }
        Identifier::Id(id) => load_all(dir)?
            .into_iter()
            .find(|doc| doc.id == *id)
            .ok_or_else(|| ResolveError::NotFound(identifier.clone())),
        Identifier::Url(url) => load_all(dir)?
            .into_iter()
            .find(|doc| doc.canonical_url.as_deref() == Some(url.as_str()))
            .ok_or_else(|| ResolveError::NotFound(identifier.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_finds_the_same_document_by_id_slug_or_url() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let mut doc = Document::new("My Title", "content\n");
        doc.canonical_url = Some("https://example.com/post".to_string());
        write(dir.path(), &doc).expect("write should succeed");

        let by_id = resolve(dir.path(), &Identifier::Id(doc.id.clone()))
            .expect("resolve by id should succeed");
        let by_slug = resolve(dir.path(), &Identifier::Slug(doc.slug.clone()))
            .expect("resolve by slug should succeed");
        let by_url = resolve(
            dir.path(),
            &Identifier::Url("https://example.com/post".to_string()),
        )
        .expect("resolve by url should succeed");

        assert_eq!(by_id, doc);
        assert_eq!(by_slug, doc);
        assert_eq!(by_url, doc);
    }

    #[test]
    fn resolve_reports_not_found_for_an_unknown_identifier() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");

        let result = resolve(dir.path(), &Identifier::Slug("nope".to_string()));

        assert!(matches!(result, Err(ResolveError::NotFound(_))));
    }

    #[test]
    fn resolve_rejects_slugs_that_would_traverse_outside_the_directory() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let outside_dir = tempfile::tempdir().expect("failed to create temp dir");
        let secret = outside_dir.path().join("secret.md");
        std::fs::write(&secret, "top secret").expect("failed to write outside file");

        // A path-traversal slug reaching for the sibling temp dir's file.
        let traversal = format!(
            "../{}/secret",
            outside_dir
                .path()
                .file_name()
                .expect("temp dir should have a name")
                .to_string_lossy()
        );
        let result = resolve(dir.path(), &Identifier::Slug(traversal));
        assert!(matches!(result, Err(ResolveError::NotFound(_))));

        // Bare separators and special components are rejected too.
        for slug in ["a/b", "..", ".", "/etc/passwd"] {
            let result = resolve(dir.path(), &Identifier::Slug(slug.to_string()));
            assert!(
                matches!(result, Err(ResolveError::NotFound(_))),
                "expected slug {slug:?} to be rejected as not found"
            );
        }
    }

    #[test]
    fn identifier_accepts_exactly_one_of_id_slug_url() {
        assert_eq!(
            identifier(Some("abc".to_string()), None, None),
            Ok(Identifier::Id("abc".to_string()))
        );
        assert_eq!(
            identifier(None, Some("my-slug".to_string()), None),
            Ok(Identifier::Slug("my-slug".to_string()))
        );
        assert_eq!(
            identifier(None, None, Some("https://example.com".to_string())),
            Ok(Identifier::Url("https://example.com".to_string()))
        );
    }

    #[test]
    fn identifier_rejects_zero_identifiers() {
        assert_eq!(identifier(None, None, None), Err(IdentifierError::None));
    }

    #[test]
    fn write_then_read_round_trips_a_document() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let doc = Document::new("My Title", "Some content.\n");

        let path = write(dir.path(), &doc).expect("write should succeed");
        let loaded = read(&path).expect("read should succeed");

        assert_eq!(loaded, doc);
    }

    #[test]
    fn write_rejects_a_slug_that_would_traverse_outside_the_directory() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let mut doc = Document::new("My Title", "content\n");
        doc.slug = "../escape".to_string();

        let result = write(dir.path(), &doc);

        assert!(matches!(result, Err(StoreError::InvalidSlug { .. })));
    }

    #[test]
    fn load_all_reads_every_document_file_in_a_directory() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let one = Document::new("One", "first\n");
        let two = Document::new("Two", "second\n");
        write(dir.path(), &one).expect("write should succeed");
        write(dir.path(), &two).expect("write should succeed");
        std::fs::write(dir.path().join("not-a-document.txt"), "ignore me")
            .expect("failed to write stray file");

        let mut loaded = load_all(dir.path()).expect("load_all should succeed");
        loaded.sort_by(|a, b| a.slug.cmp(&b.slug));

        assert_eq!(loaded, vec![one, two]);
    }

    #[test]
    fn remove_deletes_a_documents_file() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let doc = Document::new("My Title", "content\n");
        let path = write(dir.path(), &doc).expect("write should succeed");
        assert!(path.is_file());

        remove(dir.path(), &doc.slug).expect("remove should succeed");

        assert!(!path.is_file());
    }

    #[test]
    fn remove_errors_when_the_file_does_not_exist() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");

        let result = remove(dir.path(), "does-not-exist");

        assert!(matches!(result, Err(StoreError::Remove { .. })));
    }

    #[test]
    fn remove_rejects_a_slug_that_would_traverse_outside_the_directory() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");

        let result = remove(dir.path(), "../escape");

        assert!(matches!(result, Err(StoreError::InvalidSlug { .. })));
    }

    #[test]
    fn existing_slugs_lists_slugs_already_written() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        write(dir.path(), &Document::new("My Title", "content\n")).expect("write should succeed");

        let slugs = existing_slugs(dir.path()).expect("existing_slugs should succeed");

        assert_eq!(slugs, HashSet::from(["my-title".to_string()]));
    }

    #[test]
    fn identifier_rejects_more_than_one_identifier() {
        assert_eq!(
            identifier(Some("abc".to_string()), Some("my-slug".to_string()), None),
            Err(IdentifierError::Ambiguous)
        );
        assert_eq!(
            identifier(
                Some("abc".to_string()),
                Some("my-slug".to_string()),
                Some("https://example.com".to_string())
            ),
            Err(IdentifierError::Ambiguous)
        );
    }
}
