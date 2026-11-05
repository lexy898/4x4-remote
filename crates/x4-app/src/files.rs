//! Reading and writing the owner's `config.toml` and the app's `state.toml`.
//!
//! The contents of both files are turned into values, and values into text, by `x4-core`; this
//! module only does the file I/O. Both are written atomically, private to the owner. No error here
//! shows what a file contains, as a config holds the password.

use std::ffi::OsString;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::{fs, str};

use tempfile::NamedTempFile;
use x4_core::{
    Config, ConfigError, ConnectionSettings, TextPosition, UiState, UiStateError,
    config_with_connection,
};

use crate::private_dir::create_private_dir;

/// Why a file could not be read or written. Every variant holds the path of the file and the
/// cause, which is not part of the message: [`FilesError::report`] gives both.
#[derive(Debug, thiserror::Error)]
pub(crate) enum FilesError {
    /// The file exists but cannot be read.
    #[error("cannot read {}", path.display())]
    Read {
        /// The file.
        path: PathBuf,
        /// What the system said.
        source: io::Error,
    },
    /// The config file is there, but its contents are unusable.
    #[error("invalid config file {}", path.display())]
    Config {
        /// The file.
        path: PathBuf,
        /// What is wrong with it.
        source: ConfigError,
    },
    /// The directory for the file cannot be created.
    #[error("cannot create the directory {}", path.display())]
    CreateDir {
        /// The directory.
        path: PathBuf,
        /// What the system said.
        source: io::Error,
    },
    /// The state cannot be turned into TOML.
    #[error("cannot serialize the state for {}", path.display())]
    Serialize {
        /// The file the state was meant for.
        path: PathBuf,
        /// What the serializer said.
        source: UiStateError,
    },
    /// A file that is not valid cannot be moved aside to make room for a new one.
    #[error("cannot move {} aside to {}", path.display(), to.display())]
    MoveAside {
        /// The file.
        path: PathBuf,
        /// Where it was to go.
        to: PathBuf,
        /// What the system said.
        source: io::Error,
    },
    /// The file cannot be written.
    #[error("cannot write {}", path.display())]
    Write {
        /// The file.
        path: PathBuf,
        /// What the system said.
        source: io::Error,
    },
}

impl FilesError {
    /// The message and the causes behind it on one line (`cannot read /x/state.toml: Is a
    /// directory (os error 21)`), for logs and for showing to the user. No cause repeats a value
    /// from the file.
    pub(crate) fn report(&self) -> String {
        let mut report = self.to_string();
        let mut cause = std::error::Error::source(self);
        while let Some(error) = cause {
            report.push_str(": ");
            report.push_str(&error.to_string());
            cause = error.source();
        }
        report
    }
}

/// What [`read_ui_state`] found.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LoadedState {
    /// The state to start with: the file's, or the defaults when there is no usable file.
    pub state: UiState,
    /// Whether the state may be saved over the file. It is not when the file is from a newer
    /// version of the app, or when it is corrupt and could not be moved out of the way: saving
    /// would destroy something the owner may want to look at.
    pub writable: bool,
}

/// What [`save_connection`] did besides writing the settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SavedConfig {
    /// Whether the old file was not TOML at all, and was moved to `config.toml.bad` (replacing an
    /// older one) before a new file was written.
    pub(crate) moved_aside: bool,
}

/// Reads the owner's `config.toml`. A missing file gives [`Config::default`].
///
/// # Errors
///
/// [`FilesError::Read`] if the file exists but cannot be read, and [`FilesError::Config`] if its
/// contents are unusable (not TOML, a value of the wrong type, an address that is not plain
/// `http://` or holds a user name or password). Reading never changes the file; the caller
/// decides what to do, such as showing the error and setting no server.
pub(crate) fn read_config(path: &Path) -> Result<Config, FilesError> {
    let Some(bytes) = read(path)? else {
        return Ok(Config::default());
    };
    let to_error = |source| FilesError::Config {
        path: path.to_path_buf(),
        source,
    };
    // A file that is not UTF-8 is not TOML either; it is reported like a parse error, at the
    // place of the first invalid byte.
    let text = str::from_utf8(&bytes).map_err(|error| {
        to_error(ConfigError::Parse {
            position: utf8_position(&bytes, error),
            message: utf8_problem(error),
        })
    })?;
    Config::parse(text).map_err(to_error)
}

