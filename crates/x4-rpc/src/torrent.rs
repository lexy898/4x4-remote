//! Torrent wire types: the `torrent_get` fields and records (rpc-spec §3.3), and the inputs and
//! outcomes of the torrent methods on [`Client`](crate::Client) (§3.1, §3.2, §3.4, §3.5).
//!
//! Records keep the server's raw values (bytes, bytes/s, seconds, Unix times, the `-1`/`-2`
//! sentinels of `eta` and `upload_ratio`); turning them into the app's model is `x4-core`'s job.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD_PAD_INDIFFERENT;
use serde::de::{Error as _, Unexpected};
use serde::{Deserialize, Deserializer, Serialize};

use crate::error::RpcError;
use crate::ids::{HashString, TorrentId};

/// A key of the `fields` array of `torrent_get`; serializes as its wire name (`hash_string`, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TorrentField {
    /// `id`
    Id,
    /// `hash_string`
    HashString,
    /// `name`
    Name,
    /// `status`
    Status,
    /// `error`
    Error,
    /// `error_string`
    ErrorString,
    /// `percent_done`
    PercentDone,
    /// `metadata_percent_complete`
    MetadataPercentComplete,
    /// `rate_download`
    RateDownload,
    /// `rate_upload`
    RateUpload,
    /// `eta`
    Eta,
    /// `upload_ratio`
    UploadRatio,
    /// `total_size`
    TotalSize,
    /// `size_when_done`
    SizeWhenDone,
    /// `left_until_done`
    LeftUntilDone,
    /// `peers_connected`
    PeersConnected,
    /// `peers_getting_from_us`
    PeersGettingFromUs,
    /// `peers_sending_to_us`
    PeersSendingToUs,
    /// `download_dir`
    DownloadDir,
    /// `added_date`
    AddedDate,
    /// `done_date`
    DoneDate,
    /// `queue_position`
    QueuePosition,
    /// `sequential_download`
    SequentialDownload,
    /// `recheck_progress`
    RecheckProgress,
    /// `is_finished`
    IsFinished,
    /// `labels`
    Labels,
    /// `comment`
    Comment,
    /// `creator`
    Creator,
    /// `date_created`
    DateCreated,
    /// `magnet_link`
    MagnetLink,
    /// `is_private`
    IsPrivate,
    /// `piece_count`
    PieceCount,
    /// `piece_size`
    PieceSize,
    /// `pieces`
    Pieces,
    /// `files`
    Files,
    /// `file_stats`
    FileStats,
    /// `downloaded_ever`
    DownloadedEver,
    /// `uploaded_ever`
    UploadedEver,
}

impl TorrentField {
    /// The fields of the torrent list.
    pub const LIST: &'static [TorrentField] = &[
        Self::Id,
        Self::HashString,
        Self::Name,
        Self::Status,
        Self::Error,
        Self::ErrorString,
        Self::PercentDone,
        Self::MetadataPercentComplete,
        Self::RateDownload,
        Self::RateUpload,
        Self::Eta,
        Self::UploadRatio,
        Self::TotalSize,
        Self::SizeWhenDone,
        Self::LeftUntilDone,
        Self::PeersConnected,
        Self::PeersGettingFromUs,
        Self::PeersSendingToUs,
        Self::DownloadDir,
        Self::AddedDate,
        Self::DoneDate,
        Self::QueuePosition,
        Self::SequentialDownload,
        Self::RecheckProgress,
        Self::IsFinished,
        Self::Labels,
    ];

    /// The fields of the details of one torrent.
    pub const DETAILS: &'static [TorrentField] = &[
        Self::Id,
        Self::HashString,
        Self::Comment,
        Self::Creator,
        Self::DateCreated,
        Self::MagnetLink,
        Self::IsPrivate,
        Self::PieceCount,
        Self::PieceSize,
        Self::Pieces,
        Self::Files,
        Self::FileStats,
        Self::DownloadedEver,
        Self::UploadedEver,
        Self::TotalSize,
    ];
}

