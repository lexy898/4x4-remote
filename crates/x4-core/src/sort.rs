//! Ordering the torrent list by a column.

use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

use crate::model::{Eta, Ratio, Torrent};
use crate::state::{DisplayState, QueueKind};
use crate::view::EtaView;

/// The column the list is sorted by. It is written to the state file by its `snake_case` name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortKey {
    /// The name, ignoring case.
    #[default]
    Name,
    /// The size of the wanted files.
    Size,
    /// The progress of the wanted files.
    Progress,
    /// The display state: errors first, paused last.
    State,
    /// The download rate.
    DownloadSpeed,
    /// The upload rate.
    UploadSpeed,
    /// The time to finish as the ETA column shows it: known estimates, then unknown, then none
    /// (every torrent that is not downloading).
    Eta,
    /// The share ratio: none, then values, then infinite.
    Ratio,
    /// The connected peers.
    Peers,
    /// When the torrent was added; torrents without a date come first.
    Added,
}

/// Which way a [`SortKey`] orders the list. It is written to the state file by its `snake_case`
/// name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    /// Smallest first.
    #[default]
    Ascending,
    /// Largest first.
    Descending,
}

/// A torrent with its name lowercased once, so no comparison has to allocate.
struct Entry<'a> {
    lowercase_name: String,
    torrent: &'a Torrent,
}

/// Sorts `torrents` by `key` in the given direction.
///
/// Torrents equal in `key` are ordered by name, ignoring case, then by hash, so the order never
/// jumps around between refreshes. That tie-break is ascending in both directions: `direction`
/// reverses the order of the key only. The sort is stable.
pub fn sort(torrents: &mut [&Torrent], key: SortKey, direction: SortDirection) {
    // The list is re-sorted on every poll, and the name decides every tie, so it is lowercased
    // once per torrent instead of twice per comparison.
    let mut entries: Vec<Entry<'_>> = torrents
        .iter()
        .map(|&torrent| Entry {
            lowercase_name: torrent.name.to_lowercase(),
            torrent,
        })
        .collect();

    entries.sort_by(|a, b| {
        let by_key = compare_by_key(key, a, b);
        let by_key = match direction {
            SortDirection::Ascending => by_key,
            SortDirection::Descending => by_key.reverse(),
        };
        by_key
            .then_with(|| a.lowercase_name.cmp(&b.lowercase_name))
            .then_with(|| a.torrent.hash.cmp(&b.torrent.hash))
    });

    for (slot, entry) in torrents.iter_mut().zip(entries) {
        *slot = entry.torrent;
    }
}

fn compare_by_key(key: SortKey, a: &Entry<'_>, b: &Entry<'_>) -> Ordering {
    let (x, y) = (a.torrent, b.torrent);
    match key {
        SortKey::Name => a.lowercase_name.cmp(&b.lowercase_name),
        SortKey::Size => x.size_when_done.cmp(&y.size_when_done),
        SortKey::Progress => x.progress.total_cmp(&y.progress),
        SortKey::State => state_rank(x.state).cmp(&state_rank(y.state)),
        SortKey::DownloadSpeed => x.rate_down.cmp(&y.rate_down),
        SortKey::UploadSpeed => x.rate_up.cmp(&y.rate_up),
        SortKey::Eta => compare_eta(shown_eta(x), shown_eta(y)),
        SortKey::Ratio => compare_ratio(x.ratio, y.ratio),
        SortKey::Peers => x.peers_connected.cmp(&y.peers_connected),
        // `None` sorts before any date.
        SortKey::Added => x.added.cmp(&y.added),
    }
}

/// The place of a state in the ascending order: what needs attention first, paused last.
fn state_rank(state: DisplayState) -> u8 {
    match state {
        DisplayState::Error => 0,
        DisplayState::Downloading => 1,
        DisplayState::Checking => 2,
        DisplayState::Queued(QueueKind::Download) => 3,
        DisplayState::Queued(QueueKind::Seed) => 4,
        DisplayState::Seeding => 5,
        DisplayState::Paused => 6,
    }
}

