//! Torrent requests and responses over a fake transport: the parameters each method sends (and
//! never sends: no `ids` meaning "all torrents", no `[]` meaning "all files"), and how the
//! recorded fixtures and synthetic results parse.

use std::sync::Arc;

use serde_json::{Value, json};
use x4_rpc::testing::fixtures::{
    TORRENT_GET_DETAILS, TORRENT_GET_LIST, TORRENT_GET_PEERS, TORRENT_GET_RECENT,
    TORRENT_GET_TRACKERS,
};
use x4_rpc::testing::{self, FakeTransport, RecordedRequest};
use x4_rpc::{
    AddOutcome, AddSource, AddedTorrent, Client, DownloadDir, ErrorKind, GetIds, HashString,
    HttpResponse, MagnetLink, Pieces, Protocol, RemoveData, RpcError, Status, TorrentAction,
    TorrentAdd, TorrentField, TorrentId, TorrentRef, TorrentSelection, TorrentSet,
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

#[expect(clippy::unwrap_used, reason = "built from a constant that is valid")]
fn magnet_add() -> TorrentAdd {
    TorrentAdd {
        source: AddSource::Magnet(
            MagnetLink::parse(&format!("magnet:?xt=urn:btih:{HASH}")).unwrap(),
        ),
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
fn parses_the_peers_fixture() {
    let (mut client, _) = client_answering(fixture_result(TORRENT_GET_PEERS));

    let response = client
        .torrent_get(&GetIds::Selection(selection()), TorrentField::PEERS)
        .unwrap();

    let torrent = &response.torrents[0];
    let peers = torrent.peers.as_ref().unwrap();
    assert_ne!(peers.len(), 0);
    for peer in peers {
        assert!(peer.address.starts_with("192.0.2."), "{peer:?}");
        assert_ne!(peer.port, 0, "{peer:?}");
        assert!(peer.client_name.starts_with("Client "), "{peer:?}");
        assert!(!peer.flag_str.is_empty(), "{peer:?}");
        assert!((0.0..=1.0).contains(&peer.progress), "{peer:?}");
    }
    let from = torrent.peers_from.unwrap();
    let total = from.from_cache
        + from.from_dht
        + from.from_incoming
        + from.from_lpd
        + from.from_ltep
        + from.from_pex
        + from.from_tracker;
    assert_eq!(usize::try_from(total).ok(), Some(peers.len()));
    assert!(torrent.tracker_stats.is_none());
}

#[test]
fn parses_the_trackers_fixture() {
    let (mut client, _) = client_answering(fixture_result(TORRENT_GET_TRACKERS));

    let response = client
        .torrent_get(&GetIds::Selection(selection()), TorrentField::TRACKERS)
        .unwrap();

    let torrent = &response.torrents[0];
    let trackers = torrent.tracker_stats.as_ref().unwrap();
    assert_ne!(trackers.len(), 0);
    for tracker in trackers {
        assert!(tracker.host.ends_with(".example.org:80"), "{tracker:?}");
        assert_eq!(tracker.sitename, "example");
        assert!(tracker.id.0 > 0, "{tracker:?}");
        assert!(!tracker.is_backup, "{tracker:?}");
        assert!(tracker.has_announced, "{tracker:?}");
    }
    assert!(
        trackers
            .iter()
            .any(|tracker| tracker.last_announce_succeeded)
    );
    assert!(torrent.peers.is_none());
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
fn a_peers_read_names_the_peers_and_where_they_came_from() {
    let (mut client, transport) = client_answering(json!({ "torrents": [] }));

    client
        .torrent_get(&GetIds::Selection(selection()), TorrentField::PEERS)
        .unwrap();

    assert_eq!(
        only_request(&transport).unwrap().params,
        json!({ "ids": [3, HASH], "fields": ["id", "hash_string", "peers", "peers_from"] })
    );
}

#[test]
fn a_trackers_read_names_the_tracker_stats_only() {
    let (mut client, transport) = client_answering(json!({ "torrents": [] }));

    client
        .torrent_get(&GetIds::Selection(selection()), TorrentField::TRACKERS)
        .unwrap();

    assert_eq!(
        only_request(&transport).unwrap().params,
        json!({ "ids": [3, HASH], "fields": ["id", "hash_string", "tracker_stats"] })
    );
}

/// A peer entry with the keys the client reads.
fn peer_entry(address: &str) -> Value {
    json!({
        "address": address,
        "port": 51413,
        "client_name": "Client/1.0",
        "flag_str": "TKEX",
        "progress": 1.0,
        "rate_to_client": 0,
        "rate_to_peer": 0,
        "is_encrypted": true,
        "is_utp": true,
    })
}

#[test]
fn an_odd_peer_entry_does_not_fail_the_whole_read() {
    let mut odd_port = peer_entry("192.0.2.2");
    odd_port["port"] = json!("x");
    let mut no_address = peer_entry("192.0.2.3");
    no_address.as_object_mut().unwrap().remove("address");
    let (mut client, _) = client_answering(json!({
        "torrents": [{
            "id": 3,
            "peers": [peer_entry("192.0.2.1"), odd_port, no_address, null, 7],
            "peers_from": { "from_dht": "x", "from_pex": 4 },
        }]
    }));

    let response = client
        .torrent_get(&GetIds::Selection(selection()), TorrentField::PEERS)
        .unwrap();

    let torrent = &response.torrents[0];
    let peers = torrent.peers.as_ref().unwrap();
    assert_eq!(peers.len(), 5);
    assert_eq!(
        (peers[0].address.as_str(), peers[0].port),
        ("192.0.2.1", 51413)
    );
    // The odd key holds its default; the entry's other keys are still read.
    assert_eq!((peers[1].address.as_str(), peers[1].port), ("192.0.2.2", 0));
    assert_eq!(peers[1].client_name, "Client/1.0");
    assert!(peers[1].is_utp);
    assert_eq!((peers[2].address.as_str(), peers[2].port), ("", 51413));
    assert_eq!(peers[2].flag_str, "TKEX");
    // Entries that are not objects at all are empty records.
    assert_eq!(peers[3], x4_rpc::PeerRecord::default());
    assert_eq!(peers[4], x4_rpc::PeerRecord::default());
    let from = torrent.peers_from.as_ref().unwrap();
    assert_eq!((from.from_dht, from.from_pex), (0, 4));
}

#[test]
fn peers_that_are_not_a_list_read_as_absent_and_do_not_fail_the_read() {
    let (mut client, _) = client_answering(json!({
        "torrents": [{ "id": 3, "peers": "none", "peers_from": "none" }]
    }));

    let response = client
        .torrent_get(&GetIds::Selection(selection()), TorrentField::PEERS)
        .unwrap();

    assert_eq!(response.torrents[0].peers, None);
    assert_eq!(response.torrents[0].peers_from, None);
}

#[test]
fn an_odd_tracker_entry_does_not_fail_the_whole_read() {
    let (mut client, _) = client_answering(json!({
        "torrents": [{
            "id": 3,
            "tracker_stats": [
                { "id": 1, "tier": 0, "host": "a.example:80", "announce_state": 1 },
                { "id": "x", "tier": -1, "host": "b.example:80", "announce_state": "soon" },
                { "host": "c.example:80", "next_announce_time": [] },
                7,
            ],
        }]
    }));

    let response = client
        .torrent_get(&GetIds::Selection(selection()), TorrentField::TRACKERS)
        .unwrap();

    let trackers = response.torrents[0].tracker_stats.as_ref().unwrap();
    let hosts: Vec<&str> = trackers
        .iter()
        .map(|tracker| tracker.host.as_str())
        .collect();
    assert_eq!(hosts, ["a.example:80", "b.example:80", "c.example:80", ""]);
    assert_eq!(trackers[0].announce_state, x4_rpc::TrackerState::Waiting);
    assert_eq!(trackers[1].id, x4_rpc::TrackerId(0));
    assert_eq!(trackers[1].announce_state, x4_rpc::TrackerState::Inactive);
    assert_eq!(trackers[2].next_announce_time, 0);
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
fn the_add_request_sends_a_magnet_as_filename_and_a_file_as_metainfo() {
    let sent = |source: AddSource| {
        let (mut client, transport) = client_answering(json!({
            "torrent_added": { "id": 9, "name": "x", "hash_string": HASH }
        }));
        let add = TorrentAdd {
            source,
            paused: true,
            download_dir: Some(DownloadDir::new("//data/torrents//video/\n").unwrap()),
            ..magnet_add()
        };
        client.torrent_add(&add).unwrap();
        let request = only_request(&transport).unwrap();
        assert_eq!(request.method, "torrent_add");
        request.params
    };
    let magnet = MagnetLink::parse(&format!(
        " MAGNET:?xt=urn:btih:{HASH}&dn=a%20b&tr=udp%3A%2F%2Fx \n"
    ))
    .unwrap();

    // The magnet goes as it was validated: trimmed, with the scheme in lower case.
    assert_eq!(
        sent(AddSource::Magnet(magnet)),
        json!({
            "filename": format!("magnet:?xt=urn:btih:{HASH}&dn=a%20b&tr=udp%3A%2F%2Fx"),
            "paused": true,
            "download_dir": "/data/torrents/video",
        })
    );
    // A file goes as its content, never as a name the daemon would read on its own disk.
    assert_eq!(
        sent(AddSource::Metainfo(Arc::from(&b"d4:infod4:name1:xee"[..]))),
        json!({
            "metainfo": "ZDQ6aW5mb2Q0Om5hbWUxOnhlZQ==",
            "paused": true,
            "download_dir": "/data/torrents/video",
        })
    );
}

#[test]
fn add_omits_download_dir_when_none_and_sends_file_choices() {
    let (mut client, transport) = client_answering(json!({
        "torrent_added": { "id": 9, "name": "x", "hash_string": HASH }
    }));
    let add = TorrentAdd {
        source: AddSource::Metainfo(Arc::from(&b"d4:infod4:name1:xee"[..])),
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
            "files_unwanted",
            "metainfo",
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
fn a_magnet_with_file_choices_is_refused_and_nothing_is_sent() {
    for add in [
        TorrentAdd {
            files_unwanted: vec![1],
            ..magnet_add()
        },
        TorrentAdd {
            priority_high: vec![0],
            ..magnet_add()
        },
        TorrentAdd {
            priority_low: vec![2],
            ..magnet_add()
        },
    ] {
        let (mut client, transport) = client_answering(json!({
            "torrent_added": { "id": 9, "name": "x", "hash_string": HASH }
        }));

        assert_eq!(
            client.torrent_add(&add),
            Err(RpcError::MagnetWithFileChoices),
            "{add:?}"
        );
        assert_eq!(transport.requests(), [], "{add:?}");
    }
}

#[test]
fn add_metainfo_is_base64() {
    let metainfo = b"d4:infod6:lengthi3e4:name5:x.bine12:piece lengthi16384eee".to_vec();
    let (mut client, transport) = client_answering(json!({
        "torrent_added": { "id": 9, "name": "x.bin", "hash_string": HASH }
    }));
    let add = TorrentAdd {
        source: AddSource::Metainfo(Arc::from(metainfo)),
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
        matches!(outcome, Err(RpcError::InvalidResponse(_))),
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
        matches!(outcome, Err(RpcError::InvalidResponse(_))),
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

/// A client that has found a legacy daemon of the RPC version `rpc_version`, which answers
/// every JSON-RPC request as one it does not know and every legacy request but `check_server`'s
/// with `answer(method, arguments)`, and its transport.
fn legacy_client_of(
    rpc_version: u32,
    answer: impl Fn(&str, &Value) -> HttpResponse + Send + 'static,
) -> (Client<FakeTransport>, FakeTransport) {
    let version_fields = json!(["version", "rpc-version", "rpc-version-semver"]);
    let transport = FakeTransport::routed(move |method, arguments| {
        Ok(if method.contains('_') {
            testing::legacy_error("method name not recognized")
        } else if method == "session-get" && arguments["fields"] == version_fields {
            testing::legacy_ok(
                json!({ "version": "4.0.5 (a6fe2a64aa)", "rpc-version": rpc_version }),
            )
        } else {
            answer(method, arguments)
        })
    });
    let mut client = Client::new(transport.clone());
    assert_eq!(
        client.check_server().map(|server| server.protocol),
        Ok(Protocol::Legacy)
    );
    (client, transport)
}

/// A client that has found a Transmission 4.0.5 (legacy RPC version 17) behind `answer`, and
/// its transport.
fn legacy_client(
    answer: impl Fn(&str, &Value) -> HttpResponse + Send + 'static,
) -> (Client<FakeTransport>, FakeTransport) {
    legacy_client_of(17, answer)
}

/// The last request `transport` received.
fn last_request(transport: &FakeTransport) -> Option<RecordedRequest> {
    transport.requests().pop()
}

#[test]
fn legacy_torrent_get_sends_legacy_names() {
    // Transmission 4.1.0-beta (legacy RPC version 18) has every field of the list.
    let (mut client, transport) = legacy_client_of(18, |_, _| {
        testing::legacy_ok(json!({
            "torrents": [{
                "id": 1,
                "hashString": HASH,
                "downloadDir": "/downloads",
                "percentDone": 0.5,
                "rateDownload": 7,
                "peersGettingFromUs": 2,
                "isFinished": false,
                "sequential_download": true,
            }],
            "removed": [4],
        }))
    });

    let response = client
        .torrent_get(&GetIds::RecentlyActive, TorrentField::LIST)
        .unwrap();

    let request = last_request(&transport).unwrap();
    assert_eq!(request.method, "torrent-get");
    assert_eq!(request.params["ids"], "recently-active");
    assert_eq!(
        request.params["fields"],
        json!([
            "id",
            "hashString",
            "name",
            "status",
            "error",
            "errorString",
            "percentDone",
            "metadataPercentComplete",
            "rateDownload",
            "rateUpload",
            "eta",
            "uploadRatio",
            "totalSize",
            "sizeWhenDone",
            "leftUntilDone",
            "peersConnected",
            "peersGettingFromUs",
            "peersSendingToUs",
            "downloadDir",
            "addedDate",
            "doneDate",
            "queuePosition",
            "sequential_download",
            "recheckProgress",
            "isFinished",
            "labels",
        ])
    );
    let [torrent] = response.torrents.as_slice() else {
        panic!("one torrent: {response:?}");
    };
    assert_eq!(torrent.id, Some(TorrentId(1)));
    assert_eq!(torrent.hash_string.as_deref(), Some(HASH));
    assert_eq!(torrent.download_dir.as_deref(), Some("/downloads"));
    assert_eq!(torrent.percent_done, Some(0.5));
    assert_eq!(torrent.rate_download, Some(7));
    assert_eq!(torrent.peers_getting_from_us, Some(2));
    assert_eq!(torrent.is_finished, Some(false));
    assert_eq!(torrent.sequential_download, Some(true));
    assert_eq!(response.removed, [TorrentId(4)]);
}

#[test]
fn legacy_list_read_on_rpc_16_never_asks_for_newer_names() {
    // Transmission 3.00 would answer `sequential_download` with 0, as any field it does not know.
    let (mut client, transport) = legacy_client_of(16, |_, _| {
        testing::legacy_ok(json!({
            "torrents": [{ "id": 1, "hashString": HASH, "name": "x", "percentDone": 0.5 }],
            "removed": [],
        }))
    });

    let response = client.torrent_get(&GetIds::RecentlyActive, TorrentField::LIST);

    let request = last_request(&transport).unwrap();
    let fields = request.params["fields"].as_array().unwrap();
    assert!(
        !fields.contains(&json!("sequential_download")),
        "{fields:?}"
    );
    assert_eq!(fields.len(), TorrentField::LIST.len() - 1);
    let response = response.unwrap();
    let [torrent] = response.torrents.as_slice() else {
        panic!("one torrent: {response:?}");
    };
    assert_eq!(torrent.percent_done, Some(0.5));
    assert_eq!(torrent.sequential_download, None);
}

#[test]
fn legacy_list_read_asks_for_sequential_download_from_rpc_18() {
    for (rpc_version, asked) in [(16, false), (17, false), (18, true), (19, true)] {
        let (mut client, transport) = legacy_client_of(rpc_version, |_, _| {
            testing::legacy_ok(json!({ "torrents": [] }))
        });

        client
            .torrent_get(&GetIds::All, TorrentField::LIST)
            .unwrap();

        let request = last_request(&transport).unwrap();
        let fields = request.params["fields"].as_array().unwrap();
        assert_eq!(
            fields.contains(&json!("sequential_download")),
            asked,
            "RPC {rpc_version}: {fields:?}"
        );
    }
}

#[test]
fn legacy_torrent_add_sends_kebab_download_dir() {
    let added = json!({ "hashString": HASH, "id": 1, "name": "x" });
    let torrent = AddedTorrent {
        id: TorrentId(1),
        name: "x".to_owned(),
        hash: HashString::parse(HASH).unwrap(),
    };
    let add = TorrentAdd {
        source: AddSource::Metainfo(Arc::from(&b"d4:infod4:name1:xee"[..])),
        download_dir: Some(DownloadDir::new("/data/new").unwrap()),
        files_unwanted: vec![1],
        ..magnet_add()
    };
    for (key, expected) in [
        ("torrent-added", AddOutcome::Added(torrent.clone())),
        ("torrent-duplicate", AddOutcome::Duplicate(torrent.clone())),
    ] {
        let answer = json!({ key: added.clone() });
        let (mut client, transport) = legacy_client(move |_, _| testing::legacy_ok(answer.clone()));

        let outcome = client.torrent_add(&add);

        assert_eq!(outcome, Ok(expected), "{key}");
        let request = last_request(&transport).unwrap();
        assert_eq!(request.method, "torrent-add");
        assert_eq!(
            param_keys(&request.params),
            ["download-dir", "files-unwanted", "metainfo", "paused"]
        );
        assert_eq!(request.params["download-dir"], "/data/new");
        assert_eq!(request.params["files-unwanted"], json!([1]));
    }
}

#[test]
fn legacy_torrent_remove_sends_delete_local_data() {
    let (mut client, transport) = legacy_client(|_, _| testing::legacy_ok(json!({})));

    client
        .torrent_remove(&selection(), RemoveData::Delete)
        .unwrap();

    let request = last_request(&transport).unwrap();
    assert_eq!(request.method, "torrent-remove");
    assert_eq!(
        request.params,
        json!({ "ids": [3, HASH], "delete-local-data": true })
    );
}

#[test]
fn legacy_torrent_set_and_actions_send_legacy_names() {
    let (mut client, transport) = legacy_client(|_, _| testing::legacy_ok(json!({})));

    client
        .torrent_set(
            &selection(),
            &TorrentSet {
                files_wanted: vec![0],
                priority_normal: vec![1],
                ..TorrentSet::default()
            },
        )
        .unwrap();
    client
        .torrent_action(TorrentAction::StartNow, &selection())
        .unwrap();

    let requests = transport.requests();
    let [.., set, start] = requests.as_slice() else {
        panic!("two more requests: {requests:?}");
    };
    assert_eq!(set.method, "torrent-set");
    assert_eq!(
        set.params,
        json!({ "ids": [3, HASH], "files-wanted": [0], "priority-normal": [1] })
    );
    assert_eq!(start.method, "torrent-start-now");
    assert_eq!(start.params, json!({ "ids": [3, HASH] }));
}

/// The fields the last legacy `torrent-get` asked for.
fn asked_fields(transport: &FakeTransport) -> Value {
    last_request(transport).map_or(Value::Null, |request| {
        request.params.get("fields").cloned().unwrap_or_default()
    })
}

#[expect(clippy::unwrap_used, reason = "built from constants that are valid")]
fn one_torrent_selection() -> GetIds {
    GetIds::Selection(TorrentSelection::new(vec![TorrentRef::Id(TorrentId(1))]).unwrap())
}

#[test]
fn legacy_progress_read_takes_bytes_completed_from_file_stats() {
    let (mut client, transport) = legacy_client_of(16, |_, _| {
        testing::legacy_ok(json!({ "torrents": [{
            "id": 1,
            "hashString": HASH,
            "fileStats": [
                { "bytesCompleted": 6, "wanted": true, "priority": 0 },
                { "bytesCompleted": 0, "wanted": false, "priority": 1 },
            ],
            "wanted": [1, 0],
            "priorities": [0, 1],
        }] }))
    });

    let response = client
        .torrent_get(&one_torrent_selection(), TorrentField::DETAILS_PROGRESS)
        .unwrap();

    let fields = asked_fields(&transport);
    assert!(fields.as_array().unwrap().contains(&json!("fileStats")));
    assert!(!fields.to_string().contains("bytes_completed"));
    assert!(!fields.to_string().contains("bytesCompleted"));
    assert!(!fields.to_string().contains("file-count"));
    let torrent = &response.torrents[0];
    assert_eq!(torrent.bytes_completed, Some(vec![6, 0]));
    assert_eq!(torrent.file_stats, None);
    assert_eq!(torrent.wanted, Some(vec![true, false]));
    assert_eq!(torrent.file_count, Some(2));
}

/// A legacy daemon's answer to `DETAILS_FULL` for a torrent of two files.
fn legacy_details(files: &Value) -> Value {
    json!({ "torrents": [{
        "id": 1,
        "hashString": HASH,
        "pieceSize": 16384,
        "pieceCount": 3,
        "files": files,
        "priorities": [0, 0],
        "wanted": [1, 1],
    }] })
}

#[test]
fn legacy_details_read_computes_piece_spans() {
    let (mut client, transport) = legacy_client_of(16, |_, _| {
        testing::legacy_ok(legacy_details(&json!([
            { "name": "a", "length": 16384, "bytesCompleted": 1 },
            { "name": "b", "length": 20000, "bytesCompleted": 2 },
        ])))
    });

    let response = client
        .torrent_get(&one_torrent_selection(), TorrentField::DETAILS_FULL)
        .unwrap();

    let fields = asked_fields(&transport);
    for name in ["files", "pieceSize", "pieceCount"] {
        assert!(fields.as_array().unwrap().contains(&json!(name)), "{name}");
    }
    assert!(!fields.as_array().unwrap().contains(&json!("source")));
    let files = response.torrents[0].files.as_ref().unwrap();
    assert_eq!((files[0].begin_piece, files[0].end_piece), (0, 1));
    assert_eq!((files[1].begin_piece, files[1].end_piece), (1, 3));
    assert_eq!(response.torrents[0].file_count, Some(2));
}

#[test]
fn legacy_details_ask_for_source_from_rpc_17() {
    let sent = |rpc_version| {
        let (mut client, transport) = legacy_client_of(rpc_version, |_, _| {
            testing::legacy_ok(json!({ "torrents": [] }))
        });
        client
            .torrent_get(&one_torrent_selection(), TorrentField::DETAILS_FULL)
            .unwrap();
        asked_fields(&transport)
            .as_array()
            .unwrap()
            .contains(&json!("source"))
    };

    assert!(!sent(16));
    assert!(sent(17));
}

#[test]
fn legacy_details_keep_the_spans_a_beta_serves() {
    let (mut client, _) = legacy_client_of(18, |_, _| {
        testing::legacy_ok(legacy_details(&json!([
            { "name": "a", "length": 16384, "bytesCompleted": 1, "begin_piece": 0, "end_piece": 1 },
            { "name": "b", "length": 20000, "bytesCompleted": 2, "begin_piece": 5, "end_piece": 9 },
        ])))
    });

    let response = client
        .torrent_get(&one_torrent_selection(), TorrentField::DETAILS_FULL)
        .unwrap();

    let files = response.torrents[0].files.as_ref().unwrap();
    assert_eq!((files[1].begin_piece, files[1].end_piece), (5, 9));
}

#[test]
fn legacy_magnet_without_metadata_reads_empty_files() {
    let (mut client, _) = legacy_client_of(16, |_, _| {
        testing::legacy_ok(json!({ "torrents": [{
            "id": 1,
            "hashString": HASH,
            "pieceSize": 0,
            "pieceCount": 0,
            "files": [],
            "priorities": [],
            "wanted": [],
        }] }))
    });

    let response = client
        .torrent_get(&one_torrent_selection(), TorrentField::DETAILS_FULL)
        .unwrap();

    let torrent = &response.torrents[0];
    assert_eq!(torrent.files, Some(vec![]));
    assert_eq!(torrent.file_count, Some(0));
}

#[test]
fn legacy_files_without_a_piece_size_are_an_invalid_response() {
    let (mut client, _) = legacy_client_of(16, |_, _| {
        testing::legacy_ok(json!({ "torrents": [{
            "id": 1,
            "hashString": HASH,
            "pieceSize": 0,
            "pieceCount": 0,
            "files": [{ "name": "a", "length": 5, "bytesCompleted": 0 }],
        }] }))
    });

    let result = client.torrent_get(&one_torrent_selection(), TorrentField::DETAILS_FULL);

    assert_eq!(
        result,
        Err(RpcError::InvalidResponse(
            "torrent_get: files without a piece size".to_owned()
        ))
    );
}
