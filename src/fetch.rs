//! Plain-HTTP fetching of a URL's HTML, per `docs/ARCHITECTURE.md`
//! ("Content extraction"): no headless browser, no JS rendering — just a
//! `reqwest` GET. This module's job ends at "here is the raw HTML that came
//! back"; turning that into Markdown is [`crate::extract`]'s job.

use std::io::Read as _;

/// The maximum response body [`fetch`]/[`fetch_bytes`] will buffer into
/// memory, in bytes, before giving up. Without this, a malicious or
/// misbehaving server (or a URL saved via the `save` tool pointing at one)
/// could exhaust memory with an unbounded — or deliberately huge — response
/// body; 20 MiB comfortably covers any real HTML page or embedded image
/// this tool is meant to save.
const MAX_RESPONSE_BYTES: u64 = 20 * 1024 * 1024;

/// An error encountered while fetching a URL.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FetchError {
    /// The HTTP request itself failed (DNS, connection, TLS, timeout...).
    #[error("failed to fetch {url}: {source}")]
    Request {
        /// The URL that was requested.
        url: String,
        /// The underlying `reqwest` failure.
        #[source]
        source: reqwest::Error,
    },
    /// The server responded, but not with a success status.
    #[error("{url} responded with {status}")]
    Status {
        /// The URL that was requested.
        url: String,
        /// The response status code.
        status: reqwest::StatusCode,
    },
    /// The response body could not be read.
    #[error("failed to read the response body from {url}: {source}")]
    Body {
        /// The URL that was requested.
        url: String,
        /// The underlying I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// The response body exceeded the size limit.
    #[error("response body from {url} exceeded the {limit}-byte limit")]
    TooLarge {
        /// The URL that was requested.
        url: String,
        /// The limit that was exceeded.
        limit: u64,
    },
}

