//! The synthetic daemon, through the real client: what it answers, how the actions change its
//! torrents and settings, and what it refuses.

use std::collections::HashSet;
use std::slice;
use std::thread;
use std::time::{Duration, Instant};

use x4_core::{
    Action, ActionId, Attention, Command, ConnectionState, DisplayState, EngineConfig, Event,
    QueueKind, Snapshot, Torrent as ShownTorrent, service,
};
use x4_rpc::{
    Client, GetIds, HashString, RemoveData, RpcError, SessionSet, TorrentAction, TorrentField,
    TorrentGetResponse, TorrentId, TorrentRecord, TorrentRef, TorrentSelection, TorrentSet,
};

use super::*;

mod details;
mod file_set;
mod pieces;
mod support;

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

/// What the daemon logs at debug about each answer: its method and the bytes of the request and
/// of the answer, in order.
fn answers_logged(run: impl FnOnce()) -> Vec<(String, u64, u64)> {
    use std::sync::{Arc, Mutex};

    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::{Context, Layer};
    use tracing_subscriber::prelude::*;

    #[derive(Default)]
    struct Answer {
        message: String,
        method: String,
        request_bytes: u64,
        response_bytes: u64,
    }
    impl Visit for Answer {
        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "method" {
                value.clone_into(&mut self.method);
            }
        }
        fn record_u64(&mut self, field: &Field, value: u64) {
            match field.name() {
                "request_bytes" => self.request_bytes = value,
                "response_bytes" => self.response_bytes = value,
                _ => {}
            }
        }
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                self.message = format!("{value:?}");
            }
        }
    }
    struct Recorder(Arc<Mutex<Vec<(String, u64, u64)>>>);
    impl<S: tracing::Subscriber> Layer<S> for Recorder {
        fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
            let mut answer = Answer::default();
            event.record(&mut answer);
            if *event.metadata().level() == tracing::Level::DEBUG
                && answer.message == "synthetic daemon answered"
            {
                self.0.lock().unwrap().push((
                    answer.method,
                    answer.request_bytes,
                    answer.response_bytes,
                ));
            }
        }
    }

    let logged = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(Recorder(Arc::clone(&logged)));
    tracing::subscriber::with_default(subscriber, run);
    logged.lock().unwrap().clone()
}

