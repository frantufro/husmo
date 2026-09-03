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
    #[error("file is not valid UTF-8 text: {0}")]
    InvalidUtf8(#[source] std::string::FromUtf8Error),
    /// A `.pdf` file's bytes couldn't be parsed.
    #[error("failed to extract text from PDF content: {0}")]
    Pdf(#[source] pdf_extract::OutputError),
}

/// An error encountered while ingesting a local file.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IngestError {
    /// The file could not be read.
    #[error("failed to read {}: {source}", path.display())]
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
    #[error("failed to extract content from {}: {source}", path.display())]
    Extract {
        /// The path that was ingested.
        path: PathBuf,
        /// The underlying extractor failure.
        #[source]
        source: ExtractError,
    },
    /// The path failed [`PathPolicy`] validation.
    #[error(transparent)]
    PathRestriction(#[from] PathRestrictionError),
}

/// Root directories a `save` call's local-file (`path`) ingestion is
/// allowed to read from, plus the `$HOME` used to derive the default
/// deny-list when no roots are configured. Built from
/// [`crate::config::Config::allowed_source_dirs`] and threaded down from
/// `crate::mcp_server::HusmoServer` through `crate::save::save`.
///
/// This is a deliberate product decision, not an incidental default: an MCP
/// client asking husmo to `save` an arbitrary local path is exactly the
/// shape of a data-exfiltration primitive (read a secret file, commit it to
/// the git-synced data repo), so every local-file save is validated against
/// this policy before a single byte is read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PathPolicy {
    /// When set, a local-file path must canonicalize to somewhere under one
    /// of these roots or the save is rejected. When unset, every path is
    /// allowed except the fixed deny-list under `home` (see
    /// [`validate_source_path`]).
    pub allowed_source_dirs: Option<Vec<PathBuf>>,
    /// The user's home directory, used to derive the default deny-list
    /// (`~/.ssh`, `~/.aws`, `~/.gnupg`, `~/.config`) when
    /// `allowed_source_dirs` is unset. `None` disables that deny-list (there
    /// is no home to derive it from), leaving only allow-list enforcement
    /// active.
    pub home: Option<PathBuf>,
}

/// Well-known secret-bearing directories, relative to `$HOME`, blocked by
/// default when [`PathPolicy::allowed_source_dirs`] is unset. Not
/// exhaustive — it's a safety-net default, not a substitute for configuring
/// `allowed_source_dirs` when the data repo is meant to be shared.
const DEFAULT_BLOCKED_RELATIVE_DIRS: &[&str] = &[".ssh", ".aws", ".gnupg", ".config"];

/// An error encountered while validating a local-file path against a
/// [`PathPolicy`], before [`ingest`] ever reads its bytes.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PathRestrictionError {
    /// The path couldn't be resolved to a canonical, symlink-free path —
    /// most commonly because it doesn't exist.
    #[error("failed to resolve {} to a canonical path: {source}", path.display())]
    Canonicalize {
        /// The path that was given.
        path: PathBuf,
        /// The underlying I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// `allowed_source_dirs` is configured, and the path resolves outside
    /// every root in it.
    #[error(
        "{} is outside every configured allowed_source_dirs root; save is refusing to read it",
        path.display()
    )]
    OutsideAllowedDirs {
        /// The path's canonical form.
        path: PathBuf,
    },
    /// `allowed_source_dirs` is unset, and the path resolves inside one of
    /// the default deny-list's well-known secret-bearing directories.
    #[error(
        "{} is inside {}, a well-known secret-bearing directory husmo refuses to read from by \
         default; set allowed_source_dirs in config.toml to override",
        path.display(), blocked.display()
    )]
    WellKnownSecretDir {
        /// The path's canonical form.
        path: PathBuf,
        /// The specific default-deny-list directory it fell under.
        blocked: PathBuf,
    },
}

