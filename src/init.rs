//! `husmo init`: bootstraps a data repo on a new machine/folder, per
//! `docs/ARCHITECTURE.md` ("Bootstrapping a data repo: `husmo init`"). Not
//! an MCP tool — a CLI subcommand run once, before the MCP server ever
//! starts.

use std::io;
use std::path::{Path, PathBuf};

use crate::config::Config;

/// An error encountered while running `husmo init`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum InitError {
    /// Prompting the user interactively for the URL failed.
    #[error("failed to read the data repo URL from stdin")]
    Prompt(#[source] io::Error),
    /// `dest_dir` already exists and has files in it — refusing to clone
    /// into it and risk clobbering whatever is already there.
    #[error(
        "{} already exists and is not empty; refusing to clone into it",
        path.display()
    )]
    DestinationNotEmpty {
        /// The non-empty destination directory.
        path: PathBuf,
    },
    /// Checking whether `dest_dir` is empty failed.
    #[error("failed to check whether {} is empty", path.display())]
    ReadDestination {
        /// The directory that could not be read.
        path: PathBuf,
        /// The underlying I/O failure.
        #[source]
        source: io::Error,
    },
    /// Cloning the data repo failed.
    #[error("failed to clone {url} into {}", dest.display())]
    Clone {
        /// The git URL that was cloned.
        url: String,
        /// The destination directory the clone was attempted into.
        dest: PathBuf,
        /// The underlying git failure.
        #[source]
        source: git2::Error,
    },
    /// Serializing the config to TOML failed.
    #[error("failed to serialize config")]
    SerializeConfig(#[source] toml::ser::Error),
    /// Creating the config file's parent directory failed.
    #[error("failed to create config directory {}", path.display())]
    CreateConfigDir {
        /// The directory that could not be created.
        path: PathBuf,
        /// The underlying I/O failure.
        #[source]
        source: io::Error,
    },
    /// Writing the config file failed.
    #[error("failed to write config file at {}", path.display())]
    WriteConfig {
        /// The path that was written.
        path: PathBuf,
        /// The underlying I/O failure.
        #[source]
        source: io::Error,
    },
}

/// Resolves the data repo's git URL to clone: `flag` (from `--repo <url>`)
/// when given, otherwise falls back to interactively prompting via
/// `prompt`, per `docs/ARCHITECTURE.md` ("Bootstrapping a data repo").
///
/// # Errors
///
/// Returns [`InitError::Prompt`] if `flag` is `None` and `prompt` fails.
pub fn resolve_repo_url(
    flag: Option<String>,
    prompt: impl FnOnce() -> io::Result<String>,
) -> Result<String, InitError> {
    match flag {
        Some(url) => Ok(url),
        None => prompt().map_err(InitError::Prompt),
    }
}

/// Runs `husmo init`: clones `repo_url` into `dest_dir`, then writes a
/// config file at `config_path` pointing at the clone, per
/// `docs/ARCHITECTURE.md` ("Bootstrapping a data repo") — so the app has a
/// working data repo location immediately after `init` completes.
///
/// # Errors
///
/// Returns [`InitError::DestinationNotEmpty`] if `dest_dir` already exists
/// and has files in it, [`InitError::Clone`] if `repo_url` can't be cloned
/// into `dest_dir`, or [`InitError::CreateConfigDir`] /
/// [`InitError::WriteConfig`] if the config file can't be written
/// afterward.
pub fn run(repo_url: &str, dest_dir: &Path, config_path: &Path) -> Result<(), InitError> {
    ensure_empty(dest_dir)?;

    git2::Repository::clone(repo_url, dest_dir).map_err(|source| InitError::Clone {
        url: repo_url.to_string(),
        dest: dest_dir.to_path_buf(),
        source,
    })?;

    write_config(config_path, dest_dir)
}

/// Returns [`InitError::DestinationNotEmpty`] if `dir` exists and contains
/// at least one entry. A missing directory counts as empty — `git2` creates
/// it as part of the clone.
fn ensure_empty(dir: &Path) -> Result<(), InitError> {
    if !dir.exists() {
        return Ok(());
    }

    let mut entries = std::fs::read_dir(dir).map_err(|source| InitError::ReadDestination {
        path: dir.to_path_buf(),
        source,
    })?;
    if entries.next().is_some() {
        return Err(InitError::DestinationNotEmpty {
            path: dir.to_path_buf(),
        });
    }
    Ok(())
}