/// With `RUST_LOG=x4_synthetic=debug`, the demo's log tells what each request cost, so that the
/// requests and the bytes the app asks of a daemon can be counted.
#[test]
fn each_answer_is_logged_at_debug_with_its_bytes_both_ways() {
    let daemon = SyntheticDaemon::new(COUNT, Activity::Static);
    let body = br#"{"jsonrpc":"2.0","id":1,"method":"session_get"}"#;
    let mut answer = None;

    let logged = answers_logged(|| {
        answer = daemon
            .post(&HttpRequest {
                body,
                session_id: None,
                timeout: None,
            })
            .ok();
    });

    let answered = answer.expect("an answer").body.len();
    assert_eq!(
        logged,
        [(
            "session_get".to_owned(),
            u64::try_from(body.len()).unwrap(),
            u64::try_from(answered).unwrap()
        )]
    );
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

/// Posts `body` to `daemon` and reads the JSON answer.
fn post(daemon: &SyntheticDaemon, body: &str) -> Value {
    let response = daemon
        .post(&HttpRequest {
            body: body.as_bytes(),
            session_id: None,
            timeout: None,
        })
        .expect("an answer");
    assert_eq!(response.status, 200);
    serde_json::from_slice(&response.body).expect("JSON")
}

#[test]
fn adding_and_unknown_methods_are_refused_and_change_nothing() {
    let daemon = SyntheticDaemon::new(COUNT, Activity::Static);
    let before = post(
        &daemon,
        r#"{"jsonrpc":"2.0","id":1,"method":"torrent_get","params":{}}"#,
    );

    for method in ["torrent_add", "torrent_set_location", "blocklist_update"] {
        let answer = post(
            &daemon,
            &format!(r#"{{"jsonrpc":"2.0","id":9,"method":"{method}","params":{{}}}}"#),
        );
        assert_eq!(answer.get("id"), Some(&json!(9)), "{answer}");
        assert_eq!(
            answer.pointer("/error/code"),
            Some(&json!(METHOD_NOT_FOUND)),
            "{answer}"
        );
    }

    let after = post(
        &daemon,
        r#"{"jsonrpc":"2.0","id":1,"method":"torrent_get","params":{}}"#,
    );
    assert_eq!(before, after);
}

#[test]
fn parameters_of_the_wrong_type_are_refused() {
    let daemon = SyntheticDaemon::new(COUNT, Activity::Static);
    for (method, params) in [
        ("torrent_stop", r#"{"ids":"some"}"#),
        ("torrent_stop", r#"{"ids":[true]}"#),
        ("torrent_get", r#"{"fields":"name"}"#),
        ("torrent_set", r#"{"ids":[1],"sequential_download":1}"#),
        ("session_set", r#"{"speed_limit_down":"fast"}"#),
    ] {
        let answer = post(
            &daemon,
            &format!(r#"{{"jsonrpc":"2.0","id":3,"method":"{method}","params":{params}}}"#),
        );
        assert_eq!(
            answer.pointer("/error/code"),
            Some(&json!(INVALID_PARAMS)),
            "{method} {params}: {answer}"
        );
    }
}

/// The torrent of `client`'s list whose hash is `hash`, as the app shows it.
fn find(client: &mut Client<SyntheticDaemon>, hash: &HashString) -> ShownTorrent {
    shown(list(client, &GetIds::All))
        .into_iter()
        .find(|torrent| torrent.hash == *hash)
        .expect("the torrent is listed")
}

/// The hash of the first torrent of `client`'s list for which `wanted` holds.
fn first(client: &mut Client<SyntheticDaemon>, wanted: fn(&ShownTorrent) -> bool) -> HashString {
    shown(list(client, &GetIds::All))
        .into_iter()
        .find(wanted)
        .expect("a torrent of that kind")
        .hash
}

fn selection(hashes: &[&HashString]) -> TorrentSelection {
    TorrentSelection::from_hashes(hashes.iter().map(|&hash| hash.clone())).expect("not empty")
}

fn act(client: &mut Client<SyntheticDaemon>, action: TorrentAction, hashes: &[&HashString]) {
    client
        .torrent_action(action, &selection(hashes))
        .expect("the daemon carries the action out");
}

/// The hashes of the torrents of a poll of the recently active ones.
fn recently_active(client: &mut Client<SyntheticDaemon>) -> Vec<HashString> {
    shown(list(client, &GetIds::RecentlyActive))
        .into_iter()
        .map(|torrent| torrent.hash)
        .collect()
}

#[test]
fn stopping_pauses_and_starting_runs_by_progress() {
    let mut client = client(Activity::Static);
    let downloading = first(&mut client, |torrent| {
        torrent.state == DisplayState::Downloading
    });
    let seeding = first(&mut client, |torrent| {
        torrent.state == DisplayState::Seeding
    });

    act(&mut client, TorrentAction::Stop, &[&downloading, &seeding]);
    assert_eq!(find(&mut client, &downloading).state, DisplayState::Paused);
    assert_eq!(find(&mut client, &seeding).state, DisplayState::Paused);
    let paused = find(&mut client, &downloading);
    assert_eq!((paused.rate_down, paused.peers_connected), (0, 0));
    assert_eq!(
        HashSet::from_iter(recently_active(&mut client)),
        HashSet::from([downloading.clone(), seeding.clone()]),
        "a changed torrent is recently active, also on a static daemon"
    );
    assert_eq!(recently_active(&mut client), [] as [HashString; 0], "once");

    act(&mut client, TorrentAction::Start, &[&seeding, &downloading]);
    assert_eq!(
        find(&mut client, &downloading).state,
        DisplayState::Downloading,
        "it lacks data"
    );
    assert_eq!(find(&mut client, &seeding).state, DisplayState::Seeding);
}

#[test]
fn starting_now_takes_a_torrent_out_of_its_queue() {
    let mut client = client(Activity::Static);
    let queued = first(&mut client, |torrent| {
        torrent.state == DisplayState::Queued(QueueKind::Download)
    });
    let paused = first(&mut client, |torrent| torrent.state == DisplayState::Paused);

    act(&mut client, TorrentAction::Start, &[&queued]);
    assert_eq!(
        find(&mut client, &queued).state,
        DisplayState::Queued(QueueKind::Download),
        "a plain start leaves it in the queue"
    );
    assert_eq!(recently_active(&mut client), [] as [HashString; 0]);

    act(&mut client, TorrentAction::StartNow, &[&queued, &paused]);
    assert_eq!(find(&mut client, &queued).state, DisplayState::Downloading);
    assert_ne!(find(&mut client, &paused).state, DisplayState::Paused);
}

#[test]
fn verifying_checks_for_some_polls_and_then_goes_back() {
    let mut client = client(Activity::Static);
    let seeding = first(&mut client, |torrent| {
        torrent.state == DisplayState::Seeding
    });

    act(&mut client, TorrentAction::Verify, &[&seeding]);
    assert_eq!(find(&mut client, &seeding).state, DisplayState::Checking);
    let mut progress = Vec::new();
    for _ in 1..VERIFY_POLLS {
        assert_eq!(recently_active(&mut client), slice::from_ref(&seeding));
        let checking = find(&mut client, &seeding);
        assert_eq!(checking.state, DisplayState::Checking);
        progress.push(checking.recheck_progress);
    }
    assert!(
        progress.windows(2).all(|pair| pair[0] < pair[1]),
        "{progress:?}"
    );

    assert_eq!(recently_active(&mut client), slice::from_ref(&seeding));
    assert_eq!(find(&mut client, &seeding).state, DisplayState::Seeding);
    assert_eq!(recently_active(&mut client), [] as [HashString; 0]);
}

#[test]
fn reannouncing_changes_nothing() {
    let mut client = client(Activity::Static);
    let seeding = first(&mut client, |torrent| {
        torrent.state == DisplayState::Seeding
    });
    let before = list(&mut client, &GetIds::All);

    act(&mut client, TorrentAction::Reannounce, &[&seeding]);

    assert_eq!(list(&mut client, &GetIds::All), before);
    assert_eq!(recently_active(&mut client), [] as [HashString; 0]);
}

#[test]
fn removed_torrents_are_gone_and_reported_removed_once() {
    let mut client = client(Activity::Static);
    let all = shown(list(&mut client, &GetIds::All));
    let (first, second) = (all.first().expect("one"), all.get(1).expect("two"));

    client
        .torrent_remove(&selection(&[&second.hash]), RemoveData::Delete)
        .expect("removed");
    client
        .torrent_remove(&selection(&[&first.hash]), RemoveData::Keep)
        .expect("removed");

    let left = shown(list(&mut client, &GetIds::All));
    assert_eq!(left.len(), all.len() - 2);
    assert!(
        left.iter()
            .all(|torrent| torrent.hash != first.hash && torrent.hash != second.hash)
    );
    assert_eq!(
        list(&mut client, &GetIds::RecentlyActive).removed,
        [second.id, first.id]
    );
    assert_eq!(
        list(&mut client, &GetIds::RecentlyActive).removed,
        [] as [TorrentId; 0]
    );
    let stats = client.session_stats().expect("the statistics");
    assert_eq!(stats.torrent_count, COUNT - 2);
}

#[test]
fn an_action_skips_unknown_torrents_and_reaches_all_without_ids() {
    let daemon = SyntheticDaemon::new(20, Activity::Static);
    let unknown = post(
        &daemon,
        r#"{"jsonrpc":"2.0","id":4,"method":"torrent_stop","params":{"ids":["0000000000000000000000000000000000000000",9999]}}"#,
    );
    assert_eq!(unknown.get("result"), Some(&json!({})), "{unknown}");

    let all = post(
        &daemon,
        r#"{"jsonrpc":"2.0","id":5,"method":"torrent_stop","params":{}}"#,
    );
    assert_eq!(all.get("result"), Some(&json!({})), "{all}");
    let mut client = Client::new(daemon);
    let states: HashSet<DisplayState> = shown(list(&mut client, &GetIds::All))
        .into_iter()
        .map(|torrent| torrent.state)
        .collect();
    assert!(
        states.is_subset(&HashSet::from([DisplayState::Paused, DisplayState::Error])),
        "{states:?}"
    );
}

#[test]
fn a_list_of_torrents_gets_only_those_and_only_the_fields_asked_for() {
    let mut client = client(Activity::Static);
    let all = shown(list(&mut client, &GetIds::All));
    let (a, b) = (all.get(4).expect("five"), all.get(1).expect("two"));
    let missing = HashString::parse("0000000000000000000000000000000000000000").expect("a hash");

    let answer = client
        .torrent_get(
            &GetIds::Selection(selection(&[&a.hash, &missing, &b.hash])),
            &[TorrentField::HashString, TorrentField::MagnetLink],
        )
        .expect("the client parses the answer");

    let links: Vec<(Option<String>, Option<String>)> = answer
        .torrents
        .iter()
        .map(|record| (record.hash_string.clone(), record.magnet_link.clone()))
        .collect();
    let link = |torrent: &ShownTorrent| {
        format!(
            "magnet:?xt=urn:btih:{}&dn={}",
            torrent.hash,
            torrent.name.replace(' ', "%20")
        )
    };
    assert_eq!(
        links,
        [
            (Some(a.hash.to_string()), Some(link(a))),
            (Some(b.hash.to_string()), Some(link(b))),
        ]
    );
    assert_eq!(
        links.first().and_then(|(_, link)| link.as_deref()),
        Some(
            format!(
                "magnet:?xt=urn:btih:{}&dn=Synthetic%20torrent%200005",
                a.hash
            )
            .as_str()
        )
    );
    assert!(
        answer
            .torrents
            .iter()
            .all(|record| record.name.is_none() && record.status.is_none()),
        "only the fields asked for"
    );
    assert!(
        list(&mut client, &GetIds::All)
            .torrents
            .iter()
            .all(|record| record.magnet_link.is_none()),
        "the list carries no magnet links"
    );
}

/// The records of `refs`, named in one `torrent_get`, with the fields of the list.
fn records_of(client: &mut Client<SyntheticDaemon>, refs: &[TorrentRef]) -> Vec<TorrentRecord> {
    let selection = TorrentSelection::new(refs.to_vec()).expect("not empty");
    client
        .torrent_get(&GetIds::Selection(selection), TorrentField::LIST)
        .expect("the client parses the answer")
        .torrents
}

#[test]
fn reading_ten_torrents_by_hash_matches_reading_them_one_by_one() {
    let mut client = client(Activity::Static);
    let all = shown(list(&mut client, &GetIds::All));
    // Ten torrents spread over the list, in no order, with one the daemon does not know in the
    // middle: eleven entries, which is past the length at which the daemon indexes the torrents
    // instead of scanning them for each entry.
    let chosen: Vec<&ShownTorrent> = [1703, 3, 951, 1999, 0, 412, 1203, 77, 1550, 600]
        .iter()
        .map(|&position| all.get(position).expect("a torrent"))
        .collect();
    assert!(chosen.len() + 1 > SHORT_LIST);
    let unknown = HashString::parse("0000000000000000000000000000000000000000").expect("a hash");
    let by_hash = |torrent: &ShownTorrent| TorrentRef::Hash(torrent.hash.clone());
    let by_id = |torrent: &ShownTorrent| TorrentRef::Id(torrent.id);
    let unknown_id = TorrentRef::Id(TorrentId(999_999));

    for (name, of, unknown) in [
        (
            "hashes",
            &by_hash as &dyn Fn(&ShownTorrent) -> TorrentRef,
            TorrentRef::Hash(unknown),
        ),
        ("ids", &by_id, unknown_id),
    ] {
        let mut refs: Vec<TorrentRef> = chosen.iter().map(|&torrent| of(torrent)).collect();
        refs.insert(5, unknown);

        let together = records_of(&mut client, &refs);

        let one_by_one: Vec<TorrentRecord> = chosen
            .iter()
            .flat_map(|&torrent| records_of(&mut client, &[of(torrent)]))
            .collect();
        assert_eq!(
            together.len(),
            chosen.len(),
            "{name}: the unknown one is skipped"
        );
        assert_eq!(
            together, one_by_one,
            "{name}: the same records, in the same order"
        );
        let hashes: Vec<Option<String>> = together
            .iter()
            .map(|record| record.hash_string.clone())
            .collect();
        let wanted: Vec<Option<String>> = chosen
            .iter()
            .map(|torrent| Some(torrent.hash.to_string()))
            .collect();
        assert_eq!(hashes, wanted, "{name}: in the order named");
        // A short list of the same kind, scanned rather than indexed, gives the same again.
        let (head, tail) = refs.split_at(4);
        let mut in_pieces = records_of(&mut client, head);
        in_pieces.extend(records_of(&mut client, tail));
        assert_eq!(
            together, in_pieces,
            "{name}: lists of 4 and 7 give the same"
        );
    }
}

#[test]
fn session_set_is_kept_and_read_back() {
    let mut client = client(Activity::Static);
    let before = client.session_get().expect("the settings");
    assert!(!before.alt_speed_enabled);

    client
        .session_set(&SessionSet {
            alt_speed_enabled: Some(true),
            speed_limit_down_kbps: Some(2048),
            speed_limit_down_enabled: Some(true),
            ..SessionSet::default()
        })
        .expect("set");

    let after = client.session_get().expect("the settings");
    assert!(after.alt_speed_enabled);
    assert_eq!(after.speed_limit_down_kbps, 2048);
    assert!(after.speed_limit_down_enabled);
    assert_eq!(after.speed_limit_up_kbps, before.speed_limit_up_kbps);
    assert_eq!(
        after.speed_limit_up_enabled, before.speed_limit_up_enabled,
        "the other direction stays"
    );

    // A refused request changes nothing, not even its settings that were valid.
    let refused = client.call_raw(
        "session_set",
        Some(json!({ "speed_limit_up_enabled": true, "speed_limit_up": -5 })),
    );
    assert!(
        matches!(refused, Err(RpcError::Server { code, .. }) if code == INVALID_PARAMS),
        "{refused:?}"
    );
    assert_eq!(client.session_get().expect("the settings"), after);
}

#[test]
fn the_sequential_download_is_kept() {
    let mut client = client(Activity::Static);
    let seeding = first(&mut client, |torrent| {
        torrent.state == DisplayState::Seeding
    });

    client
        .torrent_set(
            &selection(&[&seeding]),
            &TorrentSet {
                sequential_download: Some(true),
                files_wanted: vec![0],
                ..TorrentSet::default()
            },
        )
        .expect("set");

    assert!(find(&mut client, &seeding).sequential);
    assert_eq!(recently_active(&mut client), [seeding]);
}

#[test]
fn a_request_that_is_not_json_is_a_transport_error() {
    let daemon = SyntheticDaemon::new(1, Activity::Static);
    let answer = daemon.post(&HttpRequest {
        body: b"<html>",
        session_id: None,
        timeout: None,
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

/// How long the service may take to connect and list the torrents.
const SERVICE_WAIT: Duration = Duration::from_secs(10);

/// The engine runs over the daemon as the demo runs it: every request it sends in this stage
/// gets an answer, so it connects, lists all the torrents and reports no error. The method list
/// of the daemon is hand-picked; this catches a request the engine sends that is not on it.
#[test]
fn the_service_connects_and_lists_every_torrent() {
    let service = service::spawn(client(Activity::Ticking), EngineConfig::default())
        .expect("the service thread starts");
    // The events channel is async; a thread hands its events over to one with a timeout, and
    // ends when the service closes the channel.
    let (events, received) = crossbeam_channel::unbounded();
    let source = service.events().clone();
    let forward = thread::spawn(move || {
        while let Ok(event) = source.recv_blocking() {
            if events.send(event).is_err() {
                break;
            }
        }
    });

    let deadline = Instant::now() + SERVICE_WAIT;
    let snapshot = loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match received.recv_timeout(left) {
            Ok(Event::Snapshot(snapshot)) => {
                assert!(
                    matches!(
                        snapshot.connection,
                        ConnectionState::Connecting | ConnectionState::Connected
                    ),
                    "{:?}",
                    snapshot.connection
                );
                if snapshot.listed
                    && snapshot.session.is_some()
                    && snapshot.free_space_bytes.is_some()
                {
                    break snapshot;
                }
            }
            Ok(Event::ActionFinished { result, .. }) => {
                assert!(result.is_ok(), "an action failed: {result:?}");
            }
            Ok(Event::MagnetLinks { result, .. }) => {
                assert!(result.is_ok(), "a magnet read failed: {result:?}");
            }
            Ok(Event::AddFinished { result, .. }) => {
                assert!(result.is_ok(), "an add failed: {result:?}");
            }
            Ok(Event::Details(_) | Event::FreeSpace { .. }) => {}
            Err(error) => panic!("no connected snapshot with the whole list: {error}"),
        }
    };
    service.shutdown();
    forward.join().expect("the forwarding thread ends");

    assert_eq!(snapshot.connection, ConnectionState::Connected);
    assert!(!snapshot.stale);
    assert_eq!(snapshot.torrents.len(), 2000);
    assert!(snapshot.rate_down > 0 && snapshot.rate_up > 0);
}

/// Waits up to [`SERVICE_WAIT`] for an event of `received` that `wanted` picks, and returns what
/// it picked; the other events must report no failure.
fn wait_for<T>(
    received: &crossbeam_channel::Receiver<Event>,
    mut wanted: impl FnMut(&Event) -> Option<T>,
) -> T {
    let deadline = Instant::now() + SERVICE_WAIT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let event = received
            .recv_timeout(left)
            .unwrap_or_else(|error| panic!("the event never came: {error}"));
        if let Some(found) = wanted(&event) {
            return found;
        }
        match event {
            Event::ActionFinished { result, .. } => assert!(result.is_ok(), "{result:?}"),
            Event::MagnetLinks { result, .. } => assert!(result.is_ok(), "{result:?}"),
            Event::AddFinished { result, .. } => assert!(result.is_ok(), "{result:?}"),
            Event::Snapshot(_) | Event::Details(_) | Event::FreeSpace { .. } => {}
        }
    }
}

/// The state of the torrent `hash` in `snapshot`.
fn state_in(snapshot: &Snapshot, hash: &HashString) -> Option<DisplayState> {
    snapshot
        .torrents
        .iter()
        .find(|torrent| torrent.hash == *hash)
        .map(|torrent| torrent.state)
}

/// An action through the engine, as the demo sends it: it is carried out, and the snapshots show
/// the torrent paused, then seeding again.
#[test]
fn an_action_through_the_service_shows_in_the_next_snapshots() {
    let service = service::spawn(client(Activity::Static), EngineConfig::default())
        .expect("the service thread starts");
    let (events, received) = crossbeam_channel::unbounded();
    let source = service.events().clone();
    let forward = thread::spawn(move || {
        while let Ok(event) = source.recv_blocking() {
            if events.send(event).is_err() {
                break;
            }
        }
    });
    let seeding = wait_for(&received, |event| match event {
        Event::Snapshot(snapshot) if snapshot.listed => snapshot
            .torrents
            .iter()
            .find(|torrent| torrent.state == DisplayState::Seeding)
            .map(|torrent| torrent.hash.clone()),
        _ => None,
    });

    for (id, action, state) in [
        (1, Action::Stop(vec![seeding.clone()]), DisplayState::Paused),
        (
            2,
            Action::Start(vec![seeding.clone()]),
            DisplayState::Seeding,
        ),
    ] {
        service
            .commands()
            .send(Command::Run(ActionId(id), action))
            .expect("the service listens");
        let mut finished = false;
        wait_for(&received, |event| {
            match event {
                Event::ActionFinished { id: done, result } if *done == ActionId(id) => {
                    assert_eq!(*result, Ok(()));
                    finished = true;
                }
                Event::Snapshot(snapshot) if finished => {
                    return (state_in(snapshot, &seeding) == Some(state)).then_some(());
                }
                _ => {}
            }
            None
        });
    }
    service.shutdown();
    forward.join().expect("the forwarding thread ends");
}