/// A torrent's activity, the wire `status` (0 to 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(from = "u8")]
pub enum Status {
    /// 0: stopped.
    Stopped,
    /// 1: queued to verify local data.
    CheckWait,
    /// 2: verifying local data.
    Checking,
    /// 3: queued to download.
    DownloadWait,
    /// 4: downloading.
    Downloading,
    /// 5: queued to seed.
    SeedWait,
    /// 6: seeding.
    Seeding,
    /// A value this version does not know, kept as sent.
    Other(u8),
}

impl From<u8> for Status {
    fn from(code: u8) -> Self {
        match code {
            0 => Self::Stopped,
            1 => Self::CheckWait,
            2 => Self::Checking,
            3 => Self::DownloadWait,
            4 => Self::Downloading,
            5 => Self::SeedWait,
            6 => Self::Seeding,
            other => Self::Other(other),
        }
    }
}

/// What kind of error a torrent has, the wire `error` (0 to 3); the text is in `error_string`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(from = "u8")]
pub enum ErrorKind {
    /// 0: no error.
    None,
    /// 1: the tracker answered with a warning.
    TrackerWarning,
    /// 2: the tracker answered with an error.
    TrackerError,
    /// 3: a local error, such as missing data or a full disk; the torrent stops.
    LocalError,
    /// A value this version does not know, kept as sent.
    Other(u8),
}

impl From<u8> for ErrorKind {
    fn from(code: u8) -> Self {
        match code {
            0 => Self::None,
            1 => Self::TrackerWarning,
            2 => Self::TrackerError,
            3 => Self::LocalError,
            other => Self::Other(other),
        }
    }
}

/// A file's download priority, the wire `priority` of `file_stats` (-1, 0, 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FilePriority {
    /// -1
    Low,
    /// 0
    Normal,
    /// 1
    High,
}

impl<'de> Deserialize<'de> for FilePriority {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match i64::deserialize(deserializer)? {
            -1 => Ok(Self::Low),
            0 => Ok(Self::Normal),
            1 => Ok(Self::High),
            other => Err(D::Error::invalid_value(
                Unexpected::Signed(other),
                &"a file priority of -1, 0 or 1",
            )),
        }
    }
}

/// The pieces a torrent has: a bitfield of `piece_count` bits, the first piece in the most
/// significant bit of the first byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pieces(Vec<u8>);

impl Pieces {
    /// Decodes the base64 `pieces` field of `torrent_get`.
    ///
    /// # Errors
    ///
    /// [`RpcError::InvalidResponse`] when `s` is not standard base64.
    pub fn decode_base64(s: &str) -> Result<Self, RpcError> {
        STANDARD_PAD_INDIFFERENT
            .decode(s)
            .map(Self)
            .map_err(|error| RpcError::InvalidResponse(format!("pieces: not base64: {error}")))
    }

    /// A bitfield of already decoded bytes, most significant bit first.
    ///
    /// For building bitfields directly, for example in tests; server data goes through
    /// [`Pieces::decode_base64`].
    #[must_use]
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Whether piece `index` is present; pieces past the end of the bitfield are absent.
    #[must_use]
    pub fn has(&self, index: u32) -> bool {
        usize::try_from(index / 8)
            .ok()
            .and_then(|byte| self.0.get(byte))
            .is_some_and(|byte| byte & (0x80_u8 >> (index % 8)) != 0)
    }
}

/// One entry of `files`: a file's fixed properties, in the order of the torrent.
///
/// The position in `files` is the file index that `torrent_set` and `torrent_add` take.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FileRecord {
    /// The path inside the torrent, `/`-separated, starting with the torrent's directory for
    /// multi-file torrents.
    pub name: String,
    /// The size in bytes.
    pub length: u64,
    /// The bytes downloaded and verified.
    pub bytes_completed: u64,
    /// The first piece holding data of this file.
    pub begin_piece: u32,
    /// One past the last piece holding data of this file (exclusive); the last piece of one
    /// file is also the first of the next when a file ends mid-piece.
    pub end_piece: u32,
}

