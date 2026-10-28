//! Errors of an RPC call.

use std::fmt;

use crate::ids::IdError;
use crate::transport::TransportError;

/// Why an RPC call failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RpcError {
    /// No HTTP response: timeout, connection failure or I/O error.
    #[error("RPC transport failed: {0}")]
    Transport(TransportError),
    /// HTTP 401: the server requires credentials or rejected them.
    #[error("the server rejected the credentials (HTTP 401)")]
    Unauthorized,
    /// HTTP 403: the server does not accept requests from this host (`rpc_whitelist`).
    #[error("the server does not allow requests from this host (HTTP 403)")]
    Forbidden,
    /// Any HTTP status other than 200, 401, 403 and 409.
    #[error("the server answered with unexpected HTTP status {0}")]
    HttpStatus(u16),
    /// HTTP 409 without an `X-Transmission-Session-Id` header to retry with.
    #[error("the server answered 409 without an X-Transmission-Session-Id header")]
    SessionIdMissing,
    /// HTTP 409 again right after retrying with the session id the server sent.
    #[error("the server rejected its own X-Transmission-Session-Id twice in a row")]
    SessionIdRejected,
    /// The request could not be built: its params do not encode as JSON, or they name nothing
    /// where the daemon would read nothing as everything (an empty file list).
    #[error("cannot encode the request: {0}")]
    InvalidRequest(String),
    /// The body is not a JSON-RPC 2.0 response to the request: not JSON, a different id,
    /// neither `result` nor `error`, or a `result` of the wrong shape.
    #[error("invalid response: {0}")]
    InvalidResponse(String),
    /// The server answered with a JSON-RPC error object.
    #[error("server error {code}: {message}{}", detail_suffix(.detail.as_deref()))]
    Server {
        /// The JSON-RPC error code.
        code: i64,
        /// The error message.
        message: String,
        /// `error.data.error_string`: what went wrong in more detail, if the server says.
        detail: Option<String>,
    },
    /// The server speaks an RPC version 4x4 does not support (needs Transmission 4.1+).
    #[error("unsupported server: {reason}; 4x4 needs Transmission 4.1 or later (RPC version 6.0)")]
    UnsupportedServer {
        /// What the server reported that makes it unsupported.
        reason: UnsupportedReason,
    },
    /// The request named no torrents or an invalid hash.
    #[error("invalid torrent selection: {0}")]
    Selection(IdError),
}

/// Why a server's RPC version is not supported (see [`RpcError::UnsupportedServer`]).
///
/// A UI picks its text by variant, through its i18n layer; the `Display` text is for logs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnsupportedReason {
    /// The server answers in the legacy (pre-JSON-RPC) protocol of Transmission 4.0 and older,
    /// which has no `rpc_version_semver`.
    Legacy,
    /// The server reports no RPC version: its `rpc_version_semver` is empty.
    NoVersion,
    /// The server reports an RPC version below 6 (before Transmission 4.1), or one that is not a
    /// version; holds its `rpc_version_semver` as the server wrote it.
    Version(String),
}

impl UnsupportedReason {
    /// The reason for a server whose `rpc_version_semver` is `rpc_version_semver` and is not
    /// supported: [`NoVersion`](Self::NoVersion) when it is empty, else
    /// [`Version`](Self::Version) with the text as sent.
    #[must_use]
    pub fn from_rpc_semver(rpc_version_semver: &str) -> Self {
        if rpc_version_semver.is_empty() {
            Self::NoVersion
        } else {
            Self::Version(rpc_version_semver.to_owned())
        }
    }
}

impl fmt::Display for UnsupportedReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Legacy => {
                formatter.write_str("it speaks the legacy protocol (Transmission 4.0 or older)")
            }
            Self::NoVersion => formatter.write_str("it reports no RPC version"),
            Self::Version(version) => write!(formatter, "its RPC version is {version:?}"),
        }
    }
}

fn detail_suffix(detail: Option<&str>) -> String {
    detail
        .map(|detail| format!(" ({detail})"))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_error_message_includes_the_detail() {
        let error = RpcError::Server {
            code: 5,
            message: "system error".to_owned(),
            detail: Some("No such file or directory".to_owned()),
        };

        assert_eq!(
            error.to_string(),
            "server error 5: system error (No such file or directory)"
        );
    }

    #[test]
    fn server_error_message_without_detail() {
        let error = RpcError::Server {
            code: -32601,
            message: "Method not found".to_owned(),
            detail: None,
        };

        assert_eq!(error.to_string(), "server error -32601: Method not found");
    }

    #[test]
    fn unsupported_server_message_says_what_the_server_reported() {
        let message =
            |reason: UnsupportedReason| RpcError::UnsupportedServer { reason }.to_string();
        let needs = "4x4 needs Transmission 4.1 or later (RPC version 6.0)";

        assert_eq!(
            message(UnsupportedReason::Version("5.4.0".to_owned())),
            format!("unsupported server: its RPC version is \"5.4.0\"; {needs}")
        );
        assert_eq!(
            message(UnsupportedReason::NoVersion),
            format!("unsupported server: it reports no RPC version; {needs}")
        );
        assert_eq!(
            message(UnsupportedReason::Legacy),
            format!(
                "unsupported server: it speaks the legacy protocol (Transmission 4.0 or older); {needs}"
            )
        );
    }

    #[test]
    fn a_reported_version_becomes_a_reason() {
        assert_eq!(
            UnsupportedReason::from_rpc_semver(""),
            UnsupportedReason::NoVersion
        );
        assert_eq!(
            UnsupportedReason::from_rpc_semver("5.4.0"),
            UnsupportedReason::Version("5.4.0".to_owned())
        );
        // A version string that is no version is still what the server sent.
        assert_eq!(
            UnsupportedReason::from_rpc_semver("abc"),
            UnsupportedReason::Version("abc".to_owned())
        );
    }

    #[test]
    fn wrapped_errors_keep_their_message() {
        let error = RpcError::Transport(TransportError::Connect("connection refused".to_owned()));

        assert_eq!(
            error.to_string(),
            "RPC transport failed: cannot connect to the server: connection refused"
        );
    }
}
