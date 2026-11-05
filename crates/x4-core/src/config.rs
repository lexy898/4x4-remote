//! The owner's `config.toml`: where the daemon is, if the owner has said.
//!
//! There is no built-in daemon: a file that is missing, or has no `connection.url`, means no
//! server is set. [`Config::parse`] reads the text, and [`config_with_connection`] puts the
//! settings of the Connection dialog into a text, keeping the comments, keys and tables the
//! app does not know. Reading and writing the file itself is left to the app.

mod write;

use serde::Deserialize;
use x4_rpc::Credentials;

use crate::connection::{ConnectionSettings, UrlProblem};
use crate::toml_error::{TextPosition, locate, with_position};

pub use write::config_with_connection;

/// What `config.toml` holds.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Config {
    /// The daemon of the `[connection]` table; `None` when the file names none.
    pub connection: Option<ConnectionSettings>,
}

/// `config.toml` as serde reads it, before the address is checked. It has no `Debug` on purpose:
/// it holds the password as written.
#[derive(Deserialize, Default)]
#[serde(default)]
struct RawConfig {
    connection: RawConnection,
}

/// The `[connection]` table as serde reads it.
#[derive(Deserialize, Default)]
#[serde(default)]
struct RawConnection {
    url: Option<String>,
    username: Option<String>,
    password: Option<String>,
}

/// Why `config.toml` cannot be used.
///
/// Neither the display nor the debug output of an error shows the address, the user name or the
/// password.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// The file is not valid TOML, or a key has the wrong type.
    #[error("cannot parse config.toml: {}", with_position(*.position, .message))]
    Parse {
        /// Where the problem is; `None` when the parser does not say.
        position: Option<TextPosition>,
        /// What the parser said, without the value it found.
        message: String,
    },
    /// The address in `connection.url`, or the user name next to it, cannot be used. The problem
    /// does not repeat any of it.
    #[error("{}", invalid_url_text(*.0))]
    InvalidUrl(UrlProblem),
}

/// What is wrong with `connection.url` (or `connection.username`), in words for the log.
fn invalid_url_text(problem: UrlProblem) -> String {
    match problem {
        // The way out is two other keys, so the text names them.
        UrlProblem::Credentials => "connection.url holds a user name or a password; keep them in \
             connection.username and connection.password"
            .to_owned(),
        // This one is about the user name, not the address.
        UrlProblem::UsernameColon => "connection.username cannot contain a colon".to_owned(),
        other => format!("connection.url {other}"),
    }
}

impl Config {
    /// Reads the contents of `config.toml`. An empty text, or one without `connection.url`, names
    /// no server; unknown keys are ignored.
    ///
    /// The optional keys `connection.username` and `connection.password` hold the credentials;
    /// one of them alone counts the other as empty, and both empty are no credentials.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Parse`] if `text` is not valid TOML or a key has the wrong type, and
    /// [`ConfigError::InvalidUrl`] if `connection.url` is not a plain `http://` address without a
    /// user name, password, query or fragment, or if `connection.username` holds a `:`.
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let raw: RawConfig = toml::from_str(text).map_err(|error| {
            let (position, message) = locate(&error, text);
            ConfigError::Parse { position, message }
        })?;
        let RawConnection {
            url,
            username,
            password,
        } = raw.connection;
        let Some(url) = url else {
            return Ok(Self::default());
        };
        let credentials = Credentials {
            username: username.unwrap_or_default(),
            password: password.unwrap_or_default(),
        };
        let settings = ConnectionSettings::from_url(&url, Some(credentials))
            .map_err(ConfigError::InvalidUrl)?;
        Ok(Self {
            connection: Some(settings),
        })
    }
}

#[cfg(test)]
mod tests;
