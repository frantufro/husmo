//! Archiving a discovered outgoing link as its own Document, per
//! `docs/ARCHITECTURE.md` ("Content extraction"): "Outgoing links discovered
//! during extraction are returned by `save` as data, not auto-followed. A
//! caller can later archive one as its own Document — one level deep per
//! save, no automatic recursive crawling." This module is that "archive one
//! as its own Document" step: it runs the same fetch -> extract -> localize
//! images -> write pipeline a top-level URL save would, for a single
//! [`OutgoingLink`], and never itself follows the outgoing links its own
//! extraction turns up — those are only ever returned as data on
//! [`ArchivedLink::outgoing_links`], exactly like `save` does for a
//! top-level URL. Wrapping this in the git pull/commit/push cycle (see
//! `crate::git_sync`) is left to the caller, the same way `crate::related`
//! leaves it to the caller.

use std::path::Path;

use crate::document::Document;
use crate::embeddings::{DocumentEmbeddings, EmbeddingsError};
use crate::extract::OutgoingLink;
use crate::fetch::FetchError;
use crate::images::ImageError;
use crate::store::{self, StoreError};
use crate::url_ingest::{UrlIngestError, fetch_and_build_document};

/// An error encountered while archiving an outgoing link.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ArchiveError {
    /// Fetching the link's page failed.
    #[error(transparent)]
    Fetch(#[from] FetchError),
    /// Downloading and localizing one of the page's images failed.
    #[error(transparent)]
    Image(#[from] ImageError),
    /// Reading or writing the new Document (or listing existing slugs)
    /// failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// Writing the new Document's embeddings sidecar failed.
    #[error(transparent)]
    Embeddings(#[from] EmbeddingsError),
}

impl From<UrlIngestError> for ArchiveError {
    fn from(error: UrlIngestError) -> Self {
        match error {
            UrlIngestError::Fetch(source) => ArchiveError::Fetch(source),
            UrlIngestError::Image(source) => ArchiveError::Image(source),
            UrlIngestError::Store(source) => ArchiveError::Store(source),
        }
    }
}

/// The result of archiving one outgoing link: the new Document it became,
/// plus every outgoing link discovered in *that* Document's own content —
/// returned as data only, per this module's docs, never followed further.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ArchivedLink {
    /// The newly created Document, with `canonical_url` set to `link.url`.
    pub document: Document,
    /// Outgoing links discovered in the new Document's own content, in the
    /// order they appear. Reported as data only — archiving one of these in
    /// turn is a separate, explicit call, never done automatically here.
    pub outgoing_links: Vec<OutgoingLink>,
}

/// Archives `link` as its own Document inside `dir`: fetches `link.url`
/// (plain HTTP, per `docs/ARCHITECTURE.md`), extracts it to Markdown,
/// downloads and localizes its images, and writes the result as a Document
/// whose `canonical_url` is `link.url` — one level deep, exactly like a
/// top-level URL save would. Per `docs/ARCHITECTURE.md`'s "Storage model"
/// invariant, a `canonical_url` identifies at most one Document: if one
/// already exists for `link.url` (whether from an earlier archiving of this
/// same link, or because it was already saved as a top-level Document),
/// this overwrites that Document's content in place — reusing its existing
/// `id` and `slug` — instead of minting a second Document for the same URL.
/// The new/updated Document's own outgoing links are reported on the
/// returned [`ArchivedLink::outgoing_links`] rather than archived
/// automatically, so this call never recurses beyond the one link it was
/// asked to archive.
///
/// The Document's title comes from the fetched page's `<title>`, or falls
/// back to `link.text` when the page has none.
///
/// # Errors
///
/// Returns [`ArchiveError::Fetch`] if `link.url` can't be fetched,
/// [`ArchiveError::Image`] if one of the page's images can't be downloaded,
/// or [`ArchiveError::Store`]/[`ArchiveError::Embeddings`] if writing the
/// Document or its embeddings sidecar fails.
pub fn archive_outgoing_link(
    dir: &Path,
    link: &OutgoingLink,
) -> Result<ArchivedLink, ArchiveError> {
    let (document, outgoing_links) = fetch_and_build_document(dir, &link.url, &link.text)?;

    store::write(dir, &document)?;
    let embeddings = DocumentEmbeddings::build(&document)?;
    crate::embeddings::write(dir, &document.slug, &embeddings)?;

    Ok(ArchivedLink {
        document,
        outgoing_links,
    })
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    /// Starts a one-shot HTTP server on an OS-assigned localhost port that
    /// replies to a single connection with an HTML page whose body is
    /// `body`, then shuts down. Mirrors the helper in `crate::fetch`'s
    /// tests.
    fn one_shot_page_server(body: &'static str) -> String {
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
    /// `second_body`. Used to archive the same URL twice against a server
    /// that stays alive for both fetches.
    fn two_shot_page_server(first_body: &'static str, second_body: &'static str) -> String {
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
    fn archive_outgoing_link_creates_a_document_with_the_links_url_as_canonical_url() {
        let url = one_shot_page_server(
            "<html><head><title>Discovered Page</title></head>\
             <body><article><p>Some discovered content.</p></article></body></html>",
        );
        let link = OutgoingLink {
            text: "Discovered".to_string(),
            url: url.clone(),
        };
        let dir = tempfile::tempdir().expect("failed to create temp dir");

        let archived = archive_outgoing_link(dir.path(), &link).expect("archiving should succeed");

        assert_eq!(archived.document.canonical_url, Some(url.clone()));
        assert_eq!(archived.document.title, "Discovered Page");
        assert_eq!(archived.document.content, "Some discovered content.");

        let on_disk = store::resolve(dir.path(), &store::Identifier::Url(url))
            .expect("the archived document should be resolvable from disk");
        assert_eq!(on_disk, archived.document);
    }

    #[test]
    fn archive_outgoing_link_reports_but_does_not_follow_the_new_documents_own_outgoing_links() {
        let further_url = "https://further.example/";
        let page_body = format!(
            "<html><body><article><p>See <a href=\"{further_url}\">more</a>.</p></article></body></html>"
        );
        let url = one_shot_page_server(Box::leak(page_body.into_boxed_str()));
        let link = OutgoingLink {
            text: "Discovered".to_string(),
            url: url.clone(),
        };
        let dir = tempfile::tempdir().expect("failed to create temp dir");

        let archived = archive_outgoing_link(dir.path(), &link).expect("archiving should succeed");

        assert_eq!(
            archived.outgoing_links,
            vec![OutgoingLink {
                text: "more".to_string(),
                url: further_url.to_string(),
            }],
            "the newly archived document's own outgoing link should be reported as data"
        );

        let further_lookup =
            store::resolve(dir.path(), &store::Identifier::Url(further_url.to_string()));
        assert!(
            matches!(further_lookup, Err(store::ResolveError::NotFound(_))),
            "archiving should not itself follow the discovered link and create a Document for it"
        );

        let all_documents = store::load_all(dir.path()).expect("load_all should succeed");
        assert_eq!(
            all_documents.len(),
            1,
            "only the one archived Document should exist — no automatic recursion"
        );
    }

    #[test]
    fn archive_outgoing_link_overwrites_an_existing_document_for_the_same_url_in_place() {
        let url = two_shot_page_server(
            "<html><head><title>Discovered Page</title></head>\
             <body><article><p>Some discovered content.</p></article></body></html>",
            "<html><head><title>Discovered Page</title></head>\
             <body><article><p>Updated discovered content.</p></article></body></html>",
        );
        let link = OutgoingLink {
            text: "Discovered".to_string(),
            url: url.clone(),
        };
        let dir = tempfile::tempdir().expect("failed to create temp dir");

        let first =
            archive_outgoing_link(dir.path(), &link).expect("first archiving should succeed");

        let second =
            archive_outgoing_link(dir.path(), &link).expect("second archiving should succeed");

        assert_eq!(
            second.document.id, first.document.id,
            "re-archiving the same url should reuse the existing Document's id"
        );
        assert_eq!(
            second.document.slug, first.document.slug,
            "re-archiving the same url should reuse the existing Document's slug"
        );
        assert_eq!(
            second.document.content, "Updated discovered content.",
            "re-archiving the same url should overwrite the existing Document's content"
        );

        let all_documents = store::load_all(dir.path()).expect("load_all should succeed");
        assert_eq!(
            all_documents.len(),
            1,
            "re-archiving the same url should not create a second Document"
        );

        let on_disk = store::resolve(dir.path(), &store::Identifier::Url(url))
            .expect("the document should still be resolvable by url");
        assert_eq!(on_disk, second.document);
    }
}
