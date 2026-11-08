//! Test doubles for code that talks to Transmission (feature `test-support`).
//!
//! [`FakeTransport`] answers requests from a script or a handler and records every request, in
//! either protocol; [`ok`], [`conflict`], [`legacy_ok`] and [`legacy_error`] build the common
//! responses. [`scrub`] removes private data from recorded responses and [`fixtures`] holds the
//! scrubbed responses of a real daemon.

pub mod scrub;

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::{Map, Value};

use crate::transport::{HttpRequest, HttpResponse, Transport, TransportError};

type Handler = Box<dyn Fn(&str, &Value) -> Result<HttpResponse, TransportError> + Send>;

enum Responder {
    Scripted(VecDeque<Result<HttpResponse, TransportError>>),
    Routed(Handler),
}

struct Shared {
    responder: Mutex<Responder>,
    requests: Mutex<Vec<RecordedRequest>>,
}

/// An in-memory [`Transport`] that records requests and answers from a script or a handler.
///
/// Clones share the script and the record: hand one clone to the client and keep another to
/// inspect [`FakeTransport::requests`].
///
/// A response body that is a JSON object without an `id` (as built by [`ok`]) gets the id of the
/// request it answers; to a legacy request, one without a `tag` (as built by [`legacy_ok`]) gets
/// its tag. Bodies with an explicit `id` or `tag` pass through unchanged, so a test can answer
/// with a mismatched one.
#[derive(Clone)]
pub struct FakeTransport {
    shared: Arc<Shared>,
}

impl FakeTransport {
    /// Answers requests with `responses` in order; once they run out, every request fails with
    /// `TransportError::Io("script exhausted")`.
    #[must_use]
    pub fn scripted(responses: Vec<Result<HttpResponse, TransportError>>) -> Self {
        Self::with(Responder::Scripted(responses.into()))
    }

    /// Answers every request with `handler(method, params)`, the two as
    /// [`RecordedRequest::method`] and [`RecordedRequest::params`] hold them; `params` is null
    /// when omitted.
    #[must_use]
    pub fn routed(
        handler: impl Fn(&str, &Value) -> Result<HttpResponse, TransportError> + Send + 'static,
    ) -> Self {
        Self::with(Responder::Routed(Box::new(handler)))
    }

    /// Every request received so far, oldest first.
    #[must_use]
    pub fn requests(&self) -> Vec<RecordedRequest> {
        lock(&self.shared.requests).clone()
    }

    fn with(responder: Responder) -> Self {
        Self {
            shared: Arc::new(Shared {
                responder: Mutex::new(responder),
                requests: Mutex::new(Vec::new()),
            }),
        }
    }
}

impl Transport for FakeTransport {
    fn post(&self, request: &HttpRequest<'_>) -> Result<HttpResponse, TransportError> {
        let envelope: Value = serde_json::from_slice(request.body).map_err(|error| {
            TransportError::Io(format!("fake transport: the request is not JSON: {error}"))
        })?;
        let Some(method) = envelope.get("method").and_then(Value::as_str) else {
            return Err(TransportError::Io(
                "fake transport: the request has no method".to_owned(),
            ));
        };
        // A JSON-RPC request names its version; a legacy one has `arguments` and a `tag`.
        let (params_key, correlator) = if envelope.get("jsonrpc").is_some() {
            ("params", "id")
        } else {
            ("arguments", "tag")
        };
        let params = envelope.get(params_key).cloned().unwrap_or(Value::Null);
        lock(&self.shared.requests).push(RecordedRequest {
            method: method.to_owned(),
            params: params.clone(),
            session_id: request.session_id.map(str::to_owned),
            timeout: request.timeout,
        });
        // Only the responder is locked here, so a handler may call `requests()`.
        let response = match &mut *lock(&self.shared.responder) {
            Responder::Scripted(script) => script
                .pop_front()
                .unwrap_or_else(|| Err(TransportError::Io("script exhausted".to_owned()))),
            Responder::Routed(handler) => handler(method, &params),
        };
        let id = envelope.get(correlator).cloned().unwrap_or(Value::Null);
        response.map(|response| with_request_id(response, correlator, id))
    }
}

