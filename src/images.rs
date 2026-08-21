//! Downloading image bytes discovered during extraction and localizing the
//! Markdown that references them, per `docs/ARCHITECTURE.md` ("Content
//! extraction"): "Images are downloaded (actual bytes, not just
//! referenced) and stored as local files alongside the Document; the
//! Markdown is rewritten to point at the local copies." [`crate::extract`]
//! only resolves each image's URL and records it as data on
//! [`crate::extract::Extracted::images`] — this module is where the actual
//! network fetch and filesystem write happen.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::extract::ExtractedImage;
use crate::fetch::{self, FetchError};

/// An error encountered while downloading and localizing images.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ImageError {
    /// Downloading an image's bytes failed.
    #[error("failed to download image from {url}")]
    Fetch {
        /// The image URL that was requested.
        url: String,
        /// The underlying fetch failure.
        #[source]
        source: FetchError,
    },
    /// Writing the downloaded bytes to disk failed.
    #[error("failed to write image to {}", path.display())]
    Write {
        /// The path that was written.
        path: PathBuf,
        /// The underlying I/O failure.
        #[source]
        source: std::io::Error,
    },
}

/// Downloads every image in `images`, saving each one as its own file
/// inside `dest_dir`, and returns `markdown` with each image's remote URL
/// replaced by the local filename it was saved under — a path relative to
/// `dest_dir`, ready to sit alongside the Document per
/// `docs/ARCHITECTURE.md`.
///
/// Local filenames are derived from each URL's last path segment, deduped
/// against collisions (including within the same batch) by appending a
/// counter before the extension, the same way Document slugs are deduped
/// (see [`crate::document::dedupe_slug`]).
///
/// Two entries that share the same URL are downloaded and written only
/// once: `markdown.replace` already rewrites every occurrence of that URL
/// the first time it's encountered, so re-downloading it for a later
/// duplicate would just write an orphan, unreferenced file to `dest_dir`.
///
/// An image whose URL isn't `http`/`https` (a `data:` URI, say) is left
/// untouched — there are no bytes to fetch for it, and `markdown` already
/// carries it inline.
///
/// # Errors
///
/// Returns [`ImageError::Fetch`] if an image's bytes can't be downloaded, or
/// [`ImageError::Write`] if the bytes can't be written into `dest_dir`.
pub fn localize_images(
    markdown: &str,
    images: &[ExtractedImage],
    dest_dir: &Path,
) -> Result<String, ImageError> {
    let mut markdown = markdown.to_string();
    let mut used_filenames = HashSet::new();
    let mut localized_urls = HashSet::new();

    for image in images {
        if !is_fetchable(&image.url) {
            continue;
        }

        if !localized_urls.insert(image.url.clone()) {
            // Already downloaded, written, and replaced throughout
            // `markdown` for an earlier entry with this same URL.
            continue;
        }

        let bytes = fetch::fetch_bytes(&image.url).map_err(|source| ImageError::Fetch {
            url: image.url.clone(),
            source,
        })?;

        let filename = local_filename(&image.url, &used_filenames);

        let path = dest_dir.join(&filename);
        std::fs::write(&path, &bytes).map_err(|source| ImageError::Write { path, source })?;

        markdown = markdown.replace(&image.url, &filename);
        used_filenames.insert(filename);
    }

    Ok(markdown)
}

/// True when `url` uses a scheme that can actually be downloaded over the
/// network (`http`/`https`). A `data:` URI, for instance, carries its bytes
/// inline rather than pointing at a server to fetch them from, so
/// `fetch::fetch_bytes` has nothing to ask for and would only fail on it.
fn is_fetchable(url: &str) -> bool {
    url::Url::parse(url)
        .map(|url| url.scheme() == "http" || url.scheme() == "https")
        .unwrap_or(false)
}

