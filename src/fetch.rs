//! Plain-HTTP fetching of a URL's HTML, per `docs/ARCHITECTURE.md`
//! ("Content extraction"): no headless browser, no JS rendering — just a
//! `reqwest` GET. This module's job ends at "here is the raw HTML that came
//! back"; turning that into Markdown is [`crate::extract`]'s job.

/// An error encountered while fetching a URL.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FetchError {
    /// The HTTP request itself failed (DNS, connection, TLS, timeout...).
    #[error("failed to fetch {url}")]
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
    #[error("failed to read the response body from {url}")]
    Body {
        /// The URL that was requested.
        url: String,
        /// The underlying `reqwest` failure.
        #[source]
        source: reqwest::Error,
    },
}

/// Fetches `url` over plain HTTP(S) — no headless browser, no JS rendering,
/// per `docs/ARCHITECTURE.md` ("Content extraction") — and returns the
/// response body as text.
///
/// # Errors
///
/// Returns [`FetchError::Request`] if the request fails outright,
/// [`FetchError::Status`] if the server responds with a non-success status,
/// or [`FetchError::Body`] if the response body can't be read.
pub fn fetch(url: &str) -> Result<String, FetchError> {
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

    response.text().map_err(|source| FetchError::Body {
        url: url.to_string(),
        source,
    })
}

/// Fetches `url` over plain HTTP(S) the same way [`fetch`] does, but returns
/// the raw response body bytes instead of decoding it as text — used for
/// binary content such as images (see `crate::images`).
///
/// # Errors
///
/// Returns [`FetchError::Request`] if the request fails outright,
/// [`FetchError::Status`] if the server responds with a non-success status,
/// or [`FetchError::Body`] if the response body can't be read.
pub fn fetch_bytes(url: &str) -> Result<Vec<u8>, FetchError> {
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

    let bytes = response.bytes().map_err(|source| FetchError::Body {
        url: url.to_string(),
        source,
    })?;
    Ok(bytes.to_vec())
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
}
