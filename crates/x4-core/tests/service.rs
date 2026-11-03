//! The service thread: it runs the engine against a fake daemon and talks to the test through
//! the same channels the UI uses.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use async_channel::TryRecvError;
use crossbeam_channel::RecvTimeoutError;
use serde_json::{Value, json};
use x4_core::service::{self, ServiceHandle, ServiceStopped};
use x4_core::{
    Action, ActionError, ActionId, AddError, Added, Command, ConnectionError, ConnectionState,
    EngineConfig, Event, FreeSpaceAnswer, Snapshot,
};
use x4_rpc::testing::fixtures::{
    FREE_SPACE, SESSION_GET, SESSION_STATS, TORRENT_GET_DETAILS, TORRENT_GET_LIST,
    TORRENT_GET_RECENT,
};
use x4_rpc::testing::{FakeTransport, ok};
use x4_rpc::{
    AddSource, Client, DownloadDir, HashString, HttpResponse, MagnetLink, TorrentAdd,
    TransportError,
};

/// How long a test waits for an event before it fails.
const WAIT: Duration = Duration::from_secs(2);

/// Short intervals, so a test sees many turns in a fraction of a second.
fn fast() -> EngineConfig {
    EngineConfig {
        visible_interval: Duration::from_millis(20),
        hidden_interval: Duration::from_millis(50),
        full_refresh_interval: Duration::from_millis(50),
        session_interval: Duration::from_millis(50),
        free_space_interval: Duration::from_millis(50),
        details_interval: Duration::from_millis(50),
        details_settle: Duration::from_millis(10),
        backoff: vec![Duration::from_millis(10), Duration::from_millis(20)],
        stable_after: Duration::from_millis(50),
    }
}

/// A tick of `tick` and every other poll a minute apart, so only ticks repeat during a test.
fn ticking_every(tick: Duration) -> EngineConfig {
    EngineConfig {
        visible_interval: tick,
        hidden_interval: tick,
        full_refresh_interval: Duration::from_secs(60),
        session_interval: Duration::from_secs(60),
        free_space_interval: Duration::from_secs(60),
        ..fast()
    }
}

/// The hash of the first torrent of the recorded list.
const FIXTURE_HASH: &str = "0000000000000000000000000000000000000001";

/// How often a test looks again while it waits.
const POLL: Duration = Duration::from_millis(2);

/// The `result` of a recorded response; null if the fixture has none.
fn result_of(envelope: &str) -> Value {
    serde_json::from_str::<Value>(envelope)
        .ok()
        .and_then(|mut envelope| envelope.get_mut("result").map(Value::take))
        .unwrap_or(Value::Null)
}

/// Answers like the recorded daemon: each read with its fixture, every action with success.
fn daemon(method: &str, params: &Value) -> HttpResponse {
    let fixture = match method {
        "session_get" => SESSION_GET,
        "session_stats" => SESSION_STATS,
        "free_space" => FREE_SPACE,
        "torrent_get" => match params.get("ids") {
            None => TORRENT_GET_LIST,
            Some(Value::String(_)) => TORRENT_GET_RECENT,
            Some(_) => TORRENT_GET_DETAILS,
        },
        _ => return ok(json!({})),
    };
    ok(result_of(fixture))
}

fn recorded_daemon() -> FakeTransport {
    FakeTransport::routed(|method, params| Ok(daemon(method, params)))
}

/// A JSON-RPC error answer, as the daemon sends when it refuses a call.
fn refused() -> HttpResponse {
    let body = json!({ "jsonrpc": "2.0", "error": { "code": 5, "message": "system error" } });
    HttpResponse {
        status: 200,
        session_id: None,
        body: body.to_string().into_bytes(),
    }
}

fn methods(fake: &FakeTransport) -> Vec<String> {
    fake.requests()
        .into_iter()
        .map(|request| request.method)
        .collect()
}

/// Whether `done` holds within [`WAIT`].
fn eventually(mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        thread::sleep(POLL);
    }
    done()
}

/// The first event `pick` takes within [`WAIT`], skipping the others; `None` if none comes or
/// the channel closes.
fn next_picked<R>(handle: &ServiceHandle, mut pick: impl FnMut(Event) -> Option<R>) -> Option<R> {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        match handle.events().try_recv() {
            Ok(event) => {
                if let Some(picked) = pick(event) {
                    return Some(picked);
                }
            }
            Err(TryRecvError::Empty) => thread::sleep(POLL),
            Err(TryRecvError::Closed) => return None,
        }
    }
    None
}

