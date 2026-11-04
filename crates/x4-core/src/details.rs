//! The details of the selected torrent: its metainfo, its trackers, its totals and its files.
//!
//! They are read in two ways. A full read ([`TorrentField::DETAILS_FULL`]) brings everything and
//! is [built](build) into [`TorrentDetails`]; a progress read
//! ([`TorrentField::DETAILS_PROGRESS`]) brings only what changes while the torrent downloads and
//! is [merged](merge_progress) into the last full details. A progress read that also asks for the
//! trackers ([`TorrentField::DETAILS_PROGRESS_TRACKERS`]) replaces them in the merge. A read of
//! the trackers alone ([`TorrentField::TRACKERS`], for the Trackers tab) replaces just them, in
//! [`merge_trackers`].
//!
//! [`TorrentField::DETAILS_FULL`]: x4_rpc::TorrentField::DETAILS_FULL
//! [`TorrentField::DETAILS_PROGRESS`]: x4_rpc::TorrentField::DETAILS_PROGRESS
//! [`TorrentField::DETAILS_PROGRESS_TRACKERS`]: x4_rpc::TorrentField::DETAILS_PROGRESS_TRACKERS
//! [`TorrentField::TRACKERS`]: x4_rpc::TorrentField::TRACKERS

use std::time::SystemTime;

use x4_rpc::{
    FilePriority, HashString, Pieces, TorrentGetResponse, TorrentRecord, TrackerId,
    TrackerStatRecord, TrackerState,
};

use crate::model::date_from_wire;
use crate::playable::{file_offsets, playable_prefix};

/// One file of a torrent, as the details panel shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// The file's position in the torrent, which [`Action`](crate::Action)s take.
    pub index: usize,
    /// The path inside the torrent, `/`-separated.
    pub name: String,
    /// The size in bytes.
    pub length: u64,
    /// The bytes downloaded and verified.
    pub bytes_completed: u64,
    /// Whether the file is to be downloaded.
    pub wanted: bool,
    /// The download priority.
    pub priority: FilePriority,
    /// The bytes at the start of the file that are downloaded without a gap, so a player can
    /// read them; see [`playable_prefix`].
    pub playable_from_start: u64,
}

/// How a tracker's last announce or scrape went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrackerOutcome {
    /// The tracker answered.
    Succeeded,
    /// The tracker did not answer in time.
    TimedOut,
    /// It failed otherwise: the tracker could not be reached, or refused; the text says which.
    Failed,
}

/// A tracker's last answer to an announce or a scrape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackerAnswer {
    /// How it went.
    pub outcome: TrackerOutcome,
    /// The daemon's text: `Success`, or why it failed, often in the tracker's own words. It is
    /// shown as sent, not localized.
    pub result: String,
    /// When the answer came; `None` when the daemon gives no time (it sends 0).
    pub at: Option<SystemTime>,
}

impl TrackerAnswer {
    /// The answer of the flags and text the daemon reports, if there was one at all.
    fn of(
        happened: bool,
        succeeded: bool,
        timed_out: bool,
        result: String,
        time: i64,
    ) -> Option<Self> {
        let outcome = if succeeded {
            TrackerOutcome::Succeeded
        } else if timed_out {
            TrackerOutcome::TimedOut
        } else {
            TrackerOutcome::Failed
        };
        happened.then(|| Self {
            outcome,
            result,
            at: date_from_wire(time),
        })
    }
}

/// When the next announce of a tracker's tier is due.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextAnnounce {
    /// The tier waits for this time, which may already be past while a read is stale.
    At(SystemTime),
    /// The tier is ready and waits for a free announce slot.
    Queued,
    /// Nothing is scheduled: the torrent is stopped or has an error, the tracker is a backup, an
    /// announce is under way, or the daemon sent a state this version does not know.
    Idle,
}

impl NextAnnounce {
    /// What the daemon's `announce_state` and `next_announce_time` say. The time counts only
    /// while the tier waits for it; elsewhere the daemon sends 0.
    fn of(state: TrackerState, time: i64) -> Self {
        match state {
            TrackerState::Waiting => date_from_wire(time).map_or(Self::Idle, Self::At),
            TrackerState::Queued => Self::Queued,
            TrackerState::Inactive | TrackerState::Active | TrackerState::Other(_) => Self::Idle,
        }
    }
}

/// What one tracker of the torrent last answered.
///
/// The announce data of a tier belongs to the tier, not to one of its trackers: the daemon shows
/// it on the tier's current tracker, and after an error it makes that tracker a backup, with no
/// data, and shows the error on the next tracker of the tier. A tracker alone in its tier keeps
/// its own error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackerStat {
    /// The daemon's id of the tracker, unique within the daemon: what a row is told apart by.
    pub id: TrackerId,
    /// The tier, counted from 0: the trackers of a tier back each other up.
    pub tier: u32,
    /// Whether another tracker of the tier is the one in use. A backup has no state of its own.
    pub is_backup: bool,
    /// The tracker's host and port, such as `tracker.example.org:443`.
    pub host: String,
    /// The first label of the host before its public suffix, such as `example`; may be empty.
    pub sitename: String,
    /// What the announce of the tier is doing.
    pub announce_state: TrackerState,
    /// When the next announce is due.
    pub next: NextAnnounce,
    /// The last announce; `None` until the torrent was announced to this tracker, which backup
    /// trackers never are.
    pub last_announce: Option<TrackerAnswer>,
    /// How many peers the last announce returned; `None` until there was one, or when the daemon
    /// sends no count. It is 0 after a failure.
    pub last_announce_peers: Option<u32>,
    /// The last scrape; `None` until the tracker was scraped.
    pub last_scrape: Option<TrackerAnswer>,
    /// How many seeders the tracker knows of; `None` when it did not say.
    pub seeders: Option<u32>,
    /// How many leechers the tracker knows of; `None` when it did not say.
    pub leechers: Option<u32>,
    /// How many times the tracker saw the torrent completed; `None` when it did not say.
    pub downloads: Option<u32>,
}

