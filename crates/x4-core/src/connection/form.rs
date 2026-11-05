//! The fields of the Connection dialog, and the rules that turn their text into
//! [`ConnectionSettings`] or into the problems to show under the fields.

use std::borrow::Cow;
use std::fmt;

use percent_encoding::percent_decode_str;
use url::Host;
use x4_rpc::Credentials;

use super::{
    Address, ConnectionSettings, DEFAULT_PATH, DEFAULT_PORT, UrlProblem, http_url,
    is_basic_username, is_stray, starts_with_ignore_case,
};

/// The text of the dialog's fields, as the user typed it.
///
/// Host, port and path are trimmed when checked; the user name and the password never are. The
/// `Debug` output hides the password.
#[derive(Clone, PartialEq, Eq)]
pub struct ConnectionForm {
    /// The host, an IPv6 literal with or without brackets.
    pub host: String,
    /// The port, in digits.
    pub port: String,
    /// The path of the RPC endpoint, with or without its leading `/`.
    pub path: String,
    /// The user name; empty when the daemon asks for none.
    pub username: String,
    /// The password; empty when the daemon asks for none.
    pub password: String,
}

/// What is wrong with each field of a [`ConnectionForm`], for the dialog to show under it. A field
/// that is fine has no problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FormProblems {
    /// What is wrong with the host.
    pub host: Option<HostProblem>,
    /// What is wrong with the port.
    pub port: Option<PortProblem>,
    /// What is wrong with the path.
    pub path: Option<PathProblem>,
    /// What is wrong with the user name.
    pub username: Option<UsernameProblem>,
}

/// What is wrong with the host field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostProblem {
    /// Nothing is typed.
    Empty,
    /// It is not a host: it holds a space, a `/`, an `@`, a `?`, a `#` or another character a host
    /// cannot, or it is not an address of any kind.
    Invalid,
    /// It starts with `https://`, which is not supported yet.
    Https,
}

/// What is wrong with the port field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortProblem {
    /// Nothing is typed.
    Empty,
    /// It holds more than ASCII digits.
    NotANumber,
    /// It is 0 or above 65535.
    OutOfRange,
}

/// What is wrong with the path field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathProblem {
    /// Nothing is typed.
    Empty,
    /// It holds a `?`, a `#`, a space or a control character.
    Invalid,
}

/// What is wrong with the user name field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsernameProblem {
    /// It holds a `:`, which HTTP basic authentication cannot carry (RFC 7617).
    Colon,
}

impl ConnectionForm {
    /// The form for `settings`, or for a first connection when there are none: no host, the
    /// default port and path of Transmission, and no credentials.
    #[must_use]
    pub fn of(settings: Option<&ConnectionSettings>) -> Self {
        let Some(settings) = settings else {
            return Self {
                host: String::new(),
                port: DEFAULT_PORT.to_string(),
                path: DEFAULT_PATH.to_owned(),
                username: String::new(),
                password: String::new(),
            };
        };
        let (username, password) = settings
            .credentials()
            .map(|credentials| (credentials.username.clone(), credentials.password.clone()))
            .unwrap_or_default();
        Self {
            host: settings.host().to_owned(),
            port: settings.port().to_string(),
            path: settings.path().to_owned(),
            username,
            password,
        }
    }

    /// The settings these fields stand for.
    ///
    /// Every field is checked, so the dialog can show all the problems at once. The settings are
    /// read from the address the fields make by [`ConnectionSettings::from_url`], so they are
    /// what `config.toml` would give for it.
    ///
    /// # Errors
    ///
    /// [`FormProblems`] with a problem for each field that is wrong, and at least one.
    pub fn check(&self) -> Result<ConnectionSettings, FormProblems> {
        let host = check_host(&self.host);
        let port = check_port(&self.port);
        let path = check_path(&self.path);
        let username = check_username(&self.username);
        if let (Ok(host), Ok(port), Ok(path), Ok(())) = (&host, &port, &path, &username) {
            let credentials = Credentials {
                username: self.username.clone(),
                password: self.password.clone(),
            };
            return ConnectionSettings::from_url(&http_url(host, *port, path), Some(credentials))
                .map_err(FormProblems::of);
        }
        Err(FormProblems {
            host: host.err(),
            port: port.err(),
            path: path.err(),
            username: username.err(),
        })
    }

    /// The form with a full address in the host field spread over the fields, or `None` if the
    /// host field does not hold one.
    ///
    /// A full address is `http://[user[:password]@]host[:port][/path]`, which fills every field it
    /// has: the user name and the password percent-decoded, no port meaning 80, and a path of `/`
    /// leaving the path as it is. It is also `host:port[/path]` with a number for a port (an IPv6
    /// host in brackets), which fills the host, the port and the path, if there is one, as a
    /// browser's address bar without the `http://` has them; it has no user name, query or
    /// fragment. Anything else is not spread, and the check tells what is wrong with it: a plain
    /// host, an IPv6 literal, an `https://` address, an address with a query or a fragment.
    ///
    /// Spreading changes nothing here; the dialog decides when to use the result.
    #[must_use]
    pub fn spread(&self) -> Option<Self> {
        let text = self.host.trim();
        if starts_with_ignore_case(text, "http://") {
            self.spread_address(text)
        } else {
            self.spread_host_port(text)
        }
    }

