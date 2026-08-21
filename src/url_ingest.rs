//! Shared "fetch a URL and build a Document for it" pipeline, used by both
//! a top-level `save` of a URL (`crate::save`) and archiving a discovered
//! outgoing link (`crate::archive`) — the same fetch -> extract -> resolve
//! existing `canonical_url` -> localize images steps both need. The two
//! callers differ only in what a missing page `<title>` falls back to (the
//! discovered link's own text for `crate::archive`; the URL itself for a
//! top-level `crate::save`) and in what happens after the Document is
//! built (`crate::save` still has tags to apply), so this module stops
//! short of writing anything to disk — that, and wrapping the whole thing
//! in the git pull/commit/push cycle for a top-level save, is left to the
//! caller.

use std::path::Path;

use crate::document::{Document, dedupe_slug};
use crate::extract::{self, OutgoingLink};
use crate::fetch::{self, FetchError};
use crate::images::{self, ImageError};
use crate::store::{self, StoreError};

/// An error encountered while fetching a URL and building its Document.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub(crate) enum UrlIngestError {
    /// Fetching the page failed.
    #[error(transparent)]
    Fetch(#[from] FetchError),
    /// Downloading and localizing one of the page's images failed.
    #[error(transparent)]
    Image(#[from] ImageError),
    /// Reading the store (to resolve an existing Document or list existing
    /// slugs) failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Fetches `url`, extracts it to Markdown, downloads and localizes its
/// images, and builds the resulting Document — reusing the `id`/`slug`/
/// `related` of an existing Document for the same `canonical_url` in `dir`,
/// if any, so the caller overwrites it in place instead of creating a
/// second one, per `docs/ARCHITECTURE.md` ("Storage model"), without
/// losing that Document's `related` edges to a plain re-fetch. The
/// Document's title comes from the fetched page's `<title>`, falling back
/// to `fallback_title` when the page has none.
///
/// Writes nothing to disk — the caller decides when/whether to
/// `crate::store::write` the result (and whether to wrap that in
/// `crate::git_sync::sync_write`).
///
/// # Errors
///
/// Returns [`UrlIngestError::Fetch`] if `url` can't be fetched,
/// [`UrlIngestError::Image`] if one of the page's images can't be
/// downloaded, or [`UrlIngestError::Store`] if resolving an existing
/// Document or listing existing slugs fails.
pub(crate) fn fetch_and_build_document(
    dir: &Path,
    url: &str,
    fallback_title: &str,
) -> Result<(Document, Vec<OutgoingLink>), UrlIngestError> {
    let html = fetch::fetch(url)?;
    let extracted = extract::extract(&html, url);

    let title = extracted
        .title
        .unwrap_or_else(|| fallback_title.to_string());
    let mut document = Document::new(title, String::new());
    document.canonical_url = Some(url.to_string());

    match store::resolve(dir, &store::Identifier::Url(url.to_string())) {
        Ok(existing) => {
            // A Document for this canonical_url already exists — overwrite
            // it in place by reusing its id and slug, rather than creating
            // a second Document for the same URL. Its `related` edges are
            // deliberate, symmetric links to other Documents (set via the
            // `relate`/`unrelate` tools, not derived from this page's own
            // content), so they carry over untouched by a re-fetch.
            document.id = existing.id;
            document.slug = existing.slug;
            document.related = existing.related;
        }
        Err(store::ResolveError::NotFound(_)) => {
            document.slug = dedupe_slug(&document.slug, &store::existing_slugs(dir)?);
        }
        Err(store::ResolveError::Store(source)) => return Err(UrlIngestError::Store(source)),
    }

    document.content = images::localize_images(&extracted.markdown, &extracted.images, dir)?;

    Ok((document, extracted.outgoing_links))
}