/// Reads the app's `state.toml`.
///
/// A missing file gives the defaults, writable. A file that is not valid, or has a value of the
/// wrong type, is moved to `state.toml.bad` (replacing an older one) with a warning that holds
/// the parse error, and the defaults are used; this never stops the app. If the file cannot be
/// moved, the defaults are used and [`LoadedState::writable`] is false. A file from a newer
/// version is left alone and not writable too.
///
/// # Errors
///
/// [`FilesError::Read`] if the file exists but cannot be read (a directory, no permission). The
/// caller should then use the defaults and not save over it.
pub(crate) fn read_ui_state(path: &Path) -> Result<LoadedState, FilesError> {
    let Some(bytes) = read(path)? else {
        return Ok(LoadedState {
            state: UiState::default(),
            writable: true,
        });
    };
    let parsed = str::from_utf8(&bytes)
        .map_err(|error| UiStateError::Parse(utf8_problem(error)))
        .and_then(UiState::parse);
    match parsed {
        Ok(parsed) => {
            if parsed.newer {
                tracing::warn!(
                    path = %path.display(),
                    "the UI state was written by a newer version; using the defaults and leaving the file alone"
                );
            }
            Ok(LoadedState {
                state: parsed.state,
                writable: !parsed.newer,
            })
        }
        Err(error) => Ok(LoadedState {
            state: UiState::default(),
            writable: move_aside(path, &error),
        }),
    }
}

/// Writes `settings` into the owner's `config.toml` at `path`, as the Connection dialog saves
/// them: the `[connection]` table's `url`, and `username` and `password` when there are
/// credentials (removed when there are none). The comments, the other keys and tables of the file
/// stay. The file is written atomically, as [`write_ui_state`] writes, with mode 0600 on Unix, as
/// it holds the password in plain text; its directory is created if needed.
///
/// A missing file is a new one. A file that is not TOML at all (not even UTF-8) cannot be edited:
/// it is moved to `config.toml.bad`, replacing an older one, and a new file is written, which the
/// log says with both paths and [`SavedConfig::moved_aside`] tells.
///
/// # Errors
///
/// [`FilesError::Read`] if the old file cannot be read, [`FilesError::CreateDir`] if the
/// directory cannot be created, [`FilesError::MoveAside`] if a file that is not TOML cannot be
/// moved aside, and [`FilesError::Write`] if the new file cannot be written. On any error the
/// old file is left as it was and no temporary file stays behind; only if the new file cannot
/// take a broken one's place once that was moved aside, and the broken one cannot be moved back
/// either, does it stay as `config.toml.bad`, which the log says.
pub(crate) fn save_connection(
    path: &Path,
    settings: &ConnectionSettings,
) -> Result<SavedConfig, FilesError> {
    let old = read(path)?.unwrap_or_default();
    let edited = str::from_utf8(&old)
        .map_err(|error| ConfigError::Parse {
            position: None,
            message: utf8_problem(error),
        })
        .and_then(|text| config_with_connection(text, settings));
    let config_error = |source| FilesError::Config {
        path: path.to_path_buf(),
        source,
    };
    let problem = match edited {
        Ok(text) => {
            write_atomically(path, &text)?;
            tracing::info!(path = %path.display(), server = %settings.server_label(), "connection saved");
            return Ok(SavedConfig { moved_aside: false });
        }
        Err(problem @ ConfigError::Parse { .. }) => problem,
        // Only reading checks the address.
        Err(other @ ConfigError::InvalidUrl(_)) => return Err(config_error(other)),
    };
    let text = config_with_connection("", settings).map_err(config_error)?;
    let bad = bad_path(path).ok_or_else(|| FilesError::Write {
        path: path.to_path_buf(),
        source: io::Error::new(io::ErrorKind::InvalidInput, "the path has no file name"),
    })?;
    // The new file is ready before the old one moves, so that a failed write leaves it in place.
    let staged = stage(path, &text)?;
    // `rename` replaces an older `.bad` file.
    fs::rename(path, &bad).map_err(|source| FilesError::MoveAside {
        path: path.to_path_buf(),
        to: bad.clone(),
        source,
    })?;
    if let Err(error) = persist(staged, path) {
        if let Err(back) = fs::rename(&bad, path) {
            tracing::warn!(path = %path.display(), moved_to = %bad.display(), error = %back, "config.toml could not be moved back after the new one failed");
        }
        return Err(error);
    }
    tracing::warn!(path = %path.display(), moved_to = %bad.display(), error = %problem, "config.toml was not valid TOML; moved it aside and wrote a new one");
    tracing::info!(path = %path.display(), server = %settings.server_label(), "connection saved");
    Ok(SavedConfig { moved_aside: true })
}

