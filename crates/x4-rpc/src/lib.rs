//! Transmission RPC protocol and transport for 4x4, with no UI dependencies.
//!
//! [`Client`] speaks Transmission's JSON-RPC 2.0 API (Transmission 4.1+) over a [`Transport`],
//! normally [`UreqTransport`], and handles the `X-Transmission-Session-Id` handshake. Torrents
//! are named by [`TorrentId`] or, for anything long-lived, [`HashString`]; requests that change
//! torrents take a [`TorrentSelection`], which cannot be empty. Every failure is an [`RpcError`].
//!
//! The torrent methods ([`Client::torrent_get`], [`Client::torrent_action`],
//! [`Client::torrent_set`], [`Client::torrent_remove`], [`Client::torrent_add`]) take typed
//! requests and return records with the server's raw values. They never send an empty
//! file-index list, which Transmission reads as "all files".
//!
//! What [`Client::torrent_add`] sends is validated first. The daemon reads its `filename` as a
//! URL to fetch or a path on its own disk too, so the only thing ever sent there is a
//! [`MagnetLink`], which [`MagnetLink::parse`] accepts only when the daemon would add it by the
//! same info hash; a `.torrent` file goes as its content. A [`DownloadDir`] is an absolute,
//! normalized folder on the server. An add uploads the whole file and the daemon reads it before
//! it answers, so [`Client::torrent_add`] waits [`ADD_TIMEOUT`] rather than the transport's own
//! timeout; every other call keeps the transport's.
//!
//! The session methods ([`Client::session_get`], [`Client::session_set`],
//! [`Client::session_stats`], [`Client::free_space`]) read and change the daemon's settings and
//! report its totals; [`Client::check_server`] tells whether the server's RPC version is one
//! 4x4 supports (Transmission 4.1+).
//!
//! The `test-support` feature adds the `testing` module with a fake transport, and
//! `Client::call_raw`, an untyped call for tooling such as the fixture recorder.

mod client;
mod download_dir;
mod error;
mod ids;
mod magnet;
mod session;
mod session_methods;
#[cfg(feature = "test-support")]
pub mod testing;
mod torrent;
mod torrent_methods;
mod transport;

pub use client::{ADD_TIMEOUT, Client, DEFAULT_TIMEOUT};
pub use download_dir::{DownloadDir, DownloadDirError, MAX_DOWNLOAD_DIR_BYTES};
pub use error::{RpcError, UnsupportedReason};
pub use ids::{GetIds, HashString, IdError, TorrentId, TorrentRef, TorrentSelection};
pub use magnet::{MAX_MAGNET_BYTES, MagnetError, MagnetLink};
pub use session::{
    FreeSpace, SESSION_FIELDS, ServerVersion, SessionInfo, SessionSet, SessionStats, StatsBlock,
};
pub use torrent::{
    AddOutcome, AddSource, AddedTorrent, ErrorKind, FilePriority, FileRecord, FileStatRecord,
    Pieces, RemoveData, Status, TorrentAction, TorrentAdd, TorrentField, TorrentGetResponse,
    TorrentRecord, TorrentSet, TrackerStatRecord,
};
pub use transport::{HttpRequest, HttpResponse, Transport, TransportError, UreqTransport};
