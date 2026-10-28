//! Session requests and the server version check over a fake transport: the parameters each
//! method sends, how the recorded fixtures and synthetic results parse, and which RPC versions
//! `check_server` accepts.

use serde_json::{Value, json};
use x4_rpc::testing::fixtures::{FREE_SPACE, SESSION_GET, SESSION_STATS};
use x4_rpc::testing::{self, FakeTransport, RecordedRequest};
use x4_rpc::{
    Client, HttpResponse, RpcError, SESSION_FIELDS, SessionInfo, SessionSet, SessionStats,
    StatsBlock, UnsupportedReason,
};

/// The `result` of a recorded fixture envelope; null if the fixture is malformed, which
/// `tests/fixtures.rs` reports.
fn fixture_result(text: &str) -> Value {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|envelope| envelope.get("result").cloned())
        .unwrap_or_default()
}

/// A client whose only response carries `result`, and the transport to inspect afterwards.
fn client_answering(result: Value) -> (Client<FakeTransport>, FakeTransport) {
    let transport = FakeTransport::scripted(vec![Ok(testing::ok(result))]);
    (Client::new(transport.clone()), transport)
}

/// The one request `transport` received, or all of them when there was not exactly one.
fn only_request(transport: &FakeTransport) -> Result<RecordedRequest, Vec<RecordedRequest>> {
    <[RecordedRequest; 1]>::try_from(transport.requests()).map(|[request]| request)
}

/// `check_server` against a server that reports `rpc_version_semver`.
fn check_server_reporting(rpc_version_semver: &str) -> Result<x4_rpc::ServerVersion, RpcError> {
    let (mut client, _) = client_answering(json!({
        "version": "4.1.3 (838877323f)",
        "rpc_version_semver": rpc_version_semver,
    }));
    client.check_server()
}

#[test]
fn parses_session_get_fixture() {
    let (mut client, _) = client_answering(fixture_result(SESSION_GET));

    let session = client.session_get().unwrap();

    assert_eq!(session.rpc_version_semver, "6.0.1");
    assert!(session.version.starts_with("4.1.3"), "{session:?}");
    assert_ne!(session.download_dir, "");
}

#[test]
fn session_get_requests_fixed_fields() {
    let (mut client, transport) = client_answering(fixture_result(SESSION_GET));

    client.session_get().unwrap();

    let request = only_request(&transport).unwrap();
    assert_eq!(request.method, "session_get");
    assert_eq!(request.params, json!({ "fields": SESSION_FIELDS }));
}

#[test]
fn session_fields_are_exactly_the_keys_of_the_session_get_fixture() {
    let result = fixture_result(SESSION_GET);

    let mut fixture_keys: Vec<&str> = result
        .as_object()
        .map(|object| object.keys().map(String::as_str).collect())
        .unwrap_or_default();
    fixture_keys.sort_unstable();
    let mut fields = SESSION_FIELDS.to_vec();
    fields.sort_unstable();

    assert_eq!(fixture_keys, fields);
}

#[test]
fn session_get_reads_each_field_from_its_wire_key() {
    // Every value differs, so a field read from the wrong key cannot pass.
    let (mut client, _) = client_answering(json!({
        "version": "4.1.3 (abc)",
        "rpc_version_semver": "6.0.1",
        "download_dir": "/downloads",
        "alt_speed_enabled": true,
        "alt_speed_down": 11,
        "alt_speed_up": 22,
        "speed_limit_down": 33,
        "speed_limit_down_enabled": true,
        "speed_limit_up": 44,
        "speed_limit_up_enabled": false,
        "sequential_download": true,
        "some_future_key": [1, 2, 3],
    }));

    let session = client.session_get().unwrap();

    assert_eq!(
        session,
        SessionInfo {
            version: "4.1.3 (abc)".to_owned(),
            rpc_version_semver: "6.0.1".to_owned(),
            download_dir: "/downloads".to_owned(),
            alt_speed_enabled: true,
            alt_speed_down_kbps: 11,
            alt_speed_up_kbps: 22,
            speed_limit_down_kbps: 33,
            speed_limit_down_enabled: true,
            speed_limit_up_kbps: 44,
            speed_limit_up_enabled: false,
            sequential_download: true,
        }
    );
}

/// A `session_get` result that differs from the recorded one in the four speed-limit fields.
fn session_result_with_speeds(down: &Value, up: &Value) -> Value {
    json!({
        "version": "4.1.0 (abc)",
        "rpc_version_semver": "6.0.0",
        "download_dir": "/downloads",
        "alt_speed_enabled": false,
        "alt_speed_down": down,
        "alt_speed_up": up,
        "speed_limit_down": down,
        "speed_limit_down_enabled": false,
        "speed_limit_up": up,
        "speed_limit_up_enabled": false,
        "sequential_download": false,
    })
}

