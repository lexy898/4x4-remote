//! The torrents the application knows about, keyed by hash.

use std::collections::HashMap;

use x4_rpc::{HashString, TorrentId, TorrentRecord};

use crate::model::{ModelError, Torrent};

/// How much of a hash a [`SkippedRecord`] keeps.
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

/// The result of [`torrents_from_records`]: the torrents, and the records that were left out.
#[derive(Debug, Clone, PartialEq)]
pub struct Converted {
    /// The torrents that could be built, in the order of the records.
    pub torrents: Vec<Torrent>,
    /// The records that could not, in the order of the records.
    pub skipped: Vec<SkippedRecord>,
}

/// A record that could not become a torrent, described without anything that identifies the
/// torrent: no name, and no more of the hash than its first 8 characters.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SkippedRecord {
    /// The first 8 characters of the hash the record carried, as sent (so it may not be valid);
    /// `None` when the record had no hash.
    pub hash_prefix: Option<String>,
    /// Why the record was left out.
    pub reason: SkipReason,
}

/// Why a record could not become a torrent.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SkipReason {
    /// The record lacks a field a torrent cannot do without; holds its name.
    MissingField(&'static str),
    /// The record's hash is not a valid info hash. The raw value is not kept.
    InvalidHash,
}

/// Turns the records of a `torrent_get` answer into torrents, leaving out those that cannot be.
///
/// A record without the fields a torrent needs, or with an invalid hash, is reported in
/// [`Converted::skipped`] and the rest are converted, in order. One bad record does not hide the
/// others: the daemon can hold a torrent the app cannot show, and the list should still load.
///
/// This function does not log. It is called on every poll, so the caller decides when a skipped
/// record is worth a message, for instance when the set of skipped records changes.
#[must_use]
pub fn torrents_from_records(records: Vec<TorrentRecord>) -> Converted {
    let mut converted = Converted {
        torrents: Vec::with_capacity(records.len()),
        skipped: Vec::new(),
    };
    for record in records {
        // Taken before the conversion consumes the record. Characters, not bytes: the value is
        // whatever the server sent and may not be ASCII.
        let hash_prefix: Option<String> = record
            .hash_string
            .as_deref()
            .map(|hash| hash.chars().take(HASH_PREFIX_CHARS).collect());

        match Torrent::from_record(record) {
            Ok(torrent) => converted.torrents.push(torrent),
            Err(error) => {
                // `ModelError::InvalidHash` carries the whole raw hash, so the reason is
                // rebuilt instead of copied. No wildcard: a new variant has to decide what may
                // be kept.
                let reason = match error {
                    ModelError::MissingField(field) => SkipReason::MissingField(field),
                    ModelError::InvalidHash(_) => SkipReason::InvalidHash,
                };
                converted.skipped.push(SkippedRecord {
                    hash_prefix,
                    reason,
                });
            }
        }
    }
    converted
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

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
    fn an_orphaned_torrent_updated_later_leaves_the_id_it_lost_alone() {
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

        // `b` takes the id `a` had, which orphans `a` ...
        let b_with_a_id = Torrent {
            id: TorrentId(1),
            ..b.clone()
        };
        store.apply_recent(vec![b_with_a_id], &[]);
        // ... and when `a` is reported again, under a new id, it must not take id 1 down with
        // it: that id is `b`'s now.
        let a_renumbered = Torrent {
            id: TorrentId(3),
            ..a.clone()
        };
        store.apply_recent(vec![a_renumbered], &[]);

        assert_eq!(store.hash_for_id(TorrentId(1)), Some(&b.hash));
        assert_eq!(store.hash_for_id(TorrentId(3)), Some(&a.hash));
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
        let converted = torrents_from_records(vec![
            record(1, GOOD_HASH_A, "a"),
            record(2, GOOD_HASH_B, "b"),
        ]);

        let ids: Vec<TorrentId> = converted.torrents.iter().map(|t| t.id).collect();
        assert_eq!(ids, [TorrentId(1), TorrentId(2)]);
        assert_eq!(converted.skipped, Vec::new());
    }

    #[test]
    fn records_that_cannot_be_torrents_are_skipped() {
        let bad_hash = record(2, "not-a-hash", "Secret Name");
        let missing_name = TorrentRecord {
            name: None,
            ..record(4, GOOD_HASH_B, "")
        };

        let converted = torrents_from_records(vec![
            record(1, GOOD_HASH_A, "a"),
            bad_hash,
            record(3, GOOD_HASH_C, "c"),
            missing_name,
        ]);

        let names: Vec<&str> = converted.torrents.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["a", "c"]);
        assert_eq!(
            converted.skipped,
            [
                SkippedRecord {
                    hash_prefix: Some("not-a-ha".to_owned()),
                    reason: SkipReason::InvalidHash,
                },
                SkippedRecord {
                    hash_prefix: Some("bbbbbbbb".to_owned()),
                    reason: SkipReason::MissingField("name"),
                },
            ]
        );
    }

    #[test]
    fn no_records_give_nothing() {
        let converted = torrents_from_records(Vec::new());

        assert_eq!(converted.torrents, Vec::new());
        assert_eq!(converted.skipped, Vec::new());
    }

    #[test]
    fn a_record_without_a_hash_is_skipped_without_one() {
        let converted = torrents_from_records(vec![TorrentRecord::default()]);

        assert_eq!(
            converted.skipped,
            [SkippedRecord {
                hash_prefix: None,
                reason: SkipReason::MissingField("id"),
            }]
        );
    }

    #[test]
    fn a_skipped_record_keeps_at_most_eight_characters_of_the_hash() {
        let skipped_prefix = |hash: &str| {
            let converted = torrents_from_records(vec![record(1, hash, "Secret Name")]);
            assert_eq!(converted.torrents, Vec::new(), "{hash:?} became a torrent");
            let [skipped] = <[SkippedRecord; 1]>::try_from(converted.skipped).unwrap();
            assert_eq!(skipped.reason, SkipReason::InvalidHash, "{hash:?}");
            skipped.hash_prefix
        };

        // Longer than 8 characters.
        assert_eq!(
            skipped_prefix("zzzzzzzzzzzzzzzzzzzzzzzz").as_deref(),
            Some("zzzzzzzz")
        );
        // Exactly 8 and fewer.
        assert_eq!(skipped_prefix("12345678").as_deref(), Some("12345678"));
        assert_eq!(skipped_prefix("abc").as_deref(), Some("abc"));
        assert_eq!(skipped_prefix("").as_deref(), Some(""));
        // Counted in characters, not bytes: each of these is two bytes long, and cutting at byte
        // 8 would keep only 4 of them.
        let greek = skipped_prefix("αβγδεζηθικλμ").unwrap();
        assert_eq!(greek, "αβγδεζηθ");
        assert_eq!(greek.chars().count(), 8);
    }

    #[test]
    fn a_skipped_record_never_shows_the_name_or_the_rest_of_the_hash() {
        let long_bad_hash = "zzzzzzzzzzzzzzzzzzzzzzzz";
        let missing_status = TorrentRecord {
            status: None,
            ..record(2, GOOD_HASH_B, "Another Secret")
        };

        let converted = torrents_from_records(vec![
            record(1, long_bad_hash, "Secret Name"),
            missing_status,
        ]);

        let shown = format!("{:?}", converted.skipped);
        assert_eq!(converted.skipped.len(), 2, "{shown}");
        assert!(!shown.contains("Secret Name"), "a name in {shown}");
        assert!(!shown.contains("Another Secret"), "a name in {shown}");
        assert!(!shown.contains("zzzzzzzzz"), "too much hash in {shown}");
        assert!(!shown.contains("bbbbbbbbb"), "too much hash in {shown}");
        assert!(shown.contains("zzzzzzzz"), "no hash prefix in {shown}");
        assert!(shown.contains("bbbbbbbb"), "no hash prefix in {shown}");
    }

    #[test]
    fn skipped_records_can_be_compared_as_a_set() {
        let skip = |hash: &str| {
            torrents_from_records(vec![record(1, hash, "a")])
                .skipped
                .into_iter()
                .collect::<HashSet<_>>()
        };

        assert_eq!(skip("not-a-hash"), skip("not-a-hash"));
        assert_ne!(skip("not-a-hash"), skip("another-one"));
    }
}
