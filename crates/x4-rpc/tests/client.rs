//! The JSON-RPC client over a fake transport: the request envelope, the
//! `X-Transmission-Session-Id` handshake, HTTP status and server error mapping, and the id types.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::{Value, json};
use x4_rpc::testing::fixtures::{FREE_SPACE, SESSION_GET, SESSION_STATS};
use x4_rpc::testing::{self, FakeTransport};
use x4_rpc::{
    ADD_TIMEOUT, AddSource, Client, GetIds, HashString, HttpRequest, HttpResponse, IdError,
    LEGACY_ERROR_CODE, MagnetLink, Protocol, RemoveData, RpcError, ServerVersion, SessionSet,
    TorrentAction, TorrentAdd, TorrentField, TorrentSelection, TorrentSet, Transport,
    TransportError,
};

/// A scripted fake that also keeps the raw request bodies, to inspect the envelope itself.
#[derive(Clone)]
struct BodyCapture {
    fake: FakeTransport,
    bodies: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl Transport for BodyCapture {
    fn post(&self, request: &HttpRequest<'_>) -> Result<HttpResponse, TransportError> {
        self.bodies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.body.to_vec());
        self.fake.post(request)
    }
}

fn json_response(body: &Value) -> HttpResponse {
    HttpResponse {
        status: 200,
        session_id: None,
        body: body.to_string().into_bytes(),
    }
}

fn status_response(status: u16) -> HttpResponse {
    HttpResponse {
        status,
        session_id: None,
        body: Vec::new(),
    }
}

fn scripted_client(responses: Vec<Result<HttpResponse, TransportError>>) -> Client<FakeTransport> {
    Client::new(FakeTransport::scripted(responses))
}

const HASH: &str = "0123456789abcdef0123456789abcdef01234567";

#[test]
fn sends_jsonrpc_2_envelope() {
    let capture = BodyCapture {
        fake: FakeTransport::scripted(vec![
            Ok(testing::ok(json!({}))),
            Ok(testing::ok(json!({ "version": "4.1.3" }))),
        ]),
        bodies: Arc::default(),
    };
    let mut client = Client::new(capture.clone());

    client.call_raw("session_stats", None).unwrap();
    client
        .call_raw("session_get", Some(json!({ "fields": ["version"] })))
        .unwrap();

    let bodies = capture.bodies.lock().unwrap();
    let first: Value = serde_json::from_slice(&bodies[0]).unwrap();
    let second: Value = serde_json::from_slice(&bodies[1]).unwrap();
    assert_eq!(first["jsonrpc"], "2.0");
    assert_eq!(first["method"], "session_stats");
    assert!(first["id"].is_u64(), "id must be an integer: {first}");
    assert!(
        first.get("params").is_none(),
        "params must be omitted: {first}"
    );
    assert_eq!(second["jsonrpc"], "2.0");
    assert_eq!(second["method"], "session_get");
    assert_eq!(second["params"], json!({ "fields": ["version"] }));
    assert!(second["id"].is_u64(), "id must be an integer: {second}");
    assert_ne!(first["id"], second["id"], "every request gets its own id");
}

#[test]
fn retries_once_after_409_with_new_session() {
    let fake = FakeTransport::scripted(vec![
        Ok(testing::conflict("abc")),
        Ok(testing::ok(json!({ "version": "4.1.3" }))),
    ]);
    let mut client = Client::new(fake.clone());

    let result = client.call_raw("session_get", Some(json!({ "fields": ["version"] })));

    assert_eq!(result, Ok(json!({ "version": "4.1.3" })));
    let requests = fake.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].session_id, None);
    assert_eq!(requests[1].session_id.as_deref(), Some("abc"));
    assert_eq!(requests[1].method, "session_get");
    assert_eq!(
        requests[1].params, requests[0].params,
        "the same request is resent"
    );
}

#[test]
fn reuses_session_id_on_next_call() {
    let fake = FakeTransport::scripted(vec![
        Ok(testing::conflict("abc")),
        Ok(testing::ok(json!({}))),
        Ok(testing::ok(json!({}))),
    ]);
    let mut client = Client::new(fake.clone());

    client.call_raw("session_stats", None).unwrap();
    client.call_raw("session_stats", None).unwrap();

    let requests = fake.requests();
    assert_eq!(requests.len(), 3, "the second call needs no handshake");
    assert_eq!(requests[2].session_id.as_deref(), Some("abc"));
}