fn snapshot_where(
    handle: &ServiceHandle,
    wanted: impl Fn(&Snapshot) -> bool,
) -> Option<Arc<Snapshot>> {
    next_picked(handle, |event| match event {
        Event::Snapshot(snapshot) if wanted(&snapshot) => Some(snapshot),
        _ => None,
    })
}

fn connected(snapshot: &Snapshot) -> bool {
    snapshot.connection == ConnectionState::Connected && !snapshot.torrents.is_empty()
}

fn reconnecting_at_least(attempts: u32) -> impl Fn(&Snapshot) -> bool {
    move |snapshot| matches!(snapshot.connection, ConnectionState::Reconnecting { attempt, .. } if attempt >= attempts)
}

fn action_result(handle: &ServiceHandle, id: ActionId) -> Option<Result<(), ActionError>> {
    next_picked(handle, |event| match event {
        Event::ActionFinished {
            id: finished,
            result,
        } if finished == id => Some(result),
        _ => None,
    })
}

/// The first event within [`WAIT`] that answers the action `id`, whichever kind it is.
fn answer_to(handle: &ServiceHandle, id: ActionId) -> Option<Event> {
    next_picked(handle, |event| match &event {
        Event::AddFinished { id: answered, .. } | Event::ActionFinished { id: answered, .. }
            if *answered == id =>
        {
            Some(event)
        }
        _ => None,
    })
}

fn add_finished(id: u64, result: Result<Added, AddError>) -> Event {
    Event::AddFinished {
        id: ActionId(id),
        result,
    }
}

/// An add of `link`.
fn magnet_add(link: MagnetLink) -> Action {
    Action::Add(TorrentAdd {
        source: AddSource::Magnet(link),
        download_dir: None,
        paused: true,
        files_unwanted: Vec::new(),
        priority_high: Vec::new(),
        priority_low: Vec::new(),
        sequential_download: None,
    })
}

#[test]
fn added_and_duplicate_answers_reach_the_app_with_the_hash() {
    const NEW: &str = "00000000000000000000000000000000000000aa";
    let fake = FakeTransport::routed(|method, params| {
        if method != "torrent_add" {
            return Ok(daemon(method, params));
        }
        // The daemon knows the torrent of the fixture list, and adds any other.
        let link = params["filename"].as_str().unwrap_or_default();
        Ok(if link.ends_with(FIXTURE_HASH) {
            ok(
                json!({ "torrent_duplicate": { "id": 1, "name": "Torrent 1", "hash_string": FIXTURE_HASH } }),
            )
        } else {
            ok(json!({ "torrent_added": { "id": 99, "name": "New torrent", "hash_string": NEW } }))
        })
    });
    let handle = service::spawn(Client::new(fake), fast()).unwrap();
    snapshot_where(&handle, connected).unwrap();
    let link = |hash: &str| MagnetLink::parse(&format!("magnet:?xt=urn:btih:{hash}")).unwrap();

    handle
        .send(Command::Run(ActionId(1), magnet_add(link(NEW))))
        .unwrap();
    assert_eq!(
        answer_to(&handle, ActionId(1)),
        Some(add_finished(
            1,
            Ok(Added {
                hash: HashString::parse(NEW).unwrap(),
                name: "New torrent".to_owned(),
                duplicate: false,
            })
        ))
    );

    handle
        .send(Command::Run(ActionId(2), magnet_add(link(FIXTURE_HASH))))
        .unwrap();
    assert_eq!(
        answer_to(&handle, ActionId(2)),
        Some(add_finished(
            2,
            Ok(Added {
                hash: HashString::parse(FIXTURE_HASH).unwrap(),
                name: "Torrent 1".to_owned(),
                duplicate: true,
            })
        ))
    );
}

#[test]
fn start_added_torrents_comes_with_the_session() {
    for start in [true, false] {
        let fake = FakeTransport::routed(move |method, params| {
            let server_check = params["fields"] == json!(["version", "rpc_version_semver"]);
            if method != "session_get" || server_check {
                return Ok(daemon(method, params));
            }
            let mut session = result_of(SESSION_GET);
            session["start_added_torrents"] = json!(start);
            Ok(ok(session))
        });
        let handle = service::spawn(Client::new(fake.clone()), fast()).unwrap();

        let snapshot = snapshot_where(&handle, |snapshot| snapshot.session.is_some()).unwrap();
        handle.shutdown();

        assert_eq!(
            snapshot
                .session
                .as_ref()
                .map(|session| session.start_added_torrents),
            Some(start)
        );
        let asked = fake
            .requests()
            .into_iter()
            .filter(|request| request.method == "session_get")
            .any(|request| {
                request.params["fields"]
                    .as_array()
                    .is_some_and(|fields| fields.contains(&json!("start_added_torrents")))
            });
        assert!(asked, "the session read asks for start_added_torrents");
    }
}

