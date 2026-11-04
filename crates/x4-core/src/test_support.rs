//! Builders for tests (feature `test-support`): a complete [`Torrent`], the details of one with
//! the files given, a [`TrackerStat`] and the `result` of a recorded fixture.
//!
//! The unit tests of this crate use them, and so can the tests of dependent crates, which enable
//! the feature as a dev-dependency. The feature also turns on `x4-rpc`'s `test-support`, so
//! `x4_rpc::testing::fixtures` is there to feed [`fixture_result`]. Not for production code.

use x4_rpc::{FilePriority, HashString, TorrentId, TrackerId, TrackerState};

use crate::details::{FileEntry, NextAnnounce, TorrentDetails, TrackerStat};
use crate::model::{Eta, Ratio, Torrent};
use crate::state::DisplayState;

/// The `result` of a recorded JSON-RPC response from `x4_rpc::testing::fixtures`; deserialize it
/// into the type the method returns.
///
/// # Panics
///
/// When `envelope` is not JSON or has no `result`, which is a broken fixture.
#[must_use]
#[expect(
    clippy::expect_used,
    reason = "a test helper: a broken fixture should fail the test that reads it"
)]
pub fn fixture_result(envelope: &str) -> serde_json::Value {
    let mut envelope: serde_json::Value =
        serde_json::from_str(envelope).expect("a fixture is valid JSON");
    envelope
        .get_mut("result")
        .map(serde_json::Value::take)
        .expect("a fixture is a success response with a result")
}

/// A complete, idle, seeding torrent called `name`, with every other field neutral.
///
/// Override fields with struct-update syntax: `Torrent { rate_up: 5, ..torrent("a") }`. The hash
/// is derived from the name: equal names give equal hashes and different names (in practice)
/// different ones, so a test that depends on the order of hashes sets `hash` itself.
#[must_use]
pub fn torrent(name: &str) -> Torrent {
    Torrent {
        id: TorrentId(0),
        hash: hash_for(name),
        name: name.to_owned(),
        state: DisplayState::Seeding,
        attention: None,
        active: false,
        progress: 1.0,
        metadata_progress: 1.0,
        total_size: 0,
        size_when_done: 0,
        left_until_done: 0,
        rate_down: 0,
        rate_up: 0,
        eta: Eta::NotAvailable,
        ratio: Ratio::NotAvailable,
        peers_connected: 0,
        peers_getting_from_us: 0,
        peers_sending_to_us: 0,
        download_dir: String::new(),
        added: None,
        done: None,
        queue_position: 0,
        sequential: false,
        recheck_progress: 0.0,
        labels: Vec::new(),
    }
}

/// The details of the torrent `hash` with these files, in order: each one is its path inside the
/// torrent, its length, the bytes downloaded, whether it is wanted and its priority.
///
/// Everything else is neutral: no metainfo, no trackers, no pieces, and nothing playable. Set
/// `playable_from_start` or any other field on the result when a test needs it.
#[must_use]
pub fn details(
    hash: &HashString,
    files: &[(&str, u64, u64, bool, FilePriority)],
) -> TorrentDetails {
    TorrentDetails {
        hash: hash.clone(),
        comment: String::new(),
        creator: String::new(),
        date_created: None,
        source: None,
        is_private: false,
        piece_count: 0,
        piece_size: 0,
        files: files
            .iter()
            .enumerate()
            .map(
                |(index, &(name, length, bytes_completed, wanted, priority))| FileEntry {
                    index,
                    name: name.to_owned(),
                    length,
                    bytes_completed,
                    wanted,
                    priority,
                    playable_from_start: 0,
                },
            )
            .collect(),
        downloaded_ever: 0,
        uploaded_ever: 0,
        trackers: Vec::new(),
    }
}

/// A tracker at `host` that the torrent has not been announced to: in the first tier, not a
/// backup, with no state, no answers and no counts.
///
/// Override fields with struct-update syntax: `TrackerStat { tier: 1, ..tracker("a.example:80") }`.
#[must_use]
pub fn tracker(host: &str) -> TrackerStat {
    TrackerStat {
        id: TrackerId(0),
        tier: 0,
        is_backup: false,
        host: host.to_owned(),
        sitename: String::new(),
        announce_state: TrackerState::Inactive,
        next: NextAnnounce::Idle,
        last_announce: None,
        last_announce_peers: None,
        last_scrape: None,
        seeders: None,
        leechers: None,
        downloads: None,
    }
}

/// A valid 40-digit hash made of two FNV-1a digests of the name and its length.
///
/// FNV is spelled out here, rather than taken from `DefaultHasher`, because the standard hasher
/// does not promise the same output across Rust versions.
#[expect(
    clippy::expect_used,
    reason = "sixteen plus sixteen plus eight hex digits is always a valid hash"
)]
fn hash_for(name: &str) -> HashString {
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    const BASIS: u64 = 0xcbf2_9ce4_8422_2325;

    let digest = |basis: u64| {
        name.bytes().fold(basis, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(PRIME)
        })
    };
    let length = u32::try_from(name.len()).unwrap_or(u32::MAX);
    let hex = format!("{:016x}{:016x}{length:08x}", digest(BASIS), digest(!BASIS));
    HashString::parse(&hex).expect("sixteen plus sixteen plus eight hex digits is a valid hash")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn details_hold_the_files_in_order_with_their_indices() {
        let hash = torrent("a").hash;

        let details = details(
            &hash,
            &[
                ("t/a", 10, 4, true, FilePriority::High),
                ("t/b", 0, 0, false, FilePriority::Low),
            ],
        );

        assert_eq!(details.hash, hash);
        let files: Vec<_> = details
            .files
            .iter()
            .map(|file| (file.index, file.name.as_str(), file.wanted, file.priority))
            .collect();
        assert_eq!(
            files,
            [
                (0, "t/a", true, FilePriority::High),
                (1, "t/b", false, FilePriority::Low)
            ]
        );
        assert_eq!(details.files[0].bytes_completed, 4);
    }

    #[test]
    fn the_hash_follows_the_name() {
        assert_eq!(torrent("a").hash, torrent("a").hash);
        assert_ne!(torrent("a").hash, torrent("b").hash);
        assert_ne!(torrent("a").hash, torrent("A").hash);
        assert_eq!(torrent("Тед Лассо").hash.as_str().len(), 40);
    }
}
