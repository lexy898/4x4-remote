//! One line of the torrent table.

use x4_rpc::HashString;

use crate::format::{HumanSize, HumanSpeed, human_size, human_speed};
use crate::model::{Ratio, Torrent};
use crate::state::DisplayState;

use super::eta::EtaView;
use super::percent::{fraction, percent};
use super::subtitle::Subtitle;

/// Everything the table shows for one torrent, decided; the view only draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// The stable identity: selection and anything persisted refer to the torrent by it.
    pub hash: HashString,
    /// The name as the server reports it.
    pub name: String,
    /// The state the torrent is shown in, which picks its icon and color.
    pub state: DisplayState,
    /// Whether the row is drawn muted: nothing moves and nothing needs the user's eye.
    pub dimmed: bool,
    /// The line under the name.
    pub subtitle: Subtitle,
    /// Progress in whole percent; 100 only when the torrent is done.
    pub percent: u8,
    /// Progress limited to 0.0 to 1.0, for the width of the bar.
    pub progress: f64,
    /// The size of the wanted files.
    pub size: HumanSize,
    /// The download rate.
    pub rate_down: HumanSpeed,
    /// The upload rate.
    pub rate_up: HumanSpeed,
    /// Whether anything is downloading, which highlights the download rate.
    pub down_active: bool,
    /// Whether anything is uploading, which highlights the upload rate.
    pub up_active: bool,
    /// The time to finish.
    pub eta: EtaView,
    /// The share ratio.
    pub ratio: Ratio,
    /// The connected peers.
    pub peers: u32,
}

impl From<&Torrent> for Row {
    fn from(torrent: &Torrent) -> Self {
        Self {
            hash: torrent.hash.clone(),
            name: torrent.name.clone(),
            state: torrent.state,
            dimmed: is_dimmed(torrent),
            subtitle: Subtitle::of(torrent),
            percent: percent(torrent.progress),
            progress: fraction(torrent.progress),
            size: human_size(torrent.size_when_done),
            rate_down: human_speed(torrent.rate_down),
            rate_up: human_speed(torrent.rate_up),
            down_active: torrent.rate_down > 0,
            up_active: torrent.rate_up > 0,
            eta: EtaView::of(torrent),
            ratio: torrent.ratio,
            peers: torrent.peers_connected,
        }
    }
}

/// Whether the row is muted: nothing moves, and the torrent is not doing something the user
/// waits for (downloading, verifying) or must act on (an error).
fn is_dimmed(torrent: &Torrent) -> bool {
    !torrent.active
        && !matches!(
            torrent.state,
            DisplayState::Downloading | DisplayState::Checking | DisplayState::Error
        )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::model::Eta;
    use crate::test_support::torrent;
    use crate::view::eta::{CompactDuration, TimeUnit};

    fn in_state(state: DisplayState) -> Torrent {
        Torrent {
            state,
            ..torrent("a")
        }
    }

    /// A seeding torrent that uploads at `rate_up` bytes per second.
    fn uploading(rate_up: u64) -> Torrent {
        Torrent {
            rate_up,
            active: rate_up > 0,
            ..in_state(DisplayState::Seeding)
        }
    }

    #[test]
    fn idle_seeder_and_paused_are_dimmed() {
        assert!(Row::from(&uploading(0)).dimmed);
        assert!(!Row::from(&uploading(1)).dimmed);
        assert!(Row::from(&in_state(DisplayState::Paused)).dimmed);
        assert!(Row::from(&in_state(DisplayState::Queued(crate::QueueKind::Download))).dimmed);
        assert!(!Row::from(&in_state(DisplayState::Error)).dimmed);
        assert!(!Row::from(&in_state(DisplayState::Downloading)).dimmed);
        assert!(!Row::from(&in_state(DisplayState::Checking)).dimmed);
    }

    #[test]
    fn a_paused_torrent_that_still_moves_data_is_not_dimmed() {
        let moving = Torrent {
            rate_down: 10,
            active: true,
            ..in_state(DisplayState::Paused)
        };
        assert!(!Row::from(&moving).dimmed);
    }

    #[test]
    fn a_row_shows_the_rates_and_whether_they_are_active() {
        let row = Row::from(&Torrent {
            rate_down: 2_516_582,
            rate_up: 0,
            active: true,
            ..in_state(DisplayState::Downloading)
        });

        assert_eq!(row.rate_down, human_speed(2_516_582));
        assert_eq!(row.rate_up, human_speed(0));
        assert!(row.down_active);
        assert!(!row.up_active);

        let row = Row::from(&uploading(5));
        assert!(!row.down_active);
        assert!(row.up_active);
    }

    #[test]
    fn a_row_carries_the_values_of_its_torrent() {
        let source = Torrent {
            progress: 0.29,
            size_when_done: 24_592_841_836,
            total_size: 1,
            ratio: Ratio::Value(1.5),
            peers_connected: 12,
            peers_sending_to_us: 5,
            peers_getting_from_us: 2,
            eta: Eta::In(Duration::from_secs(3_725)),
            ..in_state(DisplayState::Downloading)
        };

        let row = Row::from(&source);

        assert_eq!(row.hash, source.hash);
        assert_eq!(row.name, "a");
        assert_eq!(row.state, DisplayState::Downloading);
        // 0.29 * 100.0 is 28.999999999999996 in f64; the row still says 29.
        assert_eq!(row.percent, 29);
        assert!((row.progress - 0.29).abs() < f64::EPSILON);
        assert_eq!(row.size, human_size(24_592_841_836));
        assert_eq!(
            row.subtitle,
            Subtitle::Downloading {
                from_peers: 5,
                connected: 12
            }
        );
        assert_eq!(
            row.eta,
            EtaView::In(CompactDuration {
                major: (1, TimeUnit::Hours),
                minor: Some((2, TimeUnit::Minutes)),
            })
        );
        assert_eq!(row.ratio, Ratio::Value(1.5));
        assert_eq!(row.peers, 12);
    }

    #[test]
    fn the_progress_of_the_bar_stays_within_the_track() {
        let progress_of = |progress: f64| {
            Row::from(&Torrent {
                progress,
                ..in_state(DisplayState::Downloading)
            })
            .progress
        };

        assert!((progress_of(0.5) - 0.5).abs() < f64::EPSILON);
        assert!((progress_of(1.5) - 1.0).abs() < f64::EPSILON);
        assert!((progress_of(-0.5)).abs() < f64::EPSILON);
        assert!((progress_of(f64::NAN)).abs() < f64::EPSILON);
    }

    #[test]
    fn a_torrent_that_is_almost_done_never_shows_100_percent() {
        let row = Row::from(&Torrent {
            progress: 0.999,
            ..in_state(DisplayState::Downloading)
        });
        assert_eq!(row.percent, 99);

        let row = Row::from(&in_state(DisplayState::Seeding));
        assert_eq!(row.percent, 100);
    }
}