#[test]
fn free_space_of_a_new_folder_comes_from_its_parent_over_the_wire() {
    let fake = FakeTransport::routed(|method, params| {
        if method == "free_space" && params["path"] == "/data/torrents/video/new" {
            let missing = json!({
                "jsonrpc": "2.0",
                "error": {
                    "code": 5,
                    "message": "system error",
                    "data": {
                        "error_string": "No such file or directory",
                        "result": { "path": "/data/torrents/video/new", "size_bytes": -1, "total_size": -1 },
                    },
                },
            });
            return Ok(HttpResponse {
                status: 200,
                session_id: None,
                body: missing.to_string().into_bytes(),
            });
        }
        Ok(daemon(method, params))
    });
    let handle = service::spawn(Client::new(fake.clone()), ticking_every(WAIT)).unwrap();
    // Once the status bar's own free space has come, it is next read a minute later.
    snapshot_where(&handle, |snapshot| {
        connected(snapshot) && snapshot.free_space_bytes.is_some()
    })
    .unwrap();
    let asked = fake.requests().len();
    let new = DownloadDir::new("/data/torrents/video/new").unwrap();

    handle.send(Command::FreeSpace(new.clone())).unwrap();
    let answer = next_picked(&handle, |event| match event {
        Event::FreeSpace { path, answer } if path == new => Some(answer),
        _ => None,
    });
    handle.shutdown();

    let free = result_of(FREE_SPACE)["size_bytes"].as_u64();
    assert_eq!(
        answer,
        Some(FreeSpaceAnswer::Missing {
            parent: Some(DownloadDir::new("/data/torrents/video").unwrap()),
            free,
        })
    );
    let paths: Vec<Value> = fake.requests()[asked..]
        .iter()
        .filter(|request| request.method == "free_space")
        .map(|request| request.params["path"].clone())
        .collect();
    assert_eq!(
        paths,
        [
            json!("/data/torrents/video/new"),
            json!("/data/torrents/video")
        ]
    );
}

#[test]
fn service_publishes_first_snapshot() {
    let handle = service::spawn(Client::new(recorded_daemon()), fast()).unwrap();

    let snapshot = snapshot_where(&handle, connected).unwrap();

    assert!(!snapshot.stale);
    let hash = HashString::parse(FIXTURE_HASH).unwrap();
    assert!(snapshot.torrents.iter().any(|torrent| torrent.hash == hash));
}

#[test]
fn the_ui_hears_that_the_service_is_connecting_before_the_daemon_answers() {
    let (open_tx, open) = crossbeam_channel::unbounded::<()>();
    let first_request = AtomicBool::new(true);
    let fake = FakeTransport::routed(move |method, params| {
        // The server check does not answer until the test has seen the first snapshot.
        if first_request.swap(false, Ordering::SeqCst) {
            open.recv_timeout(WAIT).unwrap();
        }
        Ok(daemon(method, params))
    });
    let handle = service::spawn(Client::new(fake), fast()).unwrap();

    let first = snapshot_where(&handle, |_| true).unwrap();
    open_tx.send(()).unwrap();

    assert_eq!(first.connection, ConnectionState::Connecting);
    assert!(first.stale);
    assert!(first.torrents.is_empty(), "{:?}", first.torrents);
}

#[test]
fn a_reconnect_shows_connecting_before_the_server_check_answers() {
    let (release_tx, release) = crossbeam_channel::unbounded::<()>();
    let checks = AtomicUsize::new(0);
    let fake = FakeTransport::routed(move |method, params| {
        let server_check = params["fields"] == json!(["version", "rpc_version_semver"]);
        // The check after the reconnect hangs until the test has seen the snapshot.
        if method == "session_get" && server_check && checks.fetch_add(1, Ordering::SeqCst) == 1 {
            release.recv_timeout(WAIT).unwrap();
        }
        Ok(daemon(method, params))
    });
    let handle = service::spawn(Client::new(fake), fast()).unwrap();
    snapshot_where(&handle, connected).unwrap();

    handle.send(Command::Reconnect).unwrap();

    let connecting = snapshot_where(&handle, |snapshot| {
        snapshot.connection == ConnectionState::Connecting
    });
    release_tx.send(()).unwrap();
    assert!(connecting.is_some_and(|snapshot| snapshot.stale));
    snapshot_where(&handle, connected).unwrap();
}