impl From<TrackerStatRecord> for TrackerStat {
    fn from(record: TrackerStatRecord) -> Self {
        // The daemon sends -1 for a count the tracker did not report.
        let count = |count: Option<i64>| count.and_then(|count| u32::try_from(count).ok());
        Self {
            id: record.id,
            tier: record.tier,
            is_backup: record.is_backup,
            host: record.host,
            sitename: record.sitename,
            announce_state: record.announce_state,
            next: NextAnnounce::of(record.announce_state, record.next_announce_time),
            last_announce: TrackerAnswer::of(
                record.has_announced,
                record.last_announce_succeeded,
                record.last_announce_timed_out,
                record.last_announce_result,
                record.last_announce_time,
            ),
            // Before the first announce the daemon's count is a 0 that says nothing.
            last_announce_peers: u32::try_from(record.last_announce_peer_count)
                .ok()
                .filter(|_| record.has_announced),
            last_scrape: TrackerAnswer::of(
                record.has_scraped,
                record.last_scrape_succeeded,
                record.last_scrape_timed_out,
                record.last_scrape_result,
                record.last_scrape_time,
            ),
            seeders: count(record.seeder_count),
            leechers: count(record.leecher_count),
            downloads: count(record.download_count),
        }
    }
}

/// What the details panel shows about the selected torrent.
///
/// Fields the server did not send hold a neutral value: empty text, zero, no date, no source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TorrentDetails {
    /// The torrent these details belong to.
    pub hash: HashString,
    /// The comment of the metainfo.
    pub comment: String,
    /// The program that created the metainfo.
    pub creator: String,
    /// When the metainfo was created; `None` when it does not say.
    pub date_created: Option<SystemTime>,
    /// The source tag of the metainfo, which private trackers set; `None` when it has none.
    pub source: Option<String>,
    /// Whether the torrent is private (no DHT or peer exchange).
    pub is_private: bool,
    /// How many pieces the torrent has.
    pub piece_count: u32,
    /// The size of a piece in bytes; 0 while a magnet link has no metadata.
    pub piece_size: u64,
    /// The files, in the order of the torrent.
    pub files: Vec<FileEntry>,
    /// Bytes downloaded over the torrent's life.
    pub downloaded_ever: u64,
    /// Bytes uploaded over the torrent's life.
    pub uploaded_ever: u64,
    /// The trackers, as of the last read that brought them: the full read, or a progress read
    /// with the trackers.
    pub trackers: Vec<TrackerStat>,
}

/// Something wrong with a details answer that the details could be built around.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DetailsIssue {
    /// The answer holds torrents, but none with the requested hash.
    NoMatchingRecord,
    /// `files`, `priorities` and `wanted` of a full read differ in length; the files past the
    /// shortest one are left out.
    FileArraysMismatch {
        /// Entries in `files`.
        files: usize,
        /// Entries in `priorities`.
        priorities: usize,
        /// Entries in `wanted`.
        wanted: usize,
    },
    /// A per-file array of a progress read is missing or does not hold one entry per file; the
    /// files keep their last values of it.
    ProgressArrayMismatch {
        /// The array's wire name.
        array: &'static str,
        /// The files of the last full read.
        files: usize,
        /// Entries in the array; `None` when the answer lacks it.
        entries: Option<usize>,
    },
    /// The torrent has files but the answer has no `pieces`; nothing counts as playable.
    PiecesMissing,
    /// `pieces` is not base64; nothing counts as playable.
    PiecesInvalid,
}

/// Details, and what was wrong with the record they were built from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Built {
    /// The details.
    pub(crate) details: TorrentDetails,
    /// What was wrong, in the order it was found; empty for a sound record.
    pub(crate) issues: Vec<DetailsIssue>,
}

/// Whether `record` is the torrent `hash`, whatever the case of the hash it carries.
pub(crate) fn has_hash(record: &TorrentRecord, hash: &HashString) -> bool {
    record
        .hash_string
        .as_deref()
        .and_then(|raw| HashString::parse(raw).ok())
        .is_some_and(|found| &found == hash)
}

/// The record of the torrent `hash` in a details answer, if it is there.
pub(crate) fn find_record(
    hash: &HashString,
    response: TorrentGetResponse,
) -> Option<TorrentRecord> {
    response
        .torrents
        .into_iter()
        .find(|record| has_hash(record, hash))
}

/// How many files `record` says the torrent has: its `file_count`, or the length of its `files`
/// when it lacks that; `None` when it says neither.
pub(crate) fn file_count(record: &TorrentRecord) -> Option<u32> {
    record.file_count.or_else(|| {
        record
            .files
            .as_ref()
            .and_then(|files| u32::try_from(files.len()).ok())
    })
}

