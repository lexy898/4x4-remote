//! The sidebar filters, their counts and the name search.

use serde::{Deserialize, Serialize};

use crate::model::Torrent;
use crate::state::{Attention, DisplayState, QueueKind};

/// A sidebar filter over the torrent list. It is written to the state file by its `snake_case`
/// name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Filter {
    /// Every torrent.
    #[default]
    All,
    /// Downloading, or waiting in the download queue.
    Downloading,
    /// Seeding, or waiting in the seed queue.
    Seeding,
    /// Stopped.
    Paused,
    /// Data moves right now.
    Active,
    /// Stopped by a local error, or the tracker reports an error.
    Error,
}

impl Filter {
    /// Every filter, in the order the sidebar lists them: everything first, then by what the
    /// torrents do, and the errors last.
    pub const ALL: [Filter; 6] = [
        Self::All,
        Self::Downloading,
        Self::Seeding,
        Self::Paused,
        Self::Active,
        Self::Error,
    ];

    /// Whether `torrent` is shown under this filter.
    ///
    /// A torrent can match several filters (an active download is in `Downloading` and `Active`),
    /// and one that is verifying its data matches only `All`, plus `Active` if data moves.
    #[must_use]
    pub fn matches(self, torrent: &Torrent) -> bool {
        match self {
            Self::All => true,
            Self::Downloading => matches!(
                torrent.state,
                DisplayState::Downloading | DisplayState::Queued(QueueKind::Download)
            ),
            Self::Seeding => matches!(
                torrent.state,
                DisplayState::Seeding | DisplayState::Queued(QueueKind::Seed)
            ),
            Self::Paused => torrent.state == DisplayState::Paused,
            Self::Active => torrent.active,
            // A tracker error leaves the state alone, so it is looked up in the attention.
            Self::Error => {
                torrent.state == DisplayState::Error
                    || matches!(torrent.attention, Some(Attention::TrackerError(_)))
            }
        }
    }
}

/// How many torrents each [`Filter`] matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FilterCounts {
    /// Torrents matching [`Filter::All`].
    pub all: usize,
    /// Torrents matching [`Filter::Downloading`].
    pub downloading: usize,
    /// Torrents matching [`Filter::Seeding`].
    pub seeding: usize,
    /// Torrents matching [`Filter::Paused`].
    pub paused: usize,
    /// Torrents matching [`Filter::Active`].
    pub active: usize,
    /// Torrents matching [`Filter::Error`].
    pub error: usize,
}

impl FilterCounts {
    /// How many torrents `filter` matches.
    #[must_use]
    pub fn get(&self, filter: Filter) -> usize {
        match filter {
            Filter::All => self.all,
            Filter::Downloading => self.downloading,
            Filter::Seeding => self.seeding,
            Filter::Paused => self.paused,
            Filter::Active => self.active,
            Filter::Error => self.error,
        }
    }

    /// Whether the count of `filter` should catch the eye: only the errors do, and only when
    /// there are any, so that a torrent in trouble is noticed under any filter.
    #[must_use]
    pub fn needs_attention(&self, filter: Filter) -> bool {
        filter == Filter::Error && self.error > 0
    }
}

/// Counts the torrents under each filter in one pass.
#[must_use]
pub fn counts<'a>(torrents: impl IntoIterator<Item = &'a Torrent>) -> FilterCounts {
    let mut counts = FilterCounts::default();
    for torrent in torrents {
        counts.all += 1;
        counts.downloading += usize::from(Filter::Downloading.matches(torrent));
        counts.seeding += usize::from(Filter::Seeding.matches(torrent));
        counts.paused += usize::from(Filter::Paused.matches(torrent));
        counts.active += usize::from(Filter::Active.matches(torrent));
        counts.error += usize::from(Filter::Error.matches(torrent));
    }
    counts
}

