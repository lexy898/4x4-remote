//! The line under a torrent's name.

use crate::model::Torrent;
use crate::state::{Attention, DisplayState, QueueKind};

use super::percent::percent;

/// What the line under a torrent's name says; the UI words it through its i18n layer.
///
/// The texts of the errors and the warning are the server's own and are shown as sent.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Subtitle {
    /// Downloading from `from_peers` of the `connected` peers.
    Downloading {
        /// Peers we download from.
        from_peers: u32,
        /// Connected peers.
        connected: u32,
    },
    /// Seeding to `to_peers` peers.
    Seeding {
        /// Peers we upload to.
        to_peers: u32,
    },
    /// Stopped.
    Paused,
    /// Waiting in a queue.
    Queued(QueueKind),
    /// Verifying local data.
    Checking {
        /// How much is verified, in whole percent.
        percent: u8,
    },
    /// A magnet link fetching the torrent's metadata.
    FetchingMetadata {
        /// How much is fetched, in whole percent.
        percent: u8,
    },
    /// A local error; empty when the server gave no text, which the UI words itself.
    Error(String),
    /// The tracker's warning.
    TrackerWarning(String),
    /// The tracker's error.
    TrackerError(String),
}

impl Subtitle {
    /// What `torrent` needs the user to see first: an error or a warning, then the metadata
    /// fetch, and only then what the torrent is doing.
    pub(crate) fn of(torrent: &Torrent) -> Self {
        match &torrent.attention {
            Some(Attention::LocalError(text)) => Self::Error(text.clone()),
            Some(Attention::TrackerError(text)) => Self::TrackerError(text.clone()),
            Some(Attention::TrackerWarning(text)) => Self::TrackerWarning(text.clone()),
            Some(Attention::FetchingMetadata) => Self::FetchingMetadata {
                percent: percent(torrent.metadata_progress),
            },
            None => Self::of_state(torrent),
        }
    }

    fn of_state(torrent: &Torrent) -> Self {
        match torrent.state {
            DisplayState::Downloading => Self::Downloading {
                from_peers: torrent.peers_sending_to_us,
                connected: torrent.peers_connected,
            },
            DisplayState::Seeding => Self::Seeding {
                to_peers: torrent.peers_getting_from_us,
            },
            DisplayState::Paused => Self::Paused,
            DisplayState::Queued(kind) => Self::Queued(kind),
            DisplayState::Checking => Self::Checking {
                percent: percent(torrent.recheck_progress),
            },
            // A local error normally comes with its text as the attention, handled above; this
            // is an error the server gave no text for, which the UI words itself.
            DisplayState::Error => Self::Error(String::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::torrent;

    fn in_state(state: DisplayState) -> Torrent {
        Torrent {
            state,
            ..torrent("a")
        }
    }

    fn with_attention(state: DisplayState, attention: Attention) -> Torrent {
        Torrent {
            attention: Some(attention),
            ..in_state(state)
        }
    }

    #[test]
    fn subtitle_precedence() {
        // A local error beats what the torrent was doing.
        assert_eq!(
            Subtitle::of(&with_attention(
                DisplayState::Seeding,
                Attention::LocalError("disk full".to_owned())
            )),
            Subtitle::Error("disk full".to_owned())
        );
        // A tracker warning keeps its text and beats the state.
        assert_eq!(
            Subtitle::of(&with_attention(
                DisplayState::Downloading,
                Attention::TrackerWarning("slow down".to_owned())
            )),
            Subtitle::TrackerWarning("slow down".to_owned())
        );
        assert_eq!(
            Subtitle::of(&with_attention(
                DisplayState::Seeding,
                Attention::TrackerError("unregistered".to_owned())
            )),
            Subtitle::TrackerError("unregistered".to_owned())
        );
        // Fetching metadata beats the state, with the percent of the metadata.
        assert_eq!(
            Subtitle::of(&Torrent {
                metadata_progress: 0.42,
                progress: 0.9,
                ..with_attention(DisplayState::Downloading, Attention::FetchingMetadata)
            }),
            Subtitle::FetchingMetadata { percent: 42 }
        );
        // Without attention the state decides.
        assert_eq!(
            Subtitle::of(&Torrent {
                peers_sending_to_us: 3,
                peers_connected: 7,
                peers_getting_from_us: 9,
                ..in_state(DisplayState::Downloading)
            }),
            Subtitle::Downloading {
                from_peers: 3,
                connected: 7
            }
        );
        assert_eq!(
            Subtitle::of(&Torrent {
                peers_sending_to_us: 3,
                peers_connected: 7,
                peers_getting_from_us: 2,
                ..in_state(DisplayState::Seeding)
            }),
            Subtitle::Seeding { to_peers: 2 }
        );
    }

    #[test]
    fn subtitle_follows_the_state_without_attention() {
        assert_eq!(
            Subtitle::of(&in_state(DisplayState::Paused)),
            Subtitle::Paused
        );
        assert_eq!(
            Subtitle::of(&in_state(DisplayState::Queued(QueueKind::Seed))),
            Subtitle::Queued(QueueKind::Seed)
        );
        assert_eq!(
            Subtitle::of(&in_state(DisplayState::Queued(QueueKind::Download))),
            Subtitle::Queued(QueueKind::Download)
        );
        assert_eq!(
            Subtitle::of(&Torrent {
                recheck_progress: 0.634,
                progress: 0.1,
                ..in_state(DisplayState::Checking)
            }),
            Subtitle::Checking { percent: 63 }
        );
    }

    #[test]
    fn an_error_without_text_has_an_empty_subtitle() {
        assert_eq!(
            Subtitle::of(&in_state(DisplayState::Error)),
            Subtitle::Error(String::new())
        );
        assert_eq!(
            Subtitle::of(&with_attention(
                DisplayState::Error,
                Attention::LocalError(String::new())
            )),
            Subtitle::Error(String::new())
        );
    }
}