/// Replaces the trackers of `details` with those of its torrent in `response`, which asked for
/// [`TorrentField::TRACKERS`](x4_rpc::TorrentField::TRACKERS) alone: the files and their progress
/// stay as the last reads left them, and their arrays are not looked for, so nothing is reported
/// as missing.
///
/// An answer that holds no record of the torrent, or none with `tracker_stats`, leaves the
/// trackers as they were; one with an empty list empties them, as a torrent whose trackers were
/// all removed has none.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the engine calls it once it reads the trackers for the Trackers tab, in the next task; the expectation then fails and goes"
    )
)]
pub(crate) fn merge_trackers(details: &mut TorrentDetails, response: &TorrentGetResponse) {
    let stats = response
        .torrents
        .iter()
        .find(|record| has_hash(record, &details.hash))
        .and_then(|record| record.tracker_stats.as_ref());
    if let Some(stats) = stats {
        details.trackers = stats.iter().cloned().map(TrackerStat::from).collect();
    }
}

/// Builds the details of `hash` from its record, which was requested with
/// [`TorrentField::DETAILS_FULL`](x4_rpc::TorrentField::DETAILS_FULL).
///
/// The files come from `files`, `priorities` and `wanted` side by side, and each one's playable
/// prefix is computed from `pieces`. That walks the pieces of every file, so it is done for the
/// selected torrent only. Nothing is logged here: the caller decides when an issue is worth a
/// message.
pub(crate) fn build(hash: HashString, record: TorrentRecord) -> Built {
    let mut issues = Vec::new();
    let files = record.files.unwrap_or_default();
    let priorities = record.priorities.unwrap_or_default();
    let wanted = record.wanted.unwrap_or_default();
    if priorities.len() != files.len() || wanted.len() != files.len() {
        issues.push(DetailsIssue::FileArraysMismatch {
            files: files.len(),
            priorities: priorities.len(),
            wanted: wanted.len(),
        });
    }
    let pieces = pieces_of(!files.is_empty(), record.pieces.as_deref(), &mut issues);
    let piece_size = record.piece_size.unwrap_or_default();
    // Offsets come from the whole file list, so a short array cannot shift them.
    let offsets = file_offsets(&files);

    let entries = files
        .into_iter()
        .zip(priorities)
        .zip(wanted)
        .zip(offsets)
        .enumerate()
        .map(|(index, (((file, priority), wanted), offset))| FileEntry {
            index,
            playable_from_start: playable_prefix(&pieces, piece_size, offset, file.length),
            name: file.name,
            length: file.length,
            bytes_completed: file.bytes_completed,
            wanted,
            priority,
        })
        .collect();

    Built {
        details: TorrentDetails {
            hash,
            comment: record.comment.unwrap_or_default(),
            creator: record.creator.unwrap_or_default(),
            date_created: record.date_created.and_then(date_from_wire),
            source: record.source.filter(|source| !source.is_empty()),
            is_private: record.is_private.unwrap_or_default(),
            piece_count: record.piece_count.unwrap_or_default(),
            piece_size,
            files: entries,
            downloaded_ever: record.downloaded_ever.unwrap_or_default(),
            uploaded_ever: record.uploaded_ever.unwrap_or_default(),
            trackers: record
                .tracker_stats
                .unwrap_or_default()
                .into_iter()
                .map(TrackerStat::from)
                .collect(),
        },
        issues,
    }
}

/// The details `last` with the progress of `record` merged in; `record` was requested with
/// [`TorrentField::DETAILS_PROGRESS`](x4_rpc::TorrentField::DETAILS_PROGRESS), or with
/// [`TorrentField::DETAILS_PROGRESS_TRACKERS`](x4_rpc::TorrentField::DETAILS_PROGRESS_TRACKERS),
/// for the same torrent, whose files have not changed since `last` was read.
///
/// Each file takes its bytes, priority and wanted switch from the arrays of the same names, and
/// its playable prefix from the new pieces. An array that is missing or does not hold one entry
/// per file is left out, so the files keep their last values of it, and is reported; the totals
/// keep their last values when the answer lacks them. The names and the metainfo stay as `last`
/// has them, and so do the trackers, unless the answer holds
/// [`tracker_stats`](x4_rpc::TorrentField::TrackerStats), when those replace them.
pub(crate) fn merge_progress(last: &TorrentDetails, record: TorrentRecord) -> Built {
    let mut issues = Vec::new();
    let mut details = last.clone();
    let count = details.files.len();
    if let Some(bytes) = per_file(
        count,
        "bytes_completed",
        record.bytes_completed,
        &mut issues,
    ) {
        for (file, bytes) in details.files.iter_mut().zip(bytes) {
            file.bytes_completed = bytes;
        }
    }
    if let Some(priorities) = per_file(count, "priorities", record.priorities, &mut issues) {
        for (file, priority) in details.files.iter_mut().zip(priorities) {
            file.priority = priority;
        }
    }
    if let Some(wanted) = per_file(count, "wanted", record.wanted, &mut issues) {
        for (file, wanted) in details.files.iter_mut().zip(wanted) {
            file.wanted = wanted;
        }
    }
    let pieces = pieces_of(count > 0, record.pieces.as_deref(), &mut issues);
    let mut offset = 0_u64;
    for file in &mut details.files {
        file.playable_from_start =
            playable_prefix(&pieces, details.piece_size, offset, file.length);
        offset = offset.saturating_add(file.length);
    }
    if let Some(downloaded) = record.downloaded_ever {
        details.downloaded_ever = downloaded;
    }
    if let Some(uploaded) = record.uploaded_ever {
        details.uploaded_ever = uploaded;
    }
    if let Some(trackers) = record.tracker_stats {
        details.trackers = trackers.into_iter().map(TrackerStat::from).collect();
    }
    Built { details, issues }
}

