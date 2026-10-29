//! What the removal dialog says about the torrents it removes.

use std::collections::{HashMap, HashSet};

use x4_rpc::HashString;

use crate::format::{HumanSize, human_size};
use crate::model::Torrent;
use crate::snapshot::Snapshot;

/// How many names the removal dialog lists; it counts the rest.
const LISTED_NAMES: usize = 5;

/// What the removal dialog tells about the torrents it is about to remove.
///
/// It is taken once, when the dialog opens, from the hashes captured then, so a refresh while
/// the dialog is open changes neither what it says nor what it removes.
#[derive(Debug, Clone, PartialEq)]
pub struct RemovalSummary {
    /// How many of the torrents the snapshot knows.
    pub count: usize,
    /// The data downloaded into the wanted files of those torrents, which "delete files"
    /// removes from the server's disk; not their total size.
    pub downloaded: HumanSize,
    /// The names of the first torrents, in the order of the hashes, at most 5.
    pub names: Vec<String>,
    /// How many torrents `names` leaves out: `count − names.len()`.
    pub more: usize,
}

/// What removing the torrents `hashes` names means, as the torrents stand in `snapshot`.
///
/// A torrent counts once, however often its hash is given. A hash the snapshot does not know,
/// such as a torrent removed meanwhile, is not counted.
#[must_use]
pub fn removal_summary(snapshot: &Snapshot, hashes: &[HashString]) -> RemovalSummary {
    let by_hash: HashMap<&HashString, &Torrent> = snapshot
        .torrents
        .iter()
        .map(|torrent| (&torrent.hash, torrent))
        .collect();
    let mut seen = HashSet::with_capacity(hashes.len());
    let found: Vec<&Torrent> = hashes
        .iter()
        .filter(|hash| seen.insert(*hash))
        .filter_map(|hash| by_hash.get(hash).copied())
        .collect();
    let downloaded = found.iter().fold(0_u64, |sum, torrent| {
        sum.saturating_add(
            torrent
                .size_when_done
                .saturating_sub(torrent.left_until_done),
        )
    });
    let names: Vec<String> = found
        .iter()
        .take(LISTED_NAMES)
        .map(|torrent| torrent.name.clone())
        .collect();
    RemovalSummary {
        count: found.len(),
        downloaded: human_size(downloaded),
        more: found.len() - names.len(),
        names,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::format::human_size;
    use crate::snapshot::ConnectionState;
    use crate::test_support::torrent;

    fn snapshot(torrents: Vec<Torrent>) -> Snapshot {
        Snapshot {
            torrents,
            listed: true,
            session: None,
            free_space_bytes: None,
            rate_down: 0,
            rate_up: 0,
            connection: ConnectionState::Connected,
            stale: false,
        }
    }

    fn hashes(torrents: &[&Torrent]) -> Vec<HashString> {
        torrents
            .iter()
            .map(|torrent| torrent.hash.clone())
            .collect()
    }

    #[test]
    fn removal_summary_counts_downloaded_bytes_not_total_size() {
        // 10 GiB in the torrent, 8 GiB of it wanted, 1 MiB of that downloaded.
        let started = Torrent {
            total_size: 10 << 30,
            size_when_done: 8 << 30,
            left_until_done: (8 << 30) - (1 << 20),
            ..torrent("started")
        };
        let done = Torrent {
            total_size: 5 << 30,
            size_when_done: 4 << 30,
            left_until_done: 0,
            ..torrent("done")
        };
        // More left than wanted is a daemon's slip; it counts as nothing downloaded.
        let odd = Torrent {
            size_when_done: 100,
            left_until_done: 200,
            ..torrent("odd")
        };
        let all = snapshot(vec![started.clone(), done.clone(), odd.clone()]);

        let summary = removal_summary(&all, &hashes(&[&started, &done, &odd]));
        assert_eq!(summary.count, 3);
        assert_eq!(summary.downloaded, human_size((4 << 30) + (1 << 20)));

        let summary = removal_summary(&all, &hashes(&[&started]));
        assert_eq!(summary.downloaded, human_size(1 << 20));
        let summary = removal_summary(&all, &hashes(&[&odd]));
        assert_eq!(summary.downloaded, human_size(0));
    }

    #[test]
    fn removal_summary_lists_five_names_and_how_many_more() {
        let torrents: Vec<Torrent> = ["a", "b", "c", "d", "e", "f", "g"]
            .into_iter()
            .map(torrent)
            .collect();
        let all = snapshot(torrents.clone());
        // The order of the hashes, not of the snapshot.
        let order: Vec<HashString> = torrents.iter().rev().map(|t| t.hash.clone()).collect();

        let summary = removal_summary(&all, &order);

        assert_eq!(summary.count, 7);
        assert_eq!(summary.names, ["g", "f", "e", "d", "c"]);
        assert_eq!(summary.more, 2);

        let few = removal_summary(&all, order.get(..3).unwrap());
        assert_eq!(few.count, 3);
        assert_eq!(few.names, ["g", "f", "e"]);
        assert_eq!(few.more, 0);

        let five = removal_summary(&all, order.get(..5).unwrap());
        assert_eq!(five.names.len(), 5);
        assert_eq!(five.more, 0);
    }

    #[test]
    fn removal_summary_skips_unknown_hashes() {
        let known = Torrent {
            size_when_done: 300,
            left_until_done: 100,
            ..torrent("known")
        };
        let gone = Torrent {
            size_when_done: 5_000,
            ..torrent("gone")
        };
        let all = snapshot(vec![known.clone()]);

        let summary = removal_summary(&all, &hashes(&[&gone, &known, &gone]));

        assert_eq!(summary.count, 1);
        assert_eq!(summary.downloaded, human_size(200));
        assert_eq!(summary.names, ["known"]);
        assert_eq!(summary.more, 0);

        let nothing = removal_summary(&all, &hashes(&[&gone]));
        assert_eq!(nothing.count, 0);
        assert_eq!(nothing.downloaded, human_size(0));
        assert_eq!(nothing.names, Vec::<String>::new());
        assert_eq!(nothing.more, 0);
    }

    #[test]
    fn removal_summary_counts_a_repeated_hash_once() {
        let one = Torrent {
            size_when_done: 300,
            ..torrent("one")
        };
        let all = snapshot(vec![one.clone()]);

        let summary = removal_summary(&all, &hashes(&[&one, &one]));

        assert_eq!(summary.count, 1);
        assert_eq!(summary.downloaded, human_size(300));
        assert_eq!(summary.names, ["one"]);
    }
}
