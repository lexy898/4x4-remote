//! The owner's `config.toml`: where the daemon is.
//!
//! The app only reads this file and never writes it, so comments and keys the app does not know
//! survive. A key that is missing takes its default; a key of the wrong type is an error.

use serde::Deserialize;

use crate::toml_error::describe;

/// Where the owner's daemon listens when `config.toml` does not say.
pub const DEFAULT_RPC_URL: &str = "http://192.168.1.5:9091/transmission/rpc";

/// What `config.toml` holds.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    /// The `[connection]` table.
    pub connection: ConnectionConfig,
}

/// The `[connection]` table of `config.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ConnectionConfig {
    /// The RPC endpoint of the daemon. Only plain `http://` is supported for now.
    pub url: String,
}

impl Default for ConnectionConfig {
    /// The owner's daemon, [`DEFAULT_RPC_URL`].
    fn default() -> Self {
        Self {
            url: DEFAULT_RPC_URL.to_owned(),
        }
    }
}

/// Why `config.toml` cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// The file is not valid TOML, or a key has the wrong type. Holds the position and the
    /// message of the parser.
    #[error("cannot parse config.toml: {0}")]
    Parse(String),
    /// `connection.url` does not start with `http://`. Holds the URL as written.
    #[error("connection.url must start with http://, got {0:?}")]
    InvalidUrl(String),
}

impl Config {
    /// Reads the contents of `config.toml`. An empty text gives the defaults, unknown keys are
    /// ignored.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Parse`] if `text` is not valid TOML or a key has the wrong type, and
    /// [`ConfigError::InvalidUrl`] if `connection.url` does not start with `http://`.
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let config: Config =
            toml::from_str(text).map_err(|error| ConfigError::Parse(describe(&error, text)))?;
        // TLS is not built in yet, so `https://` would only fail later with a worse message.
        if !config.connection.url.starts_with("http://") {
            return Err(ConfigError::InvalidUrl(config.connection.url));
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_to_owner_pi() {
        let config = Config::parse("").expect("an empty file is valid");

        assert_eq!(config.connection.url, DEFAULT_RPC_URL);
        assert_eq!(config, Config::default());
    }

    #[test]
    fn config_rejects_non_http_url() {
        for url in [
            "ftp://x",
            "https://pi.local/transmission/rpc",
            "192.168.1.5:9091",
            "",
        ] {
            let text = format!("[connection]\nurl = {url:?}\n");

            assert_eq!(
                Config::parse(&text),
                Err(ConfigError::InvalidUrl(url.to_owned())),
                "{url:?}"
            );
        }
    }

    #[test]
    fn config_takes_the_url_it_is_given() {
        let config =
            Config::parse("[connection]\nurl = \"http://pi.local:9091/transmission/rpc\"\n")
                .expect("a valid file");

        assert_eq!(
            config.connection.url,
            "http://pi.local:9091/transmission/rpc"
        );
    }

    #[test]
    fn config_tolerates_unknown_keys_and_comments() {
        let text = "# my daemon\n[connection]\nurl = \"http://x/rpc\" # here\ntimeout = 5\n\n[future]\nkey = true\n";

        let config = Config::parse(text).expect("unknown keys are ignored");

        assert_eq!(config.connection.url, "http://x/rpc");
    }

    #[test]
    fn config_without_the_connection_table_uses_the_default_url() {
        let config = Config::parse("[other]\nkey = 1\n").expect("a valid file");

        assert_eq!(config.connection.url, DEFAULT_RPC_URL);
        let config = Config::parse("[connection]\n").expect("a valid file");
        assert_eq!(config.connection.url, DEFAULT_RPC_URL);
    }

    #[test]
    fn config_wrong_type_is_a_parse_error_with_a_position() {
        let Err(ConfigError::Parse(message)) = Config::parse("[connection]\nurl = 5\n") else {
            panic!("a number is not a URL");
        };

        assert!(message.starts_with("line 2, column 7: "), "{message}");
        assert!(!message.contains('\n'), "{message}");
    }

    #[test]
    fn config_broken_toml_is_a_parse_error() {
        assert!(matches!(
            Config::parse("[connection\nurl = 1"),
            Err(ConfigError::Parse(_))
        ));
    }

    #[test]
    fn config_errors_say_what_is_wrong() {
        let invalid = ConfigError::InvalidUrl("ftp://x".to_owned());

        assert_eq!(
            invalid.to_string(),
            "connection.url must start with http://, got \"ftp://x\""
        );
        assert!(
            ConfigError::Parse("line 1, column 2: boom".to_owned())
                .to_string()
                .contains("line 1, column 2: boom")
        );
    }
}