#[test]
fn second_consecutive_409_is_an_error() {
    let fake =
        FakeTransport::scripted(vec![Ok(testing::conflict("a")), Ok(testing::conflict("b"))]);
    let mut client = Client::new(fake.clone());

    let result = client.call_raw("session_stats", None);

    assert_eq!(result, Err(RpcError::SessionIdRejected));
    assert_eq!(fake.requests().len(), 2, "exactly one retry");
}

#[test]
fn next_call_after_a_rejection_uses_the_latest_session_id() {
    let fake = FakeTransport::scripted(vec![
        Ok(testing::conflict("a")),
        Ok(testing::conflict("b")),
        Ok(testing::ok(json!({}))),
    ]);
    let mut client = Client::new(fake.clone());

    assert_eq!(
        client.call_raw("session_stats", None),
        Err(RpcError::SessionIdRejected)
    );
    client.call_raw("session_stats", None).unwrap();

    assert_eq!(fake.requests()[2].session_id.as_deref(), Some("b"));
}

#[test]
fn conflict_without_header_is_an_error() {
    let mut client = scripted_client(vec![Ok(status_response(409))]);

    assert_eq!(
        client.call_raw("session_stats", None),
        Err(RpcError::SessionIdMissing)
    );
}

#[test]
fn maps_401_and_403() {
    let mut client = scripted_client(vec![Ok(status_response(401)), Ok(status_response(403))]);

    assert_eq!(
        client.call_raw("session_stats", None),
        Err(RpcError::Unauthorized)
    );
    assert_eq!(
        client.call_raw("session_stats", None),
        Err(RpcError::Forbidden)
    );
}

#[test]
fn maps_other_http_statuses() {
    let mut client = scripted_client(vec![Ok(status_response(500)), Ok(status_response(204))]);

    assert_eq!(
        client.call_raw("session_stats", None),
        Err(RpcError::HttpStatus(500))
    );
    assert_eq!(
        client.call_raw("session_stats", None),
        Err(RpcError::HttpStatus(204))
    );
}

#[test]
fn maps_server_error_with_detail() {
    let mut client = scripted_client(vec![Ok(json_response(&json!({
        "error": {
            "code": 5,
            "message": "system error",
            "data": { "error_string": "No such file or directory" }
        },
        "id": 1,
        "jsonrpc": "2.0"
    })))]);

    assert_eq!(
        client.call_raw("free_space", Some(json!({ "path": "/missing" }))),
        Err(RpcError::Server {
            code: 5,
            message: "system error".to_owned(),
            detail: Some("No such file or directory".to_owned()),
        })
    );
}

#[test]
fn maps_method_not_found() {
    let mut client = scripted_client(vec![Ok(json_response(&json!({
        "error": { "code": -32601, "message": "Method not found" },
        "id": 1,
        "jsonrpc": "2.0"
    })))]);

    assert_eq!(
        client.call_raw("no_such_method", None),
        Err(RpcError::Server {
            code: -32601,
            message: "Method not found".to_owned(),
            detail: None,
        })
    );
}

#[test]
fn rejects_mismatched_response_id() {
    let mut client = scripted_client(vec![Ok(json_response(
        &json!({ "jsonrpc": "2.0", "id": 42, "result": {} }),
    ))]);

    let result = client.call_raw("session_stats", None);

    assert!(
        matches!(result, Err(RpcError::InvalidResponse(_))),
        "got {result:?}"
    );
}

#[test]
fn rejects_malformed_responses() {
    let not_json = HttpResponse {
        status: 200,
        session_id: None,
        body: b"<html>not json</html>".to_vec(),
    };
    let neither = json_response(&json!({ "jsonrpc": "2.0", "id": 2 }));
    // Not the legacy shape: without `jsonrpc`, only a string `result` marks a legacy daemon.
    let object_result_without_jsonrpc = json_response(&json!({ "result": {}, "id": 3 }));
    let mut client = scripted_client(vec![
        Ok(not_json),
        Ok(neither),
        Ok(object_result_without_jsonrpc),
    ]);

    for _ in 0..3 {
        let result = client.call_raw("session_stats", None);
        assert!(
            matches!(result, Err(RpcError::InvalidResponse(_))),
            "got {result:?}"
        );
    }
}

