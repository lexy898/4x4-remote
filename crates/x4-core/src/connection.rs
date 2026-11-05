//! Where the daemon is and who 4x4 talks to it as: one [`ConnectionSettings`], and the
//! [`ConnectionForm`] in which the user types them.
//!
//! The settings are the facts: a host, a port, a path and, if the daemon asks for them, the
//! [`Credentials`]. They are made from an address by [`ConnectionSettings::from_url`], which takes
//! `http://` only, and the user name and password apart from it: [`ConnectionSettings::url`]
//! never holds them, so it is safe to log. The form is the text of the dialog's fields and the
//! rules that turn it into settings, or into [`FormProblems`] to show under the fields.
//!
//! The form builds the address from its fields and reads it with `from_url`, so the dialog and
//! `config.toml` can never disagree about what an address is. The `url` crate has the last word on
//! hosts, ports and paths.

mod check;
mod form;

use std::fmt;

use url::{Host, ParseError, Url};
use x4_rpc::Credentials;

pub use check::CheckOutcome;
pub use form::{
    ConnectionForm, FormProblems, HostProblem, PathProblem, PortProblem, UsernameProblem,
};

/// The port Transmission listens on unless it is told otherwise.
pub const DEFAULT_PORT: u16 = 9091;

/// The path Transmission serves its RPC on unless it is told otherwise.
pub const DEFAULT_PATH: &str = "/transmission/rpc";

/// Where one daemon is and the credentials it asks for, if any.
///
/// The host is kept without the brackets of an IPv6 literal, and the path as the `url` crate
/// writes it (it starts with `/`, dot segments are resolved and what needs it is percent-encoded),
/// so reading [`url`](Self::url) again gives the same settings. The `Debug` output shows the user
/// name and hides the password.
#[derive(Clone, PartialEq, Eq)]
pub struct ConnectionSettings {
    host: String,
    port: u16,
    path: String,
    credentials: Option<Credentials>,
}

impl ConnectionSettings {
    /// Reads an `http://` address and takes the credentials to go with it.
    ///
    /// The scheme and the host are case-insensitive, no port means 80, and spaces around the text
    /// are ignored. Credentials with an empty user name and an empty password are no credentials.
    ///
    /// # Errors
    ///
    /// A [`UrlProblem`] says what is wrong with `url`, or with the user name, without repeating
    /// either: the scheme is not `http` or missing, the address has no host, holds a user name or
    /// password (they come apart from it), has a query or a fragment, or is not a valid address
    /// (this includes a port of 0, a space or a line break inside it, and `http:` without `//`).
    /// A user name with a `:` in it is refused too (RFC 7617).
    pub fn from_url(url: &str, credentials: Option<Credentials>) -> Result<Self, UrlProblem> {
        let address = Address::parse(url)?;
        if address.has_credentials() {
            return Err(UrlProblem::Credentials);
        }
        if credentials
            .as_ref()
            .is_some_and(|credentials| !is_basic_username(&credentials.username))
        {
            return Err(UrlProblem::UsernameColon);
        }
        Ok(Self {
            host: address.host,
            port: address.port,
            path: address.path,
            credentials: credentials.filter(|credentials| !credentials.is_empty()),
        })
    }

    /// The host, without the brackets of an IPv6 literal.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The port, spelled out even when it is the default 80.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The path, which starts with `/`.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The user name and password, if the daemon asks for them.
    #[must_use]
    pub fn credentials(&self) -> Option<&Credentials> {
        self.credentials.as_ref()
    }

    /// The address to send requests to: `http://host:port/path`, an IPv6 host in brackets. It
    /// never holds the credentials.
    #[must_use]
    pub fn url(&self) -> String {
        http_url(&self.host, self.port, &self.path)
    }

    /// The server as people name it: `host:port`, or `[v6]:port` for an IPv6 host.
    #[must_use]
    pub fn server_label(&self) -> String {
        format!("{}:{}", bracketed(&self.host), self.port)
    }
}

impl fmt::Debug for ConnectionSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `Credentials` hides its password in its own `Debug`.
        f.debug_struct("ConnectionSettings")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("path", &self.path)
            .field("credentials", &self.credentials)
            .finish()
    }
}

