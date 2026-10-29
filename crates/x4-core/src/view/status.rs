//! The status bar and the window's visibility.

use x4_rpc::UnsupportedReason;

use crate::engine::Visibility;
use crate::format::{HumanSize, HumanSpeed, human_size, human_speed};
use crate::limits::{kbps_to_bytes_per_sec, kbps_to_kib};
use crate::snapshot::{ConnectionError, ConnectionState, Snapshot, SpeedLimit};

/// Whether the service thread is there to send snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ServiceStatus {
    /// It runs; the snapshots tell how the connection is doing.
    #[default]
    Running,
    /// It stopped or panicked: its events channel closed, so nothing will change any more.
    Stopped,
    /// It could not be started, so no data will come.
    Failed,
}

/// What the connection indicator of the status bar shows, one state at a time.
///
/// A service that is not running outranks what its last snapshot said: [`Indicator::ServiceFailed`],
/// then [`Indicator::ServiceStopped`], then the state of the link to the daemon, of which only one
/// holds at a time ([`Indicator::Unsupported`], [`Indicator::Reconnecting`],
/// [`Indicator::Connecting`], [`Indicator::Connected`]). The UI words it and picks its color.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Indicator {
    /// The service could not be started.
    ServiceFailed,
    /// The service stopped; the data shown is the last it sent.
    ServiceStopped,
    /// The daemon speaks an RPC version 4x4 cannot use, for this reason.
    Unsupported(UnsupportedReason),
    /// The daemon stopped answering and the service tries again.
    Reconnecting {
        /// How many tries have failed in a row.
        attempt: u32,
        /// Why the latest one failed.
        last_error: ConnectionError,
    },
    /// The first connection is being made.
    Connecting,
    /// The daemon answers.
    Connected,
}

/// What the status bar shows.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusModel {
    /// The daemon's total download rate.
    pub rate_down: HumanSpeed,
    /// The daemon's total upload rate.
    pub rate_up: HumanSpeed,
    /// Free space where new torrents are saved; `None` when unknown.
    pub free_space: Option<HumanSize>,
    /// How the link to the daemon is doing.
    pub connection: ConnectionState,
    /// Whether the numbers are old, because the daemon has not answered lately.
    pub stale: bool,
    /// Whether the alternative speed limits apply; `false` while the session is not known.
    pub alt_speed: bool,
    /// The alternative download limit; zero while the session is not known.
    pub alt_speed_down: HumanSpeed,
    /// The alternative upload limit; zero while the session is not known.
    pub alt_speed_up: HumanSpeed,
    /// The normal download limit in KiB/s; `None` while it is off or the session is not known.
    pub limit_down: Option<u32>,
    /// The normal upload limit in KiB/s; `None` while it is off or the session is not known.
    pub limit_up: Option<u32>,
    /// The download limit the daemon stores in KiB/s, also while it is off, to prefill its
    /// input; 0 while the session is not known.
    pub stored_limit_down: u32,
    /// The upload limit the daemon stores in KiB/s, also while it is off, to prefill its input;
    /// 0 while the session is not known.
    pub stored_limit_up: u32,
    /// Whether the service still runs.
    pub service: ServiceStatus,
}

impl StatusModel {
    /// The service stopped: the numbers will not change again, so they are stale. A service
    /// that never started stays [`ServiceStatus::Failed`].
    pub fn service_stopped(&mut self) {
        if self.service == ServiceStatus::Running {
            self.service = ServiceStatus::Stopped;
        }
        self.stale = true;
    }

    /// The service could not be started: nothing will come, and what is shown is stale.
    pub fn service_failed(&mut self) {
        self.service = ServiceStatus::Failed;
        self.stale = true;
    }

    /// What the connection indicator shows (see [`Indicator`] for which state wins).
    #[must_use]
    pub fn indicator(&self) -> Indicator {
        match self.service {
            ServiceStatus::Failed => Indicator::ServiceFailed,
            ServiceStatus::Stopped => Indicator::ServiceStopped,
            ServiceStatus::Running => match &self.connection {
                ConnectionState::Unsupported { reason } => Indicator::Unsupported(reason.clone()),
                ConnectionState::Reconnecting {
                    attempt,
                    last_error,
                } => Indicator::Reconnecting {
                    attempt: *attempt,
                    last_error: last_error.clone(),
                },
                ConnectionState::Connecting => Indicator::Connecting,
                ConnectionState::Connected => Indicator::Connected,
            },
        }
    }
}

impl Default for StatusModel {
    /// The status bar before the first answer: the service runs and connects, no traffic,
    /// nothing known.
    fn default() -> Self {
        Self {
            rate_down: human_speed(0),
            rate_up: human_speed(0),
            free_space: None,
            connection: ConnectionState::Connecting,
            stale: false,
            alt_speed: false,
            alt_speed_down: human_speed(0),
            alt_speed_up: human_speed(0),
            limit_down: None,
            limit_up: None,
            stored_limit_down: 0,
            stored_limit_up: 0,
            service: ServiceStatus::Running,
        }
    }
}