#[test]
fn a_legacy_answer_in_json_rpc_mode_is_an_invalid_response() {
    // What Transmission 4.0 and older answer to a JSON-RPC 2.0 method name, and a legacy success.
    // Outside `check_server` it means the daemon behind the address changed: the call fails as
    // one that cannot be read, so that the caller reconnects and detects the protocol again.
    let unknown_method =
        json_response(&json!({ "arguments": {}, "result": "method name not recognized" }));
    let with_tag = json_response(&json!({ "arguments": {}, "result": "success", "tag": 7 }));
    let fake = FakeTransport::scripted(vec![Ok(unknown_method), Ok(with_tag)]);
    let mut client = Client::new(fake.clone());

    for _ in 0..2 {
        assert_eq!(
            client.session_stats(),
            Err(RpcError::InvalidResponse(
                "session_stats: the server answered in the legacy protocol".to_owned()
            ))
        );
    }
    assert_eq!(client.protocol(), Protocol::JsonRpc);
    assert_eq!(fake.requests().len(), 2, "nothing is asked again");
}

#[test]
fn transport_errors_pass_through() {
    let mut client = scripted_client(vec![Err(TransportError::Timeout)]);

    assert_eq!(
        client.call_raw("session_stats", None),
        Err(RpcError::Transport(TransportError::Timeout))
    );
}

#[test]
fn exhausted_script_is_a_transport_error() {
    let mut client = scripted_client(Vec::new());

    assert_eq!(
        client.call_raw("session_stats", None),
        Err(RpcError::Transport(TransportError::Io(
            "script exhausted".to_owned()
        )))
    );
}

#[test]
fn routed_fake_answers_by_method_and_records_params() {
    let fake = FakeTransport::routed(|method, params| match method {
        "free_space" => Ok(testing::ok(
            json!({ "path": params["path"], "size_bytes": 7 }),
        )),
        _ => Ok(status_response(500)),
    });
    let mut client = Client::new(fake.clone());

    let result = client.call_raw("free_space", Some(json!({ "path": "/data" })));
    let other = client.call_raw("session_stats", None);

    assert_eq!(result, Ok(json!({ "path": "/data", "size_bytes": 7 })));
    assert_eq!(other, Err(RpcError::HttpStatus(500)));
    let requests = fake.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "free_space");
    assert_eq!(requests[0].params, json!({ "path": "/data" }));
    assert_eq!(
        requests[1].params,
        Value::Null,
        "no params recorded as null"
    );
}

/// The `result` of a recorded fixture envelope; null if the fixture is malformed, which
/// `tests/fixtures.rs` reports.
fn fixture_result(text: &str) -> Value {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|envelope| envelope.get("result").cloned())
        .unwrap_or_default()
}

/// A result of the right shape for each method a 4.1 daemon has.
fn any_method(method: &str) -> HttpResponse {
    testing::ok(match method {
        "session_get" => fixture_result(SESSION_GET),
        "session_stats" => fixture_result(SESSION_STATS),
        "free_space" => fixture_result(FREE_SPACE),
        "torrent_get" => json!({ "torrents": [] }),
        "torrent_add" => {
            json!({ "torrent_added": { "id": 9, "name": "x", "hash_string": HASH } })
        }
        _ => json!({}),
    })
}

