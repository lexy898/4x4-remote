//! What one test of a connection found.

use x4_rpc::{RpcError, ServerVersion, UnsupportedReason};

use crate::snapshot::ConnectionError;

/// The answer to the Test button of the Connection dialog: one `session_get` of the version
/// fields, through a client of its own.
///
/// The dialog words it by variant through its i18n layer, with the same texts the status bar uses
/// for the same reasons.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    /// The daemon answered and 4x4 can talk to it.
    Connected {
        /// The daemon's version string, such as `4.1.0 (abcdef0123)`.
        version: String,
    },
    /// The daemon could not be used, for this reason.
    Failed(ConnectionError),
    /// The daemon answered, but speaks an RPC version 4x4 cannot use.
    Unsupported(UnsupportedReason),
    /// The check never ran: its thread could not be started or ended without an answer. The
    /// dialog says so instead of staying on "Testing…" for good.
    NotRun,
}

impl From<Result<ServerVersion, RpcError>> for CheckOutcome {
    /// Sorts the answer of `check_server`: an [`RpcError::UnsupportedServer`] is its own outcome
    /// (the dialog names the version), any other error becomes the [`ConnectionError`] the status
    /// bar would show for it.
    fn from(answer: Result<ServerVersion, RpcError>) -> Self {
        match answer {
            Ok(server) => Self::Connected {
                version: server.version,
            },
            Err(RpcError::UnsupportedServer { reason }) => Self::Unsupported(reason),
            Err(error) => Self::Failed(ConnectionError::from(&error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use x4_rpc::{Protocol, RpcError, ServerVersion, TransportError, UnsupportedReason};

    use super::*;
    use crate::snapshot::ConnectionError;

    #[test]
    fn check_outcome_sorts_every_rpc_error() {
        let outcome = |error: RpcError| CheckOutcome::from(Err(error));

        assert_eq!(
            CheckOutcome::from(Ok(ServerVersion {
                version: "4.1.0 (abcdef0123)".to_owned(),
                rpc_semver: "6.0.0".to_owned(),
                rpc_version: None,
                protocol: Protocol::JsonRpc,
            })),
            CheckOutcome::Connected {
                version: "4.1.0 (abcdef0123)".to_owned()
            }
        );
        assert_eq!(
            outcome(RpcError::Unauthorized),
            CheckOutcome::Failed(ConnectionError::Unauthorized)
        );
        assert_eq!(
            outcome(RpcError::Forbidden),
            CheckOutcome::Failed(ConnectionError::Forbidden)
        );
        assert_eq!(
            outcome(RpcError::Transport(TransportError::Timeout)),
            CheckOutcome::Failed(ConnectionError::Timeout)
        );
        assert_eq!(
            outcome(RpcError::Transport(TransportError::Connect(
                "connection refused".to_owned()
            ))),
            CheckOutcome::Failed(ConnectionError::Unreachable)
        );
        assert_eq!(
            outcome(RpcError::UnsupportedServer {
                reason: UnsupportedReason::Version("5.0.1".to_owned())
            }),
            CheckOutcome::Unsupported(UnsupportedReason::Version("5.0.1".to_owned()))
        );
        // Not a Transmission server: the text of the error is all there is to show.
        let invalid = RpcError::InvalidResponse("not JSON".to_owned());
        assert_eq!(
            outcome(invalid.clone()),
            CheckOutcome::Failed(ConnectionError::Protocol(invalid.to_string()))
        );
    }
}
