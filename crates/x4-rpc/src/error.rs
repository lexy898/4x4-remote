//! Errors of an RPC call.

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
    /// The request could not be encoded as JSON.
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
    #[error(
        "unsupported server: {}; 4x4 needs Transmission 4.1 or later (RPC version 6.0)",
        describe_version(.rpc_version)
    )]
    UnsupportedServer {
        /// The server's `rpc_version_semver`, or `"legacy protocol (Transmission 4.0 or
        /// older)"` when it answers in the protocol of those versions, which has no such field.
        rpc_version: String,
    },
    /// The request named no torrents or an invalid hash.
    #[error("invalid torrent selection: {0}")]
    Selection(IdError),
}

/// The `rpc_version` of [`RpcError::UnsupportedServer`] for a daemon that answers in the legacy
/// (pre-JSON-RPC) protocol of Transmission 4.0 and older.
pub(crate) const LEGACY_PROTOCOL: &str = "legacy protocol (Transmission 4.0 or older)";

fn describe_version(rpc_version: &str) -> String {
    if rpc_version.is_empty() {
        "it reports no RPC version".to_owned()
    } else if rpc_version == LEGACY_PROTOCOL {
        format!("it speaks the {LEGACY_PROTOCOL}")
    } else {
        format!("its RPC version is {rpc_version:?}")
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
        let message = |rpc_version: &str| {
            RpcError::UnsupportedServer {
                rpc_version: rpc_version.to_owned(),
            }
            .to_string()
        };
        let needs = "4x4 needs Transmission 4.1 or later (RPC version 6.0)";

        assert_eq!(
            message("5.4.0"),
            format!("unsupported server: its RPC version is \"5.4.0\"; {needs}")
        );
        assert_eq!(
            message(""),
            format!("unsupported server: it reports no RPC version; {needs}")
        );
        assert_eq!(
            message(LEGACY_PROTOCOL),
            format!(
                "unsupported server: it speaks the legacy protocol (Transmission 4.0 or older); {needs}"
            )
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
