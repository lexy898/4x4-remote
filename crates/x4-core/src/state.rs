//! What a torrent is doing and what needs the user's attention, from the server's raw codes.

use x4_rpc::{ErrorKind, Status};

/// Which queue a waiting torrent is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QueueKind {
    /// Waiting for a free download slot.
    Download,
    /// Waiting for a free seeding slot.
    Seed,
}

/// The one state a torrent is shown in, deciding its icon and which filters it matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DisplayState {
    /// Downloading.
    Downloading,
    /// Seeding.
    Seeding,
    /// Stopped by the user (or by the server, for a state this version does not know).
    Paused,
    /// Waiting in a queue.
    Queued(QueueKind),
    /// Verifying local data, or waiting to.
    Checking,
    /// Stopped by a local error such as missing data or a full disk.
    Error,
}

/// A reason to draw the user's eye to a torrent, next to its [`DisplayState`].
///
/// The texts are the server's own and are shown as sent; they are not localized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attention {
    /// The tracker answered with a warning.
    TrackerWarning(String),
    /// The tracker answered with an error.
    TrackerError(String),
    /// A local error; the torrent is stopped.
    LocalError(String),
    /// A magnet link still fetching the torrent's metadata.
    FetchingMetadata,
}

/// The state a torrent is shown in.
///
/// A local error wins over the status because the torrent is stopped by it. A tracker problem
/// does not change the state: the torrent keeps downloading or seeding and shows an
/// [`Attention`] instead. A status this version does not know counts as paused.
#[must_use]
pub fn classify(status: Status, error: ErrorKind) -> DisplayState {
    if error == ErrorKind::LocalError {
        return DisplayState::Error;
    }
    match status {
        Status::Stopped | Status::Other(_) => DisplayState::Paused,
        Status::CheckWait | Status::Checking => DisplayState::Checking,
        Status::DownloadWait => DisplayState::Queued(QueueKind::Download),
        Status::Downloading => DisplayState::Downloading,
        Status::SeedWait => DisplayState::Queued(QueueKind::Seed),
        Status::Seeding => DisplayState::Seeding,
    }
}

/// The reason to draw the user's eye to a torrent, if any.
///
/// The server's error wins; otherwise a torrent whose `metadata_progress` is below 1.0 is
/// fetching its metadata. An error kind this version does not know counts as a tracker warning,
/// the mildest one that is still shown.
#[must_use]
pub fn attention(
    error: ErrorKind,
    error_string: &str,
    metadata_progress: f64,
) -> Option<Attention> {
    match error {
        ErrorKind::TrackerWarning | ErrorKind::Other(_) => {
            Some(Attention::TrackerWarning(error_string.to_owned()))
        }
        ErrorKind::TrackerError => Some(Attention::TrackerError(error_string.to_owned())),
        ErrorKind::LocalError => Some(Attention::LocalError(error_string.to_owned())),
        ErrorKind::None if metadata_progress < 1.0 => Some(Attention::FetchingMetadata),
        ErrorKind::None => None,
    }
}

/// Whether any data moves: the torrent is downloading or uploading right now.
#[must_use]
pub fn is_active(rate_down: u64, rate_up: u64) -> bool {
    rate_down > 0 || rate_up > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_error_beats_status() {
        assert_eq!(
            classify(Status::Seeding, ErrorKind::LocalError),
            DisplayState::Error
        );
        assert_eq!(
            classify(Status::Downloading, ErrorKind::LocalError),
            DisplayState::Error
        );
        assert_eq!(
            classify(Status::Stopped, ErrorKind::LocalError),
            DisplayState::Error
        );
    }

    #[test]
    fn tracker_error_keeps_status() {
        assert_eq!(
            classify(Status::Seeding, ErrorKind::TrackerError),
            DisplayState::Seeding
        );
        assert_eq!(
            classify(Status::Downloading, ErrorKind::TrackerWarning),
            DisplayState::Downloading
        );
        assert_eq!(
            attention(ErrorKind::TrackerError, "x", 1.0),
            Some(Attention::TrackerError("x".to_owned()))
        );
    }

    #[test]
    fn maps_every_status() {
        let expected = [
            (Status::Stopped, DisplayState::Paused),
            (Status::CheckWait, DisplayState::Checking),
            (Status::Checking, DisplayState::Checking),
            (
                Status::DownloadWait,
                DisplayState::Queued(QueueKind::Download),
            ),
            (Status::Downloading, DisplayState::Downloading),
            (Status::SeedWait, DisplayState::Queued(QueueKind::Seed)),
            (Status::Seeding, DisplayState::Seeding),
            (Status::Other(9), DisplayState::Paused),
        ];
        for (status, state) in expected {
            assert_eq!(classify(status, ErrorKind::None), state, "{status:?}");
        }
    }

    #[test]
    fn maps_every_wire_status_code() {
        // The wire codes 0 to 6 in order, as `Status::from(u8)` reads them.
        let expected = [
            DisplayState::Paused,
            DisplayState::Checking,
            DisplayState::Checking,
            DisplayState::Queued(QueueKind::Download),
            DisplayState::Downloading,
            DisplayState::Queued(QueueKind::Seed),
            DisplayState::Seeding,
        ];
        for (code, state) in (0_u8..=6).zip(expected) {
            assert_eq!(
                classify(Status::from(code), ErrorKind::None),
                state,
                "{code}"
            );
        }
    }

    #[test]
    fn magnet_without_metadata() {
        assert_eq!(
            attention(ErrorKind::None, "", 0.0),
            Some(Attention::FetchingMetadata)
        );
        assert_eq!(
            attention(ErrorKind::None, "", 0.99),
            Some(Attention::FetchingMetadata)
        );
    }

    #[test]
    fn complete_metadata_and_no_error_needs_no_attention() {
        assert_eq!(attention(ErrorKind::None, "", 1.0), None);
    }

    #[test]
    fn server_error_beats_missing_metadata() {
        assert_eq!(
            attention(ErrorKind::TrackerWarning, "slow down", 0.0),
            Some(Attention::TrackerWarning("slow down".to_owned()))
        );
        assert_eq!(
            attention(ErrorKind::LocalError, "no space", 0.5),
            Some(Attention::LocalError("no space".to_owned()))
        );
    }

    #[test]
    fn unknown_error_kind_is_a_tracker_warning() {
        assert_eq!(
            attention(ErrorKind::Other(7), "odd", 1.0),
            Some(Attention::TrackerWarning("odd".to_owned()))
        );
    }

    #[test]
    fn unknown_error_kind_keeps_status() {
        assert_eq!(
            classify(Status::Seeding, ErrorKind::Other(7)),
            DisplayState::Seeding
        );
        assert_eq!(
            classify(Status::Stopped, ErrorKind::Other(7)),
            DisplayState::Paused
        );
    }

    #[test]
    fn seeding_without_peers_is_idle() {
        assert!(!is_active(0, 0));
        assert!(is_active(0, 1));
        assert!(is_active(1, 0));
        assert!(is_active(5, 5));
    }
}