/// Whether a torrent called `name` matches the search `query`.
///
/// The query is split on whitespace and every term must occur in the name, in any order and
/// ignoring case (Unicode lowercasing, so Cyrillic works). A query with no terms matches every
/// name.
#[must_use]
pub fn matches_search(name: &str, query: &str) -> bool {
    let mut terms = query.split_whitespace().peekable();
    // Most of the time the box is empty; do not lowercase every name for nothing.
    if terms.peek().is_none() {
        return true;
    }
    let name = name.to_lowercase();
    terms.all(|term| name.contains(&term.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::torrent;

    fn in_state(name: &str, state: DisplayState) -> Torrent {
        Torrent {
            state,
            ..torrent(name)
        }
    }

    /// One torrent per display state and per kind of attention.
    fn zoo() -> Vec<Torrent> {
        vec![
            Torrent {
                active: true,
                ..in_state("downloading", DisplayState::Downloading)
            },
            in_state("queued-download", DisplayState::Queued(QueueKind::Download)),
            Torrent {
                active: true,
                ..in_state("seeding", DisplayState::Seeding)
            },
            in_state("queued-seed", DisplayState::Queued(QueueKind::Seed)),
            in_state("paused", DisplayState::Paused),
            in_state("checking", DisplayState::Checking),
            Torrent {
                attention: Some(Attention::LocalError("disk full".to_owned())),
                ..in_state("local-error", DisplayState::Error)
            },
            Torrent {
                attention: Some(Attention::TrackerError("unreachable".to_owned())),
                ..torrent("tracker-error")
            },
            Torrent {
                attention: Some(Attention::TrackerWarning("slow down".to_owned())),
                ..torrent("tracker-warning")
            },
            Torrent {
                attention: Some(Attention::FetchingMetadata),
                ..in_state("fetching-metadata", DisplayState::Downloading)
            },
        ]
    }

    fn matching(filter: Filter, torrents: &[Torrent]) -> Vec<&str> {
        torrents
            .iter()
            .filter(|t| filter.matches(t))
            .map(|t| t.name.as_str())
            .collect()
    }

    #[test]
    fn filters_follow_rules() {
        let zoo = zoo();

        assert_eq!(
            matching(Filter::All, &zoo),
            [
                "downloading",
                "queued-download",
                "seeding",
                "queued-seed",
                "paused",
                "checking",
                "local-error",
                "tracker-error",
                "tracker-warning",
                "fetching-metadata",
            ]
        );
        assert_eq!(
            matching(Filter::Downloading, &zoo),
            ["downloading", "queued-download", "fetching-metadata"]
        );
        assert_eq!(
            matching(Filter::Seeding, &zoo),
            ["seeding", "queued-seed", "tracker-error", "tracker-warning"]
        );
        assert_eq!(matching(Filter::Paused, &zoo), ["paused"]);
        assert_eq!(matching(Filter::Active, &zoo), ["downloading", "seeding"]);
        assert_eq!(
            matching(Filter::Error, &zoo),
            ["local-error", "tracker-error"]
        );
    }

    #[test]
    fn counts_match_filters() {
        let zoo = zoo();
        let total = |filter| matching(filter, &zoo).len();

        assert_eq!(
            counts(&zoo),
            FilterCounts {
                all: total(Filter::All),
                downloading: total(Filter::Downloading),
                seeding: total(Filter::Seeding),
                paused: total(Filter::Paused),
                active: total(Filter::Active),
                error: total(Filter::Error),
            }
        );
        // Guards the comparison above against every count being zero.
        assert_eq!(
            counts(&zoo),
            FilterCounts {
                all: 10,
                downloading: 3,
                seeding: 4,
                paused: 1,
                active: 2,
                error: 2,
            }
        );
    }

    #[test]
    fn the_sidebar_lists_every_filter_once_in_order() {
        assert_eq!(
            Filter::ALL,
            [
                Filter::All,
                Filter::Downloading,
                Filter::Seeding,
                Filter::Paused,
                Filter::Active,
                Filter::Error,
            ]
        );
    }

    #[test]
    fn get_reads_the_count_of_each_filter() {
        let distinct = FilterCounts {
            all: 10,
            downloading: 3,
            seeding: 4,
            paused: 1,
            active: 2,
            error: 5,
        };

        assert_eq!(
            Filter::ALL.map(|filter| distinct.get(filter)),
            [10, 3, 4, 1, 2, 5]
        );
        assert_eq!(counts(&zoo()).get(Filter::Seeding), 4);
    }

    #[test]
    fn only_errors_need_attention_and_only_when_there_are_some() {
        let busy = FilterCounts {
            all: 9,
            downloading: 9,
            seeding: 9,
            paused: 9,
            active: 9,
            error: 1,
        };

        for filter in Filter::ALL {
            assert_eq!(
                busy.needs_attention(filter),
                filter == Filter::Error,
                "{filter:?}"
            );
        }
        let calm = FilterCounts { error: 0, ..busy };
        assert!(
            Filter::ALL
                .iter()
                .all(|filter| !calm.needs_attention(*filter))
        );
    }

    #[test]
    fn counts_of_nothing_are_zero() {
        assert_eq!(counts(std::iter::empty()), FilterCounts::default());
    }

    #[test]
    fn checking_only_in_all() {
        let checking = [in_state("checking", DisplayState::Checking)];

        assert_eq!(
            counts(&checking),
            FilterCounts {
                all: 1,
                ..FilterCounts::default()
            }
        );
        for filter in Filter::ALL {
            assert_eq!(
                filter.matches(&checking[0]),
                filter == Filter::All,
                "{filter:?}"
            );
        }
    }

    #[test]
    fn a_checking_torrent_that_moves_data_is_also_active() {
        let checking = [Torrent {
            active: true,
            ..in_state("checking", DisplayState::Checking)
        }];

        assert_eq!(
            counts(&checking),
            FilterCounts {
                all: 1,
                active: 1,
                ..FilterCounts::default()
            }
        );
    }

    #[test]
    fn search_is_case_insensitive_cyrillic() {
        assert!(matches_search("Тед Лассо.S01", "лассо s01"));
        assert!(matches_search("тед лассо.s01", "ЛАССО S01"));
        assert!(!matches_search("Тед Лассо.S01", "лассо s02"));
        assert!(matches_search("Тед Лассо.S01", ""));
    }

    #[test]
    fn search_needs_every_term_in_any_order() {
        let name = "Ubuntu 26.04 Desktop amd64.iso";

        assert!(matches_search(name, "desktop ubuntu"));
        assert!(matches_search(name, "26.04"));
        assert!(!matches_search(name, "ubuntu server"));
        assert!(!matches_search(name, "arm64"));
    }

    #[test]
    fn search_terms_are_separated_by_any_whitespace() {
        assert!(matches_search("Some Show S01", "  some\t s01\n"));
        assert!(!matches_search("Some Show S01", "some\ts02"));
    }

    #[test]
    fn search_of_only_whitespace_matches_everything() {
        assert!(matches_search("anything", "   "));
        assert!(matches_search("anything", " \t\n "));
        assert!(matches_search("", "   "));
    }

    #[test]
    fn search_terms_may_overlap_in_the_name() {
        assert!(matches_search("abcd", "abc bcd"));
    }

    #[test]
    fn search_of_an_empty_name_needs_an_empty_query() {
        assert!(matches_search("", ""));
        assert!(!matches_search("", "a"));
    }
}
