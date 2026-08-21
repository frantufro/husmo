//! Configuration loading: locates and parses the config file that points at
//! the data repo (see `docs/ARCHITECTURE.md`, "Repo split"). The app never
//! hardcodes a data repo location — it always comes from this file.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Application configuration.
///
/// Currently just the data repo path, per `docs/ARCHITECTURE.md`: "Its path
/// is supplied via a config file — the app should never hardcode a data
/// repo location."
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Config {
    /// Filesystem path to the data repo (the git-tracked store of
    /// Documents).
    pub data_repo_path: PathBuf,
}

/// An error encountered while locating or parsing a config file.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// No file exists at the given path.
    #[error("config file not found at {}", path.display())]
    NotFound {
        /// The path that was checked.
        path: PathBuf,
    },
    /// The file exists but could not be read.
    #[error("failed to read config file at {}", path.display())]
    Read {
        /// The path that was read.
        path: PathBuf,
        /// The underlying I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// The file was read but its contents are not valid config TOML.
    #[error("failed to parse config file at {}", path.display())]
    Parse {
        /// The path that was parsed.
        path: PathBuf,
        /// The underlying TOML parse failure.
        #[source]
        source: toml::de::Error,
    },
}

/// Loads and parses the config file at `path`.
///
/// # Errors
///
/// Returns [`ConfigError::NotFound`] if no file exists at `path`,
/// [`ConfigError::Read`] if it exists but can't be read, and
/// [`ConfigError::Parse`] if its contents aren't valid config TOML.
pub fn load(path: &Path) -> Result<Config, ConfigError> {
    if !path.is_file() {
        return Err(ConfigError::NotFound {
            path: path.to_path_buf(),
        });
    }

    let contents = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;

    toml::from_str(&contents).map_err(|source| ConfigError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Resolves the config file path from explicit environment inputs, without
/// touching the real environment. Kept separate from [`default_path`] so the
/// resolution logic is testable without mutating process-global env vars.
///
/// Precedence: `husmo_config` (an explicit override) wins outright; failing
/// that, `xdg_config_home` joined with `husmo/config.toml`; failing that,
/// `home` joined with `.config/husmo/config.toml`. Returns `None` only when
/// none of the three inputs are available.
fn resolve_path(
    husmo_config: Option<&str>,
    xdg_config_home: Option<&str>,
    home: Option<&str>,
) -> Option<PathBuf> {
    if let Some(explicit) = husmo_config {
        return Some(PathBuf::from(explicit));
    }
    if let Some(xdg) = xdg_config_home {
        return Some(PathBuf::from(xdg).join("husmo/config.toml"));
    }
    let home = home?;
    Some(PathBuf::from(home).join(".config/husmo/config.toml"))
}

/// Locates the default config file path by reading `HUSMO_CONFIG`,
/// `XDG_CONFIG_HOME`, and `HOME` from the real process environment.
///
/// Precedence: `HUSMO_CONFIG` (an explicit override) wins outright; failing
/// that, `XDG_CONFIG_HOME` joined with `husmo/config.toml`; failing that,
/// `HOME` joined with `.config/husmo/config.toml`. Returns `None` only when
/// none of the three are set.
#[must_use]
pub fn default_path() -> Option<PathBuf> {
    resolve_path(
        std::env::var("HUSMO_CONFIG").ok().as_deref(),
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    #[test]
    fn load_returns_not_found_when_file_is_missing() {
        let dir = tempdir().expect("failed to create temp dir");
        let missing_path = dir.path().join("does-not-exist.toml");

        let result = super::load(&missing_path);

        match result {
            Err(super::ConfigError::NotFound { path }) => assert_eq!(path, missing_path),
            other => panic!("expected ConfigError::NotFound, got {other:?}"),
        }
    }

    #[test]
    fn load_returns_parse_error_when_file_is_malformed() {
        let dir = tempdir().expect("failed to create temp dir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "this is not valid toml =====").expect("failed to write config");

        let result = super::load(&path);

        match result {
            Err(super::ConfigError::Parse { path: err_path, .. }) => {
                assert_eq!(err_path, path);
            }
            other => panic!("expected ConfigError::Parse, got {other:?}"),
        }
    }

    #[test]
    fn load_returns_config_when_file_is_valid() {
        let dir = tempdir().expect("failed to create temp dir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, r#"data_repo_path = "/home/fran/husmo-data""#)
            .expect("failed to write config");

        let result = super::load(&path);

        match result {
            Ok(config) => {
                assert_eq!(
                    config.data_repo_path,
                    std::path::PathBuf::from("/home/fran/husmo-data")
                );
            }
            other => panic!("expected Ok(Config), got {other:?}"),
        }
    }

    #[test]
    fn resolve_path_prefers_explicit_override() {
        let path = super::resolve_path(Some("/explicit/config.toml"), Some("/xdg"), Some("/home"));

        assert_eq!(
            path,
            Some(std::path::PathBuf::from("/explicit/config.toml"))
        );
    }

    #[test]
    fn resolve_path_falls_back_to_xdg_config_home() {
        let path = super::resolve_path(None, Some("/xdg"), Some("/home"));

        assert_eq!(
            path,
            Some(std::path::PathBuf::from("/xdg/husmo/config.toml"))
        );
    }

    #[test]
    fn resolve_path_falls_back_to_home_when_xdg_unset() {
        let path = super::resolve_path(None, None, Some("/home"));

        assert_eq!(
            path,
            Some(std::path::PathBuf::from("/home/.config/husmo/config.toml"))
        );
    }

    #[test]
    fn resolve_path_is_none_when_nothing_is_set() {
        let path = super::resolve_path(None, None, None);

        assert_eq!(path, None);
    }
}