/// The per-file array `values` of a progress read when it holds one entry for each of `files`;
/// otherwise `None`, with the issue noted. A torrent without files needs no array.
fn per_file<T>(
    files: usize,
    array: &'static str,
    values: Option<Vec<T>>,
    issues: &mut Vec<DetailsIssue>,
) -> Option<Vec<T>> {
    match values {
        Some(values) if values.len() == files => Some(values),
        None if files == 0 => None,
        values => {
            issues.push(DetailsIssue::ProgressArrayMismatch {
                array,
                files,
                entries: values.as_ref().map(Vec::len),
            });
            None
        }
    }
}

/// The bitfield of `pieces` for a torrent that `has_files`; an empty one, with the issue noted,
/// when it is missing or invalid. A torrent without files needs none.
fn pieces_of(has_files: bool, pieces: Option<&str>, issues: &mut Vec<DetailsIssue>) -> Pieces {
    if !has_files {
        return Pieces::from_bytes(Vec::new());
    }
    let decoded = match pieces {
        None => Err(DetailsIssue::PiecesMissing),
        Some(text) => Pieces::decode_base64(text).map_err(|_| DetailsIssue::PiecesInvalid),
    };
    decoded.unwrap_or_else(|issue| {
        issues.push(issue);
        Pieces::from_bytes(Vec::new())
    })
}

#[cfg(test)]
mod tests {
    use x4_rpc::testing::fixtures::TORRENT_GET_DETAILS;
    use x4_rpc::{FileRecord, TorrentId, TrackerId, TrackerStatRecord, TrackerState};

    use super::*;

    use crate::test_support::fixture_result;

    const HASH: &str = "0123456789abcdef0123456789abcdef01234567";
    /// The hash of the torrent in the details fixture.
    const FIXTURE_HASH: &str = "0000000000000000000000000000000000000001";

    fn hash() -> HashString {
        HashString::parse(HASH).unwrap()
    }

    fn file(name: &str, length: u64, bytes_completed: u64) -> FileRecord {
        FileRecord {
            name: name.to_owned(),
            length,
            bytes_completed,
            begin_piece: 0,
            end_piece: 0,
        }
    }

    /// Two files of 6 and 10 bytes in pieces of 4 bytes, with pieces 0 and 1 present: the first
    /// file is whole, the second has bytes 6 to 7 of its 10.
    fn two_files() -> TorrentRecord {
        TorrentRecord {
            id: Some(TorrentId(1)),
            hash_string: Some(HASH.to_owned()),
            comment: Some("a comment".to_owned()),
            piece_size: Some(4),
            piece_count: Some(4),
            // 0b1100_0000: pieces 0 and 1.
            pieces: Some("wA==".to_owned()),
            file_count: Some(2),
            files: Some(vec![file("t/a.mkv", 6, 6), file("t/b.mkv", 10, 2)]),
            priorities: Some(vec![FilePriority::High, FilePriority::Low]),
            wanted: Some(vec![true, false]),
            downloaded_ever: Some(8),
            uploaded_ever: Some(3),
            ..TorrentRecord::default()
        }
    }

    /// The progress of [`two_files`] once both files are complete and the user changed their
    /// priorities and wanted switches.
    fn two_files_progress() -> TorrentRecord {
        TorrentRecord {
            hash_string: Some(HASH.to_owned()),
            file_count: Some(2),
            bytes_completed: Some(vec![6, 10]),
            priorities: Some(vec![FilePriority::Normal, FilePriority::High]),
            wanted: Some(vec![false, true]),
            // 0b1111_0000: every piece.
            pieces: Some("8A==".to_owned()),
            downloaded_ever: Some(16),
            uploaded_ever: Some(5),
            ..TorrentRecord::default()
        }
    }

    /// A tracker whose last announce timed out and whose last scrape succeeded.
    fn tracker(host: &str, seeder_count: i64, leecher_count: i64) -> TrackerStatRecord {
        TrackerStatRecord {
            host: host.to_owned(),
            has_announced: true,
            last_announce_succeeded: false,
            last_announce_timed_out: true,
            last_announce_result: "Tracker did not respond".to_owned(),
            has_scraped: true,
            last_scrape_succeeded: true,
            last_scrape_timed_out: false,
            last_scrape_result: "Success".to_owned(),
            seeder_count: Some(seeder_count),
            leecher_count: Some(leecher_count),
            ..TrackerStatRecord::default()
        }
    }

    fn playable(details: &TorrentDetails) -> Vec<u64> {
        details
            .files
            .iter()
            .map(|file| file.playable_from_start)
            .collect()
    }

    #[test]
    fn files_take_their_state_from_the_arrays_and_their_playable_prefix_from_pieces() {
        let built = build(hash(), two_files());

        assert_eq!(built.issues, []);
        assert_eq!(
            built.details.files,
            [
                FileEntry {
                    index: 0,
                    name: "t/a.mkv".to_owned(),
                    length: 6,
                    bytes_completed: 6,
                    wanted: true,
                    priority: FilePriority::High,
                    playable_from_start: 6,
                },
                FileEntry {
                    index: 1,
                    name: "t/b.mkv".to_owned(),
                    length: 10,
                    bytes_completed: 2,
                    wanted: false,
                    priority: FilePriority::Low,
                    playable_from_start: 2,
                },
            ]
        );
    }

