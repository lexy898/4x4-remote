//! The time to finish, reduced to what the ETA column shows.

use std::time::Duration;

use crate::format::duration_parts;
use crate::model::{Eta, Torrent};
use crate::state::DisplayState;

/// A unit of a [`CompactDuration`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimeUnit {
    /// 24 hours.
    Days,
    /// 60 minutes.
    Hours,
    /// 60 seconds.
    Minutes,
    /// One second.
    Seconds,
}

/// A duration reduced to its two largest non-zero units, as the ETA column shows it: `1 d 1 h`,
/// `2 m 5 s`, `59 s`.
///
/// Units in between that are zero are skipped, so a day and five minutes is `1 d 5 m`. A zero
/// duration is `0 s`. The UI words the units through its i18n layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CompactDuration {
    /// The largest non-zero unit and its amount.
    pub major: (u64, TimeUnit),
    /// The next non-zero unit below `major`; `None` when nothing is left.
    pub minor: Option<(u64, TimeUnit)>,
}

impl From<Duration> for CompactDuration {
    /// Drops fractions of a second.
    fn from(duration: Duration) -> Self {
        let parts = duration_parts(duration);
        let mut units = [
            (parts.days, TimeUnit::Days),
            (u64::from(parts.hours), TimeUnit::Hours),
            (u64::from(parts.minutes), TimeUnit::Minutes),
            (u64::from(parts.seconds), TimeUnit::Seconds),
        ]
        .into_iter()
        .filter(|&(amount, _)| amount > 0);
        Self {
            major: units.next().unwrap_or((0, TimeUnit::Seconds)),
            minor: units.next(),
        }
    }
}

/// What the ETA column shows for a torrent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EtaView {
    /// Nothing: the torrent is not downloading, or has no estimate.
    None,
    /// The server cannot tell yet.
    Unknown,
    /// Done in about this long.
    In(CompactDuration),
}

impl EtaView {
    /// The estimate is shown only while the torrent downloads: a seeder or a paused torrent has
    /// none to show, whatever the server reports.
    pub(crate) fn of(torrent: &Torrent) -> Self {
        match (torrent.state, torrent.eta) {
            (DisplayState::Downloading, Eta::In(left)) => Self::In(left.into()),
            (DisplayState::Downloading, Eta::Unknown) => Self::Unknown,
            _ => Self::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::torrent;

    fn compact(seconds: u64) -> CompactDuration {
        CompactDuration::from(Duration::from_secs(seconds))
    }

    #[test]
    fn compact_duration_two_largest_units() {
        assert_eq!(
            compact(90_061),
            CompactDuration {
                major: (1, TimeUnit::Days),
                minor: Some((1, TimeUnit::Hours)),
            }
        );
        assert_eq!(
            compact(59),
            CompactDuration {
                major: (59, TimeUnit::Seconds),
                minor: None,
            }
        );
        assert_eq!(
            compact(3_600),
            CompactDuration {
                major: (1, TimeUnit::Hours),
                minor: None,
            }
        );
    }

    #[test]
    fn compact_duration_skips_zero_units_between_the_two_it_shows() {
        // 1 d 0 h 5 m, and 1 h 0 m 1 s.
        assert_eq!(
            compact(86_700),
            CompactDuration {
                major: (1, TimeUnit::Days),
                minor: Some((5, TimeUnit::Minutes)),
            }
        );
        assert_eq!(
            compact(3_601),
            CompactDuration {
                major: (1, TimeUnit::Hours),
                minor: Some((1, TimeUnit::Seconds)),
            }
        );
    }

    #[test]
    fn compact_duration_of_nothing_is_zero_seconds() {
        assert_eq!(
            compact(0),
            CompactDuration {
                major: (0, TimeUnit::Seconds),
                minor: None,
            }
        );
        assert_eq!(
            CompactDuration::from(Duration::from_millis(999)),
            compact(0)
        );
    }

    #[test]
    fn compact_duration_counts_days_beyond_a_month() {
        assert_eq!(
            compact(400 * 86_400 + 7_200),
            CompactDuration {
                major: (400, TimeUnit::Days),
                minor: Some((2, TimeUnit::Hours)),
            }
        );
    }

    fn in_state(state: DisplayState, eta: Eta) -> Torrent {
        Torrent {
            state,
            eta,
            ..torrent("a")
        }
    }

    #[test]
    fn eta_only_while_downloading() {
        let ninety_seconds = Eta::In(Duration::from_secs(90));
        assert_eq!(
            EtaView::of(&in_state(DisplayState::Seeding, ninety_seconds)),
            EtaView::None
        );
        assert_eq!(
            EtaView::of(&in_state(DisplayState::Paused, ninety_seconds)),
            EtaView::None
        );
        assert_eq!(
            EtaView::of(&in_state(DisplayState::Downloading, Eta::Unknown)),
            EtaView::Unknown
        );
        assert_eq!(
            EtaView::of(&in_state(DisplayState::Downloading, Eta::NotAvailable)),
            EtaView::None
        );
        assert_eq!(
            EtaView::of(&in_state(
                DisplayState::Downloading,
                Eta::In(Duration::from_secs(3_725))
            )),
            EtaView::In(CompactDuration {
                major: (1, TimeUnit::Hours),
                minor: Some((2, TimeUnit::Minutes)),
            })
        );
    }

    #[test]
    fn eta_is_hidden_outside_downloading_even_when_unknown() {
        assert_eq!(
            EtaView::of(&in_state(DisplayState::Seeding, Eta::Unknown)),
            EtaView::None
        );
        assert_eq!(
            EtaView::of(&in_state(DisplayState::Checking, Eta::Unknown)),
            EtaView::None
        );
    }
}