    /// Spreads `http://…`.
    fn spread_address(&self, text: &str) -> Option<Self> {
        let address = Address::parse(text).ok()?;
        let mut form = self.clone();
        form.host = address.host;
        form.port = address.port.to_string();
        if address.path != "/" {
            form.path = address.path;
        }
        if !address.username.is_empty() {
            form.username = decode(&address.username)?;
        }
        if let Some(password) = &address.password {
            form.password = decode(password)?;
        }
        Some(form)
    }

    /// Spreads `host:port` and `[v6]:port`, each with a path or without.
    fn spread_host_port(&self, text: &str) -> Option<Self> {
        // The host and the port end at the first `/`; the rest is the path.
        let (authority, path) = text.split_at(text.find('/').unwrap_or(text.len()));
        let (host, port) = match authority.strip_prefix('[') {
            Some(rest) => rest
                .split_once("]:")
                .filter(|(host, _)| host.contains(':'))?,
            None => authority.split_once(':')?,
        };
        // An IPv6 literal without brackets has more colons, and no digits-only port after the first.
        if host.is_empty() || port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let mut form = Self {
            host: host.to_owned(),
            port: port.to_owned(),
            ..self.clone()
        };
        if !path.is_empty() {
            // The rules of a full address for the path: a query, a fragment, a space or a control
            // character refuses it, as does a user name before the host.
            let address = Address::parse(&format!("http://{text}")).ok()?;
            if address.has_credentials() {
                return None;
            }
            if address.path != "/" {
                form.path = address.path;
            }
        }
        Some(form)
    }
}

impl fmt::Debug for ConnectionForm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionForm")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("path", &self.path)
            .field("username", &self.username)
            .field("password", &"<hidden>")
            .finish()
    }
}

impl FormProblems {
    /// The problem of the field that `from_url` found fault with.
    ///
    /// The field checks refuse everything the address could be refused for, so this only guards
    /// against the two disagreeing: the user name is blamed for its colon and the host for the
    /// rest, as the host is the part of the address the `url` crate has the last word on.
    fn of(problem: UrlProblem) -> Self {
        match problem {
            UrlProblem::UsernameColon => Self {
                username: Some(UsernameProblem::Colon),
                ..Self::default()
            },
            _ => Self {
                host: Some(HostProblem::Invalid),
                ..Self::default()
            },
        }
    }
}

/// The host text as the address wants it, without the brackets of an IPv6 literal.
fn check_host(text: &str) -> Result<String, HostProblem> {
    let text = text.trim();
    if text.is_empty() {
        return Err(HostProblem::Empty);
    }
    if starts_with_ignore_case(text, "https://") {
        return Err(HostProblem::Https);
    }
    // Brackets belong to an IPv6 literal, which is the only host with a colon in it. Hand the
    // `url` crate's host parser the literal in brackets whatever way it was typed, so that `[nas]`
    // is refused.
    let (bare, in_brackets) = match text
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    {
        Some(bare) => (bare, Cow::Borrowed(text)),
        None if text.contains(':') => (text, Cow::Owned(format!("[{text}]"))),
        None => (text, Cow::Borrowed(text)),
    };
    // The parser the address uses for its host, so a `/`, `@`, `?`, `#`, `\` or space cannot slip
    // through to change the address.
    Host::parse(&in_brackets).map_err(|_| HostProblem::Invalid)?;
    Ok(bare.to_owned())
}

/// The port as a number: ASCII digits only, from 1 to 65535.
fn check_port(text: &str) -> Result<u16, PortProblem> {
    let text = text.trim();
    if text.is_empty() {
        return Err(PortProblem::Empty);
    }
    // `parse` alone would also take a leading `+`.
    if !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(PortProblem::NotANumber);
    }
    // Only digits are left, so a failure is an overflow.
    match text.parse::<u16>() {
        Ok(port) if port != 0 => Ok(port),
        _ => Err(PortProblem::OutOfRange),
    }
}

/// The path with its leading `/`.
fn check_path(text: &str) -> Result<String, PathProblem> {
    let text = text.trim();
    if text.is_empty() {
        return Err(PathProblem::Empty);
    }
    if text.contains(['?', '#']) || text.chars().any(is_stray) {
        return Err(PathProblem::Invalid);
    }
    if text.starts_with('/') {
        Ok(text.to_owned())
    } else {
        Ok(format!("/{text}"))
    }
}

/// The user name, which `from_url` refuses for a colon as well.
fn check_username(username: &str) -> Result<(), UsernameProblem> {
    if is_basic_username(username) {
        Ok(())
    } else {
        Err(UsernameProblem::Colon)
    }
}

/// `text` with its percent-escapes decoded; `None` if they do not make UTF-8.
fn decode(text: &str) -> Option<String> {
    percent_decode_str(text)
        .decode_utf8()
        .ok()
        .map(Cow::into_owned)
}

#[cfg(test)]
mod tests;