/// One entry of `file_stats`: a file's changing properties, in the same order as `files`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FileStatRecord {
    /// The bytes downloaded and verified.
    pub bytes_completed: u64,
    /// Whether the file is to be downloaded.
    pub wanted: bool,
    /// The download priority.
    pub priority: FilePriority,
}

/// One torrent of a `torrent_get` response; a field is `Some` only if it was requested (and
/// the server sent it).
///
/// Values are as on the wire: sizes in bytes, rates in bytes/s, dates as Unix seconds (`0` when
/// unset), `eta` in seconds or `-1` (not available) / `-2` (unknown), `upload_ratio` as a ratio
/// or `-1` (not available) / `-2` (infinite).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct TorrentRecord {
    /// `id`: valid only until the daemon restarts.
    pub id: Option<TorrentId>,
    /// `hash_string`: the stable identity.
    pub hash_string: Option<HashString>,
    /// `name`
    pub name: Option<String>,
    /// `status`
    pub status: Option<Status>,
    /// `error`
    pub error: Option<ErrorKind>,
    /// `error_string`: empty when there is no error.
    pub error_string: Option<String>,
    /// `percent_done`: progress of the wanted files, 0.0 to 1.0.
    pub percent_done: Option<f64>,
    /// `metadata_percent_complete`: below 1.0 while a magnet link fetches its metadata.
    pub metadata_percent_complete: Option<f64>,
    /// `rate_download`, bytes/s.
    pub rate_download: Option<u64>,
    /// `rate_upload`, bytes/s.
    pub rate_upload: Option<u64>,
    /// `eta`, seconds; `-1` not available, `-2` unknown.
    pub eta: Option<i64>,
    /// `upload_ratio`; `-1` not available, `-2` infinite.
    pub upload_ratio: Option<f64>,
    /// `total_size`, bytes of all files.
    pub total_size: Option<u64>,
    /// `size_when_done`, bytes of the wanted files.
    pub size_when_done: Option<u64>,
    /// `left_until_done`, bytes of the wanted files still missing.
    pub left_until_done: Option<u64>,
    /// `peers_connected`
    pub peers_connected: Option<u32>,
    /// `peers_getting_from_us`
    pub peers_getting_from_us: Option<u32>,
    /// `peers_sending_to_us`
    pub peers_sending_to_us: Option<u32>,
    /// `download_dir`, a path on the server.
    pub download_dir: Option<String>,
    /// `added_date`, Unix seconds.
    pub added_date: Option<i64>,
    /// `done_date`, Unix seconds; `0` while not done.
    pub done_date: Option<i64>,
    /// `queue_position`
    pub queue_position: Option<i64>,
    /// `sequential_download`
    pub sequential_download: Option<bool>,
    /// `recheck_progress`, 0.0 to 1.0.
    pub recheck_progress: Option<f64>,
    /// `is_finished`: seeding reached its ratio or idle limit.
    pub is_finished: Option<bool>,
    /// `labels`
    pub labels: Option<Vec<String>>,
    /// `comment`
    pub comment: Option<String>,
    /// `creator`
    pub creator: Option<String>,
    /// `date_created`, Unix seconds; `0` when the metainfo has none.
    pub date_created: Option<i64>,
    /// `magnet_link`
    pub magnet_link: Option<String>,
    /// `is_private`
    pub is_private: Option<bool>,
    /// `piece_count`
    pub piece_count: Option<u32>,
    /// `piece_size`, bytes.
    pub piece_size: Option<u64>,
    /// `pieces`, base64; decode with [`Pieces::decode_base64`].
    pub pieces: Option<String>,
    /// `files`
    pub files: Option<Vec<FileRecord>>,
    /// `file_stats`, in the same order as `files`.
    pub file_stats: Option<Vec<FileStatRecord>>,
    /// `downloaded_ever`, bytes.
    pub downloaded_ever: Option<u64>,
    /// `uploaded_ever`, bytes.
    pub uploaded_ever: Option<u64>,
}

