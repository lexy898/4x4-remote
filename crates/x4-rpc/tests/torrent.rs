//! Torrent requests and responses over a fake transport: the parameters each method sends (and
//! never sends: no `ids` meaning "all torrents", no `[]` meaning "all files"), and how the
//! recorded fixtures and synthetic results parse.

use serde_json::{Value, json};
use x4_rpc::testing::fixtures::{TORRENT_GET_DETAILS, TORRENT_GET_LIST, TORRENT_GET_RECENT};
use x4_rpc::testing::{self, FakeTransport, RecordedRequest};
use x4_rpc::{
    AddOutcome, AddSource, AddedTorrent, Client, ErrorKind, GetIds, HashString, Pieces, RemoveData,
    Status, TorrentAction, TorrentAdd, TorrentField, TorrentId, TorrentRef, TorrentSelection,
    TorrentSet,
};

const HASH: &str = "0123456789abcdef0123456789abcdef01234567";

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

#[expect(clippy::unwrap_used, reason = "built from constants that are valid")]
fn selection() -> TorrentSelection {
    TorrentSelection::new(vec![
        TorrentRef::Id(TorrentId(3)),
        TorrentRef::Hash(HashString::parse(HASH).unwrap()),
    ])
    .unwrap()
}

/// The keys of the `params` object, sorted; empty when it is not an object.
fn param_keys(params: &Value) -> Vec<String> {
    let mut keys: Vec<String> = params
        .as_object()
        .map(|object| object.keys().cloned().collect())
        .unwrap_or_default();
    keys.sort();
    keys
}

fn magnet_add() -> TorrentAdd {
    TorrentAdd {
        source: AddSource::Magnet(format!("magnet:?xt=urn:btih:{HASH}")),
        download_dir: None,
        paused: false,
        files_unwanted: Vec::new(),
        priority_high: Vec::new(),
        priority_low: Vec::new(),
        sequential_download: None,
    }
}

#[test]
fn parses_list_fixture() {
    let (mut client, _) = client_answering(fixture_result(TORRENT_GET_LIST));

    let response = client
        .torrent_get(&GetIds::All, TorrentField::LIST)
        .unwrap();

    assert_ne!(response.torrents.len(), 0);
    for torrent in &response.torrents {
        assert!(torrent.id.is_some(), "{torrent:?}");
        assert!(torrent.hash_string.is_some(), "{torrent:?}");
        assert!(torrent.name.is_some(), "{torrent:?}");
        assert!(torrent.status.is_some(), "{torrent:?}");
    }
    assert_eq!(response.removed, Vec::new());
}

#[test]
fn parses_recent_fixture() {
    let (mut client, _) = client_answering(fixture_result(TORRENT_GET_RECENT));

    let response = client
        .torrent_get(&GetIds::RecentlyActive, TorrentField::LIST)
        .unwrap();

    assert!(response.torrents.iter().all(|torrent| torrent.id.is_some()));
}

#[test]
fn parses_details_fixture() {
    let (mut client, _) = client_answering(fixture_result(TORRENT_GET_DETAILS));

    let response = client
        .torrent_get(
            &GetIds::Selection(TorrentSelection::new(vec![TorrentRef::Id(TorrentId(1))]).unwrap()),
            TorrentField::DETAILS_FULL,
        )
        .unwrap();

    let torrent = &response.torrents[0];
    let files = torrent.files.as_ref().unwrap();
    assert!(files.len() > 1);
    assert_eq!(torrent.file_count, u32::try_from(files.len()).ok());
    assert_eq!(torrent.priorities.as_ref().map(Vec::len), Some(files.len()));
    assert_eq!(torrent.wanted.as_ref().map(Vec::len), Some(files.len()));
    let trackers = torrent.tracker_stats.as_ref().unwrap();
    assert!(trackers[0].has_announced);
    assert_eq!(trackers[0].seeder_count, Some(57));
    let piece_count = torrent.piece_count.unwrap();
    // `end_piece` is exclusive: the last file ends at the piece count.
    assert_eq!(files.last().unwrap().end_piece, piece_count);
    let complete = files.iter().all(|file| file.bytes_completed == file.length);
    assert!(complete, "the recorded details torrent is complete");
    let pieces = Pieces::decode_base64(torrent.pieces.as_deref().unwrap()).unwrap();
    assert!(pieces.has(0));
    assert!(pieces.has(piece_count - 1));
    assert!(!pieces.has(piece_count));
}

#[test]
fn parses_recently_active_with_removed() {
    let (mut client, _) = client_answering(json!({ "torrents": [], "removed": [3, 4] }));

    let response = client
        .torrent_get(&GetIds::RecentlyActive, &[TorrentField::Id])
        .unwrap();

    assert_eq!(response.torrents, Vec::new());
    assert_eq!(response.removed, [TorrentId(3), TorrentId(4)]);
}