    #[test]
    fn a_short_per_file_array_cuts_the_files_and_is_reported() {
        let short_priorities = TorrentRecord {
            priorities: Some(vec![FilePriority::Normal]),
            ..two_files()
        };
        let short_files = TorrentRecord {
            files: Some(vec![file("t/a.mkv", 6, 6)]),
            ..two_files()
        };
        let no_wanted = TorrentRecord {
            wanted: None,
            ..two_files()
        };

        let built = build(hash(), short_priorities);
        assert_eq!(built.details.files.len(), 1);
        assert_eq!(
            built.issues,
            [DetailsIssue::FileArraysMismatch {
                files: 2,
                priorities: 1,
                wanted: 2
            }]
        );

        let built = build(hash(), short_files);
        assert_eq!(playable(&built.details), [6]);
        assert_eq!(
            built.issues,
            [DetailsIssue::FileArraysMismatch {
                files: 1,
                priorities: 2,
                wanted: 2
            }]
        );

        let built = build(hash(), no_wanted);
        assert_eq!(built.details.files, []);
        assert_eq!(
            built.issues,
            [DetailsIssue::FileArraysMismatch {
                files: 2,
                priorities: 2,
                wanted: 0
            }]
        );
    }

    #[test]
    fn missing_or_invalid_pieces_make_nothing_playable_and_are_reported() {
        for (pieces, issue) in [
            (None, DetailsIssue::PiecesMissing),
            (Some("not base64!".to_owned()), DetailsIssue::PiecesInvalid),
        ] {
            let built = build(
                hash(),
                TorrentRecord {
                    pieces,
                    ..two_files()
                },
            );

            assert_eq!(built.issues, std::slice::from_ref(&issue));
            assert_eq!(playable(&built.details), [0, 0], "{issue:?}");
        }
    }

    #[test]
    fn a_magnet_without_metadata_has_no_files_and_no_issue() {
        let record = TorrentRecord {
            hash_string: Some(HASH.to_owned()),
            piece_size: Some(0),
            piece_count: Some(0),
            file_count: Some(0),
            files: Some(Vec::new()),
            priorities: Some(Vec::new()),
            wanted: Some(Vec::new()),
            pieces: Some(String::new()),
            ..TorrentRecord::default()
        };

        let built = build(hash(), record);

        assert_eq!(built.issues, []);
        assert_eq!(built.details.files, []);
        assert_eq!(built.details.piece_size, 0);
    }

    #[test]
    fn missing_fields_fall_back_to_neutral_values() {
        let built = build(hash(), TorrentRecord::default());

        assert_eq!(
            built.details,
            TorrentDetails {
                hash: hash(),
                comment: String::new(),
                creator: String::new(),
                date_created: None,
                source: None,
                is_private: false,
                piece_count: 0,
                piece_size: 0,
                files: Vec::new(),
                downloaded_ever: 0,
                uploaded_ever: 0,
                trackers: Vec::new(),
            }
        );
        assert_eq!(built.issues, []);
    }

    #[test]
    fn an_empty_source_is_none() {
        let source = |source: &str| {
            build(
                hash(),
                TorrentRecord {
                    source: Some(source.to_owned()),
                    ..two_files()
                },
            )
            .details
            .source
        };

        assert_eq!(source(""), None);
        assert_eq!(source("TAG"), Some("TAG".to_owned()));
    }

    #[test]
    fn trackers_keep_what_they_answered_and_unknown_counts_are_none() {
        let record = TorrentRecord {
            tracker_stats: Some(vec![
                tracker("a.example:443", 12, 3),
                tracker("b.example:80", -1, -1),
                TrackerStatRecord {
                    host: "c.example:6969".to_owned(),
                    ..TrackerStatRecord::default()
                },
            ]),
            ..two_files()
        };

        let trackers = build(hash(), record).details.trackers;

        assert_eq!(
            trackers.first(),
            Some(&TrackerStat {
                last_announce: Some(TrackerAnswer {
                    outcome: TrackerOutcome::TimedOut,
                    result: "Tracker did not respond".to_owned(),
                    at: None,
                }),
                last_scrape: Some(TrackerAnswer {
                    outcome: TrackerOutcome::Succeeded,
                    result: "Success".to_owned(),
                    at: None,
                }),
                last_announce_peers: Some(0),
                seeders: Some(12),
                leechers: Some(3),
                ..crate::test_support::tracker("a.example:443")
            })
        );
        // Never announced to nor scraped, as a backup tracker.
        assert_eq!(
            trackers
                .get(2)
                .map(|tracker| (&tracker.last_announce, &tracker.last_scrape)),
            Some((&None, &None))
        );
        let counts: Vec<(Option<u32>, Option<u32>)> = trackers
            .iter()
            .map(|tracker| (tracker.seeders, tracker.leechers))
            .collect();
        assert_eq!(counts, [(Some(12), Some(3)), (None, None), (None, None)]);
    }

    #[test]
    fn a_tracker_answer_that_neither_succeeded_nor_timed_out_failed() {
        let refused = TrackerStatRecord {
            has_announced: true,
            last_announce_result: "Unregistered torrent".to_owned(),
            ..TrackerStatRecord::default()
        };

        assert_eq!(
            TrackerStat::from(refused).last_announce,
            Some(TrackerAnswer {
                outcome: TrackerOutcome::Failed,
                result: "Unregistered torrent".to_owned(),
                at: None,
            })
        );
    }

