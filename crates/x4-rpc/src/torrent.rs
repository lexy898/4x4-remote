//! Torrent wire types: the `torrent_get` fields and records (rpc-spec §3.3), and the inputs and
//! outcomes of the torrent methods on [`Client`](crate::Client) (§3.1, §3.2, §3.4, §3.5).
//!
//! Records keep the server's raw values (bytes, bytes/s, seconds, Unix times, the `-1`/`-2`
//! sentinels of `eta` and `upload_ratio`); turning them into the app's model is `x4-core`'s job.

use std::fmt;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD_PAD_INDIFFERENT;
use serde::de::{Error as _, Unexpected};
use serde::{Deserialize, Deserializer, Serialize};

use crate::download_dir::DownloadDir;
use crate::error::RpcError;
use crate::ids::{HashString, TorrentId};
use crate::lenient::{lenient, lenient_list};
use crate::magnet::MagnetLink;
use crate::peers::{PeerRecord, PeersFromRecord};

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
    /// `source`: the private tracker's tag in the metainfo, served since Transmission 4.1.0.
    Source,
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
    /// `file_count`
    FileCount,
    /// `bytes_completed`: one number per file.
    BytesCompleted,
    /// `priorities`: one priority per file.
    Priorities,
    /// `wanted`: one switch per file.
    Wanted,
    /// `downloaded_ever`
    DownloadedEver,
    /// `uploaded_ever`
    UploadedEver,
    /// `tracker_stats`
    TrackerStats,
    /// `peers`
    Peers,
    /// `peers_from`
    PeersFrom,
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

    /// The fields of the whole details of one torrent: its metainfo, its files, its trackers and
    /// everything of [`TorrentField::DETAILS_PROGRESS`].
    ///
    /// `files` carries each file's bytes completed, and the compact `priorities` and `wanted`
    /// arrays stand in for `file_stats`, which costs several times as much per file.
    pub const DETAILS_FULL: &'static [TorrentField] = &[
        Self::Id,
        Self::HashString,
        Self::Comment,
        Self::Creator,
        Self::DateCreated,
        Self::Source,
        Self::IsPrivate,
        Self::PieceCount,
        Self::PieceSize,
        Self::FileCount,
        Self::Files,
        Self::Priorities,
        Self::Wanted,
        Self::Pieces,
        Self::DownloadedEver,
        Self::UploadedEver,
        Self::TrackerStats,
    ];

    /// The fields of the details of one torrent that change while it downloads: the per-file
    /// arrays, the pieces and the totals. The file names, the metainfo and the trackers are left
    /// out (the trackers come with [`TorrentField::DETAILS_PROGRESS_TRACKERS`]); `file_count`
    /// tells when the files themselves changed.
    pub const DETAILS_PROGRESS: &'static [TorrentField] = &[
        Self::Id,
        Self::HashString,
        Self::FileCount,
        Self::BytesCompleted,
        Self::Priorities,
        Self::Wanted,
        Self::Pieces,
        Self::DownloadedEver,
        Self::UploadedEver,
    ];

    /// [`TorrentField::DETAILS_PROGRESS`] and the trackers' state.
    ///
    /// The trackers change on a minute's scale (an announce every half hour or so, a scrape, a
    /// seeder count), not by the second, so only the progress read that goes out with the whole
    /// list asks for them; the one every few seconds does not.
    pub const DETAILS_PROGRESS_TRACKERS: &'static [TorrentField] = &[
        Self::Id,
        Self::HashString,
        Self::FileCount,
        Self::BytesCompleted,
        Self::Priorities,
        Self::Wanted,
        Self::Pieces,
        Self::DownloadedEver,
        Self::UploadedEver,
        Self::TrackerStats,
    ];

    /// The fields of the Peers tab: the connected peers and where they came from.
    ///
    /// `peers_connected` is left out: the number of peers in the same answer is that number.
    pub const PEERS: &'static [TorrentField] =
        &[Self::Id, Self::HashString, Self::Peers, Self::PeersFrom];

    /// The fields of the Trackers tab: every tracker's state.
    pub const TRACKERS: &'static [TorrentField] = &[Self::Id, Self::HashString, Self::TrackerStats];
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