/// The result of `torrent_get`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TorrentGetResponse {
    /// The requested torrents.
    pub torrents: Vec<TorrentRecord>,
    /// Ids of torrents removed recently; sent only for [`GetIds::RecentlyActive`], else empty.
    ///
    /// [`GetIds::RecentlyActive`]: crate::GetIds::RecentlyActive
    #[serde(default)]
    pub removed: Vec<TorrentId>,
}

/// An action on torrents (rpc-spec §3.1); each is its own method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TorrentAction {
    /// `torrent_start`: start, or queue if the download queue is full.
    Start,
    /// `torrent_start_now`: start, ignoring the queue.
    StartNow,
    /// `torrent_stop`
    Stop,
    /// `torrent_verify`: check the local data against the piece hashes.
    Verify,
    /// `torrent_reannounce`: announce to the trackers now.
    Reannounce,
}

impl TorrentAction {
    /// The RPC method of this action.
    pub(crate) fn method(self) -> &'static str {
        match self {
            Self::Start => "torrent_start",
            Self::StartNow => "torrent_start_now",
            Self::Stop => "torrent_stop",
            Self::Verify => "torrent_verify",
            Self::Reannounce => "torrent_reannounce",
        }
    }
}

/// What `torrent_remove` does with the downloaded data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RemoveData {
    /// Keep the files on the server's disk.
    Keep,
    /// Delete the files from the server's disk too.
    Delete,
}

/// Changes for `torrent_set`; `None` and empty lists are left out of the request.
///
/// An empty file-index list would mean *all files* on the wire, so it is never sent: to address
/// every file, list every index (`0..file_count`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TorrentSet {
    /// Download pieces in order.
    pub sequential_download: Option<bool>,
    /// Indices of files to download.
    pub files_wanted: Vec<usize>,
    /// Indices of files not to download.
    pub files_unwanted: Vec<usize>,
    /// Indices of files to give high priority.
    pub priority_high: Vec<usize>,
    /// Indices of files to give normal priority.
    pub priority_normal: Vec<usize>,
    /// Indices of files to give low priority.
    pub priority_low: Vec<usize>,
}

/// Where a torrent to add comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddSource {
    /// A magnet link (or a URL of a `.torrent` the server fetches), sent as `filename`.
    Magnet(String),
    /// The content of a `.torrent` file, sent base64-encoded as `metainfo`.
    Metainfo(Vec<u8>),
}

/// A `torrent_add` request; `None` and empty lists are left out of the request.
///
/// An empty file-index list would mean *all files* on the wire, so it is never sent: to address
/// every file, list every index (`0..file_count`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TorrentAdd {
    /// The magnet link or metainfo.
    pub source: AddSource,
    /// Where on the server to download to; `None` uses the session's default directory.
    pub download_dir: Option<String>,
    /// Add without starting.
    pub paused: bool,
    /// Indices of files not to download.
    pub files_unwanted: Vec<usize>,
    /// Indices of files to give high priority.
    pub priority_high: Vec<usize>,
    /// Indices of files to give low priority.
    pub priority_low: Vec<usize>,
    /// Download pieces in order; `None` uses the session default.
    pub sequential_download: Option<bool>,
}

/// The torrent that `torrent_add` added or found already present.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AddedTorrent {
    /// The torrent's id.
    pub id: TorrentId,
    /// The torrent's name.
    pub name: String,
    /// The torrent's info hash.
    #[serde(rename = "hash_string")]
    pub hash: HashString,
}

