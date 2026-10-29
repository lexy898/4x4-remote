//! The synthetic daemon, through the real client: what it answers and what it refuses.

use std::collections::HashSet;

use x4_core::{Attention, DisplayState, QueueKind, Torrent as ShownTorrent};
use x4_rpc::{Client, GetIds, TorrentField, TorrentGetResponse, TorrentId, TorrentRecord};

use super::*;

const COUNT: u32 = 2000;

fn client(activity: Activity) -> Client<SyntheticDaemon> {
    Client::new(SyntheticDaemon::new(COUNT, activity))
}

fn list(client: &mut Client<SyntheticDaemon>, ids: &GetIds) -> TorrentGetResponse {
    client
        .torrent_get(ids, TorrentField::LIST)
        .expect("the client parses the list")
}

/// The torrents of a list as the app shows them; every record must convert.
fn shown(response: TorrentGetResponse) -> Vec<ShownTorrent> {
    let converted = x4_core::torrents_from_records(response.torrents);
    assert!(converted.skipped.is_empty(), "{:?}", converted.skipped);
    converted.torrents
}

#[test]
fn every_read_of_the_engine_gets_an_answer_the_client_parses() {
    let mut client = client(Activity::Ticking);

    let server = client.check_server().expect("a supported server");
    assert_eq!(server.rpc_major(), Some(6));
    let session = client.session_get().expect("the session settings");
    assert_eq!(session.download_dir, DOWNLOAD_DIR);
    let stats = client.session_stats().expect("the statistics");
    assert_eq!(stats.torrent_count, COUNT);
    assert!(stats.download_speed > 0 && stats.upload_speed > 0);
    let free = client.free_space(DOWNLOAD_DIR).expect("the free space");
    assert_eq!(free.size_bytes, 800 << 30);

    let all = shown(list(&mut client, &GetIds::All));
    assert_eq!(all.len(), 2000);
    let first = all.first().expect("a first torrent");
    let last = all.last().expect("a last torrent");
    assert_eq!(first.name, "Synthetic torrent 0001");
    assert_eq!(last.name, "Synthetic torrent 2000");
    let hashes: HashSet<_> = all.iter().map(|torrent| torrent.hash.clone()).collect();
    assert_eq!(hashes.len(), all.len(), "every hash is unique");
}

#[test]
fn the_torrents_come_in_every_state() {
    let all = shown(list(&mut client(Activity::Ticking), &GetIds::All));
    let states: HashSet<DisplayState> = all.iter().map(|torrent| torrent.state).collect();

    for state in [
        DisplayState::Downloading,
        DisplayState::Seeding,
        DisplayState::Paused,
        DisplayState::Queued(QueueKind::Download),
        DisplayState::Queued(QueueKind::Seed),
        DisplayState::Checking,
        DisplayState::Error,
    ] {
        assert!(states.contains(&state), "no {state:?} torrent");
    }
    let has = |wanted: fn(&Attention) -> bool| {
        all.iter()
            .any(|torrent| torrent.attention.as_ref().is_some_and(wanted))
    };
    assert!(has(|attention| matches!(
        attention,
        Attention::TrackerWarning(_)
    )));
    assert!(has(|attention| matches!(
        attention,
        Attention::TrackerError(_)
    )));
}

#[test]
fn rates_change_on_every_poll_of_the_recently_active() {
    let mut client = client(Activity::Ticking);
    let rates = |response: TorrentGetResponse| -> Vec<(u64, u64)> {
        assert_eq!(response.removed, Vec::<TorrentId>::new());
        shown(response)
            .iter()
            .map(|torrent| (torrent.rate_down, torrent.rate_up))
            .collect()
    };

    let first = rates(list(&mut client, &GetIds::RecentlyActive));
    let second = rates(list(&mut client, &GetIds::RecentlyActive));

    assert!(first.len() > 200, "{} recently active", first.len());
    assert_eq!(first.len(), second.len(), "the same torrents are active");
    assert_ne!(first, second, "the rates moved");
    let stats_before = client.session_stats().expect("the statistics");
    list(&mut client, &GetIds::RecentlyActive);
    let stats_after = client.session_stats().expect("the statistics");
    assert_ne!(
        (stats_before.download_speed, stats_before.upload_speed),
        (stats_after.download_speed, stats_after.upload_speed),
        "the totals follow the rates"
    );
}

#[test]
fn a_static_daemon_is_idle() {
    let mut client = client(Activity::Static);

    assert_eq!(
        list(&mut client, &GetIds::RecentlyActive).torrents,
        Vec::<TorrentRecord>::new()
    );
    let all = shown(list(&mut client, &GetIds::All));
    assert!(
        all.iter()
            .all(|torrent| torrent.rate_down == 0 && torrent.rate_up == 0)
    );
    let stats = client.session_stats().expect("the statistics");
    assert_eq!((stats.download_speed, stats.upload_speed), (0, 0));
    assert_eq!(
        list(&mut client, &GetIds::All),
        list(&mut client, &GetIds::All),
        "nothing changes between polls"
    );
}

#[test]
fn every_other_method_is_refused_and_changes_nothing() {
    let daemon = SyntheticDaemon::new(COUNT, Activity::Static);
    let post = |body: &str| -> Value {
        let response = daemon
            .post(&HttpRequest {
                body: body.as_bytes(),
                session_id: None,
            })
            .expect("an answer");
        assert_eq!(response.status, 200);
        serde_json::from_slice(&response.body).expect("JSON")
    };
    let before = post(r#"{"jsonrpc":"2.0","id":1,"method":"torrent_get","params":{}}"#);

    for method in [
        "torrent_stop",
        "torrent_remove",
        "session_set",
        "torrent_set",
    ] {
        let answer = post(&format!(
            r#"{{"jsonrpc":"2.0","id":9,"method":"{method}","params":{{"ids":[1]}}}}"#
        ));
        assert_eq!(answer.get("id"), Some(&json!(9)), "{answer}");
        assert_eq!(
            answer.pointer("/error/code"),
            Some(&json!(METHOD_NOT_FOUND)),
            "{answer}"
        );
    }
    let details = post(r#"{"jsonrpc":"2.0","id":2,"method":"torrent_get","params":{"ids":[1]}}"#);
    assert!(details.get("error").is_some(), "{details}");

    let after = post(r#"{"jsonrpc":"2.0","id":1,"method":"torrent_get","params":{}}"#);
    assert_eq!(before, after);
}

#[test]
fn a_request_that_is_not_json_is_a_transport_error() {
    let daemon = SyntheticDaemon::new(1, Activity::Static);
    let answer = daemon.post(&HttpRequest {
        body: b"<html>",
        session_id: None,
    });
    assert!(
        matches!(answer, Err(TransportError::Io(ref message)) if message.contains("not JSON")),
        "{answer:?}"
    );
}

#[test]
fn the_generator_stays_in_its_bounds() {
    let mut random = Lcg(SEED);
    for _ in 0..1000 {
        assert!(random.below(7) < 7);
        let value = random.between(10, 20);
        assert!((10..20).contains(&value));
    }
    assert_eq!(random.below(0), 0);
    assert_eq!(random.between(5, 5), 5);
    assert_eq!(random.between(5, 3), 5);
}
