//! The names of the legacy protocol, and how they map to the snake case names of JSON-RPC.
//!
//! The legacy names mix kebab-case and camelCase with no rule (`hashString`, `file-count`,
//! `download-dir`, and `downloadDir` for the same thing in a torrent), so every name 4x4 sends or
//! reads is listed here, the ones that stay the same too: a name with no entry cannot be sent.
//! The names are the facts of the legacy specs (`docs/rpc-spec.md` of Transmission 4.0.6 and
//! `extras/rpc-spec.txt` of 3.00).

use std::collections::HashMap;
use std::sync::LazyLock;

/// Where a name stands in a request, which decides its legacy spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NameContext {
    /// A value of the `fields` of `torrent-get`: a torrent's field, mostly camelCase.
    TorrentField,
    /// A key of a request's arguments, or a value of the `fields` of `session-get`: mostly
    /// kebab-case.
    Argument,
}

/// The legacy RPC version of Transmission 3.00, which has every torrent field below unless one
/// says otherwise.
const RPC_3_00: u32 = super::OLDEST_RPC_VERSION;
/// The legacy RPC version of Transmission 4.0.
const RPC_4_0: u32 = 17;
/// The legacy RPC version of Transmission 4.1.0-beta.
const RPC_4_1_BETA: u32 = 18;

/// The torrent fields: snake case, legacy, and the legacy RPC version that brought the field.
///
/// A daemon is never asked for a field newer than it: Transmission 3.00 answers a field it does
/// not know with `0` rather than leaving it out, which would read as a value.
const TORRENT_FIELDS: &[(&str, &str, u32)] = &[
    ("id", "id", RPC_3_00),
    ("hash_string", "hashString", RPC_3_00),
    ("name", "name", RPC_3_00),
    ("status", "status", RPC_3_00),
    ("error", "error", RPC_3_00),
    ("error_string", "errorString", RPC_3_00),
    ("percent_done", "percentDone", RPC_3_00),
    (
        "metadata_percent_complete",
        "metadataPercentComplete",
        RPC_3_00,
    ),
    ("rate_download", "rateDownload", RPC_3_00),
    ("rate_upload", "rateUpload", RPC_3_00),
    ("eta", "eta", RPC_3_00),
    ("upload_ratio", "uploadRatio", RPC_3_00),
    ("total_size", "totalSize", RPC_3_00),
    ("size_when_done", "sizeWhenDone", RPC_3_00),
    ("left_until_done", "leftUntilDone", RPC_3_00),
    ("peers_connected", "peersConnected", RPC_3_00),
    ("peers_getting_from_us", "peersGettingFromUs", RPC_3_00),
    ("peers_sending_to_us", "peersSendingToUs", RPC_3_00),
    ("download_dir", "downloadDir", RPC_3_00),
    ("added_date", "addedDate", RPC_3_00),
    ("done_date", "doneDate", RPC_3_00),
    ("queue_position", "queuePosition", RPC_3_00),
    // Transmission 4.1.0-beta serves it under its JSON-RPC name.
    ("sequential_download", "sequential_download", RPC_4_1_BETA),
    ("recheck_progress", "recheckProgress", RPC_3_00),
    ("is_finished", "isFinished", RPC_3_00),
    ("labels", "labels", RPC_3_00),
    ("comment", "comment", RPC_3_00),
    ("creator", "creator", RPC_3_00),
    ("date_created", "dateCreated", RPC_3_00),
    ("magnet_link", "magnetLink", RPC_3_00),
    ("is_private", "isPrivate", RPC_3_00),
    ("piece_count", "pieceCount", RPC_3_00),
    ("piece_size", "pieceSize", RPC_3_00),
    ("pieces", "pieces", RPC_3_00),
    ("files", "files", RPC_3_00),
    ("file_stats", "fileStats", RPC_3_00),
    ("file_count", "file-count", RPC_4_0),
    ("priorities", "priorities", RPC_3_00),
    ("wanted", "wanted", RPC_3_00),
    ("downloaded_ever", "downloadedEver", RPC_3_00),
    ("uploaded_ever", "uploadedEver", RPC_3_00),
    ("tracker_stats", "trackerStats", RPC_3_00),
    ("peers", "peers", RPC_3_00),
    ("peers_from", "peersFrom", RPC_3_00),
];

