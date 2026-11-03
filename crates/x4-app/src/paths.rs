//! Where the app keeps its files: the owner's `config.toml`, the app's `state.toml` and the logs;
//! and the user's runtime directory, where the single instance keeps its lock and socket.

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;

/// The three parts of the reverse-DNS name the platform directories are derived from. On Linux
/// only the application name counts: the directories are `~/.config/4x4` and `~/.local/state/4x4`.
const QUALIFIER: &str = "io.github";
const ORGANIZATION: &str = "lexy898";
const APPLICATION: &str = "4x4";

/// The files and directories the app reads and writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    /// The owner's `config.toml`. The app only reads it.
    pub config_file: PathBuf,
    /// The app's own `state.toml`.
    pub state_file: PathBuf,
    /// The directory of the daily log files.
    pub log_dir: PathBuf,
}

/// Why the directories of the app cannot be found.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathsError {
    /// There is no home directory, so there is nowhere to look for the config and no place to
    /// keep the state and the logs.
    #[error("cannot find the home directory, so the config, state and log locations are unknown")]
    NoHome,
}

impl AppPaths {
    /// Finds the directories of the platform: `$XDG_CONFIG_HOME/4x4` or `~/.config/4x4` for the
    /// config, and `$XDG_STATE_HOME/4x4` or `~/.local/state/4x4` for the state and the logs. On
    /// platforms without a state directory the local data directory takes its place.
    ///
    /// Nothing is read from disk and nothing is created.
    ///
    /// # Errors
    ///
    /// [`PathsError::NoHome`] if the home directory is unknown.
    pub fn discover() -> Result<Self, PathsError> {
        let dirs =
            ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION).ok_or(PathsError::NoHome)?;
        let state_dir = dirs.state_dir().unwrap_or_else(|| dirs.data_local_dir());
        Ok(Self::in_dirs(dirs.config_dir(), state_dir))
    }

    /// The paths for a given config directory and state directory. The logs go in the state
    /// directory.
    #[must_use]
    pub fn in_dirs(config_dir: &Path, state_dir: &Path) -> Self {
        Self {
            config_file: config_dir.join("config.toml"),
            state_file: state_dir.join("state.toml"),
            log_dir: state_dir.to_path_buf(),
        }
    }
}

/// The user's runtime directory: `$XDG_RUNTIME_DIR`, when it is set to an absolute path. The XDG
/// base directory spec has a relative one ignored. Nothing is read from disk.
#[must_use]
pub fn runtime_dir() -> Option<PathBuf> {
    runtime_dir_of(env::var_os("XDG_RUNTIME_DIR"))
}

/// The runtime directory `$XDG_RUNTIME_DIR` = `value` names, if it is absolute.
fn runtime_dir_of(value: Option<OsString>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|dir| dir.is_absolute())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_absolute_xdg_runtime_dir_is_the_runtime_dir() {
        assert_eq!(
            runtime_dir_of(Some("/run/user/1000".into())),
            Some(PathBuf::from("/run/user/1000"))
        );
        assert_eq!(runtime_dir_of(Some("run/user/1000".into())), None);
        assert_eq!(runtime_dir_of(Some(OsString::new())), None);
        assert_eq!(runtime_dir_of(None), None);
    }

    #[test]
    fn paths_in_dirs_name_the_files_and_log_in_the_state_dir() {
        let paths = AppPaths::in_dirs(Path::new("/c/4x4"), Path::new("/s/4x4"));

        assert_eq!(paths.config_file, Path::new("/c/4x4/config.toml"));
        assert_eq!(paths.state_file, Path::new("/s/4x4/state.toml"));
        assert_eq!(paths.log_dir, Path::new("/s/4x4"));
    }

    #[test]
    fn discovered_paths_live_in_directories_named_after_the_app() {
        // Only the home directory and the XDG variables are read; nothing touches the disk.
        let Ok(paths) = AppPaths::discover() else {
            return; // No home directory in this environment.
        };

        assert!(paths.config_file.ends_with("4x4/config.toml"), "{paths:?}");
        assert!(paths.state_file.ends_with("state.toml"), "{paths:?}");
        assert!(paths.log_dir.ends_with("4x4"), "{paths:?}");
        assert_eq!(paths.state_file.parent(), Some(paths.log_dir.as_path()));
    }
}
