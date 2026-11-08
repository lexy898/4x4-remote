//! Errors of an RPC call.

use std::fmt;

use crate::ids::IdError;
use crate::transport::TransportError;

/// The `code` of an [`RpcError::Server`] from a legacy daemon, whose errors carry only text.
pub const LEGACY_ERROR_CODE: i64 = 0;

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
    /// The request could not be built: its params do not encode as JSON, they name nothing
    /// where the daemon would read nothing as everything (an empty file list), or they name
    /// something the legacy protocol has no name for.
    #[error("cannot encode the request: {0}")]
    InvalidRequest(String),
    /// A `torrent_add` of a magnet link with file choices (`files_unwanted`, `priority_high`,
    /// `priority_low`). A magnet's file list is empty when it is added and rebuilt when the
    /// metadata arrives, so the daemon would drop them (T/torrent.cc:858-866); the request is
    /// never sent.
    #[error(
        "torrent_add: a magnet link cannot carry file choices, which the daemon drops once it has \
         the metadata"
    )]
    MagnetWithFileChoices,
    /// The body is not a response to the request in the protocol the client speaks: not JSON, a
    /// different id or tag, neither `result` nor `error`, a `result` of the wrong shape, or an
    /// answer in the legacy protocol from a daemon the client found to speak JSON-RPC.
    #[error("invalid response: {0}")]
    InvalidResponse(String),
    /// The server answered with an error: a JSON-RPC error object, or a legacy `result` other
    /// than `"success"`.
    #[error("server error {code}: {message}{}", detail_suffix(.detail.as_deref()))]
    Server {
        /// The JSON-RPC error code; [`LEGACY_ERROR_CODE`] from a legacy daemon, except for a
        /// failed `free_space`, which gets the code 4.1 gives it (5).
        code: i64,
        /// The error message; from a legacy daemon, its text.
        message: String,
        /// `error.data.error_string`: what went wrong in more detail, if the server says; for a
        /// failed `free_space` of a legacy daemon, its text.
        detail: Option<String>,
    },
    /// The server speaks an RPC version 4x4 does not support (needs Transmission 3.00+).
    #[error("unsupported server: {reason}; 4x4 needs Transmission 3.00 or later (RPC version 16)")]
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
    /// The server reports no RPC version: its `rpc_version_semver` is empty, or a legacy daemon
    /// sends no `rpc-version`.
    NoVersion,
    /// The server reports an RPC version 4x4 cannot use: a legacy daemon's `rpc-version` below 16
    /// (before Transmission 3.00), held as its number in text (`"15"`); or from a JSON-RPC
    /// daemon, an `rpc_version_semver` below 6 or one that is not a version, held as the server
    /// wrote it.
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
        let needs = "4x4 needs Transmission 3.00 or later (RPC version 16)";

        assert_eq!(
            message(UnsupportedReason::Version("5.4.0".to_owned())),
            format!("unsupported server: its RPC version is \"5.4.0\"; {needs}")
        );
        assert_eq!(
            message(UnsupportedReason::NoVersion),
            format!("unsupported server: it reports no RPC version; {needs}")
        );
        assert_eq!(
            message(UnsupportedReason::Version("15".to_owned())),
            format!("unsupported server: its RPC version is \"15\"; {needs}")
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
