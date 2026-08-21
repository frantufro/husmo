//! Local file ingestion, per `docs/ARCHITECTURE.md` ("Content extraction"):
//! "Local file ingestion uses an extensible per-file-type extractor. Start
//! with plain text and PDF; more formats slot in behind the same interface
//! later."

use std::path::{Path, PathBuf};

/// One file-type-specific extractor: converts a file's raw bytes into its
/// text content, ready to become a Document's Markdown body.
type Extractor = fn(&[u8]) -> Result<String, ExtractError>;

/// Extractors registered so far, keyed by lowercased file extension
/// (without the leading `.`), checked in order by [`extractor_for`]. Adding
/// a new format means adding one entry here — [`ingest`]'s signature, and
/// every caller of it, stays the same.
const EXTRACTORS: &[(&str, Extractor)] = &[("txt", extract_text), ("pdf", extract_pdf)];

/// Looks up the extractor registered for `extension` in [`EXTRACTORS`], if
/// any.
fn extractor_for(extension: &str) -> Option<Extractor> {
    EXTRACTORS
        .iter()
        .find(|(ext, _)| *ext == extension)
        .map(|(_, extractor)| *extractor)
}

/// Extracts `bytes` as plain text: it's already exactly the content a
/// Document's Markdown body should hold.
///
/// # Errors
///
/// Returns [`ExtractError::InvalidUtf8`] if `bytes` isn't valid UTF-8.
fn extract_text(bytes: &[u8]) -> Result<String, ExtractError> {
    String::from_utf8(bytes.to_vec()).map_err(ExtractError::InvalidUtf8)
}

/// Extracts `bytes` (a PDF file's raw contents) to plain text via
/// `pdf-extract`.
///
/// # Errors
///
/// Returns [`ExtractError::Pdf`] if the PDF can't be parsed.
fn extract_pdf(bytes: &[u8]) -> Result<String, ExtractError> {
    pdf_extract::extract_text_from_mem(bytes).map_err(ExtractError::Pdf)
}

/// An error encountered while running a specific file type's extractor,
/// wrapped by [`IngestError::Extract`] with the path it happened on.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ExtractError {
    /// A `.txt` file's bytes weren't valid UTF-8.
    #[error("file is not valid UTF-8 text")]
    InvalidUtf8(#[source] std::string::FromUtf8Error),
    /// A `.pdf` file's bytes couldn't be parsed.
    #[error("failed to extract text from PDF content")]
    Pdf(#[source] pdf_extract::OutputError),
}

/// An error encountered while ingesting a local file.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IngestError {
    /// The file could not be read.
    #[error("failed to read {}", path.display())]
    Read {
        /// The path that was read.
        path: PathBuf,
        /// The underlying I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// No extractor is registered for the file's extension.
    #[error(
        "unsupported file type \"{}\" — no extractor is registered for it",
        extension.as_deref().unwrap_or("(no extension)")
    )]
    UnsupportedType {
        /// The file's lowercased extension, or `None` if it had none.
        extension: Option<String>,
    },
    /// The file's extension was recognized, but its own extractor failed.
    #[error("failed to extract content from {}", path.display())]
    Extract {
        /// The path that was ingested.
        path: PathBuf,
        /// The underlying extractor failure.
        #[source]
        source: ExtractError,
    },
}

/// Ingests the local file at `path`: reads its bytes and dispatches to the
/// extractor registered for its extension in [`EXTRACTORS`], returning the
/// extracted text ready to become a Document's Markdown body.
///
/// # Errors
///
/// Returns [`IngestError::Read`] if `path` can't be read,
/// [`IngestError::UnsupportedType`] if no extractor is registered for its
/// extension, or [`IngestError::Extract`] if the registered extractor fails
/// on its content.
pub fn ingest(path: &Path) -> Result<String, IngestError> {
    let bytes = std::fs::read(path).map_err(|source| IngestError::Read {
        path: path.to_path_buf(),
        source,
    })?;

    let extension = path
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .map(str::to_lowercase);

    let extractor = extension
        .as_deref()
        .and_then(extractor_for)
        .ok_or_else(|| IngestError::UnsupportedType {
            extension: extension.clone(),
        })?;

    extractor(&bytes).map_err(|source| IngestError::Extract {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingest_reads_a_plain_text_file_as_is() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, "Some plain text notes.\n").expect("failed to write test file");

        let content = ingest(&path).expect("ingest should succeed");

        assert_eq!(content, "Some plain text notes.\n");
    }

    /// Builds a minimal one-page PDF whose content stream prints `text`,
    /// and saves it at `path`.
    fn write_minimal_pdf(path: &Path, text: &str) {
        use lopdf::content::{Content, Operation};
        use lopdf::{dictionary, Document, Object, Stream};

        let mut doc = Document::with_version("1.5");
        let page_tree_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Courier",
        });
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 24.into()]),
                Operation::new("Td", vec![72.into(), 700.into()]),
                Operation::new("Tj", vec![Object::string_literal(text)]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(
            dictionary! {},
            content.encode().expect("failed to encode test PDF content"),
        ));
        let single_page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => page_tree_id,
            "Contents" => content_id,
        });
        let page_tree = dictionary! {
            "Type" => "Pages",
            "Kids" => vec![single_page_id.into()],
            "Count" => 1,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
        };
        doc.objects.insert(page_tree_id, Object::Dictionary(page_tree));
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => page_tree_id,
        });
        doc.trailer.set("Root", catalog_id);
        doc.save(path).expect("failed to save test PDF");
    }

    #[test]
    fn ingest_extracts_text_from_a_pdf_file() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("report.pdf");
        write_minimal_pdf(&path, "Hello from a PDF");

        let content = ingest(&path).expect("ingest should succeed");

        assert!(
            content.contains("Hello from a PDF"),
            "expected extracted PDF text to contain the page's text, got {content:?}"
        );
    }

    #[test]
    fn ingest_reports_a_clear_error_for_an_unsupported_file_type() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("archive.docx");
        std::fs::write(&path, b"not actually a docx").expect("failed to write test file");

        let result = ingest(&path);

        match result {
            Err(IngestError::UnsupportedType { extension }) => {
                assert_eq!(extension, Some("docx".to_string()));
            }
            other => panic!("expected Err(IngestError::UnsupportedType), got {other:?}"),
        }
    }

    #[test]
    fn ingest_reports_a_clear_error_for_a_file_with_no_extension() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("README");
        std::fs::write(&path, b"hello").expect("failed to write test file");

        let result = ingest(&path);

        match result {
            Err(IngestError::UnsupportedType { extension }) => {
                assert_eq!(extension, None);
            }
            other => panic!("expected Err(IngestError::UnsupportedType), got {other:?}"),
        }
    }
}