#[test]
fn only_torrent_add_asks_for_a_timeout_of_its_own() {
    let fake = FakeTransport::routed(|method, _| Ok(any_method(method)));
    let mut client = Client::new(fake.clone());
    let selection = TorrentSelection::from_hashes([HashString::parse(HASH).unwrap()]).unwrap();
    let add = TorrentAdd {
        source: AddSource::Magnet(
            MagnetLink::parse(&format!("magnet:?xt=urn:btih:{HASH}")).unwrap(),
        ),
        download_dir: None,
        paused: false,
        files_unwanted: Vec::new(),
        priority_high: Vec::new(),
        priority_low: Vec::new(),
        sequential_download: None,
    };

    client.check_server().unwrap();
    client.session_get().unwrap();
    client
        .session_set(&SessionSet {
            alt_speed_enabled: Some(true),
            ..SessionSet::default()
        })
        .unwrap();
    client.session_stats().unwrap();
    client.free_space("/data").unwrap();
    client
        .torrent_get(&GetIds::All, &[TorrentField::Id])
        .unwrap();
    client
        .torrent_action(TorrentAction::Stop, &selection)
        .unwrap();
    client
        .torrent_set(
            &selection,
            &TorrentSet {
                sequential_download: Some(true),
                ..TorrentSet::default()
            },
        )
        .unwrap();
    client.torrent_remove(&selection, RemoveData::Keep).unwrap();
    client.torrent_add(&add).unwrap();

    let waits: Vec<(String, Option<Duration>)> = fake
        .requests()
        .into_iter()
        .map(|request| (request.method, request.timeout))
        .collect();
    let usual = |method: &str| (method.to_owned(), None);
    assert_eq!(
        waits,
        [
            usual("session_get"),
            usual("session_get"),
            usual("session_set"),
            usual("session_stats"),
            usual("free_space"),
            usual("torrent_get"),
            usual("torrent_stop"),
            usual("torrent_set"),
            usual("torrent_remove"),
            ("torrent_add".to_owned(), Some(ADD_TIMEOUT)),
        ]
    );
    assert_eq!(ADD_TIMEOUT, Duration::from_secs(60));
}

#[test]
fn an_add_resent_after_a_session_conflict_waits_as_long() {
    let fake = FakeTransport::scripted(vec![
        Ok(testing::conflict("fresh")),
        Ok(any_method("torrent_add")),
    ]);
    let mut client = Client::new(fake.clone());
    let add = TorrentAdd {
        source: AddSource::Metainfo(Arc::from(&b"d4:infod4:name1:xee"[..])),
        download_dir: None,
        paused: true,
        files_unwanted: Vec::new(),
        priority_high: Vec::new(),
        priority_low: Vec::new(),
        sequential_download: None,
    };

    client.torrent_add(&add).unwrap();

    let timeouts: Vec<Option<Duration>> = fake
        .requests()
        .into_iter()
        .map(|request| request.timeout)
        .collect();
    assert_eq!(timeouts, [Some(ADD_TIMEOUT), Some(ADD_TIMEOUT)]);
}

#[test]
fn selection_cannot_be_empty() {
    assert_eq!(TorrentSelection::new(vec![]), Err(IdError::EmptySelection));
    assert_eq!(
        TorrentSelection::from_hashes(Vec::<HashString>::new()),
        Err(IdError::EmptySelection)
    );
}

#[test]
fn hash_string_validates() {
    assert_eq!(HashString::parse(HASH).unwrap().as_str(), HASH);
    assert_eq!(
        HashString::parse(&HASH.to_uppercase()).unwrap().as_str(),
        HASH,
        "uppercase hex is normalised to lowercase"
    );
    let sha256 = "ab".repeat(32);
    assert_eq!(HashString::parse(&sha256).unwrap().as_str(), sha256);

    let short = &HASH[..39];
    let long = format!("{HASH}0");
    let not_hex = "g".repeat(40);
    for bad in ["xyz", short, &long, &not_hex, ""] {
        assert_eq!(
            HashString::parse(bad),
            Err(IdError::InvalidHash(bad.to_owned())),
            "{bad:?} must be rejected"
        );
    }
}

/// What Transmission 4.0 and older answer to a JSON-RPC 2.0 request.
fn not_recognized() -> HttpResponse {
    testing::legacy_error("method name not recognized")
}

/// The version fields of a legacy `session-get`, as `check_server` asks for them.
fn legacy_version_fields() -> Value {
    json!(["version", "rpc-version", "rpc-version-semver"])
}

/// A Transmission 4.0.5: it answers JSON-RPC requests as it does not know them, the version
/// fields of a legacy `session-get` with its own, and every other legacy request with
/// `answer(method, arguments)`.
fn legacy_daemon(answer: impl Fn(&str, &Value) -> HttpResponse + Send + 'static) -> FakeTransport {
    FakeTransport::routed(move |method, arguments| {
        Ok(if method.contains('_') {
            not_recognized()
        } else if method == "session-get" && arguments["fields"] == legacy_version_fields() {
            testing::legacy_ok(json!({
                "version": "4.0.5 (a6fe2a64aa)",
                "rpc-version": 17,
                "rpc-version-semver": "5.3.0",
            }))
        } else {
            answer(method, arguments)
        })
    })
}

