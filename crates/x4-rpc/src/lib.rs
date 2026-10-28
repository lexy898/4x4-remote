//! Transmission RPC protocol and transport for 4x4, with no UI dependencies.
//!
//! [`Client`] speaks Transmission's JSON-RPC 2.0 API (Transmission 4.1+) over a [`Transport`],
//! normally [`UreqTransport`], and handles the `X-Transmission-Session-Id` handshake. Torrents
//! are named by [`TorrentId`] or, for anything long-lived, [`HashString`]; requests that change
//! torrents take a [`TorrentSelection`], which cannot be empty. Every failure is an [`RpcError`].
//!
//! The `test-support` feature adds the `testing` module with a fake transport.

mod client;
mod error;
mod ids;
#[cfg(feature = "test-support")]
pub mod testing;
mod transport;

pub use client::{Client, DEFAULT_TIMEOUT};
pub use error::RpcError;
pub use ids::{GetIds, HashString, IdError, TorrentId, TorrentRef, TorrentSelection};
pub use transport::{HttpRequest, HttpResponse, Transport, TransportError, UreqTransport};
