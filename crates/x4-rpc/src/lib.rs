//! Transmission RPC protocol and transport for 4x4, with no UI dependencies.
//!
//! [`Client`] talks to a Transmission daemon (3.00 or newer) over a [`Transport`], normally
//! [`UreqTransport`], and handles the `X-Transmission-Session-Id` handshake. Torrents are named
//! by [`TorrentId`] or, for anything long-lived, [`HashString`]; requests that change torrents
//! take a [`TorrentSelection`], which cannot be empty. Every failure is an [`RpcError`].
//!
//! It speaks two protocols behind the same typed calls ([`Protocol`]): JSON-RPC 2.0, of
//! Transmission 4.1 and newer, and the legacy protocol of Transmission 3.00 to 4.1.0-beta, whose
//! methods and fields have names of their own (`torrent-get`, `hashString`, `download-dir`) and
//! whose errors are only text ([`LEGACY_ERROR_CODE`]). [`Client::check_server`] decides which, at
//! every connect: it asks in JSON-RPC, so a 4.1 daemon costs one request as ever, and only a
//! daemon that answers in the legacy protocol is asked once more, in that one, for its legacy
//! `rpc-version`, which decides the torrent fields asked of it from then on (Transmission 3.00
//! answers a field it does not know with `0`). The calls go in JSON-RPC until a check finds a legacy daemon; an answer in the
//! legacy protocol to any other JSON-RPC call is an [`RpcError::InvalidResponse`], so that a
//! daemon replaced behind the same address is checked again rather than misread.
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
//! A `torrent_get` can also read the connected peers ([`PeerRecord`], [`PeersFromRecord`]) and
//! every tracker's state ([`TrackerStatRecord`]); the records of both are read leniently, so
//! that one odd value in a peer's or tracker's entry cannot fail the read of the rest.
//!
//! The session methods ([`Client::session_get`], [`Client::session_set`],
//! [`Client::session_stats`], [`Client::free_space`]) read and change the daemon's settings and
//! report its totals; [`Client::check_server`] tells whether the server's RPC version is one
//! 4x4 supports ([`ServerVersion::unsupported_reason`]: Transmission 3.00 or newer).
//!
//! A daemon that asks for a user name and password gets them as HTTP basic authentication on
//! every request, from [`UreqTransport::with_credentials`]; [`Credentials`] never shows the
//! password in its `Debug`.
//!
//! The `test-support` feature adds the `testing` module with a fake transport, and
//! `Client::call_raw`, an untyped call for tooling such as the fixture recorder.

mod client;
mod credentials;
mod download_dir;
mod error;
mod ids;
mod legacy;
mod lenient;
mod magnet;
mod peers;
mod session;
mod session_methods;
#[cfg(feature = "test-support")]
pub mod testing;
mod torrent;
mod torrent_methods;
mod transport;

pub use client::{ADD_TIMEOUT, Client, DEFAULT_TIMEOUT, Protocol};
pub use credentials::Credentials;
pub use download_dir::{DownloadDir, DownloadDirError, MAX_DOWNLOAD_DIR_BYTES};
pub use error::{LEGACY_ERROR_CODE, RpcError, UnsupportedReason};
pub use ids::{GetIds, HashString, IdError, TorrentId, TorrentRef, TorrentSelection};
pub use magnet::{MAX_MAGNET_BYTES, MagnetError, MagnetLink};
pub use peers::{PeerRecord, PeersFromRecord};
pub use session::{
    FreeSpace, SESSION_FIELDS, ServerVersion, SessionInfo, SessionSet, SessionStats, StatsBlock,
};
pub use torrent::{
    AddOutcome, AddSource, AddedTorrent, ErrorKind, FilePriority, FileRecord, FileStatRecord,
    Pieces, RemoveData, Status, TorrentAction, TorrentAdd, TorrentField, TorrentGetResponse,
    TorrentRecord, TorrentSet, TrackerId, TrackerStatRecord, TrackerState,
};
pub use transport::{HttpRequest, HttpResponse, Transport, TransportError, UreqTransport};
