//! The legacy RPC protocol of Transmission 3.00 to 4.1.0-beta, behind the same typed calls as
//! JSON-RPC.
//!
//! A legacy request is `{"method", "arguments", "tag"}` and its answer `{"result", "arguments",
//! "tag"}`, with `result` `"success"` or the daemon's error text, and HTTP 200 either way. The
//! names differ from JSON-RPC's (`torrent-get`, `hashString`, `download-dir`), so [`encode`]
//! renames what the typed calls build in JSON-RPC's names, and [`decode`] renames the answer back,
//! so that the same records read it.

mod names;
mod request;
mod response;

use serde_json::Value;

use crate::error::RpcError;

/// A legacy request, ready to send, and what its answer needs to be read as the request asked.
pub(crate) struct LegacyRequest {
    /// The whole request body.
    pub(crate) body: Value,
    /// What [`decode`] fills in on the answer.
    pub(crate) fill: Fill,
}

/// What the answer to a legacy request must have filled in that the legacy daemons do not send.
///
/// Nothing yet: every name the client asks for has a legacy counterpart.
#[derive(Debug, Default)]
pub(crate) struct Fill {}

/// The legacy request for the JSON-RPC call `method` with `params` (no `arguments` when `None`),
/// tagged `tag`.
///
/// # Errors
///
/// [`RpcError::InvalidRequest`] when `params` is not an object or names something the legacy
/// protocol has no name for.
pub(crate) fn encode(
    method: &str,
    params: Option<Value>,
    tag: u64,
) -> Result<LegacyRequest, RpcError> {
    Ok(LegacyRequest {
        body: request::body(method, params, tag)?,
        fill: Fill::default(),
    })
}

/// The `arguments` of the answer `body` to the legacy request for `method` tagged `tag`, with
/// JSON-RPC's names, and filled in as `_fill` says.
///
/// # Errors
///
/// [`RpcError::InvalidResponse`] when `body` is no legacy answer or answers another request;
/// [`RpcError::Server`] when the daemon reports an error.
pub(crate) fn decode(method: &str, tag: u64, _fill: &Fill, body: &[u8]) -> Result<Value, RpcError> {
    response::arguments(method, tag, body)
}
