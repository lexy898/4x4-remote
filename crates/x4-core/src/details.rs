//! The details of the selected torrent: its metainfo, its totals and its files.

use std::time::SystemTime;

use x4_rpc::{FilePriority, HashString, Pieces, TorrentGetResponse, TorrentRecord};

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

/// What the details panel shows about the selected torrent.
///
/// Fields the server did not send hold a neutral value: empty text, zero, no date.
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
    /// A magnet link for the torrent.
    pub magnet_link: String,
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
}

/// Something wrong with a details answer that the details could be built around.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DetailsIssue {
    /// The answer holds torrents, but none with the requested hash.
    NoMatchingRecord,
    /// `files` and `file_stats` differ in length; the files past the shorter one are left out.
    FileStatsMismatch {
        /// Entries in `files`.
        files: usize,
        /// Entries in `file_stats`.
        file_stats: usize,
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

/// The record of the torrent `hash` in a details answer, if it is there.
pub(crate) fn find_record(
    hash: &HashString,
    response: TorrentGetResponse,
) -> Option<TorrentRecord> {
    response.torrents.into_iter().find(|record| {
        record
            .hash_string
            .as_deref()
            .and_then(|raw| HashString::parse(raw).ok())
            .is_some_and(|found| &found == hash)
    })
}

/// Builds the details of `hash` from its record, which was requested with
/// [`TorrentField::DETAILS`](x4_rpc::TorrentField::DETAILS).
///
/// The files come from `files` and `file_stats` side by side, and each one's playable prefix is
/// computed from `pieces`. That walks the pieces of every file, so it is done for the selected
/// torrent only. Nothing is logged here: the caller decides when an issue is worth a message.
pub(crate) fn build(hash: HashString, record: TorrentRecord) -> Built {
    let mut issues = Vec::new();
    let files = record.files.unwrap_or_default();
    let file_stats = record.file_stats.unwrap_or_default();
    if files.len() != file_stats.len() {
        issues.push(DetailsIssue::FileStatsMismatch {
            files: files.len(),
            file_stats: file_stats.len(),
        });
    }
    let pieces = if files.is_empty() {
        Pieces::from_bytes(Vec::new())
    } else {
        decode_pieces(record.pieces.as_deref(), &mut issues)
    };
    let piece_size = record.piece_size.unwrap_or_default();
    // Offsets come from the whole file list, so a short `file_stats` cannot shift them.
    let offsets = file_offsets(&files);

    let entries = files
        .into_iter()
        .zip(file_stats)
        .zip(offsets)
        .enumerate()
        .map(|(index, ((file, stats), offset))| FileEntry {
            index,
            playable_from_start: playable_prefix(&pieces, piece_size, offset, file.length),
            name: file.name,
            length: file.length,
            bytes_completed: stats.bytes_completed,
            wanted: stats.wanted,
            priority: stats.priority,
        })
        .collect();

    Built {
        details: TorrentDetails {
            hash,
            comment: record.comment.unwrap_or_default(),
            creator: record.creator.unwrap_or_default(),
            date_created: record.date_created.and_then(date_from_wire),
            magnet_link: record.magnet_link.unwrap_or_default(),
            is_private: record.is_private.unwrap_or_default(),
            piece_count: record.piece_count.unwrap_or_default(),
            piece_size,
            files: entries,
            downloaded_ever: record.downloaded_ever.unwrap_or_default(),
            uploaded_ever: record.uploaded_ever.unwrap_or_default(),
        },
        issues,
    }
}

/// The bitfield of `pieces`; an empty one, with the issue noted, when it is missing or invalid.
fn decode_pieces(pieces: Option<&str>, issues: &mut Vec<DetailsIssue>) -> Pieces {
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
    use x4_rpc::{FileRecord, FileStatRecord, TorrentId};

    use super::*;

    use crate::test_support::fixture_result;

    const HASH: &str = "0123456789abcdef0123456789abcdef01234567";
    /// The hash of the torrent in the details fixture.
    const FIXTURE_HASH: &str = "0000000000000000000000000000000000000001";

    fn hash() -> HashString {
        HashString::parse(HASH).unwrap()
    }

    fn file(name: &str, length: u64) -> FileRecord {
        FileRecord {
            name: name.to_owned(),
            length,
            bytes_completed: 0,
            begin_piece: 0,
            end_piece: 0,
        }
    }

    fn stats(bytes_completed: u64, wanted: bool, priority: FilePriority) -> FileStatRecord {
        FileStatRecord {
            bytes_completed,
            wanted,
            priority,
        }
    }

    /// Two files of 6 and 10 bytes in pieces of 4 bytes, with pieces 0 and 1 present: the first
    /// file is whole, the second has bytes 6 to 7 of its 10.
    fn two_files() -> TorrentRecord {
        TorrentRecord {
            id: Some(TorrentId(1)),
            hash_string: Some(HASH.to_owned()),
            piece_size: Some(4),
            piece_count: Some(4),
            // 0b1100_0000: pieces 0 and 1.
            pieces: Some("wA==".to_owned()),
            files: Some(vec![file("t/a.mkv", 6), file("t/b.mkv", 10)]),
            file_stats: Some(vec![
                stats(6, true, FilePriority::High),
                stats(2, false, FilePriority::Low),
            ]),
            ..TorrentRecord::default()
        }
    }

    #[test]
    fn files_take_their_state_from_file_stats_and_their_playable_prefix_from_pieces() {
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
    fn a_short_file_stats_list_cuts_the_files_and_is_reported() {
        let record = TorrentRecord {
            file_stats: Some(vec![stats(6, true, FilePriority::Normal)]),
            ..two_files()
        };

        let built = build(hash(), record);

        assert_eq!(built.details.files.len(), 1);
        assert_eq!(
            built.issues,
            [DetailsIssue::FileStatsMismatch {
                files: 2,
                file_stats: 1
            }]
        );
    }

    #[test]
    fn a_short_file_list_cuts_the_stats_and_is_reported() {
        let record = TorrentRecord {
            files: Some(vec![file("t/a.mkv", 6)]),
            ..two_files()
        };

        let built = build(hash(), record);

        assert_eq!(built.details.files.len(), 1);
        assert_eq!(built.details.files[0].playable_from_start, 6);
        assert_eq!(
            built.issues,
            [DetailsIssue::FileStatsMismatch {
                files: 1,
                file_stats: 2
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
            let playable: Vec<u64> = built
                .details
                .files
                .iter()
                .map(|file| file.playable_from_start)
                .collect();
            assert_eq!(playable, [0, 0], "{issue:?}");
        }
    }

    #[test]
    fn a_magnet_without_metadata_has_no_files_and_no_issue() {
        let record = TorrentRecord {
            hash_string: Some(HASH.to_owned()),
            piece_size: Some(0),
            piece_count: Some(0),
            files: Some(Vec::new()),
            file_stats: Some(Vec::new()),
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
                magnet_link: String::new(),
                is_private: false,
                piece_count: 0,
                piece_size: 0,
                files: Vec::new(),
                downloaded_ever: 0,
                uploaded_ever: 0,
            }
        );
        assert_eq!(built.issues, []);
    }

    #[test]
    fn the_fixture_builds_complete_details() {
        let response: TorrentGetResponse =
            serde_json::from_value(fixture_result(TORRENT_GET_DETAILS)).unwrap();
        let fixture_hash = HashString::parse(FIXTURE_HASH).unwrap();

        let record = find_record(&fixture_hash, response).unwrap();
        let built = build(fixture_hash.clone(), record);

        assert_eq!(built.issues, []);
        let details = built.details;
        assert_eq!(details.hash, fixture_hash);
        assert_eq!(details.comment, "redacted");
        assert_eq!(details.date_created, None);
        assert_eq!(details.piece_count, 14685);
        assert_eq!(details.piece_size, 4_194_304);
        assert_eq!(details.files.len(), 10);
        assert_eq!(details.files[0].priority, FilePriority::High);
        let indices: Vec<usize> = details.files.iter().map(|file| file.index).collect();
        assert_eq!(indices, (0..10).collect::<Vec<_>>());
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
