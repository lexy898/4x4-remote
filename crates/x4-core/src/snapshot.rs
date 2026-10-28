//! What the UI reads: the torrents, the session, the totals and how the connection is doing.

use x4_rpc::{RpcError, SessionInfo, TransportError};

use crate::model::Torrent;

/// How the link to the daemon is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    /// The first connection is being made.
    Connecting,
    /// The daemon answers.
    Connected,
    /// The daemon stopped answering and the worker is trying again.
    Reconnecting {
        /// How many tries have failed in a row.
        attempt: u32,
        /// Why the latest one failed.
        last_error: ConnectionError,
    },
    /// The daemon answers, but speaks an RPC version 4x4 cannot use.
    Unsupported {
        /// The version the daemon reported.
        rpc_version: String,
    },
}

/// Why the daemon cannot be used right now, in the terms the UI tells the user.
///
/// The UI picks its text by variant, through its i18n layer; the `Display` text is for logs.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConnectionError {
    /// Nothing answers at the address: refused, no route, unknown host or a broken connection.
    #[error("the daemon cannot be reached")]
    Unreachable,
    /// The daemon did not answer in time.
    #[error("the daemon did not answer in time")]
    Timeout,
    /// The daemon does not accept requests from this host.
    #[error("the daemon does not accept requests from this host")]
    Forbidden,
    /// The daemon wants credentials, or rejected the ones given.
    #[error("the daemon rejected the credentials")]
    Unauthorized,
    /// The daemon answered, but not as a Transmission server does; holds what went wrong.
    #[error("the daemon answered unexpectedly: {0}")]
    Protocol(String),
    /// The daemon reported an error of its own; holds its explanation.
    #[error("the daemon reported an error: {0}")]
    Server(String),
}

impl From<&RpcError> for ConnectionError {
    /// Sorts a failed call into what the user can do about it.
    ///
    /// An [`RpcError::UnsupportedServer`] is not a failure to connect, and the worker shows it as
    /// [`ConnectionState::Unsupported`]; mapped here it is a [`ConnectionError::Protocol`] like
    /// any other answer 4x4 cannot use.
    fn from(error: &RpcError) -> Self {
        match error {
            RpcError::Transport(TransportError::Timeout) => Self::Timeout,
            RpcError::Transport(TransportError::Connect(_) | TransportError::Io(_)) => {
                Self::Unreachable
            }
            RpcError::Unauthorized => Self::Unauthorized,
            RpcError::Forbidden => Self::Forbidden,
            // Transmission's `message` is generic ("system error"); the detail has the cause.
            RpcError::Server {
                message, detail, ..
            } => Self::Server(
                detail
                    .as_deref()
                    .filter(|detail| !detail.is_empty())
                    .unwrap_or(message)
                    .to_owned(),
            ),
            RpcError::HttpStatus(_)
            | RpcError::SessionIdMissing
            | RpcError::SessionIdRejected
            | RpcError::InvalidResponse(_)
            | RpcError::InvalidRequest(_)
            | RpcError::Selection(_)
            | RpcError::UnsupportedServer { .. } => Self::Protocol(error.to_string()),
        }
    }
}

/// The daemon's settings the main window shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    /// The daemon's version string.
    pub version: String,
    /// Where new torrents are saved by default.
    pub download_dir: String,
    /// Whether the alternative speed limits apply.
    pub alt_speed_enabled: bool,
    /// The download limit in kB/s; `None` when there is no limit.
    pub speed_limit_down_kbps: Option<u32>,
    /// The upload limit in kB/s; `None` when there is no limit.
    pub speed_limit_up_kbps: Option<u32>,
}

impl From<&SessionInfo> for SessionSummary {
    /// Takes the normal limits, which count only while their switch is on, and the alternative
    /// speed switch as it stands.
    fn from(info: &SessionInfo) -> Self {
        Self {
            version: info.version.clone(),
            download_dir: info.download_dir.clone(),
            alt_speed_enabled: info.alt_speed_enabled,
            speed_limit_down_kbps: info
                .speed_limit_down_enabled
                .then_some(info.speed_limit_down_kbps),
            speed_limit_up_kbps: info
                .speed_limit_up_enabled
                .then_some(info.speed_limit_up_kbps),
        }
    }
}

/// Everything the UI shows, as of one moment.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    /// Every torrent, in no particular order; sort them with [`sort`](crate::sort()).
    pub torrents: Vec<Torrent>,
    /// The daemon's settings; `None` until they have been read.
    pub session: Option<SessionSummary>,
    /// Free space where new torrents are saved; `None` when unknown.
    pub free_space_bytes: Option<u64>,
    /// The daemon's total download rate, in bytes per second.
    pub rate_down: u64,
    /// The daemon's total upload rate, in bytes per second.
    pub rate_up: u64,
    /// How the link to the daemon is doing.
    pub connection: ConnectionState,
    /// Whether the numbers are old: the daemon has not answered lately.
    pub stale: bool,
}