/// The request arguments and session fields, snake case to legacy.
const ARGUMENTS: &[(&str, &str)] = &[
    ("ids", "ids"),
    ("fields", "fields"),
    ("path", "path"),
    ("filename", "filename"),
    ("metainfo", "metainfo"),
    ("paused", "paused"),
    ("sequential_download", "sequential_download"),
    ("version", "version"),
    ("rpc_version", "rpc-version"),
    ("rpc_version_semver", "rpc-version-semver"),
    ("download_dir", "download-dir"),
    ("alt_speed_enabled", "alt-speed-enabled"),
    ("alt_speed_down", "alt-speed-down"),
    ("alt_speed_up", "alt-speed-up"),
    ("speed_limit_down", "speed-limit-down"),
    ("speed_limit_down_enabled", "speed-limit-down-enabled"),
    ("speed_limit_up", "speed-limit-up"),
    ("speed_limit_up_enabled", "speed-limit-up-enabled"),
    ("start_added_torrents", "start-added-torrents"),
    ("files_wanted", "files-wanted"),
    ("files_unwanted", "files-unwanted"),
    ("priority_high", "priority-high"),
    ("priority_normal", "priority-normal"),
    ("priority_low", "priority-low"),
    ("delete_local_data", "delete-local-data"),
];

/// The keys that only answers carry (in files, trackers, peers, the statistics and the outcome
/// of an add), legacy to snake case.
const ANSWER_ONLY: &[(&str, &str)] = &[
    ("bytesCompleted", "bytes_completed"),
    ("size-bytes", "size_bytes"),
    ("torrent-added", "torrent_added"),
    ("torrent-duplicate", "torrent_duplicate"),
    ("activeTorrentCount", "active_torrent_count"),
    ("pausedTorrentCount", "paused_torrent_count"),
    ("torrentCount", "torrent_count"),
    ("downloadSpeed", "download_speed"),
    ("uploadSpeed", "upload_speed"),
    ("cumulative-stats", "cumulative_stats"),
    ("current-stats", "current_stats"),
    ("downloadedBytes", "downloaded_bytes"),
    ("uploadedBytes", "uploaded_bytes"),
    ("filesAdded", "files_added"),
    ("secondsActive", "seconds_active"),
    ("sessionCount", "session_count"),
    ("isBackup", "is_backup"),
    ("announceState", "announce_state"),
    ("hasAnnounced", "has_announced"),
    ("lastAnnounceTime", "last_announce_time"),
    ("lastAnnounceSucceeded", "last_announce_succeeded"),
    ("lastAnnounceTimedOut", "last_announce_timed_out"),
    ("lastAnnounceResult", "last_announce_result"),
    ("lastAnnouncePeerCount", "last_announce_peer_count"),
    ("nextAnnounceTime", "next_announce_time"),
    ("hasScraped", "has_scraped"),
    ("lastScrapeTime", "last_scrape_time"),
    ("lastScrapeSucceeded", "last_scrape_succeeded"),
    ("lastScrapeTimedOut", "last_scrape_timed_out"),
    ("lastScrapeResult", "last_scrape_result"),
    ("seederCount", "seeder_count"),
    ("leecherCount", "leecher_count"),
    ("downloadCount", "download_count"),
    ("clientName", "client_name"),
    ("flagStr", "flag_str"),
    ("rateToClient", "rate_to_client"),
    ("rateToPeer", "rate_to_peer"),
    ("isEncrypted", "is_encrypted"),
    ("isUTP", "is_utp"),
    ("isIncoming", "is_incoming"),
    ("isDownloadingFrom", "is_downloading_from"),
    ("isUploadingTo", "is_uploading_to"),
    ("clientIsChoked", "client_is_choked"),
    ("clientIsInterested", "client_is_interested"),
    ("peerIsChoked", "peer_is_choked"),
    ("peerIsInterested", "peer_is_interested"),
    ("fromCache", "from_cache"),
    ("fromDht", "from_dht"),
    ("fromIncoming", "from_incoming"),
    ("fromLpd", "from_lpd"),
    ("fromLtep", "from_ltep"),
    ("fromPex", "from_pex"),
    ("fromTracker", "from_tracker"),
];