#[test]
fn session_get_accepts_integer_and_float_speed_limits() {
    // Transmission 4.1.0 (RPC 6.0.0) sent the limits as floats; 4.1.1 went back to integers.
    for (down, up, expected) in [
        (json!(100), json!(50), (100, 50)),
        (json!(100.0), json!(50.0), (100, 50)),
        (json!(100.9), json!(0.5), (100, 0)),
        (json!(0.0), json!(4_294_967_295.0), (0, u32::MAX)),
    ] {
        let (mut client, _) = client_answering(session_result_with_speeds(&down, &up));

        let session = client.session_get().unwrap();

        let read = [
            session.alt_speed_down_kbps,
            session.speed_limit_down_kbps,
            session.alt_speed_up_kbps,
            session.speed_limit_up_kbps,
        ];
        assert_eq!(
            read,
            [expected.0, expected.0, expected.1, expected.1],
            "{down} {up}"
        );
    }
}

#[test]
fn session_get_rejects_speed_limits_that_do_not_fit() {
    for bad in [
        json!(-1),
        json!(-1.5),
        json!(4_294_967_296_u64),
        json!(4_294_967_296.0),
        json!(1e300),
        json!("100"),
        json!(null),
    ] {
        let (mut client, _) = client_answering(session_result_with_speeds(&bad, &json!(1)));

        let error = client.session_get().unwrap_err();

        assert!(
            matches!(error, RpcError::InvalidResponse(_)),
            "{bad}: {error:?}"
        );
    }
}

#[test]
fn session_get_with_a_missing_field_is_an_invalid_response() {
    let (mut client, _) = client_answering(json!({ "version": "4.1.3 (abc)" }));

    let error = client.session_get().unwrap_err();

    assert!(matches!(error, RpcError::InvalidResponse(_)), "{error:?}");
}

#[test]
fn session_set_omits_none() {
    let (mut client, transport) = client_answering(json!({}));

    client
        .session_set(&SessionSet {
            alt_speed_enabled: Some(true),
            ..SessionSet::default()
        })
        .unwrap();

    let request = only_request(&transport).unwrap();
    assert_eq!(request.method, "session_set");
    assert_eq!(request.params, json!({ "alt_speed_enabled": true }));
}

#[test]
fn session_set_sends_the_wire_names() {
    let (mut client, transport) = client_answering(json!({}));

    client
        .session_set(&SessionSet {
            alt_speed_enabled: Some(false),
            speed_limit_down_kbps: Some(500),
            speed_limit_down_enabled: Some(true),
            speed_limit_up_kbps: Some(0),
            speed_limit_up_enabled: Some(false),
        })
        .unwrap();

    let request = only_request(&transport).unwrap();
    assert_eq!(
        request.params,
        json!({
            "alt_speed_enabled": false,
            "speed_limit_down": 500,
            "speed_limit_down_enabled": true,
            "speed_limit_up": 0,
            "speed_limit_up_enabled": false,
        })
    );
}

#[test]
fn parses_session_stats_fixture() {
    let (mut client, transport) = client_answering(fixture_result(SESSION_STATS));

    let stats = client.session_stats().unwrap();

    assert!(stats.torrent_count >= 1, "{stats:?}");
    assert!(
        stats.cumulative_stats.uploaded_bytes >= stats.current_stats.uploaded_bytes,
        "{stats:?}"
    );
    let request = only_request(&transport).unwrap();
    assert_eq!(request.method, "session_stats");
    assert!(
        request.params.is_null(),
        "session_stats takes no parameters"
    );
}

#[test]
fn session_stats_reads_each_field_from_its_wire_key() {
    let (mut client, _) = client_answering(json!({
        "download_speed": 1,
        "upload_speed": 2,
        "active_torrent_count": 3,
        "paused_torrent_count": 4,
        "torrent_count": 5,
        "current_stats": {
            "downloaded_bytes": 10,
            "uploaded_bytes": 20,
            "files_added": 30,
            "seconds_active": 40,
            "session_count": 50,
        },
        "cumulative_stats": {
            "downloaded_bytes": 11,
            "uploaded_bytes": 21,
            "files_added": 31,
            "seconds_active": 41,
            "session_count": 51,
        },
    }));

    let stats = client.session_stats().unwrap();

    assert_eq!(
        stats,
        SessionStats {
            download_speed: 1,
            upload_speed: 2,
            active_torrent_count: 3,
            paused_torrent_count: 4,
            torrent_count: 5,
            current_stats: StatsBlock {
                downloaded_bytes: 10,
                uploaded_bytes: 20,
                files_added: 30,
                seconds_active: 40,
                session_count: 50,
            },
            cumulative_stats: StatsBlock {
                downloaded_bytes: 11,
                uploaded_bytes: 21,
                files_added: 31,
                seconds_active: 41,
                session_count: 51,
            },
        }
    );
}

