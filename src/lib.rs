//! husmo: a local-first, git-backed document/link database.
//!
//! See `docs/ARCHITECTURE.md` in the repo root for the full design, and
//! `docs/adr/0001-local-first-no-external-services.md` for why this project
//! avoids external services.

pub mod chunk;
pub mod config;
pub mod document;
pub mod embed;
pub mod embeddings;
pub mod extract;
pub mod fetch;
pub mod git_sync;
pub mod images;
pub mod local_file;
pub mod pasted_text;
pub mod store;
