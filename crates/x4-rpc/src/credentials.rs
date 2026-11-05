//! The user name and password the daemon asks for (HTTP basic authentication, RFC 7617).

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

/// A user name and password for the daemon's HTTP basic authentication.
///
/// Its `Debug` prints the user name and hides the password, so a value that ends up in a log line
/// or a panic message does not leak it.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    /// The user name, which may not contain `:` (RFC 7617). Never trimmed.
    pub username: String,
    /// The password, never trimmed.
    pub password: String,
}

impl Credentials {
    /// Whether both the user name and the password are empty, which means no credentials at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.username.is_empty() && self.password.is_empty()
    }

    /// The `Authorization` header value: `Basic ` and the base64 of `username:password` as UTF-8.
    pub(crate) fn header_value(&self) -> String {
        let pair = format!("{}:{}", self.username, self.password);
        format!("Basic {}", STANDARD.encode(pair))
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("username", &self.username)
            .field("password", &"<hidden>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UreqTransport;
    use std::time::Duration;

    fn credentials(username: &str, password: &str) -> Credentials {
        Credentials {
            username: username.to_owned(),
            password: password.to_owned(),
        }
    }

    #[test]
    fn header_is_basic_base64_of_user_colon_password() {
        // The example of RFC 7617, section 2.
        assert_eq!(
            credentials("Aladdin", "open sesame").header_value(),
            "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
        );
    }

    #[test]
    fn header_encodes_utf8() {
        let expected = format!("Basic {}", STANDARD.encode("user:pässword".as_bytes()));

        assert_eq!(credentials("user", "pässword").header_value(), expected);
        // `ä` is two bytes in UTF-8 (not one, as in Latin-1), so the encoding is the longer one.
        assert_eq!(expected, "Basic dXNlcjpww6Rzc3dvcmQ=");
    }

    #[test]
    fn debug_never_shows_the_password() {
        let secret = credentials("alice", "hunter2");
        let transport = UreqTransport::new("http://127.0.0.1:9091/transmission/rpc", TIMEOUT)
            .with_credentials(Some(secret.clone()));

        for text in [
            format!("{secret:?}"),
            format!("{secret:#?}"),
            format!("{transport:?}"),
            format!("{transport:#?}"),
        ] {
            assert!(!text.contains("hunter2"), "{text}");
            // The header value holds the password too, in base64.
            assert!(!text.contains("YWxpY2U6aHVudGVyMg"), "{text}");
        }
        assert!(format!("{secret:?}").contains("alice"));
        assert!(format!("{secret:?}").contains("<hidden>"));
    }

    const TIMEOUT: Duration = Duration::from_secs(5);
}
