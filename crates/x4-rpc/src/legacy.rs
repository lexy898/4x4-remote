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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::names::tests::every_torrent_field;
    use crate::testing::{self, FakeTransport};
    use crate::{
        AddSource, Client, DownloadDir, GetIds, HashString, MagnetLink, Protocol, RemoveData,
        RpcError, SessionSet, TorrentAction, TorrentAdd, TorrentId, TorrentRef, TorrentSelection,
        TorrentSet,
    };

    const HASH: &str = "0123456789abcdef0123456789abcdef01234567";

    /// A client that has found a Transmission 4.1.0-beta.2, whose legacy RPC version 18 has every
    /// torrent field of the table, so that no field is left out of a request as too new. Past the
    /// server check, the daemon refuses every request with an error of its own, which only a
    /// request that was encoded and sent gets back.
    fn newest_legacy_client() -> (Client<FakeTransport>, FakeTransport) {
        let transport = FakeTransport::routed(|method, arguments| {
            let version_fields = json!(["version", "rpc-version", "rpc-version-semver"]);
            Ok(if method.contains('_') {
                testing::legacy_error("method name not recognized")
            } else if method == "session-get" && arguments["fields"] == version_fields {
                testing::legacy_ok(json!({
                    "version": "4.1.0-beta.2 (4a9b5b2d4d)",
                    "rpc-version": 18,
                    "rpc-version-semver": "5.4.0",
                }))
            } else {
                testing::legacy_error("refused by the test")
            })
        });
        let mut client = Client::new(transport.clone());
        assert_eq!(
            client.check_server().map(|server| server.protocol),
            Ok(Protocol::Legacy)
        );
        (client, transport)
    }

    /// Every [`TorrentAction`]; the match has no wildcard, so a new one joins the list.
    fn every_action() -> [TorrentAction; 5] {
        use TorrentAction as A;
        let every = [A::Start, A::StartNow, A::Stop, A::Verify, A::Reannounce];
        for action in every {
            match action {
                A::Start | A::StartNow | A::Stop | A::Verify | A::Reannounce => {}
            }
        }
        every
    }

    #[test]
    fn every_typed_call_with_everything_set_has_legacy_names() {
        // A key with no legacy name would fail only at run time, on a legacy daemon, as "no
        // legacy name for ...". The structs are written out in full, without a default for the
        // rest, so that a field added to one does not compile until it is set here too.
        let (mut client, transport) = newest_legacy_client();
        let selection = TorrentSelection::new(vec![
            TorrentRef::Id(TorrentId(3)),
            TorrentRef::Hash(HashString::parse(HASH).unwrap()),
        ])
        .unwrap();
        let set = TorrentSet {
            sequential_download: Some(true),
            files_wanted: vec![0],
            files_unwanted: vec![1],
            priority_high: vec![2],
            priority_normal: vec![3],
            priority_low: vec![4],
        };
        let from_file = TorrentAdd {
            source: AddSource::Metainfo(Arc::from(&b"d4:infod4:name1:xee"[..])),
            download_dir: Some(DownloadDir::new("/data/new").unwrap()),
            paused: true,
            files_unwanted: vec![1],
            priority_high: vec![2],
            priority_low: vec![3],
            sequential_download: Some(true),
        };
        // A magnet link takes no file choices: the client refuses them before encoding.
        let from_link = TorrentAdd {
            source: AddSource::Magnet(
                MagnetLink::parse(&format!("magnet:?xt=urn:btih:{HASH}")).unwrap(),
            ),
            download_dir: Some(DownloadDir::new("/data/new").unwrap()),
            paused: false,
            files_unwanted: Vec::new(),
            priority_high: Vec::new(),
            priority_low: Vec::new(),
            sequential_download: Some(false),
        };
        let limits = SessionSet {
            alt_speed_enabled: Some(true),
            speed_limit_down_kbps: Some(100),
            speed_limit_down_enabled: Some(true),
            speed_limit_up_kbps: Some(50),
            speed_limit_up_enabled: Some(true),
        };
        let fields = every_torrent_field();
        let selected = GetIds::Selection(selection.clone());

        let mut calls = vec![
            (
                "torrent_get all",
                client.torrent_get(&GetIds::All, &fields).err(),
            ),
            (
                "torrent_get recently active",
                client.torrent_get(&GetIds::RecentlyActive, &fields).err(),
            ),
            (
                "torrent_get selected",
                client.torrent_get(&selected, &fields).err(),
            ),
            ("torrent_set", client.torrent_set(&selection, &set).err()),
            ("torrent_add file", client.torrent_add(&from_file).err()),
            ("torrent_add link", client.torrent_add(&from_link).err()),
            (
                "torrent_remove keep",
                client.torrent_remove(&selection, RemoveData::Keep).err(),
            ),
            (
                "torrent_remove delete",
                client.torrent_remove(&selection, RemoveData::Delete).err(),
            ),
            ("session_get", client.session_get().err()),
            ("session_set", client.session_set(&limits).err()),
            ("session_stats", client.session_stats().err()),
            ("free_space", client.free_space("/data").err()),
        ];
        for action in every_action() {
            calls.push((
                action.method(),
                client.torrent_action(action, &selection).err(),
            ));
        }

        for (call, error) in &calls {
            assert!(
                matches!(error, Some(RpcError::Server { .. })),
                "{call} was not sent: {error:?}"
            );
        }
        // The server check's two requests, then one legacy request per call.
        let requests = transport.requests();
        assert_eq!(requests.len(), 2 + calls.len());
        for request in requests.iter().skip(1) {
            assert!(!request.method.contains('_'), "{request:?}");
        }
    }
}