impl fmt::Debug for FakeTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FakeTransport")
            .field("recorded_requests", &lock(&self.shared.requests).len())
            .finish_non_exhaustive()
    }
}

/// A request as the [`FakeTransport`] received it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedRequest {
    /// The method as sent: snake case in JSON-RPC (`session_get`), kebab-case in the legacy
    /// protocol (`session-get`).
    pub method: String,
    /// The `params` object, or the `arguments` of a legacy request; null when the request had
    /// none.
    pub params: Value,
    /// The `X-Transmission-Session-Id` the request carried.
    pub session_id: Option<String>,
    /// The timeout the request asked for in place of the transport's own; `None` when it took
    /// the transport's.
    pub timeout: Option<Duration>,
}

/// HTTP 200 with a JSON-RPC success response carrying `result`; a [`FakeTransport`] fills in the
/// request's id.
#[must_use]
pub fn ok(result: Value) -> HttpResponse {
    let mut body = Map::new();
    body.insert("jsonrpc".to_owned(), Value::from("2.0"));
    body.insert("result".to_owned(), result);
    HttpResponse {
        status: 200,
        session_id: None,
        body: Value::Object(body).to_string().into_bytes(),
    }
}

/// HTTP 200 with a legacy success answer carrying `arguments`; a [`FakeTransport`] fills in the
/// request's tag.
#[must_use]
pub fn legacy_ok(arguments: Value) -> HttpResponse {
    legacy_answer("success", arguments)
}

/// HTTP 200 with a legacy answer that reports the error `text`, as a legacy daemon reports every
/// error; a [`FakeTransport`] fills in the request's tag.
#[must_use]
pub fn legacy_error(text: &str) -> HttpResponse {
    legacy_answer(text, Value::Object(Map::new()))
}

fn legacy_answer(result: &str, arguments: Value) -> HttpResponse {
    let mut body = Map::new();
    body.insert("result".to_owned(), Value::from(result));
    body.insert("arguments".to_owned(), arguments);
    HttpResponse {
        status: 200,
        session_id: None,
        body: Value::Object(body).to_string().into_bytes(),
    }
}

/// HTTP 409 with a new `X-Transmission-Session-Id`, as the daemon answers a missing or stale one.
#[must_use]
pub fn conflict(session_id: &str) -> HttpResponse {
    HttpResponse {
        status: 409,
        session_id: Some(session_id.to_owned()),
        body: b"<h1>409: Conflict</h1>".to_vec(),
    }
}

/// `response` with `id` under `key` (`id` or `tag`) when its body is a JSON object without one.
fn with_request_id(mut response: HttpResponse, key: &str, id: Value) -> HttpResponse {
    if let Ok(Value::Object(mut body)) = serde_json::from_slice::<Value>(&response.body)
        && !body.contains_key(key)
    {
        body.insert(key.to_owned(), id);
        response.body = Value::Object(body).to_string().into_bytes();
    }
    response
}