#[test]
fn service_runs_actions() {
    let fake = recorded_daemon();
    let handle = service::spawn(Client::new(fake.clone()), fast()).unwrap();
    snapshot_where(&handle, connected).unwrap();
    let hash = HashString::parse(FIXTURE_HASH).unwrap();

    handle
        .send(Command::Run(ActionId(7), Action::Stop(vec![hash.clone()])))
        .unwrap();

    assert_eq!(action_result(&handle, ActionId(7)), Some(Ok(())));
    let stops: Vec<Value> = fake
        .requests()
        .into_iter()
        .filter(|request| request.method == "torrent_stop")
        .map(|request| request.params)
        .collect();
    assert_eq!(stops, [json!({ "ids": [hash.as_str()] })]);
}

#[test]
fn a_failed_action_is_not_retried() {
    let fake = FakeTransport::routed(|method, params| {
        if method == "torrent_stop" {
            Ok(refused())
        } else {
            Ok(daemon(method, params))
        }
    });
    let handle = service::spawn(Client::new(fake.clone()), fast()).unwrap();
    snapshot_where(&handle, connected).unwrap();

    handle
        .send(Command::Run(
            ActionId(3),
            Action::Stop(vec![HashString::parse(FIXTURE_HASH).unwrap()]),
        ))
        .unwrap();

    assert_eq!(
        action_result(&handle, ActionId(3)),
        Some(Err(ActionError::Rpc(ConnectionError::Server(
            "system error".to_owned()
        ))))
    );
    // Several ticks later the action has still gone out only once.
    thread::sleep(Duration::from_millis(150));
    let stops = methods(&fake)
        .into_iter()
        .filter(|method| method == "torrent_stop")
        .count();
    assert_eq!(stops, 1);
}

#[test]
fn service_reports_unreachable() {
    let unreachable =
        FakeTransport::routed(|_, _| Err(TransportError::Connect("connection refused".to_owned())));
    let handle = service::spawn(Client::new(unreachable), fast()).unwrap();

    let snapshot = snapshot_where(&handle, reconnecting_at_least(1)).unwrap();

    assert!(snapshot.stale);
    assert!(matches!(
        snapshot.connection,
        ConnectionState::Reconnecting {
            last_error: ConnectionError::Unreachable,
            ..
        }
    ));
}

#[test]
fn polls_that_are_no_longer_current_are_not_sent() {
    // The server check passes, but the first poll of every connection breaks it, which makes
    // the rest of that batch out of date.
    let fake = FakeTransport::routed(|method, params| {
        let server_check = params["fields"] == json!(["version", "rpc_version_semver"]);
        if method == "session_get" && !server_check {
            Err(TransportError::Io("connection reset".to_owned()))
        } else {
            Ok(daemon(method, params))
        }
    });
    let handle = service::spawn(Client::new(fake.clone()), fast()).unwrap();

    snapshot_where(&handle, reconnecting_at_least(2)).unwrap();
    handle.shutdown();

    let sent = methods(&fake);
    assert!(
        sent.iter().all(|method| method == "session_get"),
        "only the server check and the failing session read go out: {sent:?}"
    );
}

#[test]
fn a_slow_daemon_rests_a_full_interval_after_each_answer() {
    let tick = Duration::from_millis(30);
    let stats_calls: Arc<Mutex<Vec<(Instant, Instant)>>> = Arc::default();
    let log = Arc::clone(&stats_calls);
    let fake = FakeTransport::routed(move |method, params| {
        if method != "session_stats" {
            return Ok(daemon(method, params));
        }
        let received = Instant::now();
        thread::sleep(Duration::from_millis(80));
        let answer = daemon(method, params);
        log.lock().unwrap().push((received, Instant::now()));
        Ok(answer)
    });
    let handle = service::spawn(Client::new(fake), ticking_every(tick)).unwrap();

    assert!(eventually(|| stats_calls.lock().unwrap().len() >= 4));
    handle.shutdown();

    // The statistics ride on every tick, and the next tick is measured from the answer.
    let calls = stats_calls.lock().unwrap().clone();
    for pair in calls.windows(2) {
        let (_, answered) = pair[0];
        let (next, _) = pair[1];
        assert!(
            next >= answered + tick,
            "the next tick came {:?} after the answer, less than {tick:?}",
            next.duration_since(answered)
        );
    }
}

