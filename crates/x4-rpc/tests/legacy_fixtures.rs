//! The fixtures recorded from Transmission 3.00, 4.0.5 and 4.1.0-beta.2: a client that checks the
//! server and reads them the way the app reads a real legacy daemon.

use serde_json::Value;
use x4_rpc::testing::fixtures::legacy::{RECORDINGS, Recording};
use x4_rpc::testing::{self, FakeTransport};
use x4_rpc::{
    Client, GetIds, HashString, HttpResponse, Protocol, RpcError, TorrentField, TorrentGetResponse,
    TorrentId, TorrentRecord, TorrentRef, TorrentSelection, TransportError,
};

/// The folder that is not there, whose free space the missing-folder recording answers.
const MISSING_FOLDER: &str = "/data/torrents/video/x4-missing";

fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

/// The recorded answer as an HTTP 200; the transport gives it the request's tag.
fn answer(text: &str) -> HttpResponse {
    let mut body = parse(text);
    if let Value::Object(body) = &mut body {
        body.remove("tag");
    }
    HttpResponse {
        status: 200,
        session_id: None,
        body: body.to_string().into_bytes(),
    }
}

fn asks_for(params: &Value, field: &str) -> bool {
    params["fields"]
        .as_array()
        .is_some_and(|fields| fields.iter().any(|name| name == field))
}

/// A transport that answers as the daemon of `recording` did: JSON-RPC with "method name not
/// recognized", every legacy request with the recorded answer to it.
fn daemon(recording: Recording) -> FakeTransport {
    FakeTransport::routed(move |method, params| match method {
        "session_get" => Ok(testing::legacy_error("method name not recognized")),
        "session-get" if asks_for(params, "download-dir") => Ok(answer(recording.session_get)),
        "session-get" => Ok(answer(recording.session_get_version)),
        "session-stats" => Ok(answer(recording.session_stats)),
        "free-space" if params["path"] == MISSING_FOLDER => {
            Ok(answer(recording.free_space_missing))
        }
        "free-space" => Ok(answer(recording.free_space)),
        "torrent-get" if params["ids"] == "recently-active" => {
            Ok(answer(recording.torrent_get_recent))
        }
        "torrent-get" if asks_for(params, "queuePosition") => {
            Ok(answer(recording.torrent_get_list))
        }
        "torrent-get" if asks_for(params, "comment") => Ok(answer(recording.torrent_get_details)),
        "torrent-get" if asks_for(params, "peers") => Ok(answer(recording.torrent_get_peers)),
        "torrent-get" if asks_for(params, "priorities") => {
            Ok(answer(recording.torrent_get_progress))
        }
        "torrent-get" if asks_for(params, "trackerStats") => {
            Ok(answer(recording.torrent_get_trackers))
        }
        other => Err(TransportError::Io(format!("no recording for {other}"))),
    })
}

/// A client that has checked the server of `recording`.
fn connected(recording: Recording) -> Result<Client<FakeTransport>, RpcError> {
    let mut client = Client::new(daemon(recording));
    client.check_server()?;
    Ok(client)
}

fn one_torrent(
    client: &mut Client<FakeTransport>,
    fields: &[TorrentField],
) -> Result<TorrentRecord, Box<dyn std::error::Error>> {
    let ids = TorrentSelection::new(vec![TorrentRef::Id(TorrentId(1))])?;
    let response = client.torrent_get(&GetIds::Selection(ids), fields)?;
    response
        .torrents
        .into_iter()
        .next()
        .ok_or_else(|| "the answer has no torrent".into())
}

fn everything(
    client: &mut Client<FakeTransport>,
    ids: &GetIds,
) -> Result<TorrentGetResponse, RpcError> {
    client.torrent_get(ids, TorrentField::LIST)
}

fn is_beta(recording: Recording) -> bool {
    recording.name == "v4.1.0-beta.2"
}

#[test]
fn every_recording_connects_as_legacy() {
    for recording in RECORDINGS {
        let mut client = Client::new(daemon(recording));

        let version = client.check_server().unwrap();

        assert_eq!(version.protocol, Protocol::Legacy, "{}", recording.name);
        assert_eq!(client.protocol(), Protocol::Legacy, "{}", recording.name);
        let release = recording.name.trim_start_matches('v');
        assert!(version.version.starts_with(release), "{version:?}");
        assert!(version.rpc_version.is_some(), "{version:?}");
    }
}

#[test]
fn every_recording_reads_the_session() {
    for recording in RECORDINGS {
        let mut client = connected(recording).unwrap();

        let session = client.session_get().unwrap();
        let space = client.free_space("/data/torrents/video").unwrap();
        client.session_stats().unwrap();

        let name = recording.name;
        assert_eq!(
            session.sequential_download.is_some(),
            is_beta(recording),
            "{name}"
        );
        assert_eq!(session.download_dir, "/data/torrents/video", "{name}");
        assert_eq!(space.total_size.is_some(), name != "v3.00", "{name}");
        assert!(space.size_bytes > 0, "{name}");
    }
}

#[test]
fn a_missing_folder_is_a_system_error_everywhere() {
    for recording in RECORDINGS {
        let mut client = connected(recording).unwrap();

        let error = client.free_space(MISSING_FOLDER).unwrap_err();

        let RpcError::Server {
            code,
            message,
            detail,
        } = error
        else {
            panic!("{}: not a server error: {error:?}", recording.name);
        };
        assert_eq!(code, 5, "{}", recording.name);
        assert_eq!(message, "system error", "{}", recording.name);
        // 4.1.0-beta.2 answers a success with -1, which has no system text to carry.
        assert_eq!(detail.is_some(), !is_beta(recording), "{}", recording.name);
    }
}

