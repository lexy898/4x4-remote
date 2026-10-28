//! The torrents the application knows about, keyed by hash.

use std::collections::HashMap;

use x4_rpc::{HashString, TorrentId, TorrentRecord};

use crate::model::{ModelError, Torrent};

/// How much of a hash a log line may show.
const HASH_PREFIX_CHARS: usize = 8;

/// The torrents of the daemon, found by their stable hash.
///
/// The daemon numbers torrents anew when it restarts, so the store keeps the hash as the key and
/// the id only as an index into it. A full refresh ([`Store::replace_all`]) replaces both; the
/// partial updates of the polling worker ([`Store::apply_recent`]) keep the index in step with
/// the ids they bring.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Store {
    by_hash: HashMap<HashString, Torrent>,
    hash_by_id: HashMap<TorrentId, HashString>,
}

impl Store {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces every torrent with `torrents`, a full refresh: what is not in the list is gone.
    ///
    /// A hash that occurs twice keeps its last torrent.
    pub fn replace_all(&mut self, torrents: Vec<Torrent>) {
        self.by_hash.clear();
        self.hash_by_id.clear();
        for torrent in torrents {
            self.insert(torrent);
        }
    }

    /// Applies a partial update: drops `removed`, then inserts or replaces `updated` by hash.
    ///
    /// Removed ids are looked up as the store knew them before this update, so a torrent that
    /// arrives under an id listed as removed is kept; an id nobody holds is ignored. An updated
    /// torrent whose hash is already known replaces it and takes over its id index: the old id no
    /// longer resolves, the new one does.
    pub fn apply_recent(&mut self, updated: Vec<Torrent>, removed: &[TorrentId]) {
        for id in removed {
            if let Some(hash) = self.hash_by_id.remove(id) {
                self.by_hash.remove(&hash);
            }
        }
        for torrent in updated {
            self.insert(torrent);
        }
    }

    /// The torrent with this hash.
    #[must_use]
    pub fn get(&self, hash: &HashString) -> Option<&Torrent> {
        self.by_hash.get(hash)
    }

    /// The hash of the torrent that currently has this daemon id.
    #[must_use]
    pub fn hash_for_id(&self, id: TorrentId) -> Option<&HashString> {
        self.hash_by_id.get(&id)
    }

    /// Every torrent, in no particular order; the UI orders them with [`sort`](crate::sort()).
    pub fn torrents(&self) -> impl Iterator<Item = &Torrent> {
        self.by_hash.values()
    }

    /// How many torrents there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_hash.len()
    }

    /// Whether there are no torrents.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_hash.is_empty()
    }

    /// Stores `torrent` under its hash and makes its id resolve to it.
    ///
    /// The id the hash was known by before no longer resolves. An id that belonged to another
    /// torrent now resolves to this one: the daemon hands out each id once, so the other torrent's
    /// entry is outdated, and it stays in the store until the daemon reports it again.
    fn insert(&mut self, torrent: Torrent) {
        if let Some(previous) = self.by_hash.get(&torrent.hash) {
            let previous_id = previous.id;
            if self.hash_by_id.get(&previous_id) == Some(&torrent.hash) {
                self.hash_by_id.remove(&previous_id);
            }
        }
        self.hash_by_id.insert(torrent.id, torrent.hash.clone());
        self.by_hash.insert(torrent.hash.clone(), torrent);
    }
}

