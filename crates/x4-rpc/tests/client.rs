//! The JSON-RPC client over a fake transport: the request envelope, the
//! `X-Transmission-Session-Id` handshake, HTTP status and server error mapping, and the id types.

use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};
use x4_rpc::testing::{self, FakeTransport};
use x4_rpc::{
    Client, HashString, HttpRequest, HttpResponse, IdError, RpcError, TorrentSelection, Transport,
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
fn legacy_protocol_reply_is_an_unsupported_server() {
    // What Transmission 4.0 and older answer to a JSON-RPC 2.0 method name.
    let unknown_method =
        json_response(&json!({ "arguments": {}, "result": "method name not recognized" }));
    let with_tag = json_response(&json!({ "arguments": {}, "result": "success", "tag": 7 }));
    let mut client = scripted_client(vec![Ok(unknown_method), Ok(with_tag)]);

    for _ in 0..2 {
        assert_eq!(
            client.call_raw("session_stats", None),
            Err(RpcError::UnsupportedServer {
                rpc_version: "legacy protocol (Transmission 4.0 or older)".to_owned()
            })
        );
    }
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