/// What `torrent_add` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddOutcome {
    /// The torrent was added (`torrent_added`).
    Added(AddedTorrent),
    /// The server already had this torrent and changed nothing (`torrent_duplicate`).
    Duplicate(AddedTorrent),
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn field_lists_use_the_wire_names() {
        assert_eq!(
            serde_json::to_value(TorrentField::LIST).unwrap(),
            json!([
                "id",
                "hash_string",
                "name",
                "status",
                "error",
                "error_string",
                "percent_done",
                "metadata_percent_complete",
                "rate_download",
                "rate_upload",
                "eta",
                "upload_ratio",
                "total_size",
                "size_when_done",
                "left_until_done",
                "peers_connected",
                "peers_getting_from_us",
                "peers_sending_to_us",
                "download_dir",
                "added_date",
                "done_date",
                "queue_position",
                "sequential_download",
                "recheck_progress",
                "is_finished",
                "labels",
            ])
        );
        assert_eq!(
            serde_json::to_value(TorrentField::DETAILS).unwrap(),
            json!([
                "id",
                "hash_string",
                "comment",
                "creator",
                "date_created",
                "magnet_link",
                "is_private",
                "piece_count",
                "piece_size",
                "pieces",
                "files",
                "file_stats",
                "downloaded_ever",
                "uploaded_ever",
                "total_size",
            ])
        );
    }

    #[test]
    fn every_status_code_maps() {
        let statuses: Vec<Status> = (0..=7).map(Status::from).collect();

        assert_eq!(
            statuses,
            [
                Status::Stopped,
                Status::CheckWait,
                Status::Checking,
                Status::DownloadWait,
                Status::Downloading,
                Status::SeedWait,
                Status::Seeding,
                Status::Other(7),
            ]
        );
    }

    #[test]
    fn every_error_code_maps() {
        let kinds: Vec<ErrorKind> = (0..=4).map(ErrorKind::from).collect();

        assert_eq!(
            kinds,
            [
                ErrorKind::None,
                ErrorKind::TrackerWarning,
                ErrorKind::TrackerError,
                ErrorKind::LocalError,
                ErrorKind::Other(4),
            ]
        );
    }

    #[test]
    fn file_priorities_parse_and_reject_unknown_values() {
        let parse = |value| serde_json::from_value::<FilePriority>(value);

        assert_eq!(parse(json!(-1)).unwrap(), FilePriority::Low);
        assert_eq!(parse(json!(0)).unwrap(), FilePriority::Normal);
        assert_eq!(parse(json!(1)).unwrap(), FilePriority::High);
        assert!(parse(json!(2)).is_err());
    }

    #[test]
    fn wanted_must_be_a_boolean() {
        let stat = |wanted| {
            serde_json::from_value::<FileStatRecord>(
                json!({ "bytes_completed": 0, "wanted": wanted, "priority": 0 }),
            )
        };

        assert!(stat(json!(true)).unwrap().wanted);
        assert!(stat(json!(1)).is_err());
    }

    #[test]
    fn missing_fields_are_none() {
        let record: TorrentRecord = serde_json::from_value(json!({ "id": 4 })).unwrap();

        assert_eq!(
            record,
            TorrentRecord {
                id: Some(TorrentId(4)),
                ..TorrentRecord::default()
            }
        );
    }

    #[test]
    fn pieces_read_across_bytes() {
        let pieces = Pieces::from_bytes(vec![0b0000_0001, 0b0100_0000]);

        let present: Vec<u32> = (0..16).filter(|&index| pieces.has(index)).collect();

        assert_eq!(present, [7, 9]);
        assert!(!pieces.has(u32::MAX));
    }

    #[test]
    fn empty_pieces_have_nothing() {
        let pieces = Pieces::decode_base64("").unwrap();

        assert!(!pieces.has(0));
    }

    #[test]
    fn pieces_accept_unpadded_base64_and_reject_garbage() {
        assert_eq!(
            Pieces::decode_base64("gA").unwrap(),
            Pieces::from_bytes(vec![0x80])
        );
        assert!(matches!(
            Pieces::decode_base64("not base64!"),
            Err(RpcError::InvalidResponse(_))
        ));
    }
}