#[test]
fn get_all_omits_ids() {
    let (mut client, transport) = client_answering(json!({ "torrents": [] }));

    client
        .torrent_get(&GetIds::All, &[TorrentField::Id, TorrentField::HashString])
        .unwrap();

    let request = only_request(&transport).unwrap();
    assert_eq!(request.method, "torrent_get");
    assert_eq!(request.params, json!({ "fields": ["id", "hash_string"] }));
}

#[test]
fn get_recent_sends_recently_active() {
    let (mut client, transport) = client_answering(json!({ "torrents": [], "removed": [] }));

    client
        .torrent_get(&GetIds::RecentlyActive, &[TorrentField::Id])
        .unwrap();

    assert_eq!(
        only_request(&transport).unwrap().params,
        json!({ "ids": "recently_active", "fields": ["id"] })
    );
}

#[test]
fn get_selection_sends_the_id_list() {
    let (mut client, transport) = client_answering(json!({ "torrents": [] }));

    client
        .torrent_get(&GetIds::Selection(selection()), &[TorrentField::Pieces])
        .unwrap();

    assert_eq!(
        only_request(&transport).unwrap().params,
        json!({ "ids": [3, HASH], "fields": ["pieces"] })
    );
}

#[test]
fn actions_always_send_non_empty_ids() {
    let actions = [
        (TorrentAction::Start, "torrent_start"),
        (TorrentAction::StartNow, "torrent_start_now"),
        (TorrentAction::Stop, "torrent_stop"),
        (TorrentAction::Verify, "torrent_verify"),
        (TorrentAction::Reannounce, "torrent_reannounce"),
    ];
    for (action, method) in actions {
        let (mut client, transport) = client_answering(json!({}));

        client.torrent_action(action, &selection()).unwrap();

        let request = only_request(&transport).unwrap();
        assert_eq!(request.method, method, "{action:?}");
        let ids = request.params["ids"].as_array();
        assert!(
            ids.is_some_and(|ids| !ids.is_empty()),
            "{action:?}: {}",
            request.params
        );
        assert_eq!(request.params, json!({ "ids": [3, HASH] }), "{action:?}");
    }
}

#[test]
fn torrent_set_never_serializes_empty_file_lists() {
    let (mut client, transport) = client_answering(json!({}));
    let set = TorrentSet {
        sequential_download: Some(true),
        ..TorrentSet::default()
    };

    client.torrent_set(&selection(), &set).unwrap();

    let request = only_request(&transport).unwrap();
    assert_eq!(request.method, "torrent_set");
    assert_eq!(param_keys(&request.params), ["ids", "sequential_download"]);
    assert_eq!(request.params["sequential_download"], true);
    assert_eq!(request.params["ids"], json!([3, HASH]));
}

#[test]
fn torrent_set_sends_priorities() {
    let (mut client, transport) = client_answering(json!({}));
    let set = TorrentSet {
        priority_high: vec![0, 2],
        ..TorrentSet::default()
    };

    client.torrent_set(&selection(), &set).unwrap();

    assert_eq!(
        only_request(&transport).unwrap().params,
        json!({ "ids": [3, HASH], "priority_high": [0, 2] })
    );
}

#[test]
fn torrent_set_sends_every_non_empty_list() {
    let (mut client, transport) = client_answering(json!({}));
    let set = TorrentSet {
        sequential_download: Some(false),
        files_wanted: vec![0],
        files_unwanted: vec![1],
        priority_high: vec![2],
        priority_normal: vec![3],
        priority_low: vec![4],
    };

    client.torrent_set(&selection(), &set).unwrap();

    assert_eq!(
        only_request(&transport).unwrap().params,
        json!({
            "ids": [3, HASH],
            "sequential_download": false,
            "files_wanted": [0],
            "files_unwanted": [1],
            "priority_high": [2],
            "priority_normal": [3],
            "priority_low": [4],
        })
    );
}

#[test]
fn remove_sends_delete_flag() {
    for (data, delete) in [(RemoveData::Delete, true), (RemoveData::Keep, false)] {
        let (mut client, transport) = client_answering(json!({}));

        client.torrent_remove(&selection(), data).unwrap();

        let request = only_request(&transport).unwrap();
        assert_eq!(request.method, "torrent_remove");
        assert_eq!(
            request.params,
            json!({ "ids": [3, HASH], "delete_local_data": delete }),
            "{data:?}"
        );
    }
}

#[test]
fn add_magnet_uses_filename() {
    let magnet = format!("magnet:?xt=urn:btih:{HASH}");
    let (mut client, transport) = client_answering(json!({
        "torrent_added": { "id": 9, "name": "x", "hash_string": HASH }
    }));
    let add = TorrentAdd {
        paused: true,
        download_dir: Some("/data/torrents/video".to_owned()),
        ..magnet_add()
    };

    client.torrent_add(&add).unwrap();

    let request = only_request(&transport).unwrap();
    assert_eq!(request.method, "torrent_add");
    assert_eq!(
        request.params,
        json!({ "filename": magnet, "paused": true, "download_dir": "/data/torrents/video" })
    );
}