/// Validates `path` against `policy` and returns its canonical form, ready
/// for [`ingest`]. Canonicalizing resolves any `..` traversal and any
/// symlink indirection before either check below runs, so both are checked
/// against where the path actually points, not its literal spelling.
///
/// - If `policy.allowed_source_dirs` is `Some`, the canonical path must fall
///   under one of those roots (also canonicalized), or this returns
///   [`PathRestrictionError::OutsideAllowedDirs`].
/// - Otherwise, the canonical path must not fall under one of
///   [`DEFAULT_BLOCKED_RELATIVE_DIRS`] beneath `policy.home` (when `home` is
///   set), or this returns [`PathRestrictionError::WellKnownSecretDir`].
///
/// # Errors
///
/// Returns [`PathRestrictionError::Canonicalize`] if `path` can't be
/// resolved (including if it doesn't exist), or one of the two rejections
/// above.
pub fn validate_source_path(
    path: &Path,
    policy: &PathPolicy,
) -> Result<PathBuf, PathRestrictionError> {
    let canonical = path
        .canonicalize()
        .map_err(|source| PathRestrictionError::Canonicalize {
            path: path.to_path_buf(),
            source,
        })?;

    if let Some(roots) = &policy.allowed_source_dirs {
        let allowed = roots.iter().any(|root| {
            root.canonicalize()
                .is_ok_and(|root| canonical.starts_with(root))
        });
        return if allowed {
            Ok(canonical)
        } else {
            Err(PathRestrictionError::OutsideAllowedDirs { path: canonical })
        };
    }

    if let Some(home) = &policy.home {
        for relative in DEFAULT_BLOCKED_RELATIVE_DIRS {
            let blocked = home.join(relative);
            if let Ok(blocked) = blocked.canonicalize()
                && canonical.starts_with(&blocked)
            {
                return Err(PathRestrictionError::WellKnownSecretDir {
                    path: canonical,
                    blocked,
                });
            }
        }
    }

    Ok(canonical)
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
        use lopdf::{Document, Object, Stream, dictionary};

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
        doc.objects
            .insert(page_tree_id, Object::Dictionary(page_tree));
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

    // -- validate_source_path -------------------------------------------

    #[test]
    fn validate_source_path_allows_anything_when_no_policy_is_configured() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, "hi").expect("failed to write test file");

        let canonical = validate_source_path(&path, &PathPolicy::default())
            .expect("validate_source_path should succeed with no policy configured");

        assert_eq!(
            canonical,
            path.canonicalize().expect("path should canonicalize")
        );
    }

    #[test]
    fn validate_source_path_rejects_a_path_outside_the_allowed_roots() {
        let allowed_dir = tempfile::tempdir().expect("failed to create temp dir");
        let outside_dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = outside_dir.path().join("secret.txt");
        std::fs::write(&path, "secret").expect("failed to write test file");
        let policy = PathPolicy {
            allowed_source_dirs: Some(vec![allowed_dir.path().to_path_buf()]),
            home: None,
        };

        let result = validate_source_path(&path, &policy);

        assert!(matches!(
            result,
            Err(PathRestrictionError::OutsideAllowedDirs { .. })
        ));
    }

    #[test]
    fn validate_source_path_allows_a_path_under_an_allowed_root() {
        let allowed_dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = allowed_dir.path().join("notes.txt");
        std::fs::write(&path, "hi").expect("failed to write test file");
        let policy = PathPolicy {
            allowed_source_dirs: Some(vec![allowed_dir.path().to_path_buf()]),
            home: None,
        };

        let canonical = validate_source_path(&path, &policy)
            .expect("validate_source_path should succeed for a path under an allowed root");

        assert_eq!(
            canonical,
            path.canonicalize().expect("path should canonicalize")
        );
    }

    #[test]
    fn validate_source_path_rejects_a_default_denied_directory_when_no_allow_list_is_set() {
        let home_dir = tempfile::tempdir().expect("failed to create temp dir");
        let ssh_dir = home_dir.path().join(".ssh");
        std::fs::create_dir_all(&ssh_dir).expect("failed to create .ssh dir");
        let path = ssh_dir.join("id_ed25519");
        std::fs::write(&path, "private key material").expect("failed to write test file");
        let policy = PathPolicy {
            allowed_source_dirs: None,
            home: Some(home_dir.path().to_path_buf()),
        };

        let result = validate_source_path(&path, &policy);

        assert!(matches!(
            result,
            Err(PathRestrictionError::WellKnownSecretDir { .. })
        ));
    }

    #[test]
    fn validate_source_path_allows_a_path_outside_the_default_denied_directories() {
        let home_dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = home_dir.path().join("notes.txt");
        std::fs::write(&path, "hi").expect("failed to write test file");
        let policy = PathPolicy {
            allowed_source_dirs: None,
            home: Some(home_dir.path().to_path_buf()),
        };

        let canonical = validate_source_path(&path, &policy)
            .expect("validate_source_path should succeed for a path outside the default deny-list");

        assert_eq!(
            canonical,
            path.canonicalize().expect("path should canonicalize")
        );
    }

    #[test]
    fn validate_source_path_reports_a_clear_error_for_a_missing_file() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let path = dir.path().join("does-not-exist.txt");

        let result = validate_source_path(&path, &PathPolicy::default());

        assert!(matches!(
            result,
            Err(PathRestrictionError::Canonicalize { .. })
        ));
    }
}
