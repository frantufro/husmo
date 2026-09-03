//! The `save` tool's ingestion dispatch, per `docs/ARCHITECTURE.md` ("MCP
//! server"): "`save` — URL, pasted text, or local file path; dispatches to
//! the right ingestion path; chunks + embeds; ... returns the saved
//! Document plus any outgoing links discovered." This module is the pure
//! business logic behind that tool: given a validated [`SaveInput`], it
//! writes the resulting Document (and its chunk-embeddings sidecar) into a
//! data repo directory. Wrapping the write in the git pull/commit/push
//! cycle (see `crate::git_sync`) is left to the caller — the MCP server
//! layer — the same way `crate::archive` and `crate::related` leave it to
//! their callers.
//!
//! For a URL, re-saving the same `canonical_url` overwrites the existing
//! Document in place (same `id`/`slug`) rather than creating a second one,
//! per `docs/ARCHITECTURE.md` ("Storage model") — the same rule
//! `crate::archive::archive_outgoing_link` follows for an archived link.

use std::path::{Path, PathBuf};

use crate::document::{Document, dedupe_slug};
use crate::embeddings::{self, DocumentEmbeddings, EmbeddingsError};
use crate::extract::OutgoingLink;
use crate::fetch::FetchError;
use crate::images::ImageError;
use crate::local_file::{self, IngestError as LocalFileIngestError, PathPolicy};
use crate::pasted_text;
use crate::store::{self, StoreError};
use crate::url_ingest::{UrlIngestError, fetch_and_build_document};

/// What to save, dispatched to the matching ingestion path. Constructed via
/// [`save_input`], which validates that exactly one of the three kinds was
/// supplied.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SaveInput {
    /// Fetch and extract a URL's content (see `crate::fetch`,
    /// `crate::extract`).
    Url(String),
    /// Ingest a local file's content (see `crate::local_file`).
    LocalFile(PathBuf),
    /// Use pasted/typed `content` directly, titled `title` (see
    /// `crate::pasted_text`).
    PastedText {
        /// The Document's title. Required for pasted text since there is
        /// no page or filename to derive one from.
        title: String,
        /// The pasted/typed content, used as the Document's body exactly
        /// as supplied.
        content: String,
    },
}

/// An error encountered while validating the `save` tool's raw parameters
/// into a [`SaveInput`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SaveInputError {
    /// None of `url`/`path`/`content` were supplied.
    #[error("no input supplied: exactly one of url, path, or content is required")]
    None,
    /// More than one of `url`/`path`/`content` were supplied.
    #[error(
        "ambiguous input: exactly one of url, path, or content is required, but more than one was supplied"
    )]
    Ambiguous,
    /// `content` was supplied (pasted text) without a `title`.
    #[error("pasted text requires a title")]
    MissingTitle,
}

/// Validates that exactly one of `url`/`path`/`content` is set and returns
/// it as a [`SaveInput`]. `title` is required when `content` is the one
/// supplied (pasted text has no page or filename to derive a title from);
/// it's ignored for the `url` and `path` kinds, which derive their own
/// title during ingestion.
///
/// # Errors
///
/// Returns [`SaveInputError::None`] if all three are `None`,
/// [`SaveInputError::Ambiguous`] if more than one is `Some`, or
/// [`SaveInputError::MissingTitle`] if `content` is supplied without a
/// `title`.
pub fn save_input(
    url: Option<String>,
    path: Option<String>,
    content: Option<String>,
    title: Option<String>,
) -> Result<SaveInput, SaveInputError> {
    match (url, path, content) {
        (Some(url), None, None) => Ok(SaveInput::Url(url)),
        (None, Some(path), None) => Ok(SaveInput::LocalFile(PathBuf::from(path))),
        (None, None, Some(content)) => {
            let title = title.ok_or(SaveInputError::MissingTitle)?;
            Ok(SaveInput::PastedText { title, content })
        }
        (None, None, None) => Err(SaveInputError::None),
        _ => Err(SaveInputError::Ambiguous),
    }
}

