//! The status bar and the window's visibility.

use crate::engine::Visibility;
use crate::format::{HumanSize, HumanSpeed, human_size, human_speed};
use crate::snapshot::{ConnectionState, Snapshot};

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
}

impl Default for StatusModel {
    /// The status bar before the first answer: connecting, no traffic, nothing known.
    fn default() -> Self {
        Self {
            rate_down: human_speed(0),
            rate_up: human_speed(0),
            free_space: None,
            connection: ConnectionState::Connecting,
            stale: false,
            alt_speed: false,
        }
    }
}

/// The status bar for `snapshot`.
#[must_use]
pub fn status_model(snapshot: &Snapshot) -> StatusModel {
    StatusModel {
        rate_down: human_speed(snapshot.rate_down),
        rate_up: human_speed(snapshot.rate_up),
        free_space: snapshot.free_space_bytes.map(human_size),
        connection: snapshot.connection.clone(),
        stale: snapshot.stale,
        alt_speed: snapshot
            .session
            .as_ref()
            .is_some_and(|session| session.alt_speed_enabled),
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
    use crate::snapshot::SessionSummary;

    fn snapshot() -> Snapshot {
        Snapshot {
            torrents: Vec::new(),
            session: None,
            free_space_bytes: None,
            rate_down: 0,
            rate_up: 0,
            connection: ConnectionState::Connected,
            stale: false,
        }
    }

    fn session(alt_speed_enabled: bool) -> SessionSummary {
        SessionSummary {
            version: "4.1.0".to_owned(),
            download_dir: "/downloads".to_owned(),
            alt_speed_enabled,
            speed_limit_down_kbps: None,
            speed_limit_up_kbps: None,
        }
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
    }

    #[test]
    fn status_model_passes_the_connection_and_staleness_on() {
        let connection = ConnectionState::Reconnecting {
            attempt: 3,
            last_error: crate::ConnectionError::Timeout,
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
