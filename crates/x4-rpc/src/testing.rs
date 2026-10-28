//! Test doubles for code that talks to Transmission (feature `test-support`).
//!
//! [`FakeTransport`] answers requests from a script or a handler and records every request;
//! [`ok`] and [`conflict`] build the common responses. [`scrub`] removes private data from
//! recorded responses and [`fixtures`] holds the scrubbed responses of a real daemon.

pub mod scrub;

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

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
/// request it answers. Bodies with an explicit `id` pass through unchanged, so a test can answer
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

    /// Answers every request with `handler(method, params)`; `params` is null when omitted.
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
        let params = envelope.get("params").cloned().unwrap_or(Value::Null);
        lock(&self.shared.requests).push(RecordedRequest {
            method: method.to_owned(),
            params: params.clone(),
            session_id: request.session_id.map(str::to_owned),
        });
        // Only the responder is locked here, so a handler may call `requests()`.
        let response = match &mut *lock(&self.shared.responder) {
            Responder::Scripted(script) => script
                .pop_front()
                .unwrap_or_else(|| Err(TransportError::Io("script exhausted".to_owned()))),
            Responder::Routed(handler) => handler(method, &params),
        };
        let id = envelope.get("id").cloned().unwrap_or(Value::Null);
        response.map(|response| with_request_id(response, id))
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
    /// The JSON-RPC method.
    pub method: String,
    /// The `params` object; null when the request had none.
    pub params: Value,
    /// The `X-Transmission-Session-Id` the request carried.
    pub session_id: Option<String>,
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

/// HTTP 409 with a new `X-Transmission-Session-Id`, as the daemon answers a missing or stale one.
#[must_use]
pub fn conflict(session_id: &str) -> HttpResponse {
    HttpResponse {
        status: 409,
        session_id: Some(session_id.to_owned()),
        body: b"<h1>409: Conflict</h1>".to_vec(),
    }
}

fn with_request_id(mut response: HttpResponse, id: Value) -> HttpResponse {
    if let Ok(Value::Object(mut body)) = serde_json::from_slice::<Value>(&response.body)
        && !body.contains_key("id")
    {
        body.insert("id".to_owned(), id);
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
}

/// A poisoned lock only means a handler panicked in another test thread; the data is still usable.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