#[test]
fn every_requested_list_field_arrives() {
    for recording in RECORDINGS {
        let mut client = connected(recording).unwrap();

        for ids in [GetIds::All, GetIds::RecentlyActive] {
            let response = everything(&mut client, &ids).unwrap();

            assert!(!response.torrents.is_empty(), "{}", recording.name);
            for torrent in &response.torrents {
                // One entry per `TorrentField::LIST` field, in its order.
                let present = [
                    ("id", torrent.id.is_some()),
                    ("hash_string", torrent.hash_string.is_some()),
                    ("name", torrent.name.is_some()),
                    ("status", torrent.status.is_some()),
                    ("error", torrent.error.is_some()),
                    ("error_string", torrent.error_string.is_some()),
                    ("percent_done", torrent.percent_done.is_some()),
                    (
                        "metadata_percent_complete",
                        torrent.metadata_percent_complete.is_some(),
                    ),
                    ("rate_download", torrent.rate_download.is_some()),
                    ("rate_upload", torrent.rate_upload.is_some()),
                    ("eta", torrent.eta.is_some()),
                    ("upload_ratio", torrent.upload_ratio.is_some()),
                    ("total_size", torrent.total_size.is_some()),
                    ("size_when_done", torrent.size_when_done.is_some()),
                    ("left_until_done", torrent.left_until_done.is_some()),
                    ("peers_connected", torrent.peers_connected.is_some()),
                    (
                        "peers_getting_from_us",
                        torrent.peers_getting_from_us.is_some(),
                    ),
                    ("peers_sending_to_us", torrent.peers_sending_to_us.is_some()),
                    ("download_dir", torrent.download_dir.is_some()),
                    ("added_date", torrent.added_date.is_some()),
                    ("done_date", torrent.done_date.is_some()),
                    ("queue_position", torrent.queue_position.is_some()),
                    ("recheck_progress", torrent.recheck_progress.is_some()),
                    ("is_finished", torrent.is_finished.is_some()),
                    ("labels", torrent.labels.is_some()),
                ];
                for (field, there) in present {
                    assert!(there, "{}: {field} is missing", recording.name);
                }
                // The only list field a daemon may lack: it came with 4.1.0-beta.
                assert_eq!(
                    torrent.sequential_download.is_some(),
                    is_beta(recording),
                    "{}",
                    recording.name
                );
            }
        }
    }
}

#[test]
fn details_and_progress_arrive_whole() {
    for recording in RECORDINGS {
        let mut client = connected(recording).unwrap();
        let name = recording.name;

        let details = one_torrent(&mut client, TorrentField::DETAILS_FULL).unwrap();
        let progress = one_torrent(&mut client, TorrentField::DETAILS_PROGRESS_TRACKERS).unwrap();

        let file_count = details.file_count.unwrap();
        assert!(file_count > 1, "{name}");
        let piece_count = details.piece_count.unwrap();
        let files = details.files.as_ref().unwrap();
        assert_eq!(files.len(), file_count as usize, "{name}");
        for file in files {
            assert!(file.begin_piece <= file.end_piece, "{name}: {file:?}");
            assert!(file.end_piece <= piece_count, "{name}: {file:?}");
        }
        assert_eq!(details.priorities.unwrap().len(), file_count as usize);
        assert_eq!(details.wanted.unwrap().len(), file_count as usize);
        assert!(!details.pieces.unwrap().is_empty(), "{name}");
        assert!(!details.tracker_stats.unwrap().is_empty(), "{name}");
        // 3.00 does not ask for `source`; the others serve it, empty or not.
        assert_eq!(details.source.is_some(), name != "v3.00", "{name}");

        assert_eq!(progress.file_count, Some(file_count), "{name}");
        assert_eq!(
            progress.bytes_completed.unwrap().len(),
            file_count as usize,
            "{name}"
        );
        assert_eq!(progress.priorities.unwrap().len(), file_count as usize);
        assert_eq!(progress.wanted.unwrap().len(), file_count as usize);
        assert!(!progress.pieces.unwrap().is_empty(), "{name}");
        assert!(!progress.tracker_stats.unwrap().is_empty(), "{name}");
    }
}

#[test]
fn peers_and_trackers_parse() {
    for recording in RECORDINGS {
        let mut client = connected(recording).unwrap();
        let name = recording.name;

        let peers = one_torrent(&mut client, TorrentField::PEERS).unwrap();
        let trackers = one_torrent(&mut client, TorrentField::TRACKERS).unwrap();

        let list = peers.peers.unwrap();
        assert!(!list.is_empty(), "{name}");
        for peer in &list {
            assert!(peer.address.starts_with("192.0.2."), "{name}: {peer:?}");
        }
        assert!(peers.peers_from.is_some(), "{name}");
        let stats = trackers.tracker_stats.unwrap();
        assert!(!stats.is_empty(), "{name}");
        for tracker in &stats {
            assert!(!tracker.host.is_empty(), "{name}: {tracker:?}");
        }
        assert!(
            HashString::parse(&peers.hash_string.unwrap()).is_ok(),
            "{name}"
        );
    }
}

#[test]
fn call_wire_returns_a_legacy_error_answer_whole() {
    let recording = RECORDINGS[0];
    let transport = daemon(recording);
    let mut client = Client::new(transport.clone());
    client.check_server().unwrap();

    let answer = client
        .call_wire(
            "free_space",
            Some(serde_json::json!({ "path": MISSING_FOLDER })),
        )
        .unwrap();

    assert_eq!(answer["result"], "No such file or directory");
    assert_eq!(answer["arguments"]["size-bytes"], -1);
    let requests = transport.requests();
    let last = requests.last().unwrap();
    assert_eq!(last.method, "free-space");
    assert_eq!(last.params["path"], MISSING_FOLDER);
}