    /// A tracker of the second tier that was announced to and scraped, as the daemon reports it
    /// while it waits for the next announce.
    fn timed_tracker() -> TrackerStatRecord {
        TrackerStatRecord {
            id: TrackerId(7),
            tier: 1,
            host: "tracker.example:6969".to_owned(),
            sitename: "example".to_owned(),
            announce_state: TrackerState::Waiting,
            has_announced: true,
            last_announce_time: 1_700_000_000,
            last_announce_succeeded: true,
            last_announce_result: "Success".to_owned(),
            last_announce_peer_count: 30,
            next_announce_time: 1_700_001_800,
            has_scraped: true,
            last_scrape_time: 1_700_000_100,
            last_scrape_succeeded: false,
            last_scrape_result: "Could not connect to tracker".to_owned(),
            seeder_count: Some(5),
            leecher_count: Some(-1),
            download_count: Some(345),
            ..TrackerStatRecord::default()
        }
    }

    fn seconds(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds)
    }

    #[test]
    fn a_tracker_keeps_its_place_its_state_and_its_times() {
        let stat = TrackerStat::from(timed_tracker());

        assert_eq!(
            stat,
            TrackerStat {
                id: TrackerId(7),
                tier: 1,
                is_backup: false,
                host: "tracker.example:6969".to_owned(),
                sitename: "example".to_owned(),
                announce_state: TrackerState::Waiting,
                next: NextAnnounce::At(seconds(1_700_001_800)),
                last_announce: Some(TrackerAnswer {
                    outcome: TrackerOutcome::Succeeded,
                    result: "Success".to_owned(),
                    at: Some(seconds(1_700_000_000)),
                }),
                last_announce_peers: Some(30),
                last_scrape: Some(TrackerAnswer {
                    outcome: TrackerOutcome::Failed,
                    result: "Could not connect to tracker".to_owned(),
                    at: Some(seconds(1_700_000_100)),
                }),
                seeders: Some(5),
                leechers: None,
                downloads: Some(345),
            }
        );
    }

    #[test]
    fn the_next_announce_is_a_time_only_while_the_tracker_waits_for_one() {
        let next = |announce_state: TrackerState, next_announce_time: i64| {
            TrackerStat::from(TrackerStatRecord {
                announce_state,
                next_announce_time,
                ..timed_tracker()
            })
            .next
        };
        let at = NextAnnounce::At(seconds(1_700_001_800));

        assert_eq!(
            next(TrackerState::Inactive, 1_700_001_800),
            NextAnnounce::Idle
        );
        assert_eq!(next(TrackerState::Waiting, 1_700_001_800), at);
        assert_eq!(next(TrackerState::Queued, 0), NextAnnounce::Queued);
        assert_eq!(
            next(TrackerState::Queued, 1_700_001_800),
            NextAnnounce::Queued
        );
        // Announcing now, there is no next time yet.
        assert_eq!(
            next(TrackerState::Active, 1_700_001_800),
            NextAnnounce::Idle
        );
        // A state this version does not know says nothing about when.
        assert_eq!(
            next(TrackerState::Other(9), 1_700_001_800),
            NextAnnounce::Idle
        );
        // Waiting without a time: nothing to count down to.
        assert_eq!(next(TrackerState::Waiting, 0), NextAnnounce::Idle);
        assert_eq!(next(TrackerState::Waiting, -5), NextAnnounce::Idle);
    }

    #[test]
    fn a_time_of_zero_is_no_time_and_the_answer_stays() {
        let stat = TrackerStat::from(TrackerStatRecord {
            last_announce_time: 0,
            last_scrape_time: 0,
            ..timed_tracker()
        });

        assert_eq!(stat.last_announce.map(|answer| answer.at), Some(None));
        assert_eq!(stat.last_scrape.map(|answer| answer.at), Some(None));
    }

    #[test]
    fn the_peers_of_the_last_announce_are_known_only_after_one() {
        let peers = |has_announced: bool, count: i64| {
            TrackerStat::from(TrackerStatRecord {
                has_announced,
                last_announce_peer_count: count,
                ..timed_tracker()
            })
            .last_announce_peers
        };

        assert_eq!(peers(true, 30), Some(30));
        assert_eq!(peers(true, 0), Some(0));
        assert_eq!(peers(true, -1), None);
        assert_eq!(peers(false, 0), None);
        assert_eq!(peers(false, 30), None);
    }

    #[test]
    fn a_backup_tracker_has_neither_answers_nor_a_next_announce() {
        let stat = TrackerStat::from(TrackerStatRecord {
            id: TrackerId(3),
            is_backup: true,
            host: "backup.example:80".to_owned(),
            ..TrackerStatRecord::default()
        });

        assert!(stat.is_backup);
        assert_eq!(stat.id, TrackerId(3));
        assert_eq!(stat.next, NextAnnounce::Idle);
        assert_eq!((stat.last_announce, stat.last_scrape), (None, None));
        assert_eq!(stat.last_announce_peers, None);
    }

    /// A `torrent_get` answer for the tab read of the trackers of `HASH`: the hash and the trackers.
    fn trackers_answer(trackers: Option<Vec<TrackerStatRecord>>) -> TorrentGetResponse {
        TorrentGetResponse {
            torrents: vec![TorrentRecord {
                id: Some(TorrentId(1)),
                hash_string: Some(HASH.to_owned()),
                tracker_stats: trackers,
                ..TorrentRecord::default()
            }],
            removed: Vec::new(),
        }
    }

    #[test]
    fn merging_trackers_replaces_them_and_leaves_everything_else() {
        let mut full = two_files();
        full.tracker_stats = Some(vec![tracker("old.example:80", 1, 2)]);
        let last = build(hash(), full).details;
        let mut details = last.clone();

        merge_trackers(
            &mut details,
            &trackers_answer(Some(vec![timed_tracker(), tracker("b.example:443", 7, 8)])),
        );

        let hosts: Vec<&str> = details.trackers.iter().map(|t| t.host.as_str()).collect();
        assert_eq!(hosts, ["tracker.example:6969", "b.example:443"]);
        assert_eq!(details.trackers[0], TrackerStat::from(timed_tracker()));
        // The files, their progress and the totals are as the last read left them.
        assert_eq!(
            details,
            TorrentDetails {
                trackers: details.trackers.clone(),
                ..last
            }
        );
        assert_eq!(details.files.len(), 2);
    }

    #[test]
    fn merging_trackers_needs_no_file_arrays() {
        // The answer holds nothing but the hash and the trackers, which is what the tab asks for.
        let mut details = build(hash(), two_files()).details;
        let files = details.files.clone();

        merge_trackers(&mut details, &trackers_answer(Some(vec![timed_tracker()])));

        assert_eq!(details.files, files);
        assert_eq!(details.trackers.len(), 1);
    }

    #[test]
    fn merging_an_answer_without_trackers_or_for_another_torrent_changes_nothing() {
        let mut full = two_files();
        full.tracker_stats = Some(vec![tracker("old.example:80", 1, 2)]);
        let last = build(hash(), full).details;
        let other = TorrentGetResponse {
            torrents: vec![TorrentRecord {
                hash_string: Some("f".repeat(40)),
                tracker_stats: Some(vec![timed_tracker()]),
                ..TorrentRecord::default()
            }],
            removed: Vec::new(),
        };
        let nothing = TorrentGetResponse {
            torrents: Vec::new(),
            removed: Vec::new(),
        };

        for response in [trackers_answer(None), other, nothing] {
            let mut details = last.clone();
            merge_trackers(&mut details, &response);
            assert_eq!(details, last);
        }
    }

    #[test]
    fn merging_an_empty_list_of_trackers_empties_them_and_the_hash_may_be_in_capitals() {
        let mut full = two_files();
        full.tracker_stats = Some(vec![tracker("old.example:80", 1, 2)]);
        let mut details = build(hash(), full).details;
        let mut response = trackers_answer(Some(Vec::new()));
        response.torrents[0].hash_string = Some(HASH.to_uppercase());

        merge_trackers(&mut details, &response);

        assert_eq!(details.trackers, []);
    }

    #[test]
    fn merging_the_trackers_fixture_gives_two_trackers_in_the_daemons_order() {
        use x4_rpc::testing::fixtures::TORRENT_GET_TRACKERS;

        let response: TorrentGetResponse =
            serde_json::from_value(fixture_result(TORRENT_GET_TRACKERS)).unwrap();
        let fixture_hash = HashString::parse("0000000000000000000000000000000000000005").unwrap();
        let mut details = build(fixture_hash, TorrentRecord::default()).details;

        merge_trackers(&mut details, &response);

        let shape: Vec<(TrackerId, u32, TrackerState)> = details
            .trackers
            .iter()
            .map(|tracker| (tracker.id, tracker.tier, tracker.announce_state))
            .collect();
        assert_eq!(
            shape,
            [
                (TrackerId(6), 0, TrackerState::Waiting),
                (TrackerId(7), 1, TrackerState::Waiting)
            ]
        );
        assert_eq!(
            details.trackers[0].next,
            NextAnnounce::At(seconds(1_793_787_926))
        );
    }

    #[test]
    fn the_fixture_builds_complete_details() {
        let response: TorrentGetResponse =
            serde_json::from_value(fixture_result(TORRENT_GET_DETAILS)).unwrap();
        let fixture_hash = HashString::parse(FIXTURE_HASH).unwrap();

        let record = find_record(&fixture_hash, response).unwrap();
        assert_eq!(file_count(&record), Some(10));
        let built = build(fixture_hash.clone(), record);

        assert_eq!(built.issues, []);
        let details = built.details;
        assert_eq!(details.hash, fixture_hash);
        assert_eq!(details.comment, "redacted");
        assert_eq!(details.source.as_deref(), Some("redacted"));
        assert_eq!(details.date_created, None);
        assert_eq!(details.piece_count, 14685);
        assert_eq!(details.piece_size, 4_194_304);
        assert_eq!(details.files.len(), 10);
        assert_eq!(details.files[0].priority, FilePriority::High);
        let indices: Vec<usize> = details.files.iter().map(|file| file.index).collect();
        assert_eq!(indices, (0..10).collect::<Vec<_>>());
        assert_eq!(details.trackers.len(), 1);
        assert_eq!(
            details.trackers[0]
                .last_announce
                .as_ref()
                .map(|answer| answer.outcome),
            Some(TrackerOutcome::Succeeded)
        );
        assert_eq!(details.trackers[0].seeders, Some(57));
    }

    #[test]
    fn the_file_count_falls_back_to_the_files() {
        assert_eq!(file_count(&two_files()), Some(2));
        let without_count = TorrentRecord {
            file_count: None,
            ..two_files()
        };
        assert_eq!(file_count(&without_count), Some(2));
        assert_eq!(file_count(&TorrentRecord::default()), None);
    }

    #[test]
    fn progress_changes_only_what_moves_while_a_torrent_downloads() {
        let last = build(hash(), two_files()).details;

        let merged = merge_progress(&last, two_files_progress());

        assert_eq!(merged.issues, []);
        assert_eq!(
            merged.details,
            TorrentDetails {
                downloaded_ever: 16,
                uploaded_ever: 5,
                files: vec![
                    FileEntry {
                        bytes_completed: 6,
                        wanted: false,
                        priority: FilePriority::Normal,
                        playable_from_start: 6,
                        ..last.files[0].clone()
                    },
                    FileEntry {
                        bytes_completed: 10,
                        wanted: true,
                        priority: FilePriority::High,
                        playable_from_start: 10,
                        ..last.files[1].clone()
                    },
                ],
                ..last.clone()
            }
        );
        // The metainfo and the names come from the full read.
        assert_eq!(merged.details.comment, "a comment");
        assert_eq!(merged.details.files[1].name, "t/b.mkv");
    }

    #[test]
    fn a_progress_array_of_the_wrong_length_keeps_its_last_values_and_is_reported() {
        let last = build(hash(), two_files()).details;
        let record = TorrentRecord {
            priorities: Some(vec![FilePriority::Normal]),
            wanted: None,
            ..two_files_progress()
        };

        let merged = merge_progress(&last, record);

        let state: Vec<(u64, bool, FilePriority)> = merged
            .details
            .files
            .iter()
            .map(|file| (file.bytes_completed, file.wanted, file.priority))
            .collect();
        // Bytes are new; the priorities and the wanted switches are the last ones.
        assert_eq!(
            state,
            [
                (6, true, FilePriority::High),
                (10, false, FilePriority::Low)
            ]
        );
        assert_eq!(
            merged.issues,
            [
                DetailsIssue::ProgressArrayMismatch {
                    array: "priorities",
                    files: 2,
                    entries: Some(1)
                },
                DetailsIssue::ProgressArrayMismatch {
                    array: "wanted",
                    files: 2,
                    entries: None
                },
            ]
        );
    }

    #[test]
    fn missing_progress_totals_keep_the_last_ones() {
        let last = build(hash(), two_files()).details;
        let record = TorrentRecord {
            downloaded_ever: None,
            uploaded_ever: None,
            ..two_files_progress()
        };

        let merged = merge_progress(&last, record);

        assert_eq!(
            (merged.details.downloaded_ever, merged.details.uploaded_ever),
            (8, 3)
        );
    }

    #[test]
    fn progress_with_trackers_replaces_them_and_one_without_keeps_them() {
        let mut full = two_files();
        full.tracker_stats = Some(vec![tracker("a.example:80", 3, 4)]);
        let last = build(hash(), full).details;
        assert_eq!(last.trackers.len(), 1);

        let without = merge_progress(&last, two_files_progress());
        let replaced = merge_progress(
            &last,
            TorrentRecord {
                tracker_stats: Some(vec![
                    tracker("b.example:443", 7, 8),
                    tracker("c.example:80", 9, 1),
                ]),
                ..two_files_progress()
            },
        );
        // A torrent whose trackers were all removed has none left.
        let emptied = merge_progress(
            &last,
            TorrentRecord {
                tracker_stats: Some(Vec::new()),
                ..two_files_progress()
            },
        );

        assert_eq!(without.details.trackers, last.trackers);
        let hosts = |built: &Built| -> Vec<String> {
            built
                .details
                .trackers
                .iter()
                .map(|tracker| tracker.host.clone())
                .collect()
        };
        assert_eq!(hosts(&replaced), ["b.example:443", "c.example:80"]);
        assert_eq!(replaced.details.trackers[0].seeders, Some(7));
        assert_eq!(replaced.details.files, without.details.files);
        assert_eq!(emptied.details.trackers, []);
        assert_eq!(emptied.issues, []);
    }

    #[test]
    fn progress_of_a_torrent_without_files_needs_no_arrays() {
        let last = build(hash(), TorrentRecord::default()).details;

        let merged = merge_progress(&last, TorrentRecord::default());

        assert_eq!(merged.issues, []);
        assert_eq!(merged.details, last);
    }

    #[test]
    fn missing_progress_pieces_make_nothing_playable_and_are_reported() {
        let last = build(hash(), two_files()).details;
        let record = TorrentRecord {
            pieces: None,
            ..two_files_progress()
        };

        let merged = merge_progress(&last, record);

        assert_eq!(merged.issues, [DetailsIssue::PiecesMissing]);
        assert_eq!(playable(&merged.details), [0, 0]);
    }

    #[test]
    fn find_record_picks_the_requested_hash_in_any_case() {
        let other = TorrentRecord {
            hash_string: Some("f".repeat(40)),
            ..TorrentRecord::default()
        };
        let wanted = TorrentRecord {
            hash_string: Some(HASH.to_uppercase()),
            comment: Some("this one".to_owned()),
            ..TorrentRecord::default()
        };
        let response = |torrents| TorrentGetResponse {
            torrents,
            removed: Vec::new(),
        };

        let found = find_record(&hash(), response(vec![other.clone(), wanted])).unwrap();
        assert_eq!(found.comment.as_deref(), Some("this one"));

        assert_eq!(find_record(&hash(), response(vec![other])), None);
        let no_hash = TorrentRecord::default();
        assert_eq!(find_record(&hash(), response(vec![no_hash])), None);
    }
}