/// Responses of a real Transmission 4.1 daemon, scrubbed by [`scrub::Scrubber`].
///
/// Each constant is a whole JSON-RPC response, `{"jsonrpc":"2.0","id":1,"result":…}`. To answer a
/// request with one, parse it and pass its `result` to [`ok`], which supplies the request's id.
/// `just record-fixtures` records them again.
pub mod fixtures {
    /// `session_get` with the session fields the app reads.
    pub const SESSION_GET: &str = include_str!("../tests/fixtures/session_get.json");
    /// `session_stats`.
    pub const SESSION_STATS: &str = include_str!("../tests/fixtures/session_stats.json");
    /// `free_space` for the session's download directory.
    pub const FREE_SPACE: &str = include_str!("../tests/fixtures/free_space.json");
    /// `torrent_get` with the list fields and no `ids`: every torrent.
    pub const TORRENT_GET_LIST: &str = include_str!("../tests/fixtures/torrent_get_list.json");
    /// `torrent_get` with the list fields and `ids: "recently_active"`: the torrents that
    /// changed lately and the `removed` ids.
    pub const TORRENT_GET_RECENT: &str = include_str!("../tests/fixtures/torrent_get_recent.json");
    /// `torrent_get` with the details fields for one multi-file torrent.
    pub const TORRENT_GET_DETAILS: &str =
        include_str!("../tests/fixtures/torrent_get_details.json");
    /// `torrent_get` with `TorrentField::PEERS` for one torrent that had peers: the exact answer
    /// of a daemon, with peers at documentation addresses.
    pub const TORRENT_GET_PEERS: &str = include_str!("../tests/fixtures/torrent_get_peers.json");
    /// `torrent_get` with `TorrentField::TRACKERS` for the same torrent: every key of every
    /// tracker, as the daemon sends them.
    pub const TORRENT_GET_TRACKERS: &str =
        include_str!("../tests/fixtures/torrent_get_trackers.json");

    /// Responses of the legacy daemons, scrubbed (see [`legacy::Recording`]).
    pub mod legacy {
        /// What one legacy daemon answered, as the whole legacy answers `{"arguments":…,"result":…,
        /// "tag":…}` of the requests 4x4 sends it, scrubbed. `just record-fixtures-legacy` records
        /// them again.
        #[derive(Debug, Clone, Copy)]
        pub struct Recording {
            /// The daemon's release, which names its directory: `v3.00`, `v4.0.5`, `v4.1.0-beta.2`.
            pub name: &'static str,
            /// `session_get` of the fields `check_server` asks a legacy daemon for.
            pub session_get_version: &'static str,
            /// `session_get` with `SESSION_FIELDS`.
            pub session_get: &'static str,
            /// `session_stats`.
            pub session_stats: &'static str,
            /// `free_space` of the download folder.
            pub free_space: &'static str,
            /// `free_space` of a folder that is not there: an error result on 3.00 and 4.0.5, a
            /// success with a free space of `-1` on 4.1.0-beta.2.
            pub free_space_missing: &'static str,
            /// `torrent_get` with `TorrentField::LIST` and no `ids`.
            pub torrent_get_list: &'static str,
            /// `torrent_get` with `TorrentField::LIST` and `ids: "recently-active"`.
            pub torrent_get_recent: &'static str,
            /// `torrent_get` with `TorrentField::DETAILS_FULL` of the multi-file torrent.
            pub torrent_get_details: &'static str,
            /// `torrent_get` with `TorrentField::DETAILS_PROGRESS_TRACKERS` of the same torrent.
            pub torrent_get_progress: &'static str,
            /// `torrent_get` with `TorrentField::PEERS` of the torrent with most peers.
            pub torrent_get_peers: &'static str,
            /// `torrent_get` with `TorrentField::TRACKERS` of the same torrent.
            pub torrent_get_trackers: &'static str,
        }

