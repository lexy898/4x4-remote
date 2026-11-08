//! The status bar and the window's visibility.

use x4_rpc::UnsupportedReason;

use crate::engine::Visibility;
use crate::format::{HumanSize, HumanSpeed, human_size, human_speed};
use crate::limits::{KibPerSecond, kbps_to_kib};
use crate::snapshot::{ConnectionError, ConnectionState, SessionSummary, Snapshot, SpeedLimit};

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
    /// No server is set, so no service was started and no data will come until the user sets one.
    NotConfigured,
}

/// What the connection indicator of the status bar shows, one state at a time.
///
/// A service that is not running outranks what its last snapshot said: [`Indicator::ServiceFailed`],
/// then [`Indicator::NotConfigured`], then [`Indicator::ServiceStopped`], then the state of the
/// link to the daemon, of which only one holds at a time ([`Indicator::Unsupported`],
/// [`Indicator::Reconnecting`], [`Indicator::Connecting`], [`Indicator::Connected`]). The UI words
/// it and picks its color.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Indicator {
    /// The service could not be started.
    ServiceFailed,
    /// No server is set, so there is no service and nothing to connect to.
    NotConfigured,
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

/// The turtle and the normal speed limits, as the daemon's session has them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeedControls {
    /// Whether the alternative speed limits apply: the turtle is on.
    pub alt_speed: bool,
    /// The alternative download limit, in whole KiB/s.
    pub alt_down: KibPerSecond,
    /// The alternative upload limit, in whole KiB/s.
    pub alt_up: KibPerSecond,
    /// The normal download limit as the daemon stores it, also while it is off.
    pub limit_down: SpeedLimit,
    /// The normal upload limit as the daemon stores it, also while it is off.
    pub limit_up: SpeedLimit,
}

impl From<&SessionSummary> for SpeedControls {
    /// Takes the switches and the stored limits; the alternative speeds turn into KiB/s.
    fn from(session: &SessionSummary) -> Self {
        Self {
            alt_speed: session.alt_speed_enabled,
            alt_down: kbps_to_kib(session.alt_speed_down_kbps),
            alt_up: kbps_to_kib(session.alt_speed_up_kbps),
            limit_down: session.speed_limit_down,
            limit_up: session.speed_limit_up,
        }
    }
}

/// Whether the "download in order" switch of the Files tab can be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequentialSwitch {
    /// Actions can go to the daemon, and it can download in order (or the session is not known
    /// yet).
    Usable,
    /// Actions cannot go to the daemon now.
    Disabled,
    /// The daemon cannot download in order: it is older than Transmission 4.1.0-beta.2, and the
    /// UI says so, whether it is connected or not.
    NeedsNewerServer,
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
    /// The turtle and the speed limits; `None` while the session is not known, when neither
    /// can be shown or changed.
    pub speed: Option<SpeedControls>,
    /// Whether the daemon can download in order; `None` while the session is not known.
    pub sequential_supported: Option<bool>,
    /// Whether the service still runs.
    pub service: ServiceStatus,
}

impl StatusModel {
    /// The service stopped: the numbers will not change again, so they are stale. A service
    /// that never started stays [`ServiceStatus::Failed`], and a window with no server stays
    /// [`ServiceStatus::NotConfigured`] and not stale.
    pub fn service_stopped(&mut self) {
        match self.service {
            ServiceStatus::Running => self.service = ServiceStatus::Stopped,
            // A window with no server has a service channel that closes at once; nothing ran,
            // so there is no data to be old and nothing to report.
            ServiceStatus::NotConfigured => return,
            ServiceStatus::Stopped | ServiceStatus::Failed => {}
        }
        self.stale = true;
    }

    /// The service could not be started: nothing will come, and what is shown is stale.
    pub fn service_failed(&mut self) {
        self.service = ServiceStatus::Failed;
        self.stale = true;
    }

    /// No server is set, so no service was started: there is no data, which is not stale. A
    /// service that never started stays [`ServiceStatus::Failed`].
    pub fn not_configured(&mut self) {
        if self.service != ServiceStatus::Failed {
            self.service = ServiceStatus::NotConfigured;
            self.stale = false;
        }
    }

    /// Whether actions can go to the daemon: the service runs and the daemon answers. Only then
    /// does [`availability`](crate::availability) allow the actions that need the daemon.
    #[must_use]
    pub fn connected(&self) -> bool {
        self.service == ServiceStatus::Running && self.connection == ConnectionState::Connected
    }

    /// Whether the alternative speed limits apply; `false` while the session is not known.
    #[must_use]
    pub fn alt_speed(&self) -> bool {
        self.speed.is_some_and(|speed| speed.alt_speed)
    }

    /// The turtle and the speed limits when they can be changed: the session is known and
    /// actions can go to the daemon ([`StatusModel::connected`]). `None` otherwise, when the UI
    /// disables them.
    #[must_use]
    pub fn changeable_speed(&self) -> Option<SpeedControls> {
        self.speed.filter(|_| self.connected())
    }

