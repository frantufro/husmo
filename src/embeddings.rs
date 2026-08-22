//! Per-Document chunk-embedding sidecar files, per `docs/ARCHITECTURE.md`
//! ("Retrieval"): "Each Document's chunk embeddings are committed to the
//! data repo as small per-Document sidecar files — not one giant shared
//! blob." Combines [`crate::chunk`] and [`crate::embed`] to build a
//! [`DocumentEmbeddings`] for a Document, and serializes it to/from a
//! small YAML sidecar file that sits alongside the Document's own
//! `{slug}.md` file (see [`crate::store`]).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::chunk::chunk;
use crate::document::Document;
use crate::embed::{EmbedError, embed};

/// File extension used for chunk-embedding sidecar files, alongside each
/// Document's own `{slug}.md` file.
const EXTENSION: &str = "embeddings.yaml";

/// One chunk of a Document's content, paired with its embedding vector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkEmbedding {
    /// The chunk's own text, kept alongside its vector so a search hit
    /// can report which portion of the Document matched without a
    /// second read of the source Markdown.
    pub chunk: String,
    /// The chunk's embedding vector (see [`crate::embed::EMBEDDING_DIM`]).
    pub vector: Vec<f32>,
}

/// A Document's full set of chunk embeddings — the sidecar file's shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentEmbeddings {
    /// The parent Document's stable id (see `Document::id`), so a
    /// sidecar file stays attributable to its Document even though this
    /// file is itself named after the Document's slug, which can change
    /// on rename.
    pub document_id: String,
    /// Every chunk's embedding, in the order [`crate::chunk::chunk`]
    /// produced them.
    pub chunks: Vec<ChunkEmbedding>,
}

impl DocumentEmbeddings {
    /// Chunks `document`'s content (via [`crate::chunk::chunk`]) and
    /// embeds every chunk (via [`crate::embed::embed`]), producing the
    /// full sidecar payload for it.
    ///
    /// # Errors
    ///
    /// Returns [`EmbeddingsError::Embed`] if embedding any of the
    /// Document's chunks fails.
    pub fn build(document: &Document) -> Result<Self, EmbeddingsError> {
        let chunks = chunk(&document.content)
            .into_iter()
            .map(|text| Ok(ChunkEmbedding { vector: embed(&text)?, chunk: text }))
            .collect::<Result<Vec<_>, EmbedError>>()?;
        Ok(DocumentEmbeddings {
            document_id: document.id.clone(),
            chunks,
        })
    }

    /// Serializes this value to the sidecar file's YAML contents.
    ///
    /// # Panics
    ///
    /// Panics if serialization to YAML fails, which isn't expected to
    /// happen for any value a `DocumentEmbeddings`'s fields can hold.
    #[must_use]
    pub fn to_yaml(&self) -> String {
        serde_norway::to_string(self).expect("DocumentEmbeddings always serializes to YAML")
    }

    /// Parses a sidecar file's YAML contents back into a
    /// `DocumentEmbeddings`.
    ///
    /// # Errors
    ///
    /// Returns an error if `input` is not valid YAML matching the
    /// expected shape.
    pub fn from_yaml(input: &str) -> Result<Self, serde_norway::Error> {
        serde_norway::from_str(input)
    }
}