        /// The recordings of Transmission 3.00, 4.0.5 and 4.1.0-beta.2.
        pub const RECORDINGS: [Recording; 3] = [
            Recording {
                name: "v3.00",
                session_get_version: include_str!(
                    "../tests/fixtures/legacy/v3.00/session_get_version.json"
                ),
                session_get: include_str!("../tests/fixtures/legacy/v3.00/session_get.json"),
                session_stats: include_str!("../tests/fixtures/legacy/v3.00/session_stats.json"),
                free_space: include_str!("../tests/fixtures/legacy/v3.00/free_space.json"),
                free_space_missing: include_str!(
                    "../tests/fixtures/legacy/v3.00/free_space_missing.json"
                ),
                torrent_get_list: include_str!(
                    "../tests/fixtures/legacy/v3.00/torrent_get_list.json"
                ),
                torrent_get_recent: include_str!(
                    "../tests/fixtures/legacy/v3.00/torrent_get_recent.json"
                ),
                torrent_get_details: include_str!(
                    "../tests/fixtures/legacy/v3.00/torrent_get_details.json"
                ),
                torrent_get_progress: include_str!(
                    "../tests/fixtures/legacy/v3.00/torrent_get_progress.json"
                ),
                torrent_get_peers: include_str!(
                    "../tests/fixtures/legacy/v3.00/torrent_get_peers.json"
                ),
                torrent_get_trackers: include_str!(
                    "../tests/fixtures/legacy/v3.00/torrent_get_trackers.json"
                ),
            },
            Recording {
                name: "v4.0.5",
                session_get_version: include_str!(
                    "../tests/fixtures/legacy/v4.0.5/session_get_version.json"
                ),
                session_get: include_str!("../tests/fixtures/legacy/v4.0.5/session_get.json"),
                session_stats: include_str!("../tests/fixtures/legacy/v4.0.5/session_stats.json"),
                free_space: include_str!("../tests/fixtures/legacy/v4.0.5/free_space.json"),
                free_space_missing: include_str!(
                    "../tests/fixtures/legacy/v4.0.5/free_space_missing.json"
                ),
                torrent_get_list: include_str!(
                    "../tests/fixtures/legacy/v4.0.5/torrent_get_list.json"
                ),
                torrent_get_recent: include_str!(
                    "../tests/fixtures/legacy/v4.0.5/torrent_get_recent.json"
                ),
                torrent_get_details: include_str!(
                    "../tests/fixtures/legacy/v4.0.5/torrent_get_details.json"
                ),
                torrent_get_progress: include_str!(
                    "../tests/fixtures/legacy/v4.0.5/torrent_get_progress.json"
                ),
                torrent_get_peers: include_str!(
                    "../tests/fixtures/legacy/v4.0.5/torrent_get_peers.json"
                ),
                torrent_get_trackers: include_str!(
                    "../tests/fixtures/legacy/v4.0.5/torrent_get_trackers.json"
                ),
            },
            Recording {
                name: "v4.1.0-beta.2",
                session_get_version: include_str!(
                    "../tests/fixtures/legacy/v4.1.0-beta.2/session_get_version.json"
                ),
                session_get: include_str!(
                    "../tests/fixtures/legacy/v4.1.0-beta.2/session_get.json"
                ),
                session_stats: include_str!(
                    "../tests/fixtures/legacy/v4.1.0-beta.2/session_stats.json"
                ),
                free_space: include_str!("../tests/fixtures/legacy/v4.1.0-beta.2/free_space.json"),
                free_space_missing: include_str!(
                    "../tests/fixtures/legacy/v4.1.0-beta.2/free_space_missing.json"
                ),
                torrent_get_list: include_str!(
                    "../tests/fixtures/legacy/v4.1.0-beta.2/torrent_get_list.json"
                ),
                torrent_get_recent: include_str!(
                    "../tests/fixtures/legacy/v4.1.0-beta.2/torrent_get_recent.json"
                ),
                torrent_get_details: include_str!(
                    "../tests/fixtures/legacy/v4.1.0-beta.2/torrent_get_details.json"
                ),
                torrent_get_progress: include_str!(
                    "../tests/fixtures/legacy/v4.1.0-beta.2/torrent_get_progress.json"
                ),
                torrent_get_peers: include_str!(
                    "../tests/fixtures/legacy/v4.1.0-beta.2/torrent_get_peers.json"
                ),
                torrent_get_trackers: include_str!(
                    "../tests/fixtures/legacy/v4.1.0-beta.2/torrent_get_trackers.json"
                ),
            },
        ];
    }
}

/// A poisoned lock only means a handler panicked in another test thread; the data is still usable.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