/// The ETA as the column shows it ([`EtaView`]): only a downloading torrent has one, ordered by
/// its exact duration rather than by the rounded text.
fn shown_eta(torrent: &Torrent) -> Eta {
    match EtaView::of(torrent) {
        EtaView::In(_) => torrent.eta,
        EtaView::Unknown => Eta::Unknown,
        EtaView::None => Eta::NotAvailable,
    }
}

/// Known ETAs, soonest first, then the unknown ones, then those with none.
fn compare_eta(a: Eta, b: Eta) -> Ordering {
    fn rank(eta: Eta) -> u8 {
        match eta {
            Eta::In(_) => 0,
            Eta::Unknown => 1,
            Eta::NotAvailable => 2,
        }
    }
    match (a, b) {
        (Eta::In(a), Eta::In(b)) => a.cmp(&b),
        _ => rank(a).cmp(&rank(b)),
    }
}

fn compare_ratio(a: Ratio, b: Ratio) -> Ordering {
    fn rank(ratio: Ratio) -> u8 {
        match ratio {
            Ratio::NotAvailable => 0,
            Ratio::Value(_) => 1,
            Ratio::Infinite => 2,
        }
    }
    match (a, b) {
        (Ratio::Value(a), Ratio::Value(b)) => a.total_cmp(&b),
        _ => rank(a).cmp(&rank(b)),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use x4_rpc::HashString;

    use super::*;
    use crate::model::{Eta, Ratio};
    use crate::state::{DisplayState, QueueKind};
    use crate::test_support::torrent;

    use SortDirection::{Ascending, Descending};

    fn order(torrents: &[Torrent], key: SortKey, direction: SortDirection) -> Vec<&str> {
        let mut refs: Vec<&Torrent> = torrents.iter().collect();
        sort(&mut refs, key, direction);
        refs.into_iter().map(|t| t.name.as_str()).collect()
    }

    fn at(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)
    }

    fn hash(digit: char) -> HashString {
        HashString::parse(&digit.to_string().repeat(40)).unwrap()
    }

    #[test]
    fn sort_by_upload_desc_breaks_ties_by_name() {
        let torrents = [
            Torrent {
                rate_up: 5,
                ..torrent("b")
            },
            Torrent {
                rate_up: 9,
                ..torrent("c")
            },
            Torrent {
                rate_up: 5,
                ..torrent("a")
            },
        ];

        assert_eq!(
            order(&torrents, SortKey::UploadSpeed, Descending),
            ["c", "a", "b"]
        );
        assert_eq!(
            order(&torrents, SortKey::UploadSpeed, Ascending),
            ["a", "b", "c"]
        );
    }

    #[test]
    fn sort_by_download_speed() {
        let torrents = [
            Torrent {
                rate_down: 30,
                rate_up: 1,
                ..torrent("fast")
            },
            Torrent {
                rate_down: 10,
                rate_up: 99,
                ..torrent("slow")
            },
        ];

        assert_eq!(
            order(&torrents, SortKey::DownloadSpeed, Ascending),
            ["slow", "fast"]
        );
    }

    /// A downloading torrent whose ETA is `eta`.
    fn downloading(name: &str, eta: Eta) -> Torrent {
        Torrent {
            state: DisplayState::Downloading,
            eta,
            ..torrent(name)
        }
    }

    #[test]
    fn sort_eta_puts_unknown_after_known() {
        let torrents = [
            downloading("none", Eta::NotAvailable),
            downloading("slow", Eta::In(Duration::from_secs(90))),
            downloading("unknown", Eta::Unknown),
            downloading("fast", Eta::In(Duration::from_secs(10))),
        ];

        assert_eq!(
            order(&torrents, SortKey::Eta, Ascending),
            ["fast", "slow", "unknown", "none"]
        );
        assert_eq!(
            order(&torrents, SortKey::Eta, Descending),
            ["none", "unknown", "slow", "fast"]
        );
    }

    #[test]
    fn sort_eta_follows_what_the_column_shows() {
        // A seeder, a paused and a queued torrent show no ETA ("—"), whatever the server reports,
        // so they sort with the torrents that have none, after the unknown one.
        let torrents = [
            Torrent {
                eta: Eta::In(Duration::from_secs(5)),
                ..torrent("seeding")
            },
            Torrent {
                state: DisplayState::Paused,
                eta: Eta::Unknown,
                ..torrent("paused")
            },
            Torrent {
                state: DisplayState::Queued(QueueKind::Download),
                eta: Eta::In(Duration::from_secs(1)),
                ..torrent("queued")
            },
            downloading("unknown", Eta::Unknown),
            downloading("hour", Eta::In(Duration::from_secs(3600))),
            downloading("minute", Eta::In(Duration::from_secs(60))),
        ];

        assert_eq!(
            order(&torrents, SortKey::Eta, Ascending),
            ["minute", "hour", "unknown", "paused", "queued", "seeding"]
        );
        assert_eq!(
            order(&torrents, SortKey::Eta, Descending),
            ["paused", "queued", "seeding", "unknown", "hour", "minute"]
        );
    }

    #[test]
    fn sort_ratio_infinite_last() {
        let torrents = [
            Torrent {
                ratio: Ratio::Infinite,
                ..torrent("infinite")
            },
            Torrent {
                ratio: Ratio::Value(2.0),
                ..torrent("two")
            },
            Torrent {
                ratio: Ratio::NotAvailable,
                ..torrent("none")
            },
            Torrent {
                ratio: Ratio::Value(0.5),
                ..torrent("half")
            },
        ];

        assert_eq!(
            order(&torrents, SortKey::Ratio, Ascending),
            ["none", "half", "two", "infinite"]
        );
    }

    #[test]
    fn sort_name_ignores_case_and_handles_cyrillic() {
        let torrents = [
            torrent("banana"),
            torrent("Яблоко"),
            torrent("Apple"),
            torrent("вишня"),
            torrent("cherry"),
        ];

        assert_eq!(
            order(&torrents, SortKey::Name, Ascending),
            ["Apple", "banana", "cherry", "вишня", "Яблоко"]
        );
        assert_eq!(
            order(&torrents, SortKey::Name, Descending),
            ["Яблоко", "вишня", "cherry", "banana", "Apple"]
        );
    }

    #[test]
    fn sort_state_follows_the_documented_order() {
        let in_state = |name, state| Torrent {
            state,
            ..torrent(name)
        };
        let torrents = [
            in_state("paused", DisplayState::Paused),
            in_state("seeding", DisplayState::Seeding),
            in_state("queued-seed", DisplayState::Queued(QueueKind::Seed)),
            in_state("checking", DisplayState::Checking),
            in_state("error", DisplayState::Error),
            in_state("queued-download", DisplayState::Queued(QueueKind::Download)),
            in_state("downloading", DisplayState::Downloading),
        ];

        assert_eq!(
            order(&torrents, SortKey::State, Ascending),
            [
                "error",
                "downloading",
                "checking",
                "queued-download",
                "queued-seed",
                "seeding",
                "paused",
            ]
        );
    }

    #[test]
    fn sort_size_uses_the_size_of_the_wanted_files() {
        let torrents = [
            Torrent {
                total_size: 100,
                size_when_done: 1,
                ..torrent("mostly-unwanted")
            },
            Torrent {
                total_size: 1,
                size_when_done: 50,
                ..torrent("wanted")
            },
        ];

        assert_eq!(
            order(&torrents, SortKey::Size, Ascending),
            ["mostly-unwanted", "wanted"]
        );
        assert_eq!(
            order(&torrents, SortKey::Size, Descending),
            ["wanted", "mostly-unwanted"]
        );
    }

    #[test]
    fn sort_peers_uses_the_connected_peers() {
        let torrents = [
            Torrent {
                peers_connected: 9,
                peers_sending_to_us: 0,
                ..torrent("busy")
            },
            Torrent {
                peers_connected: 2,
                peers_sending_to_us: 7,
                ..torrent("quiet")
            },
        ];

        assert_eq!(
            order(&torrents, SortKey::Peers, Ascending),
            ["quiet", "busy"]
        );
    }

    #[test]
    fn sort_progress_orders_by_fraction_and_survives_nan() {
        let torrents = [
            Torrent {
                progress: 0.9,
                ..torrent("nearly")
            },
            Torrent {
                progress: f64::NAN,
                ..torrent("nan")
            },
            Torrent {
                progress: 0.1,
                ..torrent("started")
            },
            Torrent {
                progress: 0.5,
                ..torrent("half")
            },
        ];

        assert_eq!(
            order(&torrents, SortKey::Progress, Ascending),
            ["started", "half", "nearly", "nan"]
        );
    }

    #[test]
    fn sort_added_puts_undated_torrents_first() {
        let torrents = [
            Torrent {
                added: Some(at(100)),
                ..torrent("new")
            },
            Torrent {
                added: None,
                ..torrent("undated")
            },
            Torrent {
                added: Some(at(50)),
                ..torrent("old")
            },
        ];

        assert_eq!(
            order(&torrents, SortKey::Added, Ascending),
            ["undated", "old", "new"]
        );
        assert_eq!(
            order(&torrents, SortKey::Added, Descending),
            ["new", "old", "undated"]
        );
    }

    #[test]
    fn ties_with_equal_names_break_by_hash_in_both_directions() {
        let torrents = [
            Torrent {
                hash: hash('b'),
                ..torrent("Same")
            },
            Torrent {
                hash: hash('c'),
                ..torrent("SAME")
            },
            Torrent {
                hash: hash('a'),
                ..torrent("same")
            },
        ];
        let hashes = |direction| {
            let mut refs: Vec<&Torrent> = torrents.iter().collect();
            sort(&mut refs, SortKey::UploadSpeed, direction);
            refs.into_iter()
                .map(|t| t.hash.as_str().chars().next())
                .collect::<Vec<_>>()
        };

        assert_eq!(hashes(Ascending), [Some('a'), Some('b'), Some('c')]);
        assert_eq!(hashes(Descending), [Some('a'), Some('b'), Some('c')]);
    }

    #[test]
    fn sort_by_name_keeps_hash_order_for_equal_names() {
        let torrents = [
            Torrent {
                hash: hash('2'),
                ..torrent("same")
            },
            Torrent {
                hash: hash('1'),
                ..torrent("SAME")
            },
        ];
        let mut refs: Vec<&Torrent> = torrents.iter().collect();

        sort(&mut refs, SortKey::Name, Descending);

        assert_eq!(
            refs.first().map(|t| t.hash.as_str()),
            Some(hash('1').as_str())
        );
    }

    #[test]
    fn sort_is_stable_for_torrents_equal_in_every_respect() {
        // 64 torrents in four groups that differ only in `peers_connected`, interleaved. Within a
        // group they are equal in key, name and hash and differ in `id` alone, so only stability
        // keeps them in input order. There are more than 20 of them because `sort_unstable_by`
        // uses insertion sort (which happens to be stable) on short slices, and the groups are
        // interleaved because an unstable sort leaves a slice that is all equal untouched.
        let torrents: Vec<Torrent> = (0..64)
            .map(|id| Torrent {
                id: x4_rpc::TorrentId(id),
                peers_connected: u32::try_from(id % 4).unwrap(),
                ..torrent("same")
            })
            .collect();
        let mut refs: Vec<&Torrent> = torrents.iter().collect();

        sort(&mut refs, SortKey::Peers, Descending);

        let ids: Vec<u64> = refs.iter().map(|t| t.id.0).collect();
        let expected: Vec<u64> = (0..4)
            .rev()
            .flat_map(|group| (0..64).filter(move |id| id % 4 == group))
            .collect();
        assert_eq!(ids, expected);
    }

    #[test]
    fn sorting_nothing_or_one_torrent_is_fine() {
        let mut none: Vec<&Torrent> = Vec::new();
        sort(&mut none, SortKey::Name, Ascending);
        assert_eq!(none.len(), 0);

        let one = torrent("only");
        let mut refs = vec![&one];
        sort(&mut refs, SortKey::Eta, Descending);
        assert_eq!(refs.len(), 1);
    }
}