/// An error encountered while saving a Document.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SaveError {
    /// Fetching a URL's page failed.
    #[error(transparent)]
    Fetch(#[from] FetchError),
    /// Downloading and localizing one of the page's images failed.
    #[error(transparent)]
    Image(#[from] ImageError),
    /// Ingesting a local file failed.
    #[error(transparent)]
    LocalFile(#[from] LocalFileIngestError),
    /// Reading or writing the Document (or listing existing slugs) failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Writing the Document's embeddings sidecar failed.
    #[error(transparent)]
    Embeddings(#[from] EmbeddingsError),
}

impl From<UrlIngestError> for SaveError {
    fn from(error: UrlIngestError) -> Self {
        match error {
            UrlIngestError::Fetch(source) => SaveError::Fetch(source),
            UrlIngestError::Image(source) => SaveError::Image(source),
            UrlIngestError::Store(source) => SaveError::Store(source),
        }
    }
}

/// The result of a `save` call: the saved Document, plus any outgoing links
/// discovered in its content (empty for the local-file and pasted-text
/// kinds, which have no extraction step). Reported as data only, per
/// `docs/ARCHITECTURE.md` ("Content extraction") — never followed
/// automatically.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct SaveOutput {
    /// The Document that was written.
    pub document: Document,
    /// Outgoing links discovered in the Document's own content, in the
    /// order they appear.
    pub outgoing_links: Vec<OutgoingLink>,
}

/// Saves `input` as a Document inside `dir`: dispatches to the matching
/// ingestion path, applies `tags`, chunks and embeds the result, and writes
/// both the Document and its chunk-embeddings sidecar. For the `Url` kind,
/// re-saving the same `canonical_url` overwrites the existing Document in
/// place (same `id`/`slug`) instead of creating a second one. `path_policy`
/// is only consulted for the `LocalFile` kind, gating which local paths
/// `save` is allowed to read from (see [`local_file::validate_source_path`]).
///
/// # Errors
///
/// Returns [`SaveError::Fetch`]/[`SaveError::Image`] if a URL or one of its
/// images can't be fetched, [`SaveError::LocalFile`] if a local file is
/// rejected by `path_policy` or can't be ingested, or
/// [`SaveError::Store`]/[`SaveError::Embeddings`] if writing the Document or
/// its embeddings sidecar fails.
pub fn save(
    dir: &Path,
    input: SaveInput,
    tags: Vec<String>,
    path_policy: &PathPolicy,
) -> Result<SaveOutput, SaveError> {
    let (mut document, outgoing_links) = match input {
        SaveInput::Url(url) => save_url(dir, &url)?,
        SaveInput::LocalFile(path) => (save_local_file(dir, &path, path_policy)?, Vec::new()),
        SaveInput::PastedText { title, content } => {
            (pasted_text::ingest(title, content), Vec::new())
        }
    };
    document.tags = tags;

    store::write(dir, &document)?;
    let embeddings = DocumentEmbeddings::build(&document)?;
    embeddings::write(dir, &document.slug, &embeddings)?;

    Ok(SaveOutput {
        document,
        outgoing_links,
    })
}

/// The `Url` branch of [`save`]: fetches and extracts `url`, downloads and
/// localizes its images, and overwrites the existing Document for this
/// `canonical_url` in place if one already exists (see
/// `crate::url_ingest`). Mirrors `crate::archive::archive_outgoing_link`,
/// whose fallback title is the discovered link's own text — here there is
/// no such text, so the URL itself is the fallback.
fn save_url(dir: &Path, url: &str) -> Result<(Document, Vec<OutgoingLink>), SaveError> {
    Ok(fetch_and_build_document(dir, url, url)?)
}

/// The `LocalFile` branch of [`save`]: validates `path` against
/// `path_policy` (see `crate::local_file::validate_source_path`), ingests
/// its content (see `crate::local_file::ingest`), and titles the new
/// Document after the file's stem (the filename without its extension).
/// There is no `canonical_url` and no extraction step for a local file, so
/// re-saving the same path always creates a new Document rather than
/// overwriting one in place.
fn save_local_file(
    dir: &Path,
    path: &Path,
    path_policy: &PathPolicy,
) -> Result<Document, SaveError> {
    let canonical_path =
        local_file::validate_source_path(path, path_policy).map_err(LocalFileIngestError::from)?;
    let content = local_file::ingest(&canonical_path)?;
    let title = path.file_stem().map_or_else(
        || path.to_string_lossy().into_owned(),
        |stem| stem.to_string_lossy().into_owned(),
    );

    let mut document = Document::new(title, content);
    document.slug = dedupe_slug(&document.slug, &store::existing_slugs(dir)?);
    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- save_input --------------------------------------------------

    #[test]
    fn save_input_returns_url_variant_when_only_url_is_set() {
        let input = save_input(Some("https://example.com".to_string()), None, None, None);

        assert_eq!(input, Ok(SaveInput::Url("https://example.com".to_string())));
    }

    #[test]
    fn save_input_returns_local_file_variant_when_only_path_is_set() {
        let input = save_input(None, Some("/tmp/notes.txt".to_string()), None, None);

        assert_eq!(
            input,
            Ok(SaveInput::LocalFile(PathBuf::from("/tmp/notes.txt")))
        );
    }

    #[test]
    fn save_input_returns_pasted_text_variant_when_content_and_title_are_set() {
        let input = save_input(
            None,
            None,
            Some("Some content".to_string()),
            Some("My Title".to_string()),
        );

        assert_eq!(
            input,
            Ok(SaveInput::PastedText {
                title: "My Title".to_string(),
                content: "Some content".to_string(),
            })
        );
    }

    #[test]
    fn save_input_rejects_pasted_text_without_a_title() {
        let input = save_input(None, None, Some("Some content".to_string()), None);

        assert_eq!(input, Err(SaveInputError::MissingTitle));
    }

    #[test]
    fn save_input_rejects_zero_inputs() {
        assert_eq!(
            save_input(None, None, None, None),
            Err(SaveInputError::None)
        );
    }

    #[test]
    fn save_input_rejects_more_than_one_input() {
        assert_eq!(
            save_input(
                Some("https://example.com".to_string()),
                Some("/tmp/notes.txt".to_string()),
                None,
                None
            ),
            Err(SaveInputError::Ambiguous)
        );
        assert_eq!(
            save_input(
                Some("https://example.com".to_string()),
                None,
                Some("content".to_string()),
                None
            ),
            Err(SaveInputError::Ambiguous)
        );
    }

    // -- save ----------------------------------------------------------

    #[test]
    fn save_creates_a_document_from_pasted_text_with_no_canonical_url() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let input = SaveInput::PastedText {
            title: "My Pasted Note".to_string(),
            content: "Some pasted content.".to_string(),
        };

        let output = save(dir.path(), input, Vec::new(), &PathPolicy::default())
            .expect("save should succeed");

        assert_eq!(output.document.title, "My Pasted Note");
        assert_eq!(output.document.content, "Some pasted content.");
        assert_eq!(output.document.canonical_url, None);
        assert!(output.outgoing_links.is_empty());

        let on_disk = crate::store::resolve(
            dir.path(),
            &crate::store::Identifier::Id(output.document.id.clone()),
        )
        .expect("the saved document should be resolvable from disk");
        assert_eq!(on_disk, output.document);
    }

    #[test]
    fn save_applies_the_given_tags_to_a_pasted_text_document() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let input = SaveInput::PastedText {
            title: "Tagged Note".to_string(),
            content: "Content.".to_string(),
        };

        let output = save(
            dir.path(),
            input,
            vec!["rust".to_string(), "notes".to_string()],
            &PathPolicy::default(),
        )
        .expect("save should succeed");

        assert_eq!(
            output.document.tags,
            vec!["rust".to_string(), "notes".to_string()]
        );
    }

    #[test]
    fn save_writes_a_chunk_embeddings_sidecar_for_a_pasted_text_document() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let input = SaveInput::PastedText {
            title: "Embedded Note".to_string(),
            content: "First paragraph.\n\nSecond paragraph.".to_string(),
        };

        let output = save(dir.path(), input, Vec::new(), &PathPolicy::default())
            .expect("save should succeed");

        let sidecar = crate::embeddings::read(&crate::embeddings::sidecar_path(
            dir.path(),
            &output.document.slug,
        ))
        .expect("sidecar file should exist and parse");
        assert_eq!(sidecar.document_id, output.document.id);
        assert_eq!(sidecar.chunks.len(), 2);
    }

    #[test]
    fn save_ingests_a_local_file_with_no_canonical_url() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let file_path = dir.path().join("notes.txt");
        std::fs::write(&file_path, "Some file content.\n").expect("failed to write test file");
        let input = SaveInput::LocalFile(file_path);

        let output = save(dir.path(), input, Vec::new(), &PathPolicy::default())
            .expect("save should succeed");

        assert_eq!(output.document.content, "Some file content.\n");
        assert_eq!(output.document.canonical_url, None);
        assert_eq!(output.document.title, "notes");
        assert!(output.outgoing_links.is_empty());
    }

    #[test]
    fn save_rejects_a_local_file_outside_the_configured_allowed_source_dirs() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let outside_dir = tempfile::tempdir().expect("failed to create temp dir");
        let file_path = outside_dir.path().join("secret.txt");
        std::fs::write(&file_path, "top secret").expect("failed to write test file");
        let policy = PathPolicy {
            allowed_source_dirs: Some(vec![dir.path().to_path_buf()]),
            home: None,
        };

        let result = save(
            dir.path(),
            SaveInput::LocalFile(file_path),
            Vec::new(),
            &policy,
        );

        assert!(
            matches!(
                result,
                Err(SaveError::LocalFile(LocalFileIngestError::PathRestriction(
                    crate::local_file::PathRestrictionError::OutsideAllowedDirs { .. }
                )))
            ),
            "expected a PathRestriction/OutsideAllowedDirs error, got {result:?}"
        );
        let on_disk = crate::store::load_all(dir.path()).expect("load_all should succeed");
        assert!(
            on_disk.is_empty(),
            "no Document should have been written for a rejected path"
        );
    }

    /// Starts a one-shot HTTP server on an OS-assigned localhost port that
    /// replies to a single connection with an HTML page whose body is
    /// `body`, then shuts down. Mirrors the helper in `crate::archive`'s
    /// tests.
    fn one_shot_page_server(body: &'static str) -> String {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind test listener");
        let addr = listener.local_addr().expect("failed to read local addr");
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("failed to accept connection");
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("failed to write test response");
        });
        format!("http://{addr}/")
    }

    /// Like [`one_shot_page_server`], but binds once and answers two
    /// sequential connections in order, with `first_body` then
    /// `second_body`. Used to save the same URL twice against a server
    /// that stays alive for both fetches.
    fn two_shot_page_server(first_body: &'static str, second_body: &'static str) -> String {
        use std::io::{Read, Write};
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind test listener");
        let addr = listener.local_addr().expect("failed to read local addr");
        std::thread::spawn(move || {
            for body in [first_body, second_body] {
                let (mut stream, _) = listener.accept().expect("failed to accept connection");
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\
                     Connection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("failed to write test response");
            }
        });
        format!("http://{addr}/")
    }

    #[test]
    fn save_fetches_a_url_and_reports_its_outgoing_links() {
        let url = one_shot_page_server(
            "<html><head><title>Fetched Page</title></head>\
             <body><article><p>Some content. See \
             <a href=\"https://further.example/\">more</a>.</p></article></body></html>",
        );

        let output = save(
            tempfile::tempdir()
                .expect("failed to create temp dir")
                .path(),
            SaveInput::Url(url.clone()),
            Vec::new(),
            &PathPolicy::default(),
        )
        .expect("save should succeed");

        assert_eq!(output.document.canonical_url, Some(url));
        assert_eq!(output.document.title, "Fetched Page");
        assert_eq!(
            output.outgoing_links,
            vec![OutgoingLink {
                text: "more".to_string(),
                url: "https://further.example/".to_string(),
            }]
        );
    }

    #[test]
    fn save_overwrites_the_existing_document_in_place_when_the_same_url_is_saved_again() {
        let url = two_shot_page_server(
            "<html><head><title>Original Title</title></head>\
             <body><article><p>Original content.</p></article></body></html>",
            "<html><head><title>Original Title</title></head>\
             <body><article><p>Updated content.</p></article></body></html>",
        );
        let dir = tempfile::tempdir().expect("failed to create temp dir");

        let first = save(
            dir.path(),
            SaveInput::Url(url.clone()),
            Vec::new(),
            &PathPolicy::default(),
        )
        .expect("first save should succeed");
        let second = save(
            dir.path(),
            SaveInput::Url(url.clone()),
            Vec::new(),
            &PathPolicy::default(),
        )
        .expect("second save should succeed");

        assert_eq!(second.document.id, first.document.id);
        assert_eq!(second.document.slug, first.document.slug);
        assert_eq!(second.document.content, "Updated content.");

        let all_documents = crate::store::load_all(dir.path()).expect("load_all should succeed");
        assert_eq!(
            all_documents.len(),
            1,
            "re-saving the same url should not create a second Document"
        );
    }

    #[test]
    fn save_preserves_related_edges_when_the_same_url_is_saved_again() {
        let url = two_shot_page_server(
            "<html><head><title>Original Title</title></head>\
             <body><article><p>Original content.</p></article></body></html>",
            "<html><head><title>Original Title</title></head>\
             <body><article><p>Updated content.</p></article></body></html>",
        );
        let dir = tempfile::tempdir().expect("failed to create temp dir");

        let first = save(
            dir.path(),
            SaveInput::Url(url.clone()),
            Vec::new(),
            &PathPolicy::default(),
        )
        .expect("first save should succeed");
        let other = Document::new("Other Document", "other content");
        crate::store::write(dir.path(), &other).expect("write should succeed");
        crate::related::relate(dir.path(), &first.document.id, &other.id)
            .expect("relate should succeed");

        let second = save(
            dir.path(),
            SaveInput::Url(url),
            Vec::new(),
            &PathPolicy::default(),
        )
        .expect("second save should succeed");

        assert_eq!(
            second.document.related,
            vec![other.id.clone()],
            "re-saving the same url should not wipe the Document's related edges"
        );

        let reloaded_other =
            crate::store::resolve(dir.path(), &crate::store::Identifier::Id(other.id.clone()))
                .expect("resolve should succeed");
        assert_eq!(
            reloaded_other.related,
            vec![second.document.id.clone()],
            "the other side of the relation should be unaffected"
        );
    }
}