    /// What the Files tab's "download in order" switch does. A daemon known to lack the feature
    /// outranks the connection; before the session is known a connected switch stays usable.
    #[must_use]
    pub fn sequential_switch(&self) -> SequentialSwitch {
        if self.sequential_supported == Some(false) {
            SequentialSwitch::NeedsNewerServer
        } else if self.connected() {
            SequentialSwitch::Usable
        } else {
            SequentialSwitch::Disabled
        }
    }

    /// What the connection indicator shows (see [`Indicator`] for which state wins).
    #[must_use]
    pub fn indicator(&self) -> Indicator {
        match self.service {
            ServiceStatus::Failed => Indicator::ServiceFailed,
            ServiceStatus::NotConfigured => Indicator::NotConfigured,
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
            speed: None,
            sequential_supported: None,
            service: ServiceStatus::Running,
        }
    }
}

/// The status bar for `snapshot`, which only a running service sends.
#[must_use]
pub fn status_model(snapshot: &Snapshot) -> StatusModel {
    StatusModel {
        rate_down: human_speed(snapshot.rate_down),
        rate_up: human_speed(snapshot.rate_up),
        free_space: snapshot.free_space_bytes.map(human_size),
        connection: snapshot.connection.clone(),
        stale: snapshot.stale,
        speed: snapshot.session.as_ref().map(SpeedControls::from),
        sequential_supported: snapshot
            .session
            .as_ref()
            .map(|session| session.sequential_download_supported),
        service: ServiceStatus::Running,
    }
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
    use crate::limits::KbPerSecond;

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
            kbps: KbPerSecond(0),
            enabled: false,
        };
        SessionSummary {
            version: "4.1.0".to_owned(),
            download_dir: "/downloads".to_owned(),
            alt_speed_enabled,
            speed_limit_down: off,
            speed_limit_up: off,
            alt_speed_down_kbps: KbPerSecond(0),
            alt_speed_up_kbps: KbPerSecond(0),
            start_added_torrents: true,
            sequential_download_supported: true,
        }
    }

    fn sequential_switch_with(supported: Option<bool>, connection: ConnectionState) -> StatusModel {
        status_model(&Snapshot {
            session: supported.map(|supported| SessionSummary {
                sequential_download_supported: supported,
                ..session(false)
            }),
            connection,
            ..snapshot()
        })
    }

    #[test]
    fn sequential_switch_needs_a_newer_server_when_the_session_lacks_it() {
        for connection in [ConnectionState::Connected, reconnecting()] {
            let status = sequential_switch_with(Some(false), connection.clone());
            assert_eq!(
                status.sequential_switch(),
                SequentialSwitch::NeedsNewerServer,
                "{connection:?}"
            );
        }
    }

    #[test]
    fn sequential_switch_is_usable_when_connected_and_supported() {
        let status = sequential_switch_with(Some(true), ConnectionState::Connected);
        assert_eq!(status.sequential_switch(), SequentialSwitch::Usable);
    }

    #[test]
    fn sequential_switch_is_usable_when_connected_before_the_session_is_known() {
        let status = sequential_switch_with(None, ConnectionState::Connected);
        assert_eq!(status.sequential_supported, None);
        assert_eq!(status.sequential_switch(), SequentialSwitch::Usable);
    }

    #[test]
    fn sequential_switch_is_disabled_while_not_connected() {
        let status = sequential_switch_with(Some(true), reconnecting());
        assert_eq!(status.sequential_switch(), SequentialSwitch::Disabled);
    }

    #[test]
    fn sequential_switch_is_disabled_once_the_service_stopped() {
        let mut status = sequential_switch_with(Some(true), ConnectionState::Connected);
        status.service_stopped();
        assert_eq!(status.sequential_switch(), SequentialSwitch::Disabled);
    }

    #[test]
    fn only_a_running_service_with_an_answering_daemon_is_connected() {
        let connected = status_model(&snapshot());
        assert!(connected.connected());

        for connection in [
            ConnectionState::Connecting,
            ConnectionState::Reconnecting {
                attempt: 1,
                last_error: ConnectionError::Timeout,
            },
            ConnectionState::Unsupported {
                reason: UnsupportedReason::NoVersion,
            },
        ] {
            let status = StatusModel {
                connection: connection.clone(),
                ..connected.clone()
            };
            assert!(!status.connected(), "{connection:?}");
        }
        let mut stopped = connected.clone();
        stopped.service_stopped();
        assert!(!stopped.connected(), "the last snapshot said connected");
        let mut failed = connected;
        failed.service_failed();
        assert!(!failed.connected());
        assert!(!StatusModel::default().connected(), "still connecting");
    }

    #[test]
    fn status_model_carries_the_e2_values() {
        let limit_down = SpeedLimit {
            kbps: KbPerSecond(1024),
            enabled: true,
        };
        // Off, but the daemon keeps the value.
        let limit_up = SpeedLimit {
            kbps: KbPerSecond(100),
            enabled: false,
        };
        let status = status_model(&Snapshot {
            session: Some(SessionSummary {
                // 48.83 and exactly 1000 KiB/s.
                alt_speed_down_kbps: KbPerSecond(50),
                alt_speed_up_kbps: KbPerSecond(1024),
                speed_limit_down: limit_down,
                speed_limit_up: limit_up,
                ..session(true)
            }),
            ..snapshot()
        });

        assert_eq!(
            status.speed,
            Some(SpeedControls {
                alt_speed: true,
                alt_down: KibPerSecond(49),
                alt_up: KibPerSecond(1000),
                limit_down,
                limit_up,
            })
        );
        assert!(status.alt_speed());

        // A session with every value zero and off still has controls; no session has none.
        let zeros = status_model(&Snapshot {
            session: Some(session(false)),
            ..snapshot()
        });
        assert!(zeros.speed.is_some());
        assert!(!zeros.alt_speed());
        assert_eq!(status_model(&snapshot()).speed, None);
    }

    #[test]
    fn the_speed_can_be_changed_only_with_a_session_and_a_connection() {
        let with_session = status_model(&Snapshot {
            session: Some(session(true)),
            ..snapshot()
        });
        assert_eq!(with_session.changeable_speed(), with_session.speed);
        assert!(with_session.changeable_speed().is_some());

        assert_eq!(
            status_model(&snapshot()).changeable_speed(),
            None,
            "no session"
        );
        let reconnecting = StatusModel {
            connection: ConnectionState::Reconnecting {
                attempt: 1,
                last_error: ConnectionError::Timeout,
            },
            ..with_session.clone()
        };
        assert_eq!(
            reconnecting.changeable_speed(),
            None,
            "the last session is kept"
        );
        let mut stopped = with_session;
        stopped.service_stopped();
        assert_eq!(stopped.changeable_speed(), None);
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
        assert!(status.alt_speed());
        assert_eq!(status.connection, ConnectionState::Connected);
        assert!(!status.stale);

        let unknown = status_model(&snapshot());
        assert_eq!(unknown.free_space, None);
        assert!(
            !unknown.alt_speed(),
            "no session means no alternative limits"
        );

        let normal = status_model(&Snapshot {
            session: Some(session(false)),
            ..snapshot()
        });
        assert!(!normal.alt_speed());
    }

    #[test]
    fn the_default_status_is_connecting_with_nothing_known() {
        let status = StatusModel::default();

        assert_eq!(status.connection, ConnectionState::Connecting);
        assert_eq!(status.rate_down, human_speed(0));
        assert_eq!(status.rate_up, human_speed(0));
        assert_eq!(status.free_space, None);
        assert!(!status.stale);
        assert!(!status.alt_speed());
        assert_eq!(status.speed, None);
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
    fn not_configured_shows_no_server_set() {
        let mut status = StatusModel::default();
        status.not_configured();

        assert_eq!(status.service, ServiceStatus::NotConfigured);
        assert_eq!(status.indicator(), Indicator::NotConfigured);
        assert!(!status.stale, "there is no data to be old");
        assert!(!status.connected(), "there is no daemon to send actions to");
        assert_eq!(status.changeable_speed(), None);

        // The link's state cannot say otherwise: no service ever sent it.
        for connection in [
            ConnectionState::Connected,
            ConnectionState::Connecting,
            reconnecting(),
            unsupported(),
        ] {
            let mut status = running(connection.clone());
            status.not_configured();
            assert_eq!(
                status.indicator(),
                Indicator::NotConfigured,
                "{connection:?}"
            );
        }
    }

    #[test]
    fn a_failed_service_outranks_no_server_set() {
        let mut status = StatusModel::default();
        status.not_configured();
        status.service_failed();
        assert_eq!(status.indicator(), Indicator::ServiceFailed);

        // In either order.
        let mut status = StatusModel::default();
        status.service_failed();
        status.not_configured();
        assert_eq!(status.indicator(), Indicator::ServiceFailed);
        assert!(status.stale, "a failure to start keeps what it said");
    }

    #[test]
    fn no_server_set_outranks_a_stopped_service_and_the_link() {
        // A window without a server has a service whose events channel closes at once.
        let mut status = StatusModel::default();
        status.not_configured();
        status.service_stopped();

        assert_eq!(status.service, ServiceStatus::NotConfigured);
        assert_eq!(status.indicator(), Indicator::NotConfigured);
        assert!(!status.stale, "nothing ran, so nothing is old");
    }

    #[test]
    fn a_new_status_model_is_running_again() {
        let mut gone = StatusModel::default();
        gone.not_configured();
        assert_eq!(gone.service, ServiceStatus::NotConfigured);

        // The next server gets a model of its own, so nothing of the empty one is left.
        let fresh = StatusModel::default();
        assert_eq!(fresh.service, ServiceStatus::Running);
        assert_eq!(fresh.indicator(), Indicator::Connecting);
        assert_eq!(status_model(&snapshot()).service, ServiceStatus::Running);
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
