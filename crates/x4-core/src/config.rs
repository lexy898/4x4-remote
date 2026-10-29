//! The owner's `config.toml`: where the daemon is.
//!
//! The app only reads this file and never writes it, so comments and keys the app does not know
//! survive. A key that is missing takes its default; a key of the wrong type is an error.

use serde::Deserialize;

use crate::toml_error::{TextPosition, locate, with_position};

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
///
/// Neither the display nor the debug output of an error shows the URL, as it may hold a password.
#[derive(Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// The file is not valid TOML, or a key has the wrong type.
    #[error("cannot parse config.toml: {}", with_position(*.position, .message))]
    Parse {
        /// Where the problem is; `None` when the parser does not say.
        position: Option<TextPosition>,
        /// What the parser said, without the value it found.
        message: String,
    },
    /// `connection.url` does not start with `http://`. Holds the URL as written, for callers
    /// that need to look at it; it must not be logged.
    #[error("connection.url {}", url_problem(.0))]
    InvalidUrl(String),
}

impl std::fmt::Debug for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse { position, message } => f
                .debug_struct("Parse")
                .field("position", position)
                .field("message", message)
                .finish(),
            // The URL may hold a password, and `{error:?}` is what gets logged by habit.
            Self::InvalidUrl(_) => f.write_str("InvalidUrl(..)"),
        }
    }
}

/// What is wrong with `url`, without repeating any of it but the scheme.
fn url_problem(url: &str) -> String {
    match scheme(url) {
        Some(scheme) if scheme.eq_ignore_ascii_case("https") => {
            "must start with http://; https is not supported yet, only plain http".to_owned()
        }
        Some(scheme) => format!("must start with http://, but it uses the scheme {scheme:?}"),
        None => "must start with http://, but it has no scheme".to_owned(),
    }
}

/// The scheme of `url`, if it starts with one: letters, digits, `+`, `-` and `.`, beginning
/// with a letter, then `://`. Anything else before the `://` (a `user:password@host`, say) is not
/// a scheme.
fn scheme(url: &str) -> Option<&str> {
    let (scheme, _) = url.split_once("://")?;
    let mut chars = scheme.chars();
    let valid = chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    valid.then_some(scheme)
}

/// Whether `url` starts with `http://`. The scheme is case-insensitive (RFC 3986).
fn is_http(url: &str) -> bool {
    url.get(.."http://".len())
        .is_some_and(|start| start.eq_ignore_ascii_case("http://"))
}

impl Config {
    /// Reads the contents of `config.toml`. An empty text gives the defaults, unknown keys are
    /// ignored.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Parse`] if `text` is not valid TOML or a key has the wrong type, and
    /// [`ConfigError::InvalidUrl`] if `connection.url` does not start with `http://` (in any
    /// case).
    pub fn parse(text: &str) -> Result<Self, ConfigError> {
        let config: Config = toml::from_str(text).map_err(|error| {
            let (position, message) = locate(&error, text);
            ConfigError::Parse { position, message }
        })?;
        // TLS is not built in yet, so `https://` would only fail later with a worse message.
        if !is_http(&config.connection.url) {
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
        let error = Config::parse("[connection]\nurl = 5\n").expect_err("a number is not a URL");
        let ConfigError::Parse { position, message } = &error else {
            panic!("a wrong type is a parse error: {error:?}");
        };

        assert_eq!(*position, Some(TextPosition { line: 2, column: 7 }));
        assert_eq!(message, "invalid type: integer, expected a string");
        assert_eq!(
            error.to_string(),
            "cannot parse config.toml: line 2, column 7: invalid type: integer, expected a string"
        );
        assert!(!error.to_string().contains('\n'), "{error}");
    }

    #[test]
    fn config_broken_toml_is_a_parse_error_with_a_position() {
        let error = Config::parse("[connection\nurl = 1").expect_err("not TOML");

        assert!(
            matches!(
                error,
                ConfigError::Parse {
                    position: Some(TextPosition { line: 1, .. }),
                    ..
                }
            ),
            "{error:?}"
        );
    }

    #[test]
    fn config_accepts_the_http_scheme_in_any_case() {
        for url in [
            "http://x/rpc",
            "HTTP://x/rpc",
            "Http://x/rpc",
            "hTTp://x/rpc",
        ] {
            let text = format!("[connection]\nurl = {url:?}\n");

            let config = Config::parse(&text).expect("the scheme is case-insensitive");

            // The URL is kept as written.
            assert_eq!(config.connection.url, url);
        }
    }

    #[test]
    fn config_rejects_urls_that_only_look_like_http() {
        for url in [
            "httpx://x",
            "xhttp://x",
            "http:/x",
            "http//x",
            "http:",
            "htt",
            " http://x",
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
    fn config_errors_say_what_is_wrong() {
        let message = |url: &str| ConfigError::InvalidUrl(url.to_owned()).to_string();

        assert_eq!(
            message("ftp://x"),
            "connection.url must start with http://, but it uses the scheme \"ftp\""
        );
        assert_eq!(
            message("https://pi.local/rpc"),
            "connection.url must start with http://; https is not supported yet, only plain http"
        );
        assert_eq!(
            message("HTTPS://pi.local/rpc"),
            message("https://pi.local/rpc")
        );
        for url in ["192.168.1.5:9091", "", "pi.local/rpc", "http:/x"] {
            assert_eq!(
                message(url),
                "connection.url must start with http://, but it has no scheme",
                "{url:?}"
            );
        }
        let parse = |position| ConfigError::Parse {
            position,
            message: "boom".to_owned(),
        };
        assert_eq!(
            parse(Some(TextPosition { line: 1, column: 2 })).to_string(),
            "cannot parse config.toml: line 1, column 2: boom"
        );
        assert_eq!(parse(None).to_string(), "cannot parse config.toml: boom");
    }

    #[test]
    fn config_errors_never_show_the_url() {
        let secret = "hunter2";
        for url in [
            format!("https://user:{secret}@pi/rpc"),
            format!("ftp://user:{secret}@pi/rpc"),
            format!("user:{secret}@pi/rpc"),
            format!("user:{secret}://pi/rpc"),
            secret.to_owned(),
        ] {
            let text = format!("[connection]\nurl = {url:?}\n");

            let error = Config::parse(&text).expect_err("the scheme is not http");

            assert_eq!(
                error,
                ConfigError::InvalidUrl(url.clone()),
                "the caller can still read it"
            );
            for shown in [
                error.to_string(),
                format!("{error:?}"),
                format!("{error:#?}"),
            ] {
                assert!(!shown.contains(secret), "{url}: {shown}");
                assert!(!shown.contains("user"), "{url}: {shown}");
            }
        }
    }

    #[test]
    fn config_parse_errors_never_show_the_value() {
        let secret = "hunter2";
        for text in [
            format!("connection = \"https://user:{secret}@pi/rpc\"\n"),
            format!("[connection]\nurl = 5\nother = \"{secret}\"\n"),
            format!("connection = {{ url = [\"{secret}\"] }}\n"),
        ] {
            let error = Config::parse(&text).expect_err("a wrong type");
            assert!(
                matches!(error, ConfigError::Parse { .. }),
                "{text}: a wrong type is a parse error"
            );

            for shown in [error.to_string(), format!("{error:?}")] {
                assert!(!shown.contains(secret), "{shown}");
                assert!(!shown.contains("user"), "{shown}");
            }
        }
    }
}