#[test]
fn commands_queued_during_a_batch_are_all_taken_before_the_next() {
    let (entered_tx, entered) = crossbeam_channel::unbounded();
    let (open_tx, open) = crossbeam_channel::unbounded::<()>();
    let first_stats = AtomicBool::new(true);
    let fake = FakeTransport::routed(move |method, params| {
        if method == "session_stats" && first_stats.swap(false, Ordering::SeqCst) {
            // Hold the worker inside its first batch after connecting.
            entered_tx.send(()).unwrap();
            open.recv_timeout(WAIT).unwrap();
        }
        Ok(daemon(method, params))
    });
    let handle = service::spawn(Client::new(fake.clone()), ticking_every(WAIT)).unwrap();
    entered.recv_timeout(WAIT).unwrap();
    let hash = HashString::parse(FIXTURE_HASH).unwrap();

    handle
        .send(Command::Run(ActionId(1), Action::Stop(vec![hash.clone()])))
        .unwrap();
    handle
        .send(Command::Run(ActionId(2), Action::Start(vec![hash])))
        .unwrap();
    open_tx.send(()).unwrap();

    assert_eq!(action_result(&handle, ActionId(1)), Some(Ok(())));
    assert_eq!(action_result(&handle, ActionId(2)), Some(Ok(())));
    let sent = methods(&fake);
    let stats = sent
        .iter()
        .position(|method| method == "session_stats")
        .unwrap();
    assert_eq!(
        sent[stats + 1..stats + 3],
        ["torrent_stop", "torrent_start"],
        "both actions lead the next batch: {sent:?}"
    );
}

#[test]
fn the_worker_thread_is_named_x4_service() {
    let names: Arc<Mutex<Vec<Option<String>>>> = Arc::default();
    let log = Arc::clone(&names);
    let fake = FakeTransport::routed(move |method, params| {
        log.lock()
            .unwrap()
            .push(thread::current().name().map(str::to_owned));
        Ok(daemon(method, params))
    });
    let handle = service::spawn(Client::new(fake), fast()).unwrap();

    snapshot_where(&handle, connected).unwrap();
    handle.shutdown();

    let names = names.lock().unwrap();
    assert!(!names.is_empty());
    assert!(
        names
            .iter()
            .all(|name| name.as_deref() == Some("x4-service")),
        "{names:?}"
    );
}

#[test]
fn service_shuts_down_cleanly() {
    let handle = service::spawn(Client::new(recorded_daemon()), fast()).unwrap();
    let events = handle.events().clone();
    let commands = handle.commands();
    snapshot_where(&handle, connected).unwrap();

    let started = Instant::now();
    handle.shutdown();

    assert!(
        started.elapsed() < Duration::from_secs(1),
        "shutdown took {:?}",
        started.elapsed()
    );
    while events.try_recv().is_ok() {}
    assert!(events.is_closed());
    assert_eq!(events.try_recv(), Err(TryRecvError::Closed));
    assert!(commands.send(Command::Refresh).is_err());
}

#[test]
fn dropping_handle_stops_thread() {
    let (probe, transport_gone) = crossbeam_channel::bounded::<()>(0);
    // No clone of this transport stays here: the worker holds the only one, so the probe
    // disconnects when the worker thread has dropped its client.
    let transport = FakeTransport::routed(move |method, params| {
        let _alive = &probe;
        Ok(daemon(method, params))
    });
    let handle = service::spawn(Client::new(transport), fast()).unwrap();
    snapshot_where(&handle, connected).unwrap();

    drop(handle);

    assert_eq!(
        transport_gone.recv_timeout(WAIT),
        Err(RecvTimeoutError::Disconnected)
    );
}

#[test]
fn dropping_the_handle_does_not_wait_for_the_request_in_flight() {
    let (entered_tx, entered) = crossbeam_channel::unbounded();
    let (release_tx, release) = crossbeam_channel::unbounded::<()>();
    let (freed_tx, freed) = crossbeam_channel::unbounded();
    let first_request = AtomicBool::new(true);
    let fake = FakeTransport::routed(move |method, params| {
        if first_request.swap(false, Ordering::SeqCst) {
            entered_tx.send(()).unwrap();
            // Whether the release or the timeout ends the wait.
            freed_tx.send(release.recv_timeout(WAIT)).unwrap();
        }
        Ok(daemon(method, params))
    });
    let handle = service::spawn(Client::new(fake), fast()).unwrap();
    entered.recv_timeout(WAIT).unwrap();

    // A drop that waited for the thread would return only after the handler timed out.
    drop(handle);

    assert!(
        release_tx.send(()).is_ok(),
        "the request in flight ended before the drop returned"
    );
    assert_eq!(freed.recv_timeout(WAIT), Ok(Ok(())));
}

#[test]
fn the_worker_stops_when_the_ui_closes_its_events() {
    let handle = service::spawn(Client::new(recorded_daemon()), fast()).unwrap();
    snapshot_where(&handle, connected).unwrap();

    handle.events().close();

    assert!(eventually(
        || handle.send(Command::Refresh) == Err(ServiceStopped)
    ));
}