/// Writes the app's `state.toml` atomically: the text goes to a temporary file in the same
/// directory, which is flushed to disk and then renamed over the old file, so a crash leaves
/// either the old file or the new one. The directory is created if needed.
///
/// # Errors
///
/// [`FilesError::Serialize`] if the state cannot be turned into TOML, [`FilesError::CreateDir`]
/// if the directory cannot be created and [`FilesError::Write`] if the file cannot be written. On
/// any error the old file is left as it was and no temporary file stays behind.
pub(crate) fn write_ui_state(path: &Path, state: &UiState) -> Result<(), FilesError> {
    let text = state.to_toml().map_err(|source| FilesError::Serialize {
        path: path.to_path_buf(),
        source,
    })?;
    write_atomically(path, &text)
}

/// Writes `text` to `path` atomically, private to the owner: [`stage`], then [`persist`].
fn write_atomically(path: &Path, text: &str) -> Result<(), FilesError> {
    persist(stage(path, text)?, path)
}

/// `text` in a temporary file next to `path`, flushed to disk and readable and writable by the
/// owner only (mode 0600 on Unix), ready to take the place of `path`. The directory is created
/// if needed, private to the owner too.
fn stage(path: &Path, text: &str) -> Result<NamedTempFile, FilesError> {
    // A bare file name has an empty parent, which means the current directory.
    let dir = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    create_private_dir(dir).map_err(|source| FilesError::CreateDir {
        path: dir.to_path_buf(),
        source,
    })?;
    let write_error = |source| FilesError::Write {
        path: path.to_path_buf(),
        source,
    };
    // In the same directory, so the rename stays on one file system and is atomic. A crash between
    // creating the file and renaming it leaves a `.tmpXXXXXX` file behind; it is harmless and
    // nothing sweeps it.
    let mut temporary = NamedTempFile::new_in(dir).map_err(write_error)?;
    // tempfile makes it 0600 already; said here, as the file may hold a password.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(write_error)?;
    }
    temporary.write_all(text.as_bytes()).map_err(write_error)?;
    // `persist` renames without flushing, and a rename that reaches the disk before the data
    // would leave an empty file after a power loss.
    temporary.as_file().sync_all().map_err(write_error)?;
    Ok(temporary)
}

/// Renames `staged` over `path`. A temporary file that cannot be renamed is removed.
fn persist(staged: NamedTempFile, path: &Path) -> Result<(), FilesError> {
    staged
        .persist(path)
        .map(drop)
        .map_err(|error| FilesError::Write {
            path: path.to_path_buf(),
            source: error.error,
        })
}