/// Derives a local filename for `url` from its last path segment, deduped
/// against `existing` by appending `-2`, `-3`, ... before the extension
/// (e.g. `photo.jpg` -> `photo-2.jpg`) until a free name is found. Falls
/// back to `"image"` if the URL has no usable path segment.
fn local_filename(url: &str, existing: &HashSet<String>) -> String {
    let base = url::Url::parse(url)
        .ok()
        .and_then(|url| url.path_segments()?.next_back().map(str::to_string))
        .filter(|segment| !segment.is_empty())
        .unwrap_or_else(|| "image".to_string());

    if !existing.contains(&base) {
        return base;
    }

    let (stem, extension) = base
        .rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map_or((base.as_str(), None), |(stem, ext)| (stem, Some(ext)));

    let mut counter = 2u64;
    loop {
        let candidate = match extension {
            Some(extension) => format!("{stem}-{counter}.{extension}"),
            None => format!("{stem}-{counter}"),
        };
        if !existing.contains(&candidate) {
            return candidate;
        }
        counter += 1;
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    /// Starts a one-shot HTTP server on an OS-assigned localhost port that
    /// replies to a single connection with a 200 response carrying `body`,
    /// then shuts down. `path_name` becomes the last path segment of the
    /// returned URL (the server itself ignores the request path — it
    /// replies with the same fixed response regardless).
    fn one_shot_image_server(body: &'static [u8], path_name: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind test listener");
        let addr = listener.local_addr().expect("failed to read local addr");
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("failed to accept connection");
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream
                .write_all(header.as_bytes())
                .expect("failed to write test response header");
            stream
                .write_all(body)
                .expect("failed to write test response body");
        });
        format!("http://{addr}/{path_name}")
    }

    #[test]
    fn localize_images_downloads_bytes_and_rewrites_markdown_to_local_paths() {
        let image_bytes: &[u8] = b"fake-png-bytes";
        let url = one_shot_image_server(image_bytes, "cat.png");
        let images = vec![ExtractedImage {
            alt: "A cat".to_string(),
            url: url.clone(),
        }];
        let markdown = format!("![A cat]({url})");
        let dest_dir = tempfile::tempdir().expect("failed to create temp dir");

        let rewritten = localize_images(&markdown, &images, dest_dir.path())
            .expect("localize_images should succeed");

        assert_eq!(rewritten, "![A cat](cat.png)");
        let saved =
            std::fs::read(dest_dir.path().join("cat.png")).expect("image file should exist");
        assert_eq!(saved, image_bytes);
    }

    #[test]
    fn localize_images_dedupes_filenames_that_would_otherwise_collide() {
        let first_bytes: &[u8] = b"first";
        let second_bytes: &[u8] = b"second";
        let first_url = one_shot_image_server(first_bytes, "photo.jpg");
        let second_url = one_shot_image_server(second_bytes, "photo.jpg");
        let images = vec![
            ExtractedImage {
                alt: "First".to_string(),
                url: first_url.clone(),
            },
            ExtractedImage {
                alt: "Second".to_string(),
                url: second_url.clone(),
            },
        ];
        let markdown = format!("![First]({first_url})\n\n![Second]({second_url})");
        let dest_dir = tempfile::tempdir().expect("failed to create temp dir");

        let rewritten = localize_images(&markdown, &images, dest_dir.path())
            .expect("localize_images should succeed");

        assert_eq!(rewritten, "![First](photo.jpg)\n\n![Second](photo-2.jpg)");
        assert_eq!(
            std::fs::read(dest_dir.path().join("photo.jpg")).expect("first image should exist"),
            first_bytes
        );
        assert_eq!(
            std::fs::read(dest_dir.path().join("photo-2.jpg"))
                .expect("second image should exist"),
            second_bytes
        );
    }

    #[test]
    fn localize_images_leaves_non_http_image_urls_untouched() {
        let images = vec![ExtractedImage {
            alt: "Inline".to_string(),
            url: "data:image/png;base64,aGVsbG8=".to_string(),
        }];
        let markdown = "![Inline](data:image/png;base64,aGVsbG8=)";
        let dest_dir = tempfile::tempdir().expect("failed to create temp dir");

        let rewritten = localize_images(markdown, &images, dest_dir.path())
            .expect("localize_images should succeed");

        assert_eq!(rewritten, markdown, "a data: URI has nothing to fetch, so it's left as-is");
        assert_eq!(
            std::fs::read_dir(dest_dir.path())
                .expect("dest dir should be readable")
                .count(),
            0,
            "no file should have been written for a non-http image"
        );
    }

    #[test]
    fn localize_images_downloads_a_repeated_url_only_once() {
        let image_bytes: &[u8] = b"shared-bytes";
        let url = one_shot_image_server(image_bytes, "shared.png");
        let images = vec![
            ExtractedImage {
                alt: "First".to_string(),
                url: url.clone(),
            },
            ExtractedImage {
                alt: "Second".to_string(),
                url: url.clone(),
            },
        ];
        let markdown = format!("![First]({url})\n\n![Second]({url})");
        let dest_dir = tempfile::tempdir().expect("failed to create temp dir");

        let rewritten = localize_images(&markdown, &images, dest_dir.path())
            .expect("localize_images should succeed");

        assert_eq!(
            rewritten,
            "![First](shared.png)\n\n![Second](shared.png)",
            "both references to the same URL should point at the same local file"
        );
        let entries: Vec<_> = std::fs::read_dir(dest_dir.path())
            .expect("dest dir should be readable")
            .collect();
        assert_eq!(
            entries.len(),
            1,
            "a repeated URL should only be downloaded and written once, not left behind as an \
             orphan duplicate file"
        );
    }

    #[test]
    fn localize_images_reports_a_fetch_error_without_writing_anything() {
        let images = vec![ExtractedImage {
            alt: "Nope".to_string(),
            // Nothing is listening on this port, so the request fails.
            url: "http://127.0.0.1:1/no-such.png".to_string(),
        }];
        let dest_dir = tempfile::tempdir().expect("failed to create temp dir");

        let result = localize_images(
            "![Nope](http://127.0.0.1:1/no-such.png)",
            &images,
            dest_dir.path(),
        );

        assert!(matches!(result, Err(ImageError::Fetch { .. })));
        assert_eq!(
            std::fs::read_dir(dest_dir.path())
                .expect("dest dir should be readable")
                .count(),
            0,
            "no file should have been written for a failed download"
        );
    }
}
