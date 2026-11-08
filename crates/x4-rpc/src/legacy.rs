//! The legacy RPC protocol of Transmission 3.00 to 4.1.0-beta, behind the same typed calls as
//! JSON-RPC.
//!
//! A legacy request is `{"method", "arguments", "tag"}` and its answer `{"result", "arguments",
//! "tag"}`, with `result` `"success"` or the daemon's error text, and HTTP 200 either way. The
//! names differ from JSON-RPC's (`torrent-get`, `hashString`, `download-dir`), so [`encode`]
//! renames what the typed calls build in JSON-RPC's names, and [`decode`] renames the answer back,
//! so that the same records read it.

mod fill;
mod names;
mod request;
mod response;

use serde_json::Value;

#[cfg(feature = "test-support")]
pub(crate) use names::snake_name;

use crate::error::RpcError;

/// The legacy RPC version of Transmission 3.00, the oldest 4x4 speaks to.
pub(crate) const OLDEST_RPC_VERSION: u32 = 16;

/// A legacy request, ready to send, and what its answer needs to be read as the request asked.
pub(crate) struct LegacyRequest {
    /// The whole request body.
    pub(crate) body: Value,
    /// What [`decode`] fills in on the answer.
    pub(crate) fill: Fill,
}

/// What the answer to a legacy request must have filled in that the legacy daemons do not send.
///
/// Only a `torrent_get` has any: see [`fill`].
#[derive(Debug, Default)]
pub(crate) struct Fill {
    /// `bytes_completed` was asked, and is read from the `bytes_completed` of each `file_stats`.
    bytes_from_file_stats: bool,
    /// `files` was asked: the files that lack their piece spans get them computed.
    piece_spans: bool,
    /// The fields asked only to fill in others (`file_stats`, `piece_size`, `piece_count`, and
    /// `priorities` to count the files), dropped from the answer.
    added: Vec<&'static str>,
    /// `file_count` was asked, and is counted when the daemon does not send it.
    file_count: bool,
}

/// The legacy request for the JSON-RPC call `method` with `params` (no `arguments` when `None`),
/// tagged `tag`, to a daemon of the legacy RPC version `rpc_version`.
///
/// A torrent field newer than `rpc_version` is left out of a `torrent_get`, and stays `None` in
/// the records: Transmission 3.00 would answer it with `0` rather than leave it out.
///
/// # Errors
///
/// [`RpcError::InvalidRequest`] when `params` is not an object or names something the legacy
/// protocol has no name for.
pub(crate) fn encode(
    method: &str,
    params: Option<Value>,
    tag: u64,
    rpc_version: u32,
) -> Result<LegacyRequest, RpcError> {
    let (body, fill) = request::body(method, params, tag, rpc_version)?;
    Ok(LegacyRequest { body, fill })
}

/// The `arguments` of the answer `body` to the legacy request for `method` tagged `tag`, with
/// JSON-RPC's names, and filled in as `fill` says.
///
/// # Errors
///
/// [`RpcError::InvalidResponse`] when `body` is no legacy answer or answers another request, or
/// the files of a torrent have no piece spans and none can be computed; [`RpcError::Server`]
/// when the daemon reports an error.
pub(crate) fn decode(method: &str, tag: u64, fill: &Fill, body: &[u8]) -> Result<Value, RpcError> {
    let mut arguments = response::arguments(method, tag, body)?;
    if method == "torrent_get" {
        fill::apply(fill, &mut arguments)?;
    }
    Ok(arguments)
}