#[cfg(test)]
mod tests {
    use x4_rpc::testing::fixtures::SESSION_GET;

    use super::*;

    fn rpc_error_to_connection_error(error: &RpcError) -> ConnectionError {
        ConnectionError::from(error)
    }

    /// The `session_get` fixture with the given limit settings.
    fn session_info(down: (u32, bool), up: (u32, bool), alt_speed_enabled: bool) -> SessionInfo {
        let envelope: serde_json::Value = serde_json::from_str(SESSION_GET).unwrap();
        let mut info: SessionInfo = serde_json::from_value(envelope["result"].clone()).unwrap();
        info.speed_limit_down_kbps = down.0;
        info.speed_limit_down_enabled = down.1;
        info.speed_limit_up_kbps = up.0;
        info.speed_limit_up_enabled = up.1;
        info.alt_speed_enabled = alt_speed_enabled;
        info
    }

    #[test]
    fn connection_error_from_rpc_error() {
        let map = rpc_error_to_connection_error;
        let server = |message: &str, detail: Option<&str>| RpcError::Server {
            code: 3,
            message: message.to_owned(),
            detail: detail.map(str::to_owned),
        };

        assert_eq!(
            map(&RpcError::Transport(TransportError::Timeout)),
            ConnectionError::Timeout
        );
        assert_eq!(
            map(&RpcError::Transport(TransportError::Connect(
                "connection refused".to_owned()
            ))),
            ConnectionError::Unreachable
        );
        assert_eq!(
            map(&RpcError::Transport(TransportError::Io(
                "broken pipe".to_owned()
            ))),
            ConnectionError::Unreachable
        );
        assert_eq!(map(&RpcError::Unauthorized), ConnectionError::Unauthorized);
        assert_eq!(map(&RpcError::Forbidden), ConnectionError::Forbidden);
        // Transmission's message is generic ("system error"); the detail carries the cause.
        assert_eq!(
            map(&server("system error", Some("No such file or directory"))),
            ConnectionError::Server("No such file or directory".to_owned())
        );
        assert_eq!(
            map(&server("Method not found", None)),
            ConnectionError::Server("Method not found".to_owned())
        );
        assert_eq!(
            map(&server("system error", Some(""))),
            ConnectionError::Server("system error".to_owned())
        );
    }

    #[test]
    fn answers_4x4_cannot_use_are_protocol_errors_with_their_text() {
        let protocol = |error: RpcError| {
            let expected = ConnectionError::Protocol(error.to_string());
            assert_eq!(ConnectionError::from(&error), expected, "{error:?}");
        };

        protocol(RpcError::HttpStatus(500));
        protocol(RpcError::SessionIdMissing);
        protocol(RpcError::SessionIdRejected);
        protocol(RpcError::InvalidResponse("not JSON".to_owned()));
        protocol(RpcError::InvalidRequest("bad value".to_owned()));
        protocol(RpcError::UnsupportedServer {
            rpc_version: "5.4.0".to_owned(),
        });
        protocol(RpcError::Selection(
            x4_rpc::TorrentSelection::new(Vec::new()).unwrap_err(),
        ));
    }

    #[test]
    fn protocol_errors_keep_the_text_of_the_rpc_error() {
        let error = RpcError::HttpStatus(502);

        assert_eq!(
            ConnectionError::from(&error),
            ConnectionError::Protocol(
                "the server answered with unexpected HTTP status 502".to_owned()
            )
        );
    }

    #[test]
    fn session_summary_keeps_the_limits_that_are_on() {
        let info = session_info((300, true), (50, true), true);

        assert_eq!(
            SessionSummary::from(&info),
            SessionSummary {
                version: info.version.clone(),
                download_dir: info.download_dir.clone(),
                alt_speed_enabled: true,
                speed_limit_down_kbps: Some(300),
                speed_limit_up_kbps: Some(50),
            }
        );
    }

    #[test]
    fn session_summary_drops_the_limits_that_are_off() {
        let info = session_info((300, false), (50, false), false);

        let summary = SessionSummary::from(&info);

        assert!(!summary.alt_speed_enabled);
        assert_eq!(summary.speed_limit_down_kbps, None);
        assert_eq!(summary.speed_limit_up_kbps, None);
    }

    #[test]
    fn session_summary_judges_each_direction_on_its_own() {
        let summary = SessionSummary::from(&session_info((300, true), (50, false), false));
        assert_eq!(summary.speed_limit_down_kbps, Some(300));
        assert_eq!(summary.speed_limit_up_kbps, None);

        let summary = SessionSummary::from(&session_info((300, false), (50, true), false));
        assert_eq!(summary.speed_limit_down_kbps, None);
        assert_eq!(summary.speed_limit_up_kbps, Some(50));
    }

    #[test]
    fn a_limit_of_zero_that_is_on_stays_a_limit() {
        let summary = SessionSummary::from(&session_info((0, true), (0, true), false));

        assert_eq!(summary.speed_limit_down_kbps, Some(0));
        assert_eq!(summary.speed_limit_up_kbps, Some(0));
    }
}