/// Every legacy name an answer may carry, to its snake case name; built once, as an answer of
/// thousands of torrents renames every key of each.
static SNAKE_NAMES: LazyLock<HashMap<&'static str, &'static str>> = LazyLock::new(|| {
    let torrent_fields = TORRENT_FIELDS
        .iter()
        .map(|&(snake, legacy, _)| (legacy, snake));
    let arguments = ARGUMENTS.iter().map(|&(snake, legacy)| (legacy, snake));
    torrent_fields
        .chain(arguments)
        .chain(ANSWER_ONLY.iter().copied())
        .collect()
});

/// The legacy name of the method `method`: the same words joined by `-` (`torrent_start_now` is
/// `torrent-start-now`).
pub(crate) fn legacy_method(method: &str) -> String {
    method.replace('_', "-")
}

/// The legacy spelling of the snake case name `snake` in `context`; `None` when the legacy
/// protocol has no such name.
pub(crate) fn legacy_name(snake: &str, context: NameContext) -> Option<&'static str> {
    match context {
        NameContext::TorrentField => torrent_field(snake).map(|&(_, legacy, _)| legacy),
        NameContext::Argument => ARGUMENTS
            .iter()
            .find(|&&(name, _)| name == snake)
            .map(|&(_, legacy)| legacy),
    }
}

/// The legacy RPC version that brought the torrent field `snake`; `None` when the legacy
/// protocol has no such field.
pub(crate) fn torrent_field_since(snake: &str) -> Option<u32> {
    torrent_field(snake).map(|&(_, _, since)| since)
}

fn torrent_field(snake: &str) -> Option<&'static (&'static str, &'static str, u32)> {
    TORRENT_FIELDS.iter().find(|&&(name, _, _)| name == snake)
}

