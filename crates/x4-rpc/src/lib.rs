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
//! The session methods ([`Client::session_get`], [`Client::session_set`],
//! [`Client::session_stats`], [`Client::free_space`]) read and change the daemon's settings and
//! report its totals; [`Client::check_server`] tells whether the server's RPC version is one
//! 4x4 supports (Transmission 4.1+).
//!
//! The `test-support` feature adds the `testing` module with a fake transport, and
//! `Client::call_raw`, an untyped call for tooling such as the fixture recorder.

mod client;
mod error;
mod ids;
mod session;
mod session_methods;
#[cfg(feature = "test-support")]
pub mod testing;
mod torrent;
mod torrent_methods;
mod transport;

pub use client::{Client, DEFAULT_TIMEOUT};
pub use error::RpcError;
pub use ids::{GetIds, HashString, IdError, TorrentId, TorrentRef, TorrentSelection};
pub use session::{
    FreeSpace, SESSION_FIELDS, ServerVersion, SessionInfo, SessionSet, SessionStats, StatsBlock,
};
pub use torrent::{
    AddOutcome, AddSource, AddedTorrent, ErrorKind, FilePriority, FileRecord, FileStatRecord,
    Pieces, RemoveData, Status, TorrentAction, TorrentAdd, TorrentField, TorrentGetResponse,
    TorrentRecord, TorrentSet,
};
pub use transport::{HttpRequest, HttpResponse, Transport, TransportError, UreqTransport};