/// The daemon's id of a tracker: unique within the daemon, so a row of the Trackers tab can be
/// told from another across reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(transparent)]
pub struct TrackerId(pub u32);

/// What a tracker's announce is doing, the wire `announce_state` (0 to 3).
///
/// It is the state of the tracker's *tier*, shown on the tier's current tracker; a backup
/// tracker is always [`TrackerState::Inactive`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize)]
#[serde(from = "u8")]
pub enum TrackerState {
    /// 0: nothing will be announced, because the torrent is stopped, has an error, or the
    /// tracker is a backup.
    #[default]
    Inactive,
    /// 1: waiting for the time of the next announce.
    Waiting,
    /// 2: waiting for a free announce slot.
    Queued,
    /// 3: announcing now.
    Active,
    /// A value this version does not know, kept as sent.
    Other(u8),
}

impl From<u8> for TrackerState {
    fn from(code: u8) -> Self {
        match code {
            0 => Self::Inactive,
            1 => Self::Waiting,
            2 => Self::Queued,
            3 => Self::Active,
            other => Self::Other(other),
        }
    }
}

/// One entry of `tracker_stats`: what one tracker last answered. Only the keys 4x4 uses are
/// kept; the daemon sends 28.
///
/// Not kept: `announce` and `scrape`, whose URLs may carry a private tracker's passkey;
/// `scrape_state`, which stays "queued" for a tracker that has no scrape URL; the `*_start_time`
/// keys, one of which is `0` while the answer's time is set; and `downloader_count`.
///
/// A key the daemon leaves out, or sends as another type, holds its default (empty text, `false`,
/// `0`, `None`), so that an odd tracker entry cannot fail the whole `torrent_get`. Times are Unix
/// seconds, `0` for never or not applicable.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "a record of the daemon's flags for one tracker, as the wire has them"
)]
pub struct TrackerStatRecord {
    /// `id`
    #[serde(deserialize_with = "lenient")]
    pub id: TrackerId,
    /// `tier`: 0-based; the trackers of a tier back each other up.
    #[serde(deserialize_with = "lenient")]
    pub tier: u32,
    /// `is_backup`: another tracker of the tier is the one in use. A backup carries no state:
    /// its times and states are `0`.
    #[serde(deserialize_with = "lenient")]
    pub is_backup: bool,
    /// `host`: the tracker's host and port, such as `tracker.example.org:443`.
    #[serde(deserialize_with = "lenient")]
    pub host: String,
    /// `sitename`: the first label of the host before its public suffix, such as `example`.
    #[serde(deserialize_with = "lenient")]
    pub sitename: String,
    /// `announce_state`
    #[serde(deserialize_with = "lenient")]
    pub announce_state: TrackerState,
    /// `has_announced`: whether the torrent was announced to this tracker since it started; the
    /// other announce keys hold nothing until then.
    #[serde(deserialize_with = "lenient")]
    pub has_announced: bool,
    /// `last_announce_time`: when the last answer came; `0` if none did.
    #[serde(deserialize_with = "lenient")]
    pub last_announce_time: i64,
    /// `last_announce_succeeded`
    #[serde(deserialize_with = "lenient")]
    pub last_announce_succeeded: bool,
    /// `last_announce_timed_out`
    #[serde(deserialize_with = "lenient")]
    pub last_announce_timed_out: bool,
    /// `last_announce_result`: the daemon's text, such as `Success` or the tracker's own error.
    #[serde(deserialize_with = "lenient")]
    pub last_announce_result: String,
    /// `last_announce_peer_count`: how many peers the last answer held.
    #[serde(deserialize_with = "lenient")]
    pub last_announce_peer_count: i64,
    /// `next_announce_time`: meaningful only while `announce_state` is
    /// [`TrackerState::Waiting`]; `0` otherwise.
    #[serde(deserialize_with = "lenient")]
    pub next_announce_time: i64,
    /// `has_scraped`: whether this tracker was scraped since the torrent started; the other
    /// scrape keys hold nothing until then.
    #[serde(deserialize_with = "lenient")]
    pub has_scraped: bool,
    /// `last_scrape_time`: when the last scrape answer came; `0` if none did.
    #[serde(deserialize_with = "lenient")]
    pub last_scrape_time: i64,
    /// `last_scrape_succeeded`
    #[serde(deserialize_with = "lenient")]
    pub last_scrape_succeeded: bool,
    /// `last_scrape_timed_out`
    #[serde(deserialize_with = "lenient")]
    pub last_scrape_timed_out: bool,
    /// `last_scrape_result`
    #[serde(deserialize_with = "lenient")]
    pub last_scrape_result: String,
    /// `seeder_count`; `-1` when the tracker did not say.
    #[serde(deserialize_with = "lenient")]
    pub seeder_count: Option<i64>,
    /// `leecher_count`; `-1` when the tracker did not say.
    #[serde(deserialize_with = "lenient")]
    pub leecher_count: Option<i64>,
    /// `download_count`: how many times the tracker saw the torrent completed; `-1` when the
    /// tracker did not say.
    #[serde(deserialize_with = "lenient")]
    pub download_count: Option<i64>,
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
    /// `hash_string`: the stable identity, as the server sent it.
    ///
    /// Left unvalidated so that one malformed hash cannot fail a whole `torrent_get`; check it
    /// with [`HashString::parse`] before use.
    pub hash_string: Option<String>,
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
    /// `source`: the metainfo's source tag; empty when it has none.
    pub source: Option<String>,
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
    /// `file_count`: how many files the torrent has; 0 while a magnet link has no metadata.
    pub file_count: Option<u32>,
    /// `bytes_completed`: each file's bytes downloaded and verified, in the order of `files`.
    pub bytes_completed: Option<Vec<u64>>,
    /// `priorities`: each file's download priority, in the order of `files`.
    pub priorities: Option<Vec<FilePriority>>,
    /// `wanted`: whether each file is to be downloaded, in the order of `files`.
    pub wanted: Option<Vec<bool>>,
    /// `downloaded_ever`, bytes.
    pub downloaded_ever: Option<u64>,
    /// `uploaded_ever`, bytes.
    pub uploaded_ever: Option<u64>,
    /// `tracker_stats`: one entry per tracker.
    ///
    /// Read leniently: a value that is not a list is `None`, and an item that is not an object
    /// is an empty record.
    #[serde(deserialize_with = "lenient_list")]
    pub tracker_stats: Option<Vec<TrackerStatRecord>>,
    /// `peers`: the connected peers, in no stable order.
    ///
    /// Read leniently, like `tracker_stats`.
    #[serde(deserialize_with = "lenient_list")]
    pub peers: Option<Vec<PeerRecord>>,
    /// `peers_from`: where the connected peers were first found.
    #[serde(deserialize_with = "lenient")]
    pub peers_from: Option<PeersFromRecord>,
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
///
/// The daemon reads `filename` as a URL to fetch or a path on its own disk too, so only a
/// validated [`MagnetLink`] is ever sent there; the content of a `.torrent` file always goes as
/// `metainfo`.
///
/// Its `Debug` shows a link as [`MagnetLink`] does, and the content only by its length.
#[derive(Clone, PartialEq, Eq)]
pub enum AddSource {
    /// A magnet link, sent as `filename`.
    Magnet(MagnetLink),
    /// The content of a `.torrent` file, sent base64-encoded as `metainfo`. It is shared, not
    /// copied, with whoever read the file.
    Metainfo(Arc<[u8]>),
}

impl fmt::Debug for AddSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Magnet(link) => f.debug_tuple("Magnet").field(link).finish(),
            Self::Metainfo(content) => f
                .debug_struct("Metainfo")
                .field("len", &content.len())
                .finish(),
        }
    }
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
    pub download_dir: Option<DownloadDir>,
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
            serde_json::to_value(TorrentField::DETAILS_FULL).unwrap(),
            json!([
                "id",
                "hash_string",
                "comment",
                "creator",
                "date_created",
                "source",
                "is_private",
                "piece_count",
                "piece_size",
                "file_count",
                "files",
                "priorities",
                "wanted",
                "pieces",
                "downloaded_ever",
                "uploaded_ever",
                "tracker_stats",
            ])
        );
        assert_eq!(
            serde_json::to_value(TorrentField::DETAILS_PROGRESS).unwrap(),
            json!([
                "id",
                "hash_string",
                "file_count",
                "bytes_completed",
                "priorities",
                "wanted",
                "pieces",
                "downloaded_ever",
                "uploaded_ever",
            ])
        );
        assert_eq!(
            serde_json::to_value(TorrentField::DETAILS_PROGRESS_TRACKERS).unwrap(),
            json!([
                "id",
                "hash_string",
                "file_count",
                "bytes_completed",
                "priorities",
                "wanted",
                "pieces",
                "downloaded_ever",
                "uploaded_ever",
                "tracker_stats",
            ])
        );
        assert_eq!(
            serde_json::to_value(TorrentField::PEERS).unwrap(),
            json!(["id", "hash_string", "peers", "peers_from"])
        );
        assert_eq!(
            serde_json::to_value(TorrentField::TRACKERS).unwrap(),
            json!(["id", "hash_string", "tracker_stats"])
        );
    }

    #[test]
    fn the_progress_read_with_trackers_is_the_progress_read_and_the_trackers() {
        let expected: Vec<TorrentField> = TorrentField::DETAILS_PROGRESS
            .iter()
            .copied()
            .chain([TorrentField::TrackerStats])
            .collect();

        assert_eq!(TorrentField::DETAILS_PROGRESS_TRACKERS, expected);
    }

    #[test]
    fn details_fields_no_longer_include_magnet_link_or_total_size() {
        for fields in [
            TorrentField::DETAILS_FULL,
            TorrentField::DETAILS_PROGRESS,
            TorrentField::DETAILS_PROGRESS_TRACKERS,
        ] {
            for dropped in [
                TorrentField::MagnetLink,
                TorrentField::TotalSize,
                TorrentField::FileStats,
            ] {
                assert!(!fields.contains(&dropped), "{dropped:?} in {fields:?}");
            }
        }
        // What does not change while a torrent downloads stays out of the frequent read.
        for fixed in [
            TorrentField::Files,
            TorrentField::TrackerStats,
            TorrentField::Comment,
            TorrentField::Creator,
            TorrentField::Source,
        ] {
            assert!(
                !TorrentField::DETAILS_PROGRESS.contains(&fixed),
                "{fixed:?} in the progress read"
            );
        }
    }

    #[test]
    fn per_file_arrays_and_tracker_stats_parse() {
        let record: TorrentRecord = serde_json::from_value(json!({
            "file_count": 2,
            "bytes_completed": [6, 0],
            "priorities": [1, -1],
            "wanted": [true, false],
            "source": "tag",
            "tracker_stats": [{
                "host": "tracker.example:443",
                "has_announced": true,
                "last_announce_succeeded": false,
                "last_announce_timed_out": true,
                "last_announce_result": "Tracker did not respond",
                "has_scraped": true,
                "last_scrape_succeeded": true,
                "last_scrape_timed_out": false,
                "last_scrape_result": "",
                "seeder_count": 12,
                "leecher_count": -1,
                "tier": 0,
                "is_backup": false,
            }],
        }))
        .unwrap();

        assert_eq!(record.file_count, Some(2));
        assert_eq!(record.bytes_completed, Some(vec![6, 0]));
        assert_eq!(
            record.priorities,
            Some(vec![FilePriority::High, FilePriority::Low])
        );
        assert_eq!(record.wanted, Some(vec![true, false]));
        assert_eq!(record.source.as_deref(), Some("tag"));
        assert_eq!(
            record.tracker_stats,
            Some(vec![TrackerStatRecord {
                host: "tracker.example:443".to_owned(),
                has_announced: true,
                last_announce_succeeded: false,
                last_announce_timed_out: true,
                last_announce_result: "Tracker did not respond".to_owned(),
                has_scraped: true,
                last_scrape_succeeded: true,
                last_scrape_timed_out: false,
                last_scrape_result: String::new(),
                seeder_count: Some(12),
                leecher_count: Some(-1),
                ..TrackerStatRecord::default()
            }])
        );
    }

    #[test]
    fn a_tracker_stat_missing_keys_still_parses() {
        let stat: TrackerStatRecord = serde_json::from_value(json!({ "host": "t:80" })).unwrap();

        assert_eq!(
            stat,
            TrackerStatRecord {
                host: "t:80".to_owned(),
                ..TrackerStatRecord::default()
            }
        );
        assert_eq!(stat.seeder_count, None);
    }

    /// A `tracker_stats` entry as a 4.1 daemon sends it: all 28 keys, a passkey in the URLs.
    fn live_shaped_tracker() -> serde_json::Value {
        json!({
            "announce": "http://tracker.example/ann?pk=SECRETPASSKEY",
            "announce_state": 1,
            "download_count": -1,
            "downloader_count": -1,
            "has_announced": true,
            "has_scraped": true,
            "host": "tracker.example:80",
            "id": 7,
            "is_backup": false,
            "last_announce_peer_count": 30,
            "last_announce_result": "Success",
            "last_announce_start_time": 0,
            "last_announce_succeeded": true,
            "last_announce_time": 1_700_000_000,
            "last_announce_timed_out": false,
            "last_scrape_result": "Could not connect to tracker",
            "last_scrape_start_time": 0,
            "last_scrape_succeeded": false,
            "last_scrape_time": 1_700_000_100,
            "last_scrape_timed_out": false,
            "leecher_count": -1,
            "next_announce_time": 1_700_001_800,
            "next_scrape_time": 0,
            "scrape": "http://tracker.example/scrape?pk=SECRETPASSKEY",
            "scrape_state": 2,
            "seeder_count": -1,
            "sitename": "example",
            "tier": 1,
        })
    }

    fn tracker(value: serde_json::Value) -> TrackerStatRecord {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn a_live_shaped_tracker_entry_parses_every_key_read() {
        assert_eq!(
            tracker(live_shaped_tracker()),
            TrackerStatRecord {
                id: TrackerId(7),
                tier: 1,
                is_backup: false,
                host: "tracker.example:80".to_owned(),
                sitename: "example".to_owned(),
                announce_state: TrackerState::Waiting,
                has_announced: true,
                last_announce_time: 1_700_000_000,
                last_announce_succeeded: true,
                last_announce_timed_out: false,
                last_announce_result: "Success".to_owned(),
                last_announce_peer_count: 30,
                next_announce_time: 1_700_001_800,
                has_scraped: true,
                last_scrape_time: 1_700_000_100,
                last_scrape_succeeded: false,
                last_scrape_timed_out: false,
                last_scrape_result: "Could not connect to tracker".to_owned(),
                seeder_count: Some(-1),
                leecher_count: Some(-1),
                download_count: Some(-1),
            }
        );
    }

    #[test]
    fn a_tracker_entry_never_holds_the_urls_with_their_passkeys() {
        let record = tracker(live_shaped_tracker());

        for text in [format!("{record:?}"), format!("{record:#?}")] {
            assert!(!text.contains("SECRETPASSKEY"), "{text}");
            assert!(!text.contains("/ann"), "{text}");
            assert!(!text.contains("/scrape"), "{text}");
        }
    }

    #[test]
    fn every_announce_state_maps() {
        let states: Vec<TrackerState> = [0, 1, 2, 3, 9]
            .into_iter()
            .map(|code| tracker(json!({ "announce_state": code })).announce_state)
            .collect();

        assert_eq!(
            states,
            [
                TrackerState::Inactive,
                TrackerState::Waiting,
                TrackerState::Queued,
                TrackerState::Active,
                TrackerState::Other(9),
            ]
        );
    }

    #[test]
    fn a_backup_row_carries_no_state() {
        let record = tracker(json!({
            "id": 8,
            "tier": 0,
            "is_backup": true,
            "host": "backup.example:80",
            "announce_state": 0,
            "has_announced": false,
            "last_announce_time": 0,
            "next_announce_time": 0,
            "last_announce_peer_count": 0,
            "last_scrape_time": 0,
            "seeder_count": -1,
            "leecher_count": -1,
            "download_count": -1,
        }));

        assert!(record.is_backup);
        assert_eq!(record.announce_state, TrackerState::Inactive);
        assert_eq!(
            (record.last_announce_time, record.next_announce_time),
            (0, 0)
        );
        assert_eq!(record.id, TrackerId(8));
    }

    #[test]
    fn counts_of_minus_one_and_times_in_the_future_are_kept_as_sent() {
        let record = tracker(json!({
            "seeder_count": -1,
            "leecher_count": 0,
            "download_count": 12,
            "last_announce_time": 4_102_444_800_i64,
            "next_announce_time": 4_102_448_400_i64,
        }));

        assert_eq!(record.seeder_count, Some(-1));
        assert_eq!(record.leecher_count, Some(0));
        assert_eq!(record.download_count, Some(12));
        assert_eq!(record.last_announce_time, 4_102_444_800);
        assert_eq!(record.next_announce_time, 4_102_448_400);
    }

    #[test]
    fn a_tracker_without_a_download_count_has_none() {
        assert_eq!(tracker(json!({ "host": "t:80" })).download_count, None);
        assert_eq!(
            tracker(json!({ "download_count": null })).download_count,
            None
        );
    }

    #[test]
    fn tracker_keys_of_another_type_hold_their_defaults_and_the_others_survive() {
        let mut entry = live_shaped_tracker();
        entry["tier"] = json!("first");
        entry["id"] = json!(-4);
        entry["announce_state"] = json!(400);
        entry["download_count"] = json!("many");
        entry["last_announce_peer_count"] = json!(null);
        entry["host"] = json!(80);
        entry["has_announced"] = json!(1);

        let record = tracker(entry);

        assert_eq!(record.tier, 0);
        assert_eq!(record.id, TrackerId(0));
        assert_eq!(record.announce_state, TrackerState::Inactive);
        assert_eq!(record.download_count, None);
        assert_eq!(record.last_announce_peer_count, 0);
        assert_eq!(record.host, "");
        assert!(!record.has_announced);
        assert_eq!(record.sitename, "example");
        assert_eq!(record.last_announce_result, "Success");
        assert_eq!(record.next_announce_time, 1_700_001_800);
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

    #[test]
    fn the_debug_text_of_an_add_shows_neither_the_link_nor_the_content() {
        let hash = "0123456789abcdef0123456789abcdef01234567";
        let link = format!(
            "magnet:?xt=urn:btih:{hash}&dn=x&tr=http%3A%2F%2Ft.example%2Fa%3Fpasskey%3DSECRET"
        );
        let content = b"d4:infod4:name6:SECRETee".to_vec();
        let add = |source| TorrentAdd {
            source,
            download_dir: None,
            paused: false,
            files_unwanted: Vec::new(),
            priority_high: Vec::new(),
            priority_low: Vec::new(),
            sequential_download: None,
        };
        let magnet = add(AddSource::Magnet(MagnetLink::parse(&link).unwrap()));
        let metainfo = add(AddSource::Metainfo(Arc::from(content.as_slice())));

        for text in [format!("{magnet:?}"), format!("{magnet:#?}")] {
            for secret in ["magnet:?", "t.example", "passkey", "SECRET"] {
                assert!(!text.contains(secret), "{secret} in {text}");
            }
            assert!(text.contains(hash), "{text}");
            assert!(text.contains(&format!("len: {}", link.len())), "{text}");
        }
        for text in [format!("{metainfo:?}"), format!("{metainfo:#?}")] {
            // The bytes as `Vec<u8>` prints them: `[100, 52, …]`.
            assert!(
                !text.contains("100, 52") && !text.contains("100,\n"),
                "{text}"
            );
            assert!(!text.contains("SECRET"), "{text}");
            assert!(text.contains(&format!("len: {}", content.len())), "{text}");
        }
    }
}