/// The snake case name of the legacy name `legacy` in an answer; `None` for a name 4x4 does not
/// know, which then stays as it is.
pub(crate) fn snake_name(legacy: &str) -> Option<&'static str> {
    SNAKE_NAMES.get(legacy).copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::SESSION_FIELDS;
    use crate::session_methods::LEGACY_VERSION_FIELDS;
    use crate::torrent::TorrentField;

    /// Every [`TorrentField`]. The match has no wildcard, so a new variant does not compile until
    /// it is added to the match, and with it to the list.
    fn every_torrent_field() -> Vec<TorrentField> {
        use TorrentField as F;
        let every = vec![
            F::Id,
            F::HashString,
            F::Name,
            F::Status,
            F::Error,
            F::ErrorString,
            F::PercentDone,
            F::MetadataPercentComplete,
            F::RateDownload,
            F::RateUpload,
            F::Eta,
            F::UploadRatio,
            F::TotalSize,
            F::SizeWhenDone,
            F::LeftUntilDone,
            F::PeersConnected,
            F::PeersGettingFromUs,
            F::PeersSendingToUs,
            F::DownloadDir,
            F::AddedDate,
            F::DoneDate,
            F::QueuePosition,
            F::SequentialDownload,
            F::RecheckProgress,
            F::IsFinished,
            F::Labels,
            F::Comment,
            F::Creator,
            F::DateCreated,
            F::Source,
            F::MagnetLink,
            F::IsPrivate,
            F::PieceCount,
            F::PieceSize,
            F::Pieces,
            F::Files,
            F::FileStats,
            F::FileCount,
            F::BytesCompleted,
            F::Priorities,
            F::Wanted,
            F::DownloadedEver,
            F::UploadedEver,
            F::TrackerStats,
            F::Peers,
            F::PeersFrom,
        ];
        for field in &every {
            match field {
                F::Id
                | F::HashString
                | F::Name
                | F::Status
                | F::Error
                | F::ErrorString
                | F::PercentDone
                | F::MetadataPercentComplete
                | F::RateDownload
                | F::RateUpload
                | F::Eta
                | F::UploadRatio
                | F::TotalSize
                | F::SizeWhenDone
                | F::LeftUntilDone
                | F::PeersConnected
                | F::PeersGettingFromUs
                | F::PeersSendingToUs
                | F::DownloadDir
                | F::AddedDate
                | F::DoneDate
                | F::QueuePosition
                | F::SequentialDownload
                | F::RecheckProgress
                | F::IsFinished
                | F::Labels
                | F::Comment
                | F::Creator
                | F::DateCreated
                | F::Source
                | F::MagnetLink
                | F::IsPrivate
                | F::PieceCount
                | F::PieceSize
                | F::Pieces
                | F::Files
                | F::FileStats
                | F::FileCount
                | F::BytesCompleted
                | F::Priorities
                | F::Wanted
                | F::DownloadedEver
                | F::UploadedEver
                | F::TrackerStats
                | F::Peers
                | F::PeersFrom => {}
            }
        }
        every
    }

    fn wire_name(field: TorrentField) -> String {
        serde_json::to_value(field)
            .ok()
            .and_then(|name| name.as_str().map(str::to_owned))
            .unwrap_or_default()
    }

    #[test]
    fn every_torrent_field_has_a_legacy_name_or_is_filled() {
        // Task 4 fills these in from what the legacy daemons do send.
        let filled = [TorrentField::Source, TorrentField::BytesCompleted];

        for field in every_torrent_field() {
            let name = wire_name(field);
            let legacy = legacy_name(&name, NameContext::TorrentField);
            if filled.contains(&field) {
                assert_eq!(legacy, None, "{name}");
            } else {
                assert!(legacy.is_some(), "{name} has no legacy name");
            }
        }
    }

    #[test]
    fn every_session_field_has_a_legacy_name() {
        for &name in SESSION_FIELDS.iter().chain(LEGACY_VERSION_FIELDS) {
            assert!(
                legacy_name(name, NameContext::Argument).is_some(),
                "{name} has no legacy name"
            );
        }
    }

    #[test]
    fn download_dir_is_camel_in_torrent_fields_and_kebab_in_arguments() {
        assert_eq!(
            legacy_name("download_dir", NameContext::TorrentField),
            Some("downloadDir")
        );
        assert_eq!(
            legacy_name("download_dir", NameContext::Argument),
            Some("download-dir")
        );
    }

    #[test]
    fn snake_name_reverses_every_pair() {
        for &(snake, legacy, _) in TORRENT_FIELDS {
            assert_eq!(snake_name(legacy), Some(snake), "{legacy}");
        }
        for &(snake, legacy) in ARGUMENTS {
            assert_eq!(snake_name(legacy), Some(snake), "{legacy}");
        }
        for &(legacy, snake) in ANSWER_ONLY {
            assert_eq!(snake_name(legacy), Some(snake), "{legacy}");
        }
    }

    #[test]
    fn legacy_method_kebabs_the_name() {
        assert_eq!(legacy_method("torrent_start_now"), "torrent-start-now");
        assert_eq!(legacy_method("free_space"), "free-space");
        assert_eq!(legacy_method("session_get"), "session-get");
    }

    #[test]
    fn answer_only_names_map_back() {
        assert_eq!(snake_name("isUTP"), Some("is_utp"));
        assert_eq!(snake_name("size-bytes"), Some("size_bytes"));
        assert_eq!(snake_name("torrent-duplicate"), Some("torrent_duplicate"));
        assert_eq!(snake_name("cumulative-stats"), Some("cumulative_stats"));
    }

    #[test]
    fn the_fields_newer_than_transmission_3_00_carry_the_version_that_brought_them() {
        assert_eq!(torrent_field_since("file_count"), Some(17));
        assert_eq!(torrent_field_since("sequential_download"), Some(18));
        assert_eq!(torrent_field_since("hash_string"), Some(16));
        assert_eq!(torrent_field_since("source"), None);
        let newer: Vec<&str> = TORRENT_FIELDS
            .iter()
            .filter(|&&(_, _, since)| since > RPC_3_00)
            .map(|&(snake, _, _)| snake)
            .collect();
        assert_eq!(newer, ["sequential_download", "file_count"]);
    }

    #[test]
    fn an_unknown_name_has_no_counterpart() {
        assert_eq!(snake_name("begin_piece"), None);
        assert_eq!(legacy_name("source", NameContext::TorrentField), None);
        assert_eq!(legacy_name("hash_string", NameContext::Argument), None);
    }
}