#[test]
fn parses_free_space_fixture() {
    let (mut client, transport) = client_answering(fixture_result(FREE_SPACE));

    let space = client.free_space("/data/torrents/video").unwrap();

    assert!(space.size_bytes > 0, "{space:?}");
    assert!(space.total_size >= space.size_bytes, "{space:?}");
    assert_eq!(space.path, "/data/torrents/video");
    let request = only_request(&transport).unwrap();
    assert_eq!(request.method, "free_space");
    assert_eq!(request.params, json!({ "path": "/data/torrents/video" }));
}

#[test]
fn free_space_keeps_the_negative_sizes_the_server_reports_on_failure() {
    let (mut client, _) = client_answering(json!({
        "path": "/nowhere",
        "size_bytes": -1,
        "total_size": -1,
    }));

    let space = client.free_space("/nowhere").unwrap();

    assert_eq!((space.size_bytes, space.total_size), (-1, -1));
}

#[test]
fn free_space_error_is_server_error() {
    let body = json!({
        "jsonrpc": "2.0",
        "error": {
            "code": 5,
            "message": "system error",
            "data": {
                "error_string": "No such file or directory",
                "result": { "path": "/nowhere", "size_bytes": -1, "total_size": -1 },
            },
        },
    });
    let transport = FakeTransport::scripted(vec![Ok(HttpResponse {
        status: 200,
        session_id: None,
        body: body.to_string().into_bytes(),
    })]);
    let mut client = Client::new(transport);

    let error = client.free_space("/nowhere").unwrap_err();

    assert_eq!(
        error,
        RpcError::Server {
            code: 5,
            message: "system error".to_owned(),
            detail: Some("No such file or directory".to_owned()),
        }
    );
}

#[test]
fn check_server_accepts_6() {
    let version = check_server_reporting("6.0.1").unwrap();

    assert_eq!(version.rpc_major(), Some(6));
    assert!(version.is_supported());
    assert_eq!(version.rpc_semver, "6.0.1");
    assert!(version.version.starts_with("4.1.3"), "{version:?}");
}

#[test]
fn check_server_requests_only_the_version_fields() {
    let (mut client, transport) = client_answering(json!({
        "version": "4.1.3 (838877323f)",
        "rpc_version_semver": "6.0.1",
    }));

    client.check_server().unwrap();

    let request = only_request(&transport).unwrap();
    assert_eq!(request.method, "session_get");
    assert_eq!(
        request.params,
        json!({ "fields": ["version", "rpc_version_semver"] })
    );
}

#[test]
fn check_server_accepts_a_later_major() {
    let version = check_server_reporting("10.2.0").unwrap();

    assert_eq!(version.rpc_major(), Some(10));
}

#[test]
fn check_server_rejects_5() {
    let error = check_server_reporting("5.4.0").unwrap_err();

    assert_eq!(
        error,
        RpcError::UnsupportedServer {
            reason: UnsupportedReason::Version("5.4.0".to_owned())
        }
    );
}

#[test]
fn check_server_rejects_a_missing_version_as_no_version() {
    let error = check_server_reporting("").unwrap_err();

    assert_eq!(
        error,
        RpcError::UnsupportedServer {
            reason: UnsupportedReason::NoVersion
        }
    );
}

#[test]
fn check_server_rejects_a_version_that_is_not_one_with_the_text_it_sent() {
    let error = check_server_reporting("abc").unwrap_err();

    assert_eq!(
        error,
        RpcError::UnsupportedServer {
            reason: UnsupportedReason::Version("abc".to_owned())
        }
    );
}

#[test]
fn check_server_rejects_legacy_protocol_reply() {
    // What Transmission 4.0 and older answer to `session_get`: HTTP 200, no `jsonrpc`, and a
    // string `result`.
    let transport = FakeTransport::scripted(vec![Ok(HttpResponse {
        status: 200,
        session_id: None,
        body: br#"{"arguments":{},"result":"method name not recognized"}"#.to_vec(),
    })]);
    let mut client = Client::new(transport);

    assert_eq!(
        client.check_server(),
        Err(RpcError::UnsupportedServer {
            reason: UnsupportedReason::Legacy
        })
    );
}

#[test]
fn check_server_passes_call_errors_through() {
    let transport =
        FakeTransport::scripted(vec![Ok(testing::conflict("a")), Ok(testing::conflict("b"))]);
    let mut client = Client::new(transport);

    assert_eq!(client.check_server(), Err(RpcError::SessionIdRejected));
}