/// The status bar for `snapshot`, which only a running service sends.
///
/// The daemon's limits are in kB/s; the status bar shows them in KiB/s, like every other speed.
#[must_use]
pub fn status_model(snapshot: &Snapshot) -> StatusModel {
    let session = snapshot.session.as_ref();
    let down = session.map(|session| session.speed_limit_down);
    let up = session.map(|session| session.speed_limit_up);
    let alt_down = session.map_or(0, |session| session.alt_speed_down_kbps);
    let alt_up = session.map_or(0, |session| session.alt_speed_up_kbps);
    StatusModel {
        rate_down: human_speed(snapshot.rate_down),
        rate_up: human_speed(snapshot.rate_up),
        free_space: snapshot.free_space_bytes.map(human_size),
        connection: snapshot.connection.clone(),
        stale: snapshot.stale,
        alt_speed: session.is_some_and(|session| session.alt_speed_enabled),
        alt_speed_down: human_speed(kbps_to_bytes_per_sec(alt_down)),
        alt_speed_up: human_speed(kbps_to_bytes_per_sec(alt_up)),
        limit_down: down.and_then(applied_kib),
        limit_up: up.and_then(applied_kib),
        stored_limit_down: down.map_or(0, |limit| kbps_to_kib(limit.kbps)),
        stored_limit_up: up.map_or(0, |limit| kbps_to_kib(limit.kbps)),
        service: ServiceStatus::Running,
    }
}

/// The limit in KiB/s while it applies.
fn applied_kib(limit: SpeedLimit) -> Option<u32> {
    limit.enabled.then(|| kbps_to_kib(limit.kbps))
}