/// A client that has found `transport` to speak the legacy protocol.
fn legacy_client(transport: &FakeTransport) -> Client<FakeTransport> {
    let mut client = Client::new(transport.clone());
    assert_eq!(
        client.check_server().map(|server| server.protocol),
        Ok(Protocol::Legacy)
    );
    client
}

#[test]
fn check_server_switches_to_legacy_for_a_legacy_answer() {
    let capture = BodyCapture {
        fake: legacy_daemon(|_, _| status_response(500)),
        bodies: Arc::default(),
    };
    let mut client = Client::new(capture.clone());

    let server = client.check_server();

    assert_eq!(
        server,
        Ok(ServerVersion {
            version: "4.0.5 (a6fe2a64aa)".to_owned(),
            rpc_semver: "5.3.0".to_owned(),
            rpc_version: Some(17),
            protocol: Protocol::Legacy,
        })
    );
    assert_eq!(client.protocol(), Protocol::Legacy);
    let requests = capture.fake.requests();
    assert_eq!(requests.len(), 2, "one more request, and only at connect");
    assert_eq!(requests[0].method, "session_get");
    assert_eq!(requests[1].method, "session-get");
    assert_eq!(
        requests[1].params,
        json!({ "fields": legacy_version_fields() })
    );
    let bodies = capture.bodies.lock().unwrap();
    let legacy: Value = serde_json::from_slice(&bodies[1]).unwrap();
    assert!(
        legacy["tag"].is_u64(),
        "a legacy request has a tag: {legacy}"
    );
    assert!(
        legacy.get("jsonrpc").is_none() && legacy.get("id").is_none(),
        "and nothing of JSON-RPC: {legacy}"
    );
}

#[test]
fn legacy_error_text_reaches_the_user() {
    let transport = legacy_daemon(|_, _| testing::legacy_error("invalid or corrupt torrent file"));
    let mut client = legacy_client(&transport);
    let add = TorrentAdd {
        source: AddSource::Metainfo(Arc::from(&b"d4:infod4:name1:xee"[..])),
        download_dir: None,
        paused: false,
        files_unwanted: Vec::new(),
        priority_high: Vec::new(),
        priority_low: Vec::new(),
        sequential_download: None,
    };

    assert_eq!(
        client.torrent_add(&add),
        Err(RpcError::Server {
            code: LEGACY_ERROR_CODE,
            message: "invalid or corrupt torrent file".to_owned(),
            detail: None,
        })
    );
}

#[test]
fn legacy_tag_mismatch_is_invalid() {
    let transport = legacy_daemon(|_, _| {
        json_response(&json!({ "arguments": {}, "result": "success", "tag": 424_242 }))
    });
    let mut client = legacy_client(&transport);

    let result = client.session_stats();

    assert!(
        matches!(&result, Err(RpcError::InvalidResponse(message)) if message.contains("tag")),
        "got {result:?}"
    );
}

#[test]
fn legacy_session_id_handshake_still_retries() {
    let fake = FakeTransport::scripted(vec![
        Ok(not_recognized()),
        Ok(testing::legacy_ok(
            json!({ "version": "3.00 (bb6b5a062e)", "rpc-version": 16 }),
        )),
        Ok(testing::conflict("rotated")),
        Ok(testing::legacy_ok(
            json!({ "path": "/data", "size-bytes": 7 }),
        )),
    ]);
    let mut client = Client::new(fake.clone());
    client.check_server().unwrap();

    let space = client.free_space("/data").unwrap();

    assert_eq!((space.path.as_str(), space.size_bytes), ("/data", 7));
    let requests = fake.requests();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[2].method, "free-space");
    assert_eq!(requests[3].method, "free-space");
    assert_eq!(requests[3].session_id.as_deref(), Some("rotated"));
    assert_eq!(
        requests[3].params, requests[2].params,
        "the same request is resent"
    );
}