/// An error encountered while building, reading, or writing a
/// chunk-embedding sidecar file.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EmbeddingsError {
    /// The sidecar file could not be written.
    #[error("failed to write {}: {source}", path.display())]
    Write {
        /// The path that was written.
        path: PathBuf,
        /// The underlying I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// The sidecar file could not be read.
    #[error("failed to read {}: {source}", path.display())]
    Read {
        /// The path that was read.
        path: PathBuf,
        /// The underlying I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// The sidecar file's contents didn't parse.
    #[error("embeddings sidecar file at {} is malformed: {source}", path.display())]
    Malformed {
        /// The path that was parsed.
        path: PathBuf,
        /// The underlying parse failure.
        #[source]
        source: serde_norway::Error,
    },
    /// The sidecar file could not be removed.
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
    /// A chunk's embedding vector could not be computed.
    #[error(transparent)]
    Embed(#[from] EmbedError),
}

/// The path a sidecar file for `slug` would live at inside `dir`, e.g.
/// `dir/{slug}.embeddings.yaml` next to `dir/{slug}.md`.
#[must_use]
pub fn sidecar_path(dir: &Path, slug: &str) -> PathBuf {
    dir.join(format!("{slug}.{EXTENSION}"))
}

/// Writes `embeddings`'s sidecar file for `slug` into `dir`, overwriting
/// any existing file with that name. Returns the path written to.
///
/// # Errors
///
/// Returns [`EmbeddingsError::Write`] if the file can't be written.
pub fn write(
    dir: &Path,
    slug: &str,
    embeddings: &DocumentEmbeddings,
) -> Result<PathBuf, EmbeddingsError> {
    let path = sidecar_path(dir, slug);
    std::fs::write(&path, embeddings.to_yaml()).map_err(|source| EmbeddingsError::Write {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

/// Removes `slug`'s sidecar file from `dir`, if one exists. A no-op, not an
/// error, when no sidecar file exists for `slug` — not every Document
/// necessarily has one (e.g. one written directly rather than through
/// `crate::save`).
///
/// # Errors
///
/// Returns [`EmbeddingsError::Remove`] if the file exists but can't be
/// removed.
pub fn remove(dir: &Path, slug: &str) -> Result<(), EmbeddingsError> {
    let path = sidecar_path(dir, slug);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(EmbeddingsError::Remove { path, source }),
    }
}

/// Reads and parses the sidecar file at `path`.
///
/// # Errors
///
/// Returns [`EmbeddingsError::Read`] if the file can't be read, or
/// [`EmbeddingsError::Malformed`] if its contents aren't a valid sidecar
/// payload.
pub fn read(path: &Path) -> Result<DocumentEmbeddings, EmbeddingsError> {
    let contents = std::fs::read_to_string(path).map_err(|source| EmbeddingsError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    DocumentEmbeddings::from_yaml(&contents).map_err(|source| EmbeddingsError::Malformed {
        path: path.to_path_buf(),
        source,
    })
}

/// Loads every chunk-embedding sidecar file (`*.{EXTENSION}`) directly
/// inside `dir`, per `docs/ARCHITECTURE.md` ("Retrieval"): the in-process
/// vector index is rebuilt at startup from these committed sidecar files.
///
/// # Errors
///
/// Returns [`EmbeddingsError::ListDir`] if `dir` can't be listed, or any
/// error [`read`] can return for one of its entries.
pub fn load_all(dir: &Path) -> Result<Vec<DocumentEmbeddings>, EmbeddingsError> {
    let entries = std::fs::read_dir(dir).map_err(|source| EmbeddingsError::ListDir {
        path: dir.to_path_buf(),
        source,
    })?;

    let mut sidecars = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| EmbeddingsError::ListDir {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        let is_sidecar = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(&format!(".{EXTENSION}")));
        if is_sidecar {
            sidecars.push(read(&path)?);
        }
    }
    Ok(sidecars)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_yaml_then_from_yaml_round_trips_document_embeddings() {
        let original = DocumentEmbeddings {
            document_id: "some-id".to_string(),
            chunks: vec![
                ChunkEmbedding {
                    chunk: "First chunk.".to_string(),
                    vector: vec![0.5, -0.5],
                },
                ChunkEmbedding {
                    chunk: "Second chunk.".to_string(),
                    vector: vec![1.0, 0.0],
                },
            ],
        };

        let yaml = original.to_yaml();
        let parsed = DocumentEmbeddings::from_yaml(&yaml).expect("yaml should parse back");

        assert_eq!(parsed, original);
    }

    #[test]
    fn build_chunks_and_embeds_a_documents_content() {
        let document = Document::new("My Title", "First paragraph.\n\nSecond paragraph.");

        let embeddings = DocumentEmbeddings::build(&document).expect("build should succeed");

        assert_eq!(embeddings.document_id, document.id);
        assert_eq!(
            embeddings.chunks,
            vec![
                ChunkEmbedding {
                    chunk: "First paragraph.".to_string(),
                    vector: embed("First paragraph.").expect("embed should succeed"),
                },
                ChunkEmbedding {
                    chunk: "Second paragraph.".to_string(),
                    vector: embed("Second paragraph.").expect("embed should succeed"),
                },
            ]
        );
    }

    #[test]
    fn write_then_read_round_trips_a_sidecar_file() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let document = Document::new("My Title", "Some content.\n\nMore content.");
        let embeddings = DocumentEmbeddings::build(&document).expect("build should succeed");

        let path =
            write(dir.path(), &document.slug, &embeddings).expect("write should succeed");
        let loaded = read(&path).expect("read should succeed");

        assert_eq!(loaded, embeddings);
        assert_eq!(path, dir.path().join("my-title.embeddings.yaml"));
    }

    #[test]
    fn remove_deletes_a_sidecar_file() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let document = Document::new("My Title", "content\n");
        let embeddings = DocumentEmbeddings::build(&document).expect("build should succeed");
        let path = write(dir.path(), &document.slug, &embeddings).expect("write should succeed");
        assert!(path.is_file());

        remove(dir.path(), &document.slug).expect("remove should succeed");

        assert!(!path.is_file());
    }

    #[test]
    fn remove_is_a_no_op_when_no_sidecar_file_exists() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");

        remove(dir.path(), "does-not-exist").expect("remove should succeed even with no sidecar file");
    }

    #[test]
    fn sidecar_path_sits_alongside_the_documents_own_md_file() {
        let dir = Path::new("/data-repo");

        assert_eq!(
            sidecar_path(dir, "my-title"),
            Path::new("/data-repo/my-title.embeddings.yaml")
        );
    }

    #[test]
    fn load_all_reads_every_sidecar_file_in_a_directory() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let one = DocumentEmbeddings::build(&Document::new("One", "First document."))
            .expect("build should succeed");
        let two = DocumentEmbeddings::build(&Document::new("Two", "Second document."))
            .expect("build should succeed");
        write(dir.path(), "one", &one).expect("write should succeed");
        write(dir.path(), "two", &two).expect("write should succeed");
        std::fs::write(dir.path().join("one.md"), "not a sidecar file")
            .expect("failed to write stray file");

        let mut loaded = load_all(dir.path()).expect("load_all should succeed");
        loaded.sort_by(|a, b| a.document_id.cmp(&b.document_id));
        let mut expected = vec![one, two];
        expected.sort_by(|a, b| a.document_id.cmp(&b.document_id));

        assert_eq!(loaded, expected);
    }
}