/// The contents of the file, or `None` if there is no such file.
fn read(path: &Path) -> Result<Option<Vec<u8>>, FilesError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(FilesError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// What is wrong with bytes that are not UTF-8, without showing any of them.
fn utf8_problem(error: str::Utf8Error) -> String {
    format!(
        "the file is not valid UTF-8 (invalid byte at offset {})",
        error.valid_up_to()
    )
}

/// Where the first invalid byte of `bytes` is, as a place in the text before it.
fn utf8_position(bytes: &[u8], error: str::Utf8Error) -> Option<TextPosition> {
    let valid = str::from_utf8(bytes.get(..error.valid_up_to())?).ok()?;
    TextPosition::of(valid, valid.len())
}

/// Moves a corrupt state file to `<name>.bad` so the next save does not destroy it, and says so.
/// Returns whether the state may be saved: false if the file could not be moved.
///
/// The warning holds the parse error, which does not repeat anything from the file.
fn move_aside(path: &Path, problem: &UiStateError) -> bool {
    let Some(bad) = bad_path(path) else {
        tracing::warn!(path = %path.display(), error = %problem, "the UI state is corrupt and cannot be moved aside; using the defaults and not saving");
        return false;
    };
    // `rename` replaces an older `.bad` file.
    match fs::rename(path, &bad) {
        Ok(()) => {
            tracing::warn!(path = %path.display(), moved_to = %bad.display(), error = %problem, "the UI state is corrupt; moved it aside and using the defaults");
            true
        }
        Err(error) => {
            tracing::warn!(path = %path.display(), moved_to = %bad.display(), error = %problem, move_error = %error, "the UI state is corrupt and cannot be moved aside; using the defaults and not saving");
            false
        }
    }
}

/// `path` with `.bad` added to the file name, or `None` if it has no file name.
fn bad_path(path: &Path) -> Option<PathBuf> {
    let mut name: OsString = path.file_name()?.to_owned();
    name.push(".bad");
    Some(path.with_file_name(name))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use x4_core::{ThemeChoice, UI_STATE_VERSION};

    use super::*;

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("a temporary directory")
    }

    /// The names of the files in `dir`, sorted.
    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .expect("a readable directory")
            .map(|entry| {
                entry
                    .expect("a directory entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn missing_config_gives_defaults() {
        let dir = dir();

        let config = read_config(&dir.path().join("config.toml")).expect("a missing file is fine");

        assert_eq!(config, Config::default());
        assert_eq!(config.connection, None, "no file, no server");
    }

    #[test]
    fn config_is_read_from_the_file() {
        let dir = dir();
        let path = dir.path().join("config.toml");
        fs::write(&path, "[connection]\nurl = \"http://pi.local:9091/rpc\"\n").expect("written");

        let config = read_config(&path).expect("a valid file");

        let settings = config.connection.expect("the file names a server");
        assert_eq!(settings.url(), "http://pi.local:9091/rpc");
    }

    #[test]
    fn config_is_never_touched() {
        let dir = dir();
        let path = dir.path().join("config.toml");
        let text = "# mine\n[connection]\nurl = \"http://pi.local/rpc\"\nextra = 1\n";
        fs::write(&path, text).expect("written");

        read_config(&path).expect("a valid file");

        assert_eq!(fs::read_to_string(&path).expect("readable"), text);
        assert_eq!(names(dir.path()), ["config.toml"]);
    }

    #[test]
    fn invalid_config_error_names_the_file_and_hides_the_url() {
        let dir = dir();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "[connection]\nurl = \"https://user:hunter2@pi/rpc\"\n",
        )
        .expect("written");

        let error = read_config(&path).expect_err("https is not supported");

        for shown in [error.to_string(), format!("{error:?}")] {
            assert!(shown.contains("config.toml"), "{shown}");
            assert!(shown.contains(&*path.to_string_lossy()), "{shown}");
            assert!(!shown.contains("hunter2"), "{shown}");
            assert!(!shown.contains("user"), "{shown}");
        }
    }

    #[test]
    fn broken_config_is_an_error_and_stays_in_place() {
        let dir = dir();
        let path = dir.path().join("config.toml");
        fs::write(&path, "[connection\nurl = 5").expect("written");

        let error = read_config(&path).expect_err("not TOML");

        assert!(error.to_string().contains("config.toml"), "{error}");
        assert_eq!(names(dir.path()), ["config.toml"]);
    }

    #[test]
    fn config_that_is_not_utf8_is_a_parse_error_at_the_bad_byte() {
        let dir = dir();
        let path = dir.path().join("config.toml");
        fs::write(&path, b"# ok\n[connection]\nurl = \"\xff\"\n").expect("written");

        let error = read_config(&path).expect_err("not UTF-8");

        assert!(
            matches!(
                &error,
                FilesError::Config {
                    source: ConfigError::Parse {
                        position: Some(TextPosition { line: 3, column: 8 }),
                        ..
                    },
                    ..
                }
            ),
            "{error:?}"
        );
    }

    #[test]
    fn config_that_is_a_directory_is_a_read_error_with_the_path() {
        let dir = dir();
        let path = dir.path().join("config.toml");
        fs::create_dir(&path).expect("a directory");

        let error = read_config(&path).expect_err("a directory cannot be read as a file");

        assert!(
            error.to_string().contains(&*path.to_string_lossy()),
            "{error}"
        );
    }

    #[test]
    fn missing_state_gives_defaults_and_is_writable() {
        let dir = dir();

        let loaded = read_ui_state(&dir.path().join("state.toml")).expect("a missing file is fine");

        assert_eq!(loaded.state, UiState::default());
        assert!(loaded.writable);
    }

    #[test]
    fn state_write_is_atomic_and_roundtrips() {
        let dir = dir();
        let path = dir.path().join("state.toml");
        let mut state = UiState::default();
        state.window.width = 900.0;
        state.theme = ThemeChoice::Dark;
        state.columns.swap(1, 2);
        state.columns[0].width = 123.0;
        state.columns[3].visible = false;
        let state = state.normalized();

        write_ui_state(&path, &state).expect("written");

        let loaded = read_ui_state(&path).expect("readable");
        assert_eq!(loaded.state, state);
        assert!(loaded.writable);
        assert_eq!(
            names(dir.path()),
            ["state.toml"],
            "no temporary file is left"
        );
    }

    #[test]
    fn state_write_replaces_the_old_file() {
        let dir = dir();
        let path = dir.path().join("state.toml");
        let first = UiState {
            theme: ThemeChoice::Light,
            ..UiState::default()
        };
        write_ui_state(&path, &first).expect("written");
        let second = UiState {
            theme: ThemeChoice::Dark,
            ..UiState::default()
        };

        write_ui_state(&path, &second).expect("written again");

        assert_eq!(read_ui_state(&path).expect("readable").state, second);
        assert_eq!(names(dir.path()), ["state.toml"]);
    }

    #[test]
    fn state_write_creates_the_missing_directories() {
        let dir = dir();
        let path = dir.path().join("a").join("b").join("state.toml");

        write_ui_state(&path, &UiState::default()).expect("written");

        assert_eq!(
            read_ui_state(&path).expect("readable").state,
            UiState::default()
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            for created in [
                path.parent().expect("a parent"),
                dir.path().join("a").as_path(),
            ] {
                let mode = fs::metadata(created)
                    .expect("metadata")
                    .permissions()
                    .mode();
                assert_eq!(mode & 0o777, 0o700, "{}", created.display());
            }
            let mode = fs::metadata(&path).expect("metadata").permissions().mode();
            assert_eq!(mode & 0o077, 0, "the state file is private too");
        }
    }

    #[test]
    fn state_write_failure_names_the_file_and_leaves_no_temporary_file() {
        let dir = dir();
        // The target is a directory, so the final rename fails after the temporary file exists.
        let path = dir.path().join("state.toml");
        fs::create_dir(&path).expect("a directory");

        let error = write_ui_state(&path, &UiState::default()).expect_err("cannot replace it");

        assert!(
            error.to_string().contains(&*path.to_string_lossy()),
            "{error}"
        );
        assert_eq!(names(dir.path()), ["state.toml"]);
    }

    #[test]
    fn corrupt_state_file_is_moved_aside() {
        let dir = dir();
        let path = dir.path().join("state.toml");
        let bad = dir.path().join("state.toml.bad");
        fs::write(&path, "this is = = not toml {{{").expect("written");

        let loaded = read_ui_state(&path).expect("a corrupt file must not stop the app");

        assert_eq!(loaded.state, UiState::default());
        assert!(loaded.writable);
        assert!(!path.exists(), "the original is gone");
        assert_eq!(
            fs::read_to_string(&bad).expect("the .bad file exists"),
            "this is = = not toml {{{"
        );
    }

    #[test]
    fn state_with_a_wrong_type_is_moved_aside() {
        let dir = dir();
        let path = dir.path().join("state.toml");
        fs::write(&path, "[window]\nwidth = \"wide\"\n").expect("written");

        let loaded = read_ui_state(&path).expect("a corrupt file must not stop the app");

        assert_eq!(loaded.state, UiState::default());
        assert!(!path.exists());
        assert!(dir.path().join("state.toml.bad").exists());
    }

    #[test]
    fn state_that_is_not_utf8_is_moved_aside() {
        let dir = dir();
        let path = dir.path().join("state.toml");
        fs::write(&path, [0x66, 0xff, 0xfe, 0x00, 0x80]).expect("written");

        let loaded = read_ui_state(&path).expect("a corrupt file must not stop the app");

        assert_eq!(loaded.state, UiState::default());
        assert!(loaded.writable);
        assert!(!path.exists());
        assert!(dir.path().join("state.toml.bad").exists());
    }

    #[test]
    fn a_new_bad_file_replaces_an_older_one() {
        let dir = dir();
        let path = dir.path().join("state.toml");
        let bad = dir.path().join("state.toml.bad");
        fs::write(&bad, "the first broken file = =").expect("written");
        fs::write(&path, "the second broken file = =").expect("written");

        let loaded = read_ui_state(&path).expect("a corrupt file must not stop the app");

        assert!(loaded.writable);
        assert!(!path.exists());
        assert_eq!(
            fs::read_to_string(&bad).expect("the .bad file exists"),
            "the second broken file = ="
        );
        assert_eq!(names(dir.path()), ["state.toml.bad"]);
    }

    #[test]
    fn corrupt_state_that_cannot_be_moved_is_not_writable() {
        let dir = dir();
        let path = dir.path().join("state.toml");
        let bad = dir.path().join("state.toml.bad");
        fs::write(&path, "broken = =").expect("written");
        // A non-empty directory cannot be replaced by a file.
        fs::create_dir(&bad).expect("a directory");
        fs::write(bad.join("keep"), "x").expect("written");

        let loaded = read_ui_state(&path).expect("a corrupt file must not stop the app");

        assert_eq!(loaded.state, UiState::default());
        assert!(!loaded.writable, "the corrupt file would be overwritten");
        assert_eq!(fs::read_to_string(&path).expect("untouched"), "broken = =");
    }

    #[test]
    fn newer_state_is_left_alone_and_not_writable() {
        let dir = dir();
        let path = dir.path().join("state.toml");
        let text = format!(
            "version = {}\n[window]\nwidth = \"a new kind of value\"\n",
            UI_STATE_VERSION + 1
        );
        fs::write(&path, &text).expect("written");

        let loaded = read_ui_state(&path).expect("a newer file must not stop the app");

        assert_eq!(loaded.state, UiState::default());
        assert!(!loaded.writable);
        assert_eq!(fs::read_to_string(&path).expect("untouched"), text);
        assert_eq!(names(dir.path()), ["state.toml"]);
    }

    #[test]
    fn state_read_returns_the_normalized_state() {
        let dir = dir();
        let path = dir.path().join("state.toml");
        fs::write(&path, "[window]\nwidth = 5.0\nheight = 5.0\n").expect("written");

        let loaded = read_ui_state(&path).expect("readable");

        assert_eq!(loaded.state, UiState::default());
        assert!(loaded.writable);
        assert!(path.exists());
    }

    #[test]
    fn state_that_is_a_directory_is_a_read_error_with_the_path() {
        let dir = dir();
        let path = dir.path().join("state.toml");
        fs::create_dir(&path).expect("a directory");

        let error = read_ui_state(&path).expect_err("a directory cannot be read as a file");

        assert!(
            error.to_string().contains(&*path.to_string_lossy()),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn state_write_into_a_read_only_dir_fails_and_keeps_the_old_file() {
        let root = dir();
        let state_dir = root.path().join("state");
        fs::create_dir(&state_dir).expect("a directory");
        let path = state_dir.join("state.toml");
        let old = UiState {
            theme: ThemeChoice::Light,
            ..UiState::default()
        };
        write_ui_state(&path, &old).expect("written");
        let Some(_locked) = crate::test_dirs::make_read_only(&state_dir) else {
            return;
        };

        let error = write_ui_state(
            &path,
            &UiState {
                theme: ThemeChoice::Dark,
                ..UiState::default()
            },
        )
        .expect_err("the directory cannot be written");

        assert!(
            matches!(&error, FilesError::Write { path: failed, .. } if failed == &path),
            "{error:?}"
        );
        assert_eq!(read_ui_state(&path).expect("readable").state, old);
        assert_eq!(
            names(&state_dir),
            ["state.toml"],
            "no temporary file is left"
        );
    }

    #[cfg(unix)]
    #[test]
    fn state_write_that_cannot_create_its_directory_names_the_directory() {
        let root = dir();
        let parent = root.path().join("locked");
        fs::create_dir(&parent).expect("a directory");
        let Some(_locked) = crate::test_dirs::make_read_only(&parent) else {
            return;
        };
        let state_dir = parent.join("4x4");

        let error = write_ui_state(&state_dir.join("state.toml"), &UiState::default())
            .expect_err("the parent cannot be written");

        assert!(
            matches!(&error, FilesError::CreateDir { path, .. } if path == &state_dir),
            "{error:?}"
        );
        assert!(!state_dir.exists());
    }

    #[test]
    fn report_holds_the_message_and_the_cause_once() {
        let dir = dir();
        let path = dir.path().join("config.toml");
        fs::create_dir(&path).expect("a directory");
        let read = read_config(&path).expect_err("a directory cannot be read as a file");
        fs::write(path.with_file_name("other.toml"), "[connection]\nurl = 5\n").expect("written");
        let parse = read_config(&path.with_file_name("other.toml")).expect_err("a number");

        let read = read.report();
        let parse = parse.report();

        assert!(
            read.starts_with(&format!("cannot read {}: ", path.display())),
            "{read}"
        );
        assert_eq!(read.matches(&*path.to_string_lossy()).count(), 1, "{read}");
        assert!(
            read.len() > format!("cannot read {}: ", path.display()).len(),
            "{read}"
        );
        assert!(
            parse.contains(": cannot parse config.toml: line 2, column 7: "),
            "{parse}"
        );
        assert_eq!(
            parse.matches("cannot parse config.toml").count(),
            1,
            "{parse}"
        );
    }

    /// A made-up password with the characters TOML and URLs treat specially.
    const PASSWORD: &str = "hu\"nt\\er#2= \n ä@:";

    /// The settings the Connection dialog saves in these tests, with credentials.
    fn nas() -> ConnectionSettings {
        ConnectionSettings::from_url(
            "http://nas:9091/transmission/rpc",
            Some(x4_rpc::Credentials {
                username: "bob".to_owned(),
                password: PASSWORD.to_owned(),
            }),
        )
        .expect("a plain http address")
    }

    /// The mode bits of `path`.
    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt as _;
        fs::metadata(path).expect("metadata").permissions().mode() & 0o777
    }

    #[test]
    fn saving_writes_mode_0600_and_reads_back() {
        let dir = dir();
        let path = dir.path().join("config.toml");

        let saved = save_connection(&path, &nas()).expect("saved");

        assert_eq!(saved, SavedConfig { moved_aside: false });
        assert_eq!(
            read_config(&path).expect("readable").connection,
            Some(nas())
        );
        assert_eq!(
            names(dir.path()),
            ["config.toml"],
            "no temporary file is left"
        );
        #[cfg(unix)]
        assert_eq!(mode(&path), 0o600);

        // A file the owner made readable to others is private once 4x4 writes it.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("chmod");
            save_connection(&path, &nas()).expect("saved again");
            assert_eq!(mode(&path), 0o600);
        }
        let without = ConnectionSettings::from_url("http://nas:9091/transmission/rpc", None)
            .expect("a plain http address");
        save_connection(&path, &without).expect("saved without credentials");
        let text = fs::read_to_string(&path).expect("readable");
        assert!(!text.contains("password"), "{text}");
        assert!(!text.contains("username"), "{text}");
        assert_eq!(
            read_config(&path).expect("readable").connection,
            Some(without)
        );
    }

    #[test]
    fn saving_keeps_the_rest_of_the_file() {
        let dir = dir();
        let path = dir.path().join("config.toml");
        let old = "# my settings\n\
                   top = true\n\
                   \n\
                   [connection]\n\
                   # the Pi\n\
                   url = 5 # was wrong\n\
                   timeout = 3\n\
                   \n\
                   [later]\n\
                   # a table 4x4 does not know\n\
                   key = \"value\"\n";
        fs::write(&path, old).expect("written");

        let saved = save_connection(&path, &nas()).expect("saved");

        assert!(!saved.moved_aside);
        let text = fs::read_to_string(&path).expect("readable");
        for kept in [
            "# my settings\ntop = true\n",
            "[connection]\n# the Pi\nurl = \"http://nas:9091/transmission/rpc\" # was wrong\n",
            "timeout = 3\n",
            "[later]\n# a table 4x4 does not know\nkey = \"value\"\n",
        ] {
            assert!(text.contains(kept), "{kept:?} in {text}");
        }
        assert_eq!(
            read_config(&path).expect("readable").connection,
            Some(nas())
        );
        assert_eq!(names(dir.path()), ["config.toml"], "nothing moved aside");

        // `connection` that is not a table becomes one; the rest stays too.
        fs::write(&path, "connection = \"x\"\n[later]\nkey = 1\n").expect("written");
        assert!(!save_connection(&path, &nas()).expect("saved").moved_aside);
        let text = fs::read_to_string(&path).expect("readable");
        assert!(text.contains("[later]\nkey = 1\n"), "{text}");
        assert_eq!(
            read_config(&path).expect("readable").connection,
            Some(nas())
        );
    }

    #[test]
    fn saving_moves_broken_toml_aside() {
        let dir = dir();
        let path = dir.path().join("config.toml");
        let bad = dir.path().join("config.toml.bad");
        fs::write(&bad, "an older broken file = =").expect("written");
        let broken = "[connection\nurl = \"http://pi\"\npassword = \"hunter2\"\n";
        fs::write(&path, broken).expect("written");

        let mut saved = None;
        let records = crate::test_log::records_of(|| {
            saved = Some(save_connection(&path, &nas()));
        });

        assert_eq!(
            saved.map(|saved| saved.expect("saved")),
            Some(SavedConfig { moved_aside: true })
        );
        assert_eq!(
            fs::read_to_string(&bad).expect("the .bad file"),
            broken,
            "the old text, in place of the older one"
        );
        assert_eq!(
            read_config(&path).expect("readable").connection,
            Some(nas())
        );
        assert_eq!(names(dir.path()), ["config.toml", "config.toml.bad"]);
        #[cfg(unix)]
        assert_eq!(mode(&path), 0o600);
        let warning = records
            .iter()
            .find(|record| record.level == tracing::Level::WARN)
            .expect("the log says that the file was moved");
        assert_eq!(warning.field("path"), Some(&*path.to_string_lossy()));
        assert_eq!(warning.field("moved_to"), Some(&*bad.to_string_lossy()));
        let logged = format!("{records:?}");
        assert!(!logged.contains("hunter2"), "{logged}");
        assert!(!logged.contains(PASSWORD), "{logged}");

        // Not even UTF-8 is not TOML either.
        fs::write(&path, [b'#', 0xff, b'\n']).expect("written");
        assert!(save_connection(&path, &nas()).expect("saved").moved_aside);
        assert_eq!(fs::read(&bad).expect("the .bad file"), [b'#', 0xff, b'\n']);
        assert_eq!(
            read_config(&path).expect("readable").connection,
            Some(nas())
        );
    }

    #[test]
    fn saving_into_a_missing_directory_creates_it() {
        let dir = dir();
        let config_dir = dir.path().join("config").join("4x4");
        let path = config_dir.join("config.toml");

        save_connection(&path, &nas()).expect("saved");

        assert_eq!(
            read_config(&path).expect("readable").connection,
            Some(nas())
        );
        #[cfg(unix)]
        {
            assert_eq!(mode(&config_dir), 0o700);
            assert_eq!(mode(&path), 0o600);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_save_that_cannot_write_leaves_the_old_file() {
        let dir = dir();
        let config_dir = dir.path().join("4x4");
        fs::create_dir(&config_dir).expect("a directory");
        let path = config_dir.join("config.toml");
        let old = "# mine\n[connection]\nurl = \"http://pi/rpc\"\n";
        fs::write(&path, old).expect("written");
        let Some(locked) = crate::test_dirs::make_read_only(&config_dir) else {
            return;
        };

        let error = save_connection(&path, &nas()).expect_err("the directory cannot be written");

        assert!(
            matches!(&error, FilesError::Write { path: failed, .. } if failed == &path),
            "{error:?}"
        );
        assert!(!error.report().contains("hunter2"), "{}", error.report());
        assert_eq!(fs::read_to_string(&path).expect("untouched"), old);
        assert_eq!(names(&config_dir), ["config.toml"]);

        // Broken TOML that cannot be moved aside stays too.
        drop(locked);
        fs::write(&path, "[connection").expect("written");
        let Some(_locked) = crate::test_dirs::make_read_only(&config_dir) else {
            return;
        };
        assert!(save_connection(&path, &nas()).is_err());
        assert_eq!(fs::read_to_string(&path).expect("untouched"), "[connection");
        assert_eq!(names(&config_dir), ["config.toml"]);
    }

    #[test]
    fn a_config_that_cannot_be_read_is_not_saved_over() {
        let dir = dir();
        let path = dir.path().join("config.toml");
        fs::create_dir(&path).expect("a directory");

        let error = save_connection(&path, &nas()).expect_err("a directory is not a file");

        assert!(matches!(&error, FilesError::Read { .. }), "{error:?}");
        assert!(path.is_dir(), "left alone");
    }

    #[test]
    fn display_leaves_the_cause_to_the_source() {
        let error = FilesError::Config {
            path: "/x/config.toml".into(),
            source: ConfigError::Parse {
                position: Some(TextPosition { line: 1, column: 2 }),
                message: "boom".to_owned(),
            },
        };

        assert_eq!(error.to_string(), "invalid config file /x/config.toml");
        assert_eq!(
            std::error::Error::source(&error).map(ToString::to_string),
            Some("cannot parse config.toml: line 1, column 2: boom".to_owned())
        );
    }
}