/// Turns the records of a `torrent_get` answer into torrents, skipping those that cannot be.
///
/// # Skipped records
///
/// A record without the fields a torrent needs, or with an invalid hash, is dropped with a
/// warning and the rest are kept, in order. One bad record does not hide the others: the daemon
/// can hold a torrent the app cannot show, and the list should still load.
///
/// The warning names the first 8 characters of the hash the record carried, if any, and never the
/// torrent's name.
#[must_use]
pub fn torrents_from_records(records: Vec<TorrentRecord>) -> Vec<Torrent> {
    records
        .into_iter()
        .filter_map(|record| {
            // Taken before the conversion consumes the record. Characters, not bytes: the value
            // is whatever the server sent and may not be ASCII.
            let hash_prefix: Option<String> = record
                .hash_string
                .as_deref()
                .map(|hash| hash.chars().take(HASH_PREFIX_CHARS).collect());

            match Torrent::from_record(record) {
                Ok(torrent) => Some(torrent),
                Err(error) => {
                    // `ModelError::InvalidHash` carries the whole raw hash, so the error is
                    // described here instead of printed. No wildcard: a new variant has to
                    // decide what may be logged.
                    let (reason, field) = match &error {
                        ModelError::MissingField(field) => {
                            ("a required field is missing", Some(*field))
                        }
                        ModelError::InvalidHash(_) => ("the hash is not a valid info hash", None),
                    };
                    tracing::warn!(
                        reason,
                        field,
                        hash = hash_prefix.as_deref(),
                        "skipped a torrent record that cannot be shown"
                    );
                    None
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::{Arc, Mutex};

    use x4_rpc::Status;

    use super::*;

    use crate::test_support::torrent;

    const GOOD_HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const GOOD_HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const GOOD_HASH_C: &str = "cccccccccccccccccccccccccccccccccccccccc";

    /// A record with every field a torrent needs.
    fn record(id: u64, hash: &str, name: &str) -> TorrentRecord {
        TorrentRecord {
            id: Some(TorrentId(id)),
            hash_string: Some(hash.to_owned()),
            name: Some(name.to_owned()),
            status: Some(Status::Stopped),
            ..TorrentRecord::default()
        }
    }

    /// Collects what `tracing` events write while a test runs.
    #[derive(Clone, Default)]
    struct LogBuffer(Arc<Mutex<Vec<u8>>>);

    impl io::Write for LogBuffer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Runs `work` and returns its result with everything it logged, formatted as text.
    fn logged<T>(work: impl FnOnce() -> T) -> (T, String) {
        let buffer = LogBuffer::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer({
                let buffer = buffer.clone();
                move || buffer.clone()
            })
            .with_ansi(false)
            .finish();
        let result = tracing::subscriber::with_default(subscriber, work);
        let text = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
        (result, text)
    }

    #[test]
    fn replace_all_is_exact() {
        let a = Torrent {
            id: TorrentId(1),
            ..torrent("a")
        };
        let b = Torrent {
            id: TorrentId(2),
            ..torrent("b")
        };
        let mut store = Store::new();

        store.replace_all(vec![a.clone(), b.clone()]);
        assert_eq!(store.len(), 2);
        assert_eq!(store.get(&a.hash), Some(&a));
        assert_eq!(store.get(&b.hash), Some(&b));

        store.replace_all(vec![b.clone()]);
        assert_eq!(store.len(), 1);
        assert_eq!(store.get(&a.hash), None);
        assert_eq!(store.get(&b.hash), Some(&b));
        assert_eq!(store.hash_for_id(TorrentId(1)), None);
        assert_eq!(store.hash_for_id(TorrentId(2)), Some(&b.hash));
    }

    #[test]
    fn replace_all_with_nothing_empties_the_store() {
        let mut store = Store::new();
        store.replace_all(vec![torrent("a")]);

        store.replace_all(Vec::new());

        assert!(store.is_empty());
        assert_eq!(store.torrents().count(), 0);
    }

    #[test]
    fn replace_all_with_a_hash_twice_keeps_the_last_and_its_id() {
        let first = Torrent {
            id: TorrentId(1),
            rate_down: 1,
            ..torrent("a")
        };
        let last = Torrent {
            id: TorrentId(2),
            rate_down: 2,
            ..torrent("a")
        };
        let mut store = Store::new();

        store.replace_all(vec![first.clone(), last.clone()]);

        assert_eq!(store.len(), 1);
        assert_eq!(store.get(&first.hash), Some(&last));
        assert_eq!(store.hash_for_id(TorrentId(1)), None);
        assert_eq!(store.hash_for_id(TorrentId(2)), Some(&last.hash));
    }

    #[test]
    fn recent_upserts_by_hash() {
        let a = Torrent {
            id: TorrentId(1),
            ..torrent("a")
        };
        let b = Torrent {
            id: TorrentId(2),
            ..torrent("b")
        };
        let mut store = Store::new();
        store.replace_all(vec![a.clone(), b.clone()]);

        let faster = Torrent {
            rate_down: 500,
            ..a.clone()
        };
        store.apply_recent(vec![faster.clone()], &[]);

        assert_eq!(store.len(), 2);
        assert_eq!(store.get(&a.hash), Some(&faster));
        assert_eq!(store.get(&b.hash), Some(&b));
    }

    #[test]
    fn recent_inserts_a_new_torrent_with_its_id() {
        let mut store = Store::new();
        let a = Torrent {
            id: TorrentId(4),
            ..torrent("a")
        };

        store.apply_recent(vec![a.clone()], &[]);

        assert_eq!(store.len(), 1);
        assert_eq!(store.hash_for_id(TorrentId(4)), Some(&a.hash));
    }

    #[test]
    fn recent_removes_by_id() {
        let a = Torrent {
            id: TorrentId(1),
            ..torrent("a")
        };
        let b = Torrent {
            id: TorrentId(2),
            ..torrent("b")
        };
        let mut store = Store::new();
        store.replace_all(vec![a.clone(), b.clone()]);

        store.apply_recent(Vec::new(), &[TorrentId(1), TorrentId(99)]);

        assert_eq!(store.len(), 1);
        assert_eq!(store.get(&a.hash), None);
        assert_eq!(store.hash_for_id(TorrentId(1)), None);
        assert_eq!(store.get(&b.hash), Some(&b));
    }

    #[test]
    fn store_survives_id_renumbering_after_full_refresh() {
        let a = Torrent {
            id: TorrentId(1),
            ..torrent("a")
        };
        let b = Torrent {
            id: TorrentId(2),
            ..torrent("b")
        };
        let mut store = Store::new();
        store.replace_all(vec![a.clone(), b.clone()]);

        // The daemon restarted and handed the ids out the other way round.
        let renumbered_a = Torrent {
            id: TorrentId(2),
            ..a.clone()
        };
        let renumbered_b = Torrent {
            id: TorrentId(1),
            ..b.clone()
        };
        store.replace_all(vec![renumbered_a, renumbered_b]);
        store.apply_recent(Vec::new(), &[TorrentId(1)]);

        assert_eq!(store.len(), 1);
        assert_eq!(store.get(&b.hash), None);
        assert_eq!(store.get(&a.hash).map(|t| t.id), Some(TorrentId(2)));
    }

    #[test]
    fn recent_with_a_known_hash_under_a_new_id_moves_the_index() {
        let a = Torrent {
            id: TorrentId(1),
            ..torrent("a")
        };
        let mut store = Store::new();
        store.replace_all(vec![a.clone()]);

        let renumbered = Torrent {
            id: TorrentId(7),
            ..a.clone()
        };
        store.apply_recent(vec![renumbered.clone()], &[]);

        assert_eq!(store.len(), 1);
        assert_eq!(store.hash_for_id(TorrentId(1)), None);
        assert_eq!(store.hash_for_id(TorrentId(7)), Some(&a.hash));
        assert_eq!(store.get(&a.hash), Some(&renumbered));

        // The old id is gone for good: removing it changes nothing, removing the new one works.
        store.apply_recent(Vec::new(), &[TorrentId(1)]);
        assert_eq!(store.len(), 1);
        store.apply_recent(Vec::new(), &[TorrentId(7)]);
        assert!(store.is_empty());
    }

    #[test]
    fn recent_taking_over_an_id_points_the_index_at_the_new_owner() {
        let a = Torrent {
            id: TorrentId(1),
            ..torrent("a")
        };
        let b = Torrent {
            id: TorrentId(2),
            ..torrent("b")
        };
        let mut store = Store::new();
        store.replace_all(vec![a.clone(), b.clone()]);

        // The daemon now calls `b` by the id `a` had; `a` has not been heard of since.
        let taker = Torrent {
            id: TorrentId(1),
            ..b.clone()
        };
        store.apply_recent(vec![taker], &[]);

        assert_eq!(store.hash_for_id(TorrentId(1)), Some(&b.hash));
        assert_eq!(store.hash_for_id(TorrentId(2)), None);
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn recent_drops_removed_ids_before_it_applies_the_updates() {
        let a = Torrent {
            id: TorrentId(1),
            ..torrent("a")
        };
        let mut store = Store::new();
        store.replace_all(vec![a.clone()]);

        // `a` is removed and a torrent that took its id arrives in the same update.
        let other = Torrent {
            id: TorrentId(1),
            ..torrent("other")
        };
        store.apply_recent(vec![other.clone()], &[TorrentId(1)]);

        assert_eq!(store.get(&a.hash), None);
        assert_eq!(store.get(&other.hash), Some(&other));
        assert_eq!(store.hash_for_id(TorrentId(1)), Some(&other.hash));
    }

    #[test]
    fn lookups_in_an_empty_store_find_nothing() {
        let store = Store::new();

        assert!(store.is_empty());
        assert_eq!(store.len(), 0);
        assert_eq!(store.get(&torrent("a").hash), None);
        assert_eq!(store.hash_for_id(TorrentId(1)), None);
    }

    #[test]
    fn torrents_lists_every_torrent_once() {
        let mut store = Store::new();
        store.replace_all(vec![
            Torrent {
                id: TorrentId(1),
                ..torrent("a")
            },
            Torrent {
                id: TorrentId(2),
                ..torrent("b")
            },
            Torrent {
                id: TorrentId(3),
                ..torrent("c")
            },
        ]);

        let mut names: Vec<&str> = store.torrents().map(|t| t.name.as_str()).collect();
        names.sort_unstable();

        assert_eq!(names, ["a", "b", "c"]);
    }

    #[test]
    fn records_become_torrents_in_order() {
        let torrents = torrents_from_records(vec![
            record(1, GOOD_HASH_A, "a"),
            record(2, GOOD_HASH_B, "b"),
        ]);

        let ids: Vec<TorrentId> = torrents.iter().map(|t| t.id).collect();
        assert_eq!(ids, [TorrentId(1), TorrentId(2)]);
    }

    #[test]
    fn records_that_cannot_be_torrents_are_skipped() {
        let bad_hash = record(2, "not-a-hash", "Secret Name");
        let missing_name = TorrentRecord {
            name: None,
            ..record(4, GOOD_HASH_B, "")
        };

        let torrents = torrents_from_records(vec![
            record(1, GOOD_HASH_A, "a"),
            bad_hash,
            record(3, GOOD_HASH_C, "c"),
            missing_name,
        ]);

        let names: Vec<&str> = torrents.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["a", "c"]);
    }

    #[test]
    fn no_records_give_no_torrents() {
        assert_eq!(torrents_from_records(Vec::new()), Vec::new());
    }

    #[test]
    fn a_skipped_record_is_logged_without_its_name_or_more_than_eight_hash_characters() {
        let long_bad_hash = "zzzzzzzzzzzzzzzzzzzzzzzz";
        let bad_hash = record(1, long_bad_hash, "Secret Name");
        let missing_name = TorrentRecord {
            name: None,
            ..record(2, GOOD_HASH_B, "")
        };

        let (torrents, log) = logged(|| torrents_from_records(vec![bad_hash, missing_name]));

        assert_eq!(torrents, Vec::new());
        assert!(log.contains("WARN"), "no warning in {log:?}");
        assert!(log.contains("zzzzzzzz"), "no hash prefix in {log:?}");
        assert!(
            !log.contains("zzzzzzzzz"),
            "more than 8 characters in {log:?}"
        );
        assert!(log.contains("bbbbbbbb"), "no hash prefix in {log:?}");
        assert!(
            !log.contains("bbbbbbbbb"),
            "more than 8 characters in {log:?}"
        );
        assert!(!log.contains("Secret Name"), "a name leaked into {log:?}");
        assert!(
            log.contains("name"),
            "the missing field is not named in {log:?}"
        );
    }

    #[test]
    fn a_skipped_record_without_a_hash_is_logged_without_one() {
        let (torrents, log) = logged(|| torrents_from_records(vec![TorrentRecord::default()]));

        assert_eq!(torrents, Vec::new());
        assert!(log.contains("WARN"), "no warning in {log:?}");
        assert!(!log.contains("hash="), "a hash field in {log:?}");
    }
}