/// Whether the user can see the window: it must be active and not hidden or minimised.
#[must_use]
pub fn visibility(window_active: bool, window_visible: bool) -> Visibility {
    if window_active && window_visible {
        Visibility::Visible
    } else {
        Visibility::Hidden
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::{SizeUnit, SpeedUnit};
    use crate::snapshot::{SessionSummary, SpeedLimit};

    fn snapshot() -> Snapshot {
        Snapshot {
            torrents: Vec::new(),
            listed: true,
            session: None,
            free_space_bytes: None,
            rate_down: 0,
            rate_up: 0,
            connection: ConnectionState::Connected,
            stale: false,
        }
    }

    fn session(alt_speed_enabled: bool) -> SessionSummary {
        let off = SpeedLimit {
            kbps: 0,
            enabled: false,
        };
        SessionSummary {
            version: "4.1.0".to_owned(),
            download_dir: "/downloads".to_owned(),
            alt_speed_enabled,
            speed_limit_down: off,
            speed_limit_up: off,
            alt_speed_down_kbps: 0,
            alt_speed_up_kbps: 0,
        }
    }

    #[test]
    fn status_model_carries_the_e2_values() {
        let status = status_model(&Snapshot {
            session: Some(SessionSummary {
                alt_speed_down_kbps: 50,
                alt_speed_up_kbps: 1024,
                // 1024 kB/s are exactly 1000 KiB/s.
                speed_limit_down: SpeedLimit {
                    kbps: 1024,
                    enabled: true,
                },
                // 100 kB/s are 97.66 KiB/s; the limit is off but keeps its value.
                speed_limit_up: SpeedLimit {
                    kbps: 100,
                    enabled: false,
                },
                ..session(true)
            }),
            ..snapshot()
        });

        assert!(status.alt_speed);
        // The daemon's kB/s are 1000 bytes per second.
        assert_eq!(status.alt_speed_down, human_speed(50_000));
        assert_eq!(status.alt_speed_up, human_speed(1_024_000));
        assert_eq!(status.alt_speed_up.unit, SpeedUnit::KiBPerSec);
        assert_eq!(status.limit_down, Some(1000));
        assert_eq!(status.limit_up, None);
        assert_eq!(status.stored_limit_down, 1000);
        assert_eq!(status.stored_limit_up, 98);

        let unknown = status_model(&snapshot());
        assert!(!unknown.alt_speed);
        assert_eq!(unknown.alt_speed_down, human_speed(0));
        assert_eq!(unknown.alt_speed_up, human_speed(0));
        assert_eq!((unknown.limit_down, unknown.limit_up), (None, None));
        assert_eq!((unknown.stored_limit_down, unknown.stored_limit_up), (0, 0));
    }

    #[test]
    fn status_model_maps_snapshot() {
        let status = status_model(&Snapshot {
            rate_down: 2_516_582,
            rate_up: 184_320,
            free_space_bytes: Some(0),
            session: Some(session(true)),
            ..snapshot()
        });

        assert_eq!(status.rate_down, human_speed(2_516_582));
        assert_eq!(status.rate_down.unit, SpeedUnit::MiBPerSec);
        assert_eq!(status.rate_up, human_speed(184_320));
        assert_eq!(
            status.free_space,
            Some(HumanSize {
                value: 0.0,
                unit: SizeUnit::B
            })
        );
        assert!(status.alt_speed);
        assert_eq!(status.connection, ConnectionState::Connected);
        assert!(!status.stale);

        let unknown = status_model(&snapshot());
        assert_eq!(unknown.free_space, None);
        assert!(!unknown.alt_speed, "no session means no alternative limits");

        let normal = status_model(&Snapshot {
            session: Some(session(false)),
            ..snapshot()
        });
        assert!(!normal.alt_speed);
    }

    #[test]
    fn the_default_status_is_connecting_with_nothing_known() {
        let status = StatusModel::default();

        assert_eq!(status.connection, ConnectionState::Connecting);
        assert_eq!(status.rate_down, human_speed(0));
        assert_eq!(status.rate_up, human_speed(0));
        assert_eq!(status.free_space, None);
        assert!(!status.stale);
        assert!(!status.alt_speed);
        assert_eq!(status.alt_speed_down, human_speed(0));
        assert_eq!(status.alt_speed_up, human_speed(0));
        assert_eq!((status.limit_down, status.limit_up), (None, None));
        assert_eq!((status.stored_limit_down, status.stored_limit_up), (0, 0));
        assert_eq!(status.service, ServiceStatus::Running);
        assert_eq!(status.indicator(), Indicator::Connecting);
    }

    fn reconnecting() -> ConnectionState {
        ConnectionState::Reconnecting {
            attempt: 2,
            last_error: ConnectionError::Timeout,
        }
    }

    fn unsupported() -> ConnectionState {
        ConnectionState::Unsupported {
            reason: UnsupportedReason::Version("5.0.1".to_owned()),
        }
    }

    fn running(connection: ConnectionState) -> StatusModel {
        StatusModel {
            connection,
            ..StatusModel::default()
        }
    }

    #[test]
    fn a_running_service_shows_the_state_of_the_link() {
        assert_eq!(
            running(ConnectionState::Connected).indicator(),
            Indicator::Connected
        );
        assert_eq!(
            running(ConnectionState::Connecting).indicator(),
            Indicator::Connecting
        );
        assert_eq!(
            running(reconnecting()).indicator(),
            Indicator::Reconnecting {
                attempt: 2,
                last_error: ConnectionError::Timeout
            }
        );
        assert_eq!(
            running(unsupported()).indicator(),
            Indicator::Unsupported(UnsupportedReason::Version("5.0.1".to_owned()))
        );
    }

    #[test]
    fn a_service_that_is_gone_outranks_every_connection_state() {
        for connection in [
            ConnectionState::Connected,
            ConnectionState::Connecting,
            reconnecting(),
            unsupported(),
        ] {
            let mut stopped = running(connection.clone());
            stopped.service_stopped();
            assert_eq!(
                stopped.indicator(),
                Indicator::ServiceStopped,
                "{connection:?}"
            );

            let mut failed = running(connection.clone());
            failed.service_failed();
            assert_eq!(
                failed.indicator(),
                Indicator::ServiceFailed,
                "{connection:?}"
            );

            // A failure to start outranks a stop, in either order.
            failed.service_stopped();
            assert_eq!(
                failed.indicator(),
                Indicator::ServiceFailed,
                "{connection:?}"
            );
            stopped.service_failed();
            assert_eq!(
                stopped.indicator(),
                Indicator::ServiceFailed,
                "{connection:?}"
            );
        }
    }

    #[test]
    fn a_service_that_is_gone_leaves_the_numbers_stale() {
        let fresh = status_model(&Snapshot {
            rate_down: 2_516_582,
            ..snapshot()
        });
        assert!(!fresh.stale);
        assert_eq!(fresh.service, ServiceStatus::Running);

        let mut stopped = fresh.clone();
        stopped.service_stopped();
        assert_eq!(stopped.service, ServiceStatus::Stopped);
        assert!(stopped.stale);
        assert_eq!(stopped.rate_down, fresh.rate_down, "the last numbers stay");

        let mut failed = StatusModel::default();
        failed.service_failed();
        assert_eq!(failed.service, ServiceStatus::Failed);
        assert!(failed.stale);
    }

    #[test]
    fn status_model_passes_the_connection_and_staleness_on() {
        let connection = ConnectionState::Reconnecting {
            attempt: 3,
            last_error: ConnectionError::Timeout,
        };

        let status = status_model(&Snapshot {
            connection: connection.clone(),
            stale: true,
            ..snapshot()
        });

        assert_eq!(status.connection, connection);
        assert!(status.stale);
    }

    #[test]
    fn visibility_requires_active_and_visible() {
        assert_eq!(visibility(true, true), Visibility::Visible);
        assert_eq!(visibility(true, false), Visibility::Hidden);
        assert_eq!(visibility(false, true), Visibility::Hidden);
        assert_eq!(visibility(false, false), Visibility::Hidden);
    }
}