/// Writes a config file at `config_path` whose `data_repo_path` is
/// `data_repo_path`, creating the config file's parent directory if it
/// doesn't exist yet.
fn write_config(config_path: &Path, data_repo_path: &Path) -> Result<(), InitError> {
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| InitError::CreateConfigDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let config = Config {
        data_repo_path: data_repo_path.to_path_buf(),
    };
    let contents = toml::to_string(&config).map_err(InitError::SerializeConfig)?;

    std::fs::write(config_path, contents).map_err(|source| InitError::WriteConfig {
        path: config_path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    /// Creates a bare "remote" repo with one seeded commit (a file named
    /// `seed.txt`), so `run` has a real, clonable git URL to work with. The
    /// same fixture shape used in `crate::git_sync`'s own tests.
    fn seeded_bare_remote() -> (tempfile::TempDir, std::path::PathBuf) {
        let remote_dir = tempfile::tempdir().expect("failed to create temp dir");
        let remote_path = remote_dir.path().join("remote.git");
        git2::Repository::init_bare(&remote_path).expect("failed to init bare remote");

        let seed_dir = tempfile::tempdir().expect("failed to create temp dir");
        let seed_repo = git2::Repository::init(seed_dir.path()).expect("failed to init seed repo");
        std::fs::write(seed_dir.path().join("seed.txt"), "seed\n").expect("failed to write seed");

        let mut index = seed_repo.index().expect("failed to get index");
        index
            .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
            .expect("failed to stage files");
        index.write().expect("failed to write index");
        let tree_id = index.write_tree().expect("failed to write tree");
        let tree = seed_repo.find_tree(tree_id).expect("failed to find tree");
        let signature =
            git2::Signature::now("Test", "test@example.com").expect("failed to build signature");
        seed_repo
            .commit(Some("HEAD"), &signature, &signature, "seed", &tree, &[])
            .expect("failed to commit seed");

        let mut remote = seed_repo
            .remote("origin", remote_path.to_str().expect("path is utf8"))
            .expect("failed to add remote");
        let head = seed_repo.head().expect("seed repo should have a HEAD");
        let refname = head.name().expect("HEAD should be named").to_string();
        remote
            .push(&[format!("{refname}:{refname}")], None)
            .expect("failed to push seed commit");

        (remote_dir, remote_path)
    }

    #[test]
    fn run_clones_the_repo_into_the_destination_and_writes_a_config_pointing_at_it() {
        let (_remote_dir, remote_path) = seeded_bare_remote();
        let dest_parent = tempfile::tempdir().expect("failed to create temp dir");
        let dest_dir = dest_parent.path().join("husmo-data");
        let config_dir = tempfile::tempdir().expect("failed to create temp dir");
        let config_path = config_dir.path().join("husmo/config.toml");

        super::run(
            remote_path.to_str().expect("path is utf8"),
            &dest_dir,
            &config_path,
        )
        .expect("run should succeed");

        assert!(
            dest_dir.join("seed.txt").is_file(),
            "the clone should have brought in the remote's seeded file"
        );

        let config = crate::config::load(&config_path).expect("config should load");
        assert_eq!(config.data_repo_path, dest_dir);
    }

    #[test]
    fn run_returns_a_clear_error_without_touching_an_already_populated_destination() {
        let (_remote_dir, remote_path) = seeded_bare_remote();
        let dest_parent = tempfile::tempdir().expect("failed to create temp dir");
        let dest_dir = dest_parent.path().join("husmo-data");
        std::fs::create_dir_all(&dest_dir).expect("failed to create dest dir");
        std::fs::write(dest_dir.join("existing.txt"), "do not lose me\n")
            .expect("failed to seed existing file");
        let config_dir = tempfile::tempdir().expect("failed to create temp dir");
        let config_path = config_dir.path().join("husmo/config.toml");

        let result = super::run(
            remote_path.to_str().expect("path is utf8"),
            &dest_dir,
            &config_path,
        );

        assert!(
            matches!(result, Err(super::InitError::DestinationNotEmpty { .. })),
            "expected InitError::DestinationNotEmpty, got {result:?}"
        );
        assert_eq!(
            std::fs::read_to_string(dest_dir.join("existing.txt")).expect("file should survive"),
            "do not lose me\n",
            "the pre-existing file must not be touched"
        );
        assert!(
            !config_path.exists(),
            "no config should be written when the clone is refused"
        );
    }

    #[test]
    fn resolve_repo_url_uses_the_flag_when_given() {
        let url = super::resolve_repo_url(Some("git@example.com:data.git".to_string()), || {
            panic!("should not prompt when --repo was given")
        })
        .expect("should resolve");

        assert_eq!(url, "git@example.com:data.git");
    }

    #[test]
    fn resolve_repo_url_prompts_when_no_flag_was_given() {
        let url = super::resolve_repo_url(None, || Ok("git@example.com:prompted.git".to_string()))
            .expect("should resolve");

        assert_eq!(url, "git@example.com:prompted.git");
    }
}