/// Why an address, or the credentials that go with it, cannot be used.
///
/// The UI words it by variant through its i18n layer. The `Display` text is for logs, and it never
/// repeats the address or the credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UrlProblem {
    /// The address has another scheme than `http`, or none.
    #[error("the address must start with http://")]
    NotHttp,
    /// The address is `https://`, which is not supported yet.
    #[error("https is not supported yet, only plain http")]
    Https,
    /// The address names no host.
    #[error("the address has no host")]
    NoHost,
    /// The address holds a user name or a password, which belong apart from it.
    #[error("the address holds a user name or a password; they are kept apart from it")]
    Credentials,
    /// The address has a query (`?`) or a fragment (`#`).
    #[error("the address has a query or a fragment")]
    QueryOrFragment,
    /// The address is not valid in some other way.
    #[error("the address is not valid")]
    Invalid,
    /// The user name holds a `:`, which HTTP basic authentication cannot carry (RFC 7617).
    #[error("the user name cannot contain a colon")]
    UsernameColon,
}

/// Whether `username` can be sent in HTTP basic authentication: it holds no `:` (RFC 7617), which
/// would end the user name early.
fn is_basic_username(username: &str) -> bool {
    !username.contains(':')
}

/// Whether `c` has no place inside an address or a path: whitespace and control characters. The
/// `url` crate would drop a tab or a line break from a URL without a word, and 4x4 would rather
/// say so.
fn is_stray(c: char) -> bool {
    c.is_whitespace() || c.is_control()
}

/// Whether `text` starts with `prefix`, whatever the case of its ASCII letters.
fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
}

/// `host` as it is written in an address: an IPv6 literal (the only host with a colon in it) in
/// brackets.
fn bracketed(host: &str) -> String {
    if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    }
}

/// `http://host:port/path`; `path` starts with `/`.
fn http_url(host: &str, port: u16, path: &str) -> String {
    format!("http://{}:{port}{path}", bracketed(host))
}

/// An `http://` address as the `url` crate reads it, with the user name and the password it holds,
/// if any, as written. The settings take it as is, the form's spread takes the credentials too.
struct Address {
    host: String,
    port: u16,
    path: String,
    /// The user name, still percent-encoded; empty when the address has none.
    username: String,
    /// The password, still percent-encoded.
    password: Option<String>,
}

impl Address {
    /// Reads `text` as an `http://` address with no query and no fragment.
    fn parse(text: &str) -> Result<Self, UrlProblem> {
        let text = text.trim();
        let url = Url::parse(text).map_err(|error| match error {
            // `x:9091` is a scheme to the `url` crate, but `192.168.1.5:9091` and "" are none.
            ParseError::RelativeUrlWithoutBase => UrlProblem::NotHttp,
            ParseError::EmptyHost => UrlProblem::NoHost,
            _ => UrlProblem::Invalid,
        })?;
        match url.scheme() {
            "http" => {}
            "https" => return Err(UrlProblem::Https),
            _ => return Err(UrlProblem::NotHttp),
        }
        // `http:nas` and `http:/nas` are the same address to the standard, but not to the user.
        if !starts_with_ignore_case(text, "http://") || text.chars().any(is_stray) {
            return Err(UrlProblem::Invalid);
        }
        let host = match url.host() {
            Some(Host::Domain(domain)) => domain.to_owned(),
            Some(Host::Ipv4(address)) => address.to_string(),
            Some(Host::Ipv6(address)) => address.to_string(),
            None => return Err(UrlProblem::NoHost),
        };
        if url.query().is_some() || url.fragment().is_some() {
            return Err(UrlProblem::QueryOrFragment);
        }
        // A daemon cannot listen on port 0. `http` always has a (known default) port.
        let port = url
            .port_or_known_default()
            .filter(|&port| port != 0)
            .ok_or(UrlProblem::Invalid)?;
        Ok(Self {
            host,
            port,
            path: url.path().to_owned(),
            username: url.username().to_owned(),
            password: url.password().map(str::to_owned),
        })
    }

    /// Whether the address holds a user name or a password.
    fn has_credentials(&self) -> bool {
        !self.username.is_empty() || self.password.is_some()
    }
}

#[cfg(test)]
mod tests;