/// Fetches `url` over plain HTTP(S) — no headless browser, no JS rendering,
/// per `docs/ARCHITECTURE.md` ("Content extraction") — and returns the
/// response body as text, reading at most [`MAX_RESPONSE_BYTES`] of it.
///
/// Bytes that aren't valid UTF-8 are replaced with the placeholder
/// character rather than failing the fetch outright — this reads the raw
/// response body directly (to enforce the size cap on the byte stream),
/// bypassing `reqwest`'s own charset-aware text decoding, so a page served
/// in a non-UTF-8 encoding may come through with the placeholder character
/// in place of extended characters; the vast majority of pages are UTF-8.
///
/// # Errors
///
/// Returns [`FetchError::Request`] if the request fails outright,
/// [`FetchError::Status`] if the server responds with a non-success status,
/// [`FetchError::Body`] if the response body can't be read, or
/// [`FetchError::TooLarge`] if it exceeds the size limit.
pub fn fetch(url: &str) -> Result<String, FetchError> {
    let bytes = fetch_bytes_capped(url, MAX_RESPONSE_BYTES)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Fetches `url` over plain HTTP(S) the same way [`fetch`] does, but returns
/// the raw response body bytes instead of decoding it as text — used for
/// binary content such as images (see `crate::images`). Reads at most
/// [`MAX_RESPONSE_BYTES`] of it.
///
/// # Errors
///
/// Returns [`FetchError::Request`] if the request fails outright,
/// [`FetchError::Status`] if the server responds with a non-success status,
/// [`FetchError::Body`] if the response body can't be read, or
/// [`FetchError::TooLarge`] if it exceeds the size limit.
pub fn fetch_bytes(url: &str) -> Result<Vec<u8>, FetchError> {
    fetch_bytes_capped(url, MAX_RESPONSE_BYTES)
}

/// Does the actual work behind [`fetch`]/[`fetch_bytes`], capped at
/// `max_bytes` rather than the fixed [`MAX_RESPONSE_BYTES`] — kept
/// `pub(crate)` and parameterized so tests can exercise the cap itself
/// without downloading tens of megabytes.
pub(crate) fn fetch_bytes_capped(url: &str, max_bytes: u64) -> Result<Vec<u8>, FetchError> {
    let response = reqwest::blocking::get(url).map_err(|source| FetchError::Request {
        url: url.to_string(),
        source,
    })?;

    let status = response.status();
    if !status.is_success() {
        return Err(FetchError::Status {
            url: url.to_string(),
            status,
        });
    }

    // Read one byte past the limit: that lets us tell "exactly at the
    // limit" apart from "over it" by checking the length afterward,
    // without trusting a `Content-Length` header a server could lie about
    // or omit.
    let mut buf = Vec::new();
    response
        .take(max_bytes + 1)
        .read_to_end(&mut buf)
        .map_err(|source| FetchError::Body {
            url: url.to_string(),
            source,
        })?;

    if u64::try_from(buf.len()).is_ok_and(|len| len > max_bytes) {
        return Err(FetchError::TooLarge {
            url: url.to_string(),
            limit: max_bytes,
        });
    }

    Ok(buf)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    /// Starts a one-shot HTTP server on an OS-assigned localhost port that
    /// replies to a single connection with `response` (a full raw HTTP
    /// response, status line included) and then shuts down. Returns the
    /// URL to hit it at.
    fn one_shot_server(response: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind test listener");
        let addr = listener.local_addr().expect("failed to read local addr");
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("failed to accept connection");
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            stream
                .write_all(response.as_bytes())
                .expect("failed to write test response");
        });
        format!("http://{addr}/")
    }

    #[test]
    fn fetch_returns_the_response_body_on_success() {
        let url = one_shot_server(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: text/html\r\n\
             Content-Length: 13\r\n\
             Connection: close\r\n\
             \r\n\
             <p>hello</p>\n",
        );

        let body = fetch(&url).expect("fetch should succeed");

        assert_eq!(body, "<p>hello</p>\n");
    }

    #[test]
    fn fetch_reports_a_non_success_status_as_an_error() {
        let url = one_shot_server(
            "HTTP/1.1 404 Not Found\r\n\
             Content-Length: 0\r\n\
             Connection: close\r\n\
             \r\n",
        );

        let result = fetch(&url);

        assert!(matches!(result, Err(FetchError::Status { .. })));
    }

    #[test]
    fn fetch_reports_a_request_error_when_nothing_is_listening() {
        // Nothing is bound on this port, so the connection itself fails.
        let result = fetch("http://127.0.0.1:1/");

        assert!(matches!(result, Err(FetchError::Request { .. })));
    }

    /// Starts a one-shot HTTP server the same way [`one_shot_server`] does,
    /// but for a raw byte response body rather than a `&'static str` one —
    /// used to exercise [`fetch_bytes`] with non-UTF-8 bytes (a PNG magic
    /// number), which can't be written as a plain string literal.
    fn one_shot_binary_server(header: &'static str, body: &'static [u8]) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind test listener");
        let addr = listener.local_addr().expect("failed to read local addr");
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("failed to accept connection");
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            stream
                .write_all(header.as_bytes())
                .expect("failed to write test response header");
            stream
                .write_all(body)
                .expect("failed to write test response body");
        });
        format!("http://{addr}/")
    }

    #[test]
    fn fetch_bytes_returns_the_response_body_as_raw_bytes() {
        let png_magic_number: &[u8] = &[0x89, b'P', b'N', b'G'];
        let url = one_shot_binary_server(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: image/png\r\n\
             Content-Length: 4\r\n\
             Connection: close\r\n\
             \r\n",
            png_magic_number,
        );

        let body = fetch_bytes(&url).expect("fetch_bytes should succeed");

        assert_eq!(body, png_magic_number);
    }

    #[test]
    fn fetch_bytes_reports_a_non_success_status_as_an_error() {
        let url = one_shot_server(
            "HTTP/1.1 404 Not Found\r\n\
             Content-Length: 0\r\n\
             Connection: close\r\n\
             \r\n",
        );

        let result = fetch_bytes(&url);

        assert!(matches!(result, Err(FetchError::Status { .. })));
    }

    #[test]
    fn fetch_bytes_capped_reports_too_large_when_the_body_exceeds_the_cap() {
        let url = one_shot_server(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: text/plain\r\n\
             Content-Length: 13\r\n\
             Connection: close\r\n\
             \r\n\
             <p>hello</p>\n",
        );

        let result = fetch_bytes_capped(&url, 5);

        assert!(matches!(result, Err(FetchError::TooLarge { limit: 5, .. })));
    }

    #[test]
    fn fetch_bytes_capped_succeeds_when_the_body_is_exactly_at_the_cap() {
        let url = one_shot_server(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: text/plain\r\n\
             Content-Length: 13\r\n\
             Connection: close\r\n\
             \r\n\
             <p>hello</p>\n",
        );

        let body = fetch_bytes_capped(&url, 13).expect("fetch_bytes_capped should succeed");

        assert_eq!(body, b"<p>hello</p>\n");
    }
}
