//! husmo: a local-first, git-backed document/link database.
//!
//! See `docs/ARCHITECTURE.md` in the repo root for the full design, and
//! `docs/adr/0001-local-first-no-external-services.md` for why this project
//! avoids external services.

pub mod config;
pub mod document;
pub mod git_sync;
pub mod store;