#[test]
fn add_omits_download_dir_when_none_and_sends_file_choices() {
    let (mut client, transport) = client_answering(json!({
        "torrent_added": { "id": 9, "name": "x", "hash_string": HASH }
    }));
    let add = TorrentAdd {
        files_unwanted: vec![1],
        priority_high: vec![0],
        priority_low: vec![2, 3],
        sequential_download: Some(true),
        ..magnet_add()
    };

    client.torrent_add(&add).unwrap();

    let params = only_request(&transport).unwrap().params;
    assert_eq!(
        param_keys(&params),
        [
            "filename",
            "files_unwanted",
            "paused",
            "priority_high",
            "priority_low",
            "sequential_download"
        ]
    );
    assert_eq!(params["paused"], false);
    assert_eq!(params["files_unwanted"], json!([1]));
    assert_eq!(params["priority_high"], json!([0]));
    assert_eq!(params["priority_low"], json!([2, 3]));
    assert_eq!(params["sequential_download"], true);
}

#[test]
fn add_metainfo_is_base64() {
    let metainfo = b"d4:infod6:lengthi3e4:name5:x.bine12:piece lengthi16384eee".to_vec();
    let (mut client, transport) = client_answering(json!({
        "torrent_added": { "id": 9, "name": "x.bin", "hash_string": HASH }
    }));
    let add = TorrentAdd {
        source: AddSource::Metainfo(metainfo),
        ..magnet_add()
    };

    let outcome = client.torrent_add(&add).unwrap();

    let params = only_request(&transport).unwrap().params;
    assert_eq!(
        params,
        json!({
            "metainfo": "ZDQ6aW5mb2Q2Omxlbmd0aGkzZTQ6bmFtZTU6eC5iaW5lMTI6cGllY2UgbGVuZ3RoaTE2Mzg0ZWVl",
            "paused": false,
        })
    );
    assert_eq!(
        outcome,
        AddOutcome::Added(AddedTorrent {
            id: TorrentId(9),
            name: "x.bin".to_owned(),
            hash: HashString::parse(HASH).unwrap(),
        })
    );
}

#[test]
fn add_duplicate_is_reported() {
    let (mut client, _) = client_answering(json!({
        "torrent_duplicate": { "id": 1, "name": "x", "hash_string": HASH }
    }));

    let outcome = client.torrent_add(&magnet_add()).unwrap();

    assert_eq!(
        outcome,
        AddOutcome::Duplicate(AddedTorrent {
            id: TorrentId(1),
            name: "x".to_owned(),
            hash: HashString::parse(HASH).unwrap(),
        })
    );
}

#[test]
fn add_without_an_outcome_is_invalid() {
    let (mut client, _) = client_answering(json!({}));

    let outcome = client.torrent_add(&magnet_add());

    assert!(
        matches!(outcome, Err(x4_rpc::RpcError::InvalidResponse(_))),
        "{outcome:?}"
    );
}

#[test]
fn unknown_status_is_preserved() {
    let (mut client, _) = client_answering(json!({
        "torrents": [
            { "id": 1, "status": 9, "error": 7 },
            { "id": 2, "status": 6, "error": 2 },
        ]
    }));

    let response = client
        .torrent_get(&GetIds::All, &[TorrentField::Status, TorrentField::Error])
        .unwrap();

    assert_eq!(response.torrents[0].status, Some(Status::Other(9)));
    assert_eq!(response.torrents[0].error, Some(ErrorKind::Other(7)));
    assert_eq!(response.torrents[1].status, Some(Status::Seeding));
    assert_eq!(response.torrents[1].error, Some(ErrorKind::TrackerError));
}

#[test]
fn malformed_hash_does_not_fail_the_list() {
    let (mut client, _) = client_answering(json!({
        "torrents": [
            { "id": 1, "hash_string": "xyz" },
            { "id": 2, "hash_string": HASH },
        ]
    }));

    let response = client
        .torrent_get(&GetIds::All, &[TorrentField::Id, TorrentField::HashString])
        .unwrap();

    assert_eq!(response.torrents.len(), 2);
    assert_eq!(response.torrents[0].hash_string.as_deref(), Some("xyz"));
    assert_eq!(response.torrents[1].hash_string.as_deref(), Some(HASH));
}

#[test]
fn malformed_hash_in_an_add_response_is_invalid() {
    let (mut client, _) = client_answering(json!({
        "torrent_added": { "id": 9, "name": "x", "hash_string": "xyz" }
    }));

    let outcome = client.torrent_add(&magnet_add());

    assert!(
        matches!(outcome, Err(x4_rpc::RpcError::InvalidResponse(_))),
        "{outcome:?}"
    );
}

#[test]
fn pieces_bit_order_is_msb_first() {
    let pieces = Pieces::decode_base64("gA==").unwrap();

    assert!(pieces.has(0));
    assert!(!pieces.has(1));
    assert!(!pieces.has(7));
    assert!(!pieces.has(8), "out of range is absent");
    assert_eq!(pieces, Pieces::from_bytes(vec![0b1000_0000]));
}
