//! Tests of the engine, driven by one base `Instant` plus offsets and answered from the
//! recorded fixtures.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use x4_rpc::testing::fixtures::{
    FREE_SPACE, SESSION_GET, SESSION_STATS, TORRENT_GET_DETAILS, TORRENT_GET_LIST,
    TORRENT_GET_RECENT,
};
use x4_rpc::{
    AddSource, FilePriority, IdError, RemoveData, TorrentAdd, TorrentId, TorrentRecord,
    TransportError,
};

use super::*;

use crate::model::Torrent;
use crate::snapshot::ConnectionError;
use crate::store::{SkipReason, SkippedRecord};
use crate::test_support::fixture_result;

/// The first torrent of every fixture; the details fixture is this torrent.
const FIRST: &str = "0000000000000000000000000000000000000001";
/// The second torrent of the list fixture.
const SECOND: &str = "0000000000000000000000000000000000000002";
/// The `download_dir` of the session fixture.
const DOWNLOAD_DIR: &str = "/data/torrents/video";
/// How many torrents the list fixture holds.
const LISTED: usize = 6;

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

fn secs(seconds: u64) -> Duration {
    Duration::from_secs(seconds)
}

fn first() -> HashString {
    HashString::parse(FIRST).unwrap()
}

fn second() -> HashString {
    HashString::parse(SECOND).unwrap()
}

fn version(rpc_semver: &str) -> ServerVersion {
    ServerVersion {
        version: "4.1.3 (838877323f)".to_owned(),
        rpc_semver: rpc_semver.to_owned(),
    }
}

fn session_info() -> SessionInfo {
    serde_json::from_value(fixture_result(SESSION_GET)).unwrap()
}

fn list() -> TorrentGetResponse {
    serde_json::from_value(fixture_result(TORRENT_GET_LIST)).unwrap()
}

fn recent() -> TorrentGetResponse {
    serde_json::from_value(fixture_result(TORRENT_GET_RECENT)).unwrap()
}

fn timeout() -> RpcError {
    RpcError::Transport(TransportError::Timeout)
}

fn free_space(path: &str, size_bytes: i64) -> Response {
    Response::FreeSpace(FreeSpace {
        path: path.to_owned(),
        size_bytes,
        total_size: 1 << 40,
    })
}

/// What the daemon the fixtures were recorded from answers.
fn answer(request: &Request) -> Response {
    match request {
        Request::CheckServer => Response::Server(version("6.0.1")),
        Request::SessionGet => Response::Session(session_info()),
        Request::SessionStats => {
            Response::Stats(serde_json::from_value(fixture_result(SESSION_STATS)).unwrap())
        }
        Request::FreeSpace(_) => {
            Response::FreeSpace(serde_json::from_value(fixture_result(FREE_SPACE)).unwrap())
        }
        Request::FullList => Response::Torrents(list()),
        Request::RecentlyActive => Response::Torrents(recent()),
        Request::Details(_) => {
            Response::Details(serde_json::from_value(fixture_result(TORRENT_GET_DETAILS)).unwrap())
        }
        Request::Action(..) => Response::Done,
    }
}

/// Hands out what is due at `now` and answers all of it like the fixture daemon; returns the
/// requests and the events their answers caused.
fn exchange(engine: &mut Engine, now: Instant) -> (Vec<Request>, Vec<Event>) {
    let due = engine.due_requests(now);
    let mut events = Vec::new();
    for request in &due {
        events.extend(engine.handle_response(now, request, Ok(answer(request))));
    }
    (due, events)
}

/// An engine that connected at `base` and has not sent anything else yet.
fn connected(base: Instant) -> Engine {
    let mut engine = Engine::new(EngineConfig::default(), base);
    assert_eq!(engine.due_requests(base), [Request::CheckServer]);
    let events = engine.handle_response(
        base,
        &Request::CheckServer,
        Ok(Response::Server(version("6.0.1"))),
    );
    assert_eq!(events, Vec::new());
    engine
}

/// An engine that connected at `base` and read everything it reads on connecting.
fn loaded(base: Instant) -> Engine {
    let mut engine = connected(base);
    while !exchange(&mut engine, base).0.is_empty() {}
    engine
}

/// An engine that lost the connection at `at`, after loading at `base`.
fn dropped(base: Instant, at: Instant) -> Engine {
    let mut engine = loaded(base);
    engine.handle_response(at, &Request::RecentlyActive, Err(timeout()));
    assert_eq!(engine.connection, reconnecting(1, ConnectionError::Timeout));
    engine
}

fn reconnecting(attempt: u32, last_error: ConnectionError) -> ConnectionState {
    ConnectionState::Reconnecting {
        attempt,
        last_error,
    }
}

fn finished(id: u64, result: Result<(), ActionError>) -> Event {
    Event::ActionFinished {
        id: ActionId(id),
        result,
    }
}

fn stop_first() -> Action {
    Action::Stop(vec![first()])
}

fn magnet() -> TorrentAdd {
    TorrentAdd {
        source: AddSource::Magnet("magnet:?xt=urn:btih:example".to_owned()),
        download_dir: None,
        paused: true,
        files_unwanted: Vec::new(),
        priority_high: Vec::new(),
        priority_low: Vec::new(),
        sequential_download: None,
    }
}

/// The torrents of a snapshot, in hash order.
fn torrents(snapshot: &Snapshot) -> Vec<Torrent> {
    let mut torrents = snapshot.torrents.clone();
    torrents.sort_by(|a, b| a.hash.cmp(&b.hash));
    torrents
}

fn details_events(events: &[Event]) -> Vec<Arc<TorrentDetails>> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Details(details) => Some(Arc::clone(details)),
            _ => None,
        })
        .collect()
}

#[test]
fn default_config_has_the_planned_values() {
    let config = EngineConfig::default();

    assert_eq!(config.visible_interval, ms(1500));
    assert_eq!(config.hidden_interval, secs(10));
    assert_eq!(config.full_refresh_interval, secs(60));
    assert_eq!(config.session_interval, secs(60));
    assert_eq!(config.free_space_interval, secs(30));
    assert_eq!(
        config.backoff,
        [secs(1), secs(2), secs(4), secs(8), secs(16), secs(30)]
    );
}

#[test]
fn starts_with_check_server_only() {
    let base = Instant::now();
    let mut engine = Engine::new(EngineConfig::default(), base);

    assert_eq!(engine.due_requests(base), [Request::CheckServer]);
    assert_eq!(engine.due_requests(base), []);
    assert_eq!(engine.connection, ConnectionState::Connecting);
}

#[test]
fn unsupported_server_stops_polling() {
    let base = Instant::now();
    let mut engine = Engine::new(EngineConfig::default(), base);
    engine.due_requests(base);

    engine.handle_response(
        base,
        &Request::CheckServer,
        Ok(Response::Server(version("5.0.0"))),
    );

    assert_eq!(
        engine.connection,
        ConnectionState::Unsupported {
            rpc_version: "5.0.0".to_owned()
        }
    );
    // Five minutes of ticks.
    for tick in 0..=200 {
        assert_eq!(
            engine.due_requests(base + ms(1500 * tick)),
            [],
            "tick {tick}"
        );
    }
    let later = base + secs(301);
    assert_eq!(engine.handle_command(later, Command::Reconnect), []);
    assert_eq!(engine.due_requests(later), [Request::CheckServer]);
}

#[test]
fn unsupported_server_error_stops_polling_too() {
    let base = Instant::now();
    let mut engine = Engine::new(EngineConfig::default(), base);
    engine.due_requests(base);
    let legacy = "legacy protocol (Transmission 4.0 or older)".to_owned();

    engine.handle_response(
        base,
        &Request::CheckServer,
        Err(RpcError::UnsupportedServer {
            rpc_version: legacy.clone(),
        }),
    );

    assert_eq!(
        engine.connection,
        ConnectionState::Unsupported {
            rpc_version: legacy
        }
    );
    assert_eq!(engine.due_requests(base + secs(60)), []);
}

#[test]
fn connect_triggers_full_refresh() {
    let base = Instant::now();
    let mut engine = connected(base);

    assert_eq!(engine.connection, ConnectionState::Connected);
    assert_eq!(
        engine.due_requests(base),
        [
            Request::SessionGet,
            Request::FullList,
            Request::SessionStats
        ]
    );
    // The directory to measure is known only once the session is read.
    engine.handle_response(base, &Request::SessionGet, Ok(answer(&Request::SessionGet)));
    assert_eq!(
        engine.due_requests(base),
        [Request::FreeSpace(DOWNLOAD_DIR.to_owned())]
    );
}

#[test]
fn visible_tick_is_1500ms() {
    let base = Instant::now();
    let mut engine = loaded(base);

    assert_eq!(engine.due_requests(base + ms(1400)), []);
    assert_eq!(
        engine.due_requests(base + ms(1500)),
        [Request::RecentlyActive, Request::SessionStats]
    );
}

#[test]
fn hidden_tick_is_10s() {
    let base = Instant::now();
    let mut engine = loaded(base);

    engine.handle_command(base, Command::SetVisibility(Visibility::Hidden));

    assert_eq!(engine.due_requests(base + ms(1500)), []);
    assert_eq!(engine.due_requests(base + ms(9999)), []);
    assert_eq!(
        engine.due_requests(base + secs(10)),
        [Request::RecentlyActive, Request::SessionStats]
    );
}

#[test]
fn becoming_visible_ticks_at_once_when_a_tick_is_overdue() {
    let base = Instant::now();
    let mut engine = loaded(base);
    engine.handle_command(base, Command::SetVisibility(Visibility::Hidden));

    let now = base + secs(5);
    engine.handle_command(now, Command::SetVisibility(Visibility::Visible));

    assert_eq!(
        engine.due_requests(now),
        [Request::RecentlyActive, Request::SessionStats]
    );
}

#[test]
fn full_refresh_every_60s() {
    let base = Instant::now();
    let mut engine = loaded(base);

    for tick in 1..=40 {
        let now = base + ms(1500 * tick);
        let (due, _) = exchange(&mut engine, now);
        assert_eq!(due.contains(&Request::FullList), tick == 40, "tick {tick}");
        // A full list holds the recently active torrents too.
        assert_eq!(
            due.contains(&Request::RecentlyActive),
            tick != 40,
            "tick {tick}"
        );
        assert!(due.contains(&Request::SessionStats), "tick {tick}");
    }
}

#[test]
fn session_and_free_space_have_their_own_intervals() {
    let base = Instant::now();
    let mut engine = loaded(base);
    let free_space = Request::FreeSpace(DOWNLOAD_DIR.to_owned());

    let mut sessions = Vec::new();
    let mut free_spaces = Vec::new();
    for tick in 1..=40 {
        let (due, _) = exchange(&mut engine, base + ms(1500 * tick));
        if due.contains(&Request::SessionGet) {
            sessions.push(tick);
        }
        if due.contains(&free_space) {
            free_spaces.push(tick);
        }
    }

    assert_eq!(sessions, [40]);
    assert_eq!(free_spaces, [20, 40]);
}

#[test]
fn transport_error_backs_off() {
    let base = Instant::now();
    let mut now = base + ms(1500);
    let mut engine = dropped(base, now);

    for (attempt, delay) in [(1, 1), (2, 2), (3, 4), (4, 8), (5, 16), (6, 30), (7, 30)] {
        assert_eq!(
            engine.connection,
            reconnecting(attempt, ConnectionError::Timeout)
        );
        assert_eq!(
            engine.due_requests(now + ms(delay * 1000 - 1)),
            [],
            "attempt {attempt}"
        );
        now += secs(delay);
        assert_eq!(
            engine.due_requests(now),
            [Request::CheckServer],
            "attempt {attempt}"
        );
        engine.handle_response(now, &Request::CheckServer, Err(timeout()));
    }
    assert_eq!(engine.connection, reconnecting(8, ConnectionError::Timeout));
}

#[test]
fn backoff_stays_at_its_last_step() {
    let base = Instant::now();
    let config = EngineConfig {
        backoff: vec![secs(3), secs(7)],
        ..EngineConfig::default()
    };
    let mut engine = Engine::new(config, base);
    let mut now = base;

    for delay in [3, 7, 7, 7] {
        assert_eq!(engine.due_requests(now), [Request::CheckServer]);
        engine.handle_response(now, &Request::CheckServer, Err(timeout()));
        assert_eq!(engine.next_wakeup(now), now + secs(delay));
        now += secs(delay);
    }
}

#[test]
fn reconnecting_succeeds_with_a_full_refresh() {
    let base = Instant::now();
    let at = base + ms(1500);
    let mut engine = dropped(base, at);
    let retry = at + secs(1);

    assert_eq!(engine.due_requests(retry), [Request::CheckServer]);
    engine.handle_response(
        retry,
        &Request::CheckServer,
        Ok(answer(&Request::CheckServer)),
    );

    assert_eq!(engine.connection, ConnectionState::Connected);
    assert_eq!(
        engine.due_requests(retry),
        [
            Request::SessionGet,
            Request::FullList,
            Request::SessionStats
        ]
    );
}

#[test]
fn snapshot_is_stale_while_reconnecting() {
    let base = Instant::now();
    let mut engine = loaded(base);
    let fresh = engine.take_snapshot().unwrap();
    assert!(!fresh.stale);
    assert_eq!(fresh.torrents.len(), LISTED);

    engine.handle_response(base + ms(1500), &Request::RecentlyActive, Err(timeout()));

    let stale = engine.take_snapshot().unwrap();
    assert!(stale.stale);
    assert_eq!(stale.connection, reconnecting(1, ConnectionError::Timeout));
    assert_eq!(torrents(&stale), torrents(&fresh));
    assert_eq!(stale.session, fresh.session);
    assert_eq!(stale.free_space_bytes, fresh.free_space_bytes);
    assert_eq!(
        (stale.rate_down, stale.rate_up),
        (fresh.rate_down, fresh.rate_up)
    );
}

#[test]
fn poll_errors_that_make_the_daemon_unusable_reconnect() {
    let base = Instant::now();
    for error in [
        timeout(),
        RpcError::Transport(TransportError::Connect("connection refused".to_owned())),
        RpcError::Transport(TransportError::Io("connection reset".to_owned())),
        RpcError::Unauthorized,
        RpcError::Forbidden,
        RpcError::HttpStatus(502),
        RpcError::SessionIdMissing,
        RpcError::SessionIdRejected,
        RpcError::InvalidResponse("not JSON".to_owned()),
    ] {
        let mut engine = loaded(base);

        engine.handle_response(base, &Request::SessionStats, Err(error.clone()));

        assert_eq!(
            engine.connection,
            reconnecting(1, ConnectionError::from(&error)),
            "{error:?}"
        );
        assert_eq!(engine.due_requests(base + ms(999)), [], "{error:?}");
        assert_eq!(
            engine.due_requests(base + secs(1)),
            [Request::CheckServer],
            "{error:?}"
        );
    }
}

#[test]
fn poll_errors_the_daemon_reports_keep_the_connection() {
    let base = Instant::now();
    let free_space = Request::FreeSpace(DOWNLOAD_DIR.to_owned());
    for error in [
        RpcError::Server {
            code: 5,
            message: "system error".to_owned(),
            detail: Some("No such file or directory".to_owned()),
        },
        RpcError::InvalidRequest("cannot encode".to_owned()),
        RpcError::Selection(IdError::EmptySelection),
    ] {
        let mut engine = loaded(base);

        engine.handle_response(base, &free_space, Err(error.clone()));

        assert_eq!(engine.connection, ConnectionState::Connected, "{error:?}");
        // Tried again at its next interval, not before.
        let (due, _) = exchange(&mut engine, base + ms(29_999));
        assert!(!due.contains(&free_space), "{error:?}");
        assert!(engine.due_requests(base + secs(30)).contains(&free_space));
    }
}

#[test]
fn unsupported_server_error_on_a_poll_stops_polling() {
    let base = Instant::now();
    let mut engine = loaded(base);

    engine.handle_response(
        base,
        &Request::FullList,
        Err(RpcError::UnsupportedServer {
            rpc_version: "5.4.0".to_owned(),
        }),
    );

    assert_eq!(
        engine.connection,
        ConnectionState::Unsupported {
            rpc_version: "5.4.0".to_owned()
        }
    );
    assert_eq!(engine.due_requests(base + secs(60)), []);
}

#[test]
fn a_server_check_the_daemon_refuses_is_retried_with_backoff() {
    let base = Instant::now();
    let mut engine = Engine::new(EngineConfig::default(), base);
    engine.due_requests(base);
    let error = RpcError::Server {
        code: -32601,
        message: "Method not found".to_owned(),
        detail: None,
    };

    engine.handle_response(base, &Request::CheckServer, Err(error.clone()));

    assert_eq!(
        engine.connection,
        reconnecting(1, ConnectionError::from(&error))
    );
    assert_eq!(engine.due_requests(base + secs(1)), [Request::CheckServer]);
}

#[test]
fn errors_after_the_connection_dropped_do_not_bump_the_attempt() {
    let base = Instant::now();
    let mut engine = loaded(base);
    engine.handle_command(base, Command::Run(ActionId(1), stop_first()));
    let tick = base + ms(1500);
    let due = engine.due_requests(tick);
    assert_eq!(
        due,
        [
            Request::Action(ActionId(1), stop_first()),
            Request::RecentlyActive,
            Request::SessionStats
        ]
    );

    // The action fails first; the polls of the same batch fail after it.
    let events = engine.handle_response(tick, &due[0], Err(timeout()));
    assert_eq!(
        events,
        [finished(1, Err(ActionError::Rpc(ConnectionError::Timeout)))]
    );
    for request in &due[1..] {
        assert_eq!(engine.handle_response(tick, request, Err(timeout())), []);
    }

    assert_eq!(engine.connection, reconnecting(1, ConnectionError::Timeout));
    assert_eq!(engine.due_requests(tick + secs(1)), [Request::CheckServer]);
}

#[test]
fn a_late_action_error_while_reconnecting_does_not_bump_the_attempt() {
    let base = Instant::now();
    let mut engine = loaded(base);
    engine.handle_command(base, Command::Run(ActionId(2), stop_first()));
    let due = engine.due_requests(base);
    engine.handle_response(base, &Request::SessionStats, Err(timeout()));

    let events = engine.handle_response(base, &due[0], Err(timeout()));

    assert_eq!(
        events,
        [finished(2, Err(ActionError::Rpc(ConnectionError::Timeout)))]
    );
    assert_eq!(engine.connection, reconnecting(1, ConnectionError::Timeout));
}

#[test]
fn late_poll_answers_are_ignored_while_not_connected() {
    let base = Instant::now();
    let at = base + ms(1500);
    let mut engine = dropped(base, at);
    let before = engine.take_snapshot().unwrap();
    let mut fewer = list();
    fewer.torrents.truncate(1);

    let late = [
        (Request::FullList, Ok(Response::Torrents(fewer))),
        (Request::SessionStats, Err(timeout())),
        (Request::SessionGet, Err(RpcError::Forbidden)),
    ];
    for (request, result) in late {
        assert_eq!(engine.handle_response(at, &request, result), []);
    }

    assert_eq!(engine.connection, reconnecting(1, ConnectionError::Timeout));
    assert_eq!(engine.take_snapshot(), None);
    assert_eq!(engine.store.len(), before.torrents.len());
}

#[test]
fn a_mismatched_answer_counts_as_an_invalid_response() {
    let base = Instant::now();
    let mut engine = loaded(base);

    engine.handle_response(base, &Request::SessionStats, Ok(Response::Done));

    assert!(
        matches!(
            &engine.connection,
            ConnectionState::Reconnecting {
                attempt: 1,
                last_error: ConnectionError::Protocol(_)
            }
        ),
        "{:?}",
        engine.connection
    );
}

#[test]
fn is_current_follows_the_connection_and_the_selection() {
    let base = Instant::now();
    let polls = [
        Request::SessionGet,
        Request::SessionStats,
        Request::FreeSpace(DOWNLOAD_DIR.to_owned()),
        Request::FullList,
        Request::RecentlyActive,
    ];
    let action = Request::Action(ActionId(1), stop_first());
    let details = Request::Details(first());
    let mut engine = Engine::new(EngineConfig::default(), base);
    engine.handle_command(base, Command::Select(Some(first())));

    // Connecting: only the server check and actions.
    assert!(engine.is_current(&Request::CheckServer));
    assert!(engine.is_current(&action));
    for poll in polls.iter().chain([&details]) {
        assert!(!engine.is_current(poll), "{poll:?} while connecting");
    }

    let mut engine = loaded(base);
    for poll in &polls {
        assert!(engine.is_current(poll), "{poll:?} while connected");
    }
    assert!(!engine.is_current(&details), "nothing is selected");
    engine.handle_command(base, Command::Select(Some(first())));
    assert!(engine.is_current(&details));
    engine.handle_command(base, Command::Select(Some(second())));
    assert!(!engine.is_current(&details));
    assert!(engine.is_current(&Request::Details(second())));

    engine.handle_response(base, &Request::SessionStats, Err(timeout()));
    for poll in polls.iter().chain([&Request::Details(second())]) {
        assert!(!engine.is_current(poll), "{poll:?} while reconnecting");
    }
    assert!(engine.is_current(&Request::CheckServer));
    assert!(engine.is_current(&action));
}

#[test]
fn action_while_disconnected_fails_fast() {
    let base = Instant::now();
    let mut engine = Engine::new(EngineConfig::default(), base);

    let events = engine.handle_command(base, Command::Run(ActionId(1), stop_first()));

    assert_eq!(events, [finished(1, Err(ActionError::NotConnected))]);
    assert_eq!(engine.due_requests(base), [Request::CheckServer]);

    let at = base + ms(1500);
    let mut engine = dropped(base, at);
    let events = engine.handle_command(at, Command::Run(ActionId(2), Action::Start(vec![first()])));
    assert_eq!(events, [finished(2, Err(ActionError::NotConnected))]);
    assert_eq!(engine.due_requests(at + secs(1)), [Request::CheckServer]);
}

#[test]
fn queued_actions_fail_when_the_connection_drops() {
    let base = Instant::now();
    let mut engine = loaded(base);
    assert_eq!(
        engine.handle_command(base, Command::Run(ActionId(3), stop_first())),
        []
    );

    let events = engine.handle_response(base, &Request::SessionStats, Err(timeout()));

    assert_eq!(events, [finished(3, Err(ActionError::NotConnected))]);
    assert_eq!(engine.due_requests(base), []);
}

#[test]
fn empty_action_fails_without_request() {
    let base = Instant::now();
    let mut engine = loaded(base);

    let events = engine.handle_command(base, Command::Run(ActionId(4), Action::Stop(Vec::new())));

    assert_eq!(events, [finished(4, Err(ActionError::EmptySelection))]);
    assert_eq!(engine.due_requests(base), []);
}

#[test]
fn empty_file_list_fails_without_request() {
    let base = Instant::now();
    let mut engine = loaded(base);
    let actions = [
        Action::SetFilePriority {
            hash: first(),
            files: Vec::new(),
            priority: FilePriority::High,
        },
        Action::SetFilesWanted {
            hash: first(),
            files: Vec::new(),
            wanted: false,
        },
    ];

    for (id, action) in (5..).zip(actions) {
        let events = engine.handle_command(base, Command::Run(ActionId(id), action));
        assert_eq!(events, [finished(id, Err(ActionError::EmptySelection))]);
    }
    assert_eq!(engine.due_requests(base), []);
}

#[test]
fn empty_remove_fails_without_request() {
    let base = Instant::now();
    let mut engine = loaded(base);
    let remove = Action::Remove {
        hashes: Vec::new(),
        data: RemoveData::Delete,
    };

    let events = engine.handle_command(base, Command::Run(ActionId(7), remove.clone()));

    assert_eq!(events, [finished(7, Err(ActionError::EmptySelection))]);
    assert_eq!(engine.due_requests(base), []);

    // Refused as empty even when not connected, so the UI learns about its bug.
    let mut engine = Engine::new(EngineConfig::default(), base);
    let events = engine.handle_command(base, Command::Run(ActionId(8), remove));
    assert_eq!(events, [finished(8, Err(ActionError::EmptySelection))]);
}

#[test]
fn action_success_pulls_tick_forward() {
    let base = Instant::now();
    let mut engine = loaded(base);
    let now = base + ms(500);
    assert_eq!(
        engine.handle_command(now, Command::Run(ActionId(9), stop_first())),
        []
    );
    assert_eq!(engine.next_wakeup(now), now);

    let due = engine.due_requests(now);
    assert_eq!(due, [Request::Action(ActionId(9), stop_first())]);
    let events = engine.handle_response(now, &due[0], Ok(Response::Done));

    assert_eq!(events, [finished(9, Ok(()))]);
    assert_eq!(
        engine.due_requests(now),
        [Request::RecentlyActive, Request::SessionStats]
    );
}

#[test]
fn add_and_remove_success_pull_tick_forward_too() {
    let base = Instant::now();
    let remove = Action::Remove {
        hashes: vec![first()],
        data: RemoveData::Keep,
    };
    for action in [Action::Add(magnet()), remove] {
        let mut engine = loaded(base);
        let now = base + ms(500);
        engine.handle_command(now, Command::Run(ActionId(10), action.clone()));

        let (due, events) = exchange(&mut engine, now);

        assert_eq!(due, [Request::Action(ActionId(10), action.clone())]);
        assert_eq!(events, [finished(10, Ok(()))]);
        assert_eq!(
            engine.due_requests(now),
            [Request::RecentlyActive, Request::SessionStats],
            "{action:?}"
        );
    }
}

#[test]
fn a_speed_action_reads_the_session_again() {
    let base = Instant::now();
    let mut engine = loaded(base);
    let now = base + ms(500);
    engine.handle_command(now, Command::Run(ActionId(11), Action::SetAltSpeed(true)));

    exchange(&mut engine, now);

    assert_eq!(
        engine.due_requests(now),
        [
            Request::SessionGet,
            Request::RecentlyActive,
            Request::SessionStats
        ]
    );
}

#[test]
fn a_refused_action_reports_why_and_keeps_the_connection() {
    let base = Instant::now();
    let mut engine = loaded(base);
    engine.handle_command(base, Command::Run(ActionId(12), stop_first()));
    let due = engine.due_requests(base);
    let error = RpcError::Server {
        code: 1,
        message: "system error".to_owned(),
        detail: Some("torrent is busy".to_owned()),
    };

    let events = engine.handle_response(base, &due[0], Err(error));

    assert_eq!(
        events,
        [finished(
            12,
            Err(ActionError::Rpc(ConnectionError::Server(
                "torrent is busy".to_owned()
            )))
        )]
    );
    assert_eq!(engine.connection, ConnectionState::Connected);
    // A refused action changed nothing, so nothing is pulled forward.
    assert_eq!(engine.due_requests(base + ms(1)), []);
}

#[test]
fn an_action_that_gets_no_answer_reconnects() {
    let base = Instant::now();
    let mut engine = loaded(base);
    engine.handle_command(base, Command::Run(ActionId(13), stop_first()));
    let due = engine.due_requests(base);

    let events = engine.handle_response(base, &due[0], Err(timeout()));

    assert_eq!(
        events,
        [finished(
            13,
            Err(ActionError::Rpc(ConnectionError::Timeout))
        )]
    );
    assert_eq!(engine.connection, reconnecting(1, ConnectionError::Timeout));
}

#[test]
fn details_only_for_selection() {
    let base = Instant::now();
    let mut engine = loaded(base);
    let (due, _) = exchange(&mut engine, base + ms(1500));
    assert_eq!(due, [Request::RecentlyActive, Request::SessionStats]);

    let now = base + ms(2000);
    engine.handle_command(now, Command::Select(Some(first())));
    assert_eq!(engine.due_requests(now), [Request::Details(first())]);
    for tick in [3000, 4500] {
        assert_eq!(
            engine.due_requests(base + ms(tick)),
            [
                Request::RecentlyActive,
                Request::SessionStats,
                Request::Details(first())
            ],
            "at {tick} ms"
        );
    }

    engine.handle_command(base + ms(5000), Command::Select(None));
    assert_eq!(
        engine.due_requests(base + ms(6000)),
        [Request::RecentlyActive, Request::SessionStats]
    );
}

#[test]
fn a_selection_made_before_connecting_is_read_on_connecting() {
    let base = Instant::now();
    let mut engine = Engine::new(EngineConfig::default(), base);
    engine.handle_command(base, Command::Select(Some(first())));
    assert_eq!(engine.due_requests(base), [Request::CheckServer]);

    engine.handle_response(
        base,
        &Request::CheckServer,
        Ok(answer(&Request::CheckServer)),
    );

    assert_eq!(
        engine.due_requests(base),
        [
            Request::SessionGet,
            Request::FullList,
            Request::SessionStats,
            Request::Details(first())
        ]
    );
}

#[test]
fn details_compute_playable() {
    let base = Instant::now();
    let mut engine = loaded(base);
    engine.handle_command(base, Command::Select(Some(first())));

    let (due, events) = exchange(&mut engine, base);

    assert_eq!(due, [Request::Details(first())]);
    let [details] = details_events(&events).try_into().unwrap();
    assert_eq!(details.hash, first());
    assert_eq!(details.files.len(), 10);
    for file in &details.files {
        assert_eq!(file.bytes_completed, file.length, "{}", file.name);
        assert_eq!(file.playable_from_start, file.length, "{}", file.name);
    }
}

#[test]
fn identical_details_send_no_event() {
    let base = Instant::now();
    let mut engine = loaded(base);
    engine.handle_command(base, Command::Select(Some(first())));
    assert_eq!(details_events(&exchange(&mut engine, base).1).len(), 1);

    let (due, events) = exchange(&mut engine, base + ms(1500));

    assert!(due.contains(&Request::Details(first())));
    assert_eq!(details_events(&events), []);
}

#[test]
fn selecting_another_torrent_drops_the_details() {
    let base = Instant::now();
    let mut engine = loaded(base);
    engine.handle_command(base, Command::Select(Some(first())));
    exchange(&mut engine, base);

    engine.handle_command(base, Command::Select(Some(second())));
    assert_eq!(engine.details, None);
    engine.handle_command(base, Command::Select(Some(first())));

    // The same details again are news: the UI dropped them with the selection.
    let (due, events) = exchange(&mut engine, base);
    assert_eq!(due, [Request::Details(first())]);
    assert_eq!(details_events(&events).len(), 1);
}

#[test]
fn details_for_a_deselected_torrent_are_ignored() {
    let base = Instant::now();
    let mut engine = loaded(base);
    engine.handle_command(base, Command::Select(Some(first())));
    let due = engine.due_requests(base);
    assert_eq!(due, [Request::Details(first())]);

    engine.handle_command(base, Command::Select(Some(second())));

    assert!(!engine.is_current(&due[0]));
    assert_eq!(
        engine.handle_response(base, &due[0], Ok(answer(&due[0]))),
        []
    );
    assert_eq!(engine.details, None);
}

#[test]
fn an_empty_details_answer_sends_no_event() {
    let base = Instant::now();
    let mut engine = loaded(base);
    engine.handle_command(base, Command::Select(Some(first())));
    let due = engine.due_requests(base);

    let empty = TorrentGetResponse {
        torrents: Vec::new(),
        removed: Vec::new(),
    };
    let events = engine.handle_response(base, &due[0], Ok(Response::Details(empty)));

    assert_eq!(events, []);
    assert_eq!(engine.details, None);
    assert_eq!(engine.connection, ConnectionState::Connected);
}

#[test]
fn details_issues_are_kept_until_they_change() {
    let base = Instant::now();
    let mut engine = loaded(base);
    engine.handle_command(base, Command::Select(Some(first())));
    let other = TorrentGetResponse {
        torrents: vec![TorrentRecord {
            hash_string: Some(SECOND.to_owned()),
            ..TorrentRecord::default()
        }],
        removed: Vec::new(),
    };

    let events = engine.handle_response(
        base,
        &Request::Details(first()),
        Ok(Response::Details(other)),
    );

    assert_eq!(events, []);
    assert_eq!(engine.details_issues, [DetailsIssue::NoMatchingRecord]);
    // A sound answer clears them.
    exchange(&mut engine, base);
    assert_eq!(engine.details_issues, []);
}

#[test]
fn snapshot_only_when_changed() {
    let base = Instant::now();
    let mut engine = Engine::new(EngineConfig::default(), base);

    let first_snapshot = engine.take_snapshot().unwrap();
    assert_eq!(first_snapshot.connection, ConnectionState::Connecting);
    assert!(first_snapshot.stale);
    assert_eq!(first_snapshot.torrents, []);
    assert_eq!(engine.take_snapshot(), None);

    let mut engine = loaded(base);
    assert!(engine.take_snapshot().is_some());
    assert_eq!(engine.take_snapshot(), None);
}

#[test]
fn an_identical_recent_answer_changes_nothing() {
    let base = Instant::now();
    let mut engine = loaded(base);
    engine.take_snapshot();

    let (due, _) = exchange(&mut engine, base + ms(1500));

    assert_eq!(due, [Request::RecentlyActive, Request::SessionStats]);
    assert_eq!(engine.take_snapshot(), None);
}

#[test]
fn a_recent_answer_updates_and_removes_torrents() {
    let base = Instant::now();
    let mut engine = loaded(base);
    engine.take_snapshot();
    let mut update = recent();
    update.torrents[0].rate_upload = Some(1);
    // The sixth torrent of the list has id 7.
    update.removed = vec![TorrentId(7)];

    engine.handle_response(
        base,
        &Request::RecentlyActive,
        Ok(Response::Torrents(update)),
    );

    let snapshot = engine.take_snapshot().unwrap();
    assert_eq!(snapshot.torrents.len(), LISTED - 1);
    let updated = snapshot
        .torrents
        .iter()
        .find(|t| t.hash == first())
        .unwrap();
    assert_eq!(updated.rate_up, 1);
}

#[test]
fn snapshot_holds_the_session_the_rates_and_the_free_space() {
    let base = Instant::now();
    let mut engine = loaded(base);

    let snapshot = engine.take_snapshot().unwrap();

    assert_eq!(snapshot.connection, ConnectionState::Connected);
    assert!(!snapshot.stale);
    assert_eq!(snapshot.torrents.len(), LISTED);
    assert_eq!(
        snapshot.session,
        Some(SessionSummary::from(&session_info()))
    );
    assert_eq!((snapshot.rate_down, snapshot.rate_up), (0, 163_970));
    assert_eq!(snapshot.free_space_bytes, Some(819_158_872_064));
}

#[test]
fn unknown_free_space_is_none() {
    let base = Instant::now();
    let mut engine = loaded(base);
    engine.take_snapshot();

    engine.handle_response(
        base,
        &Request::FreeSpace(DOWNLOAD_DIR.to_owned()),
        Ok(free_space(DOWNLOAD_DIR, -1)),
    );

    assert_eq!(engine.take_snapshot().unwrap().free_space_bytes, None);
}

#[test]
fn free_space_follows_the_download_dir() {
    let base = Instant::now();
    let mut engine = loaded(base);
    let now = base + ms(700);
    let moved = SessionInfo {
        download_dir: "/data/other".to_owned(),
        ..session_info()
    };

    engine.handle_response(now, &Request::SessionGet, Ok(Response::Session(moved)));

    assert_eq!(
        engine.due_requests(now),
        [Request::FreeSpace("/data/other".to_owned())]
    );
    // A late answer for the old directory is dropped.
    engine.handle_response(
        now,
        &Request::FreeSpace(DOWNLOAD_DIR.to_owned()),
        Ok(free_space(DOWNLOAD_DIR, 5)),
    );
    assert_eq!(engine.free_space_bytes, Some(819_158_872_064));
    engine.handle_response(
        now,
        &Request::FreeSpace("/data/other".to_owned()),
        Ok(free_space("/data/other", 5)),
    );
    assert_eq!(engine.free_space_bytes, Some(5));
}

#[test]
fn renumbered_ids_make_a_full_list_due_now() {
    let base = Instant::now();
    let now = base + ms(1500);

    // The first torrent comes back under a new id.
    let mut moved = recent();
    moved.torrents[0].id = Some(TorrentId(9));
    // A torrent nobody knows arrives under the id of the second torrent.
    let mut taken = recent();
    taken.torrents[0].hash_string = Some("00000000000000000000000000000000000000aa".to_owned());
    taken.torrents[0].id = Some(TorrentId(2));

    for update in [moved, taken] {
        let mut engine = loaded(base);
        engine.due_requests(now);

        engine.handle_response(
            now,
            &Request::RecentlyActive,
            Ok(Response::Torrents(update)),
        );

        assert_eq!(engine.due_requests(now), [Request::FullList]);
    }
}

#[test]
fn a_new_torrent_under_a_new_id_needs_no_full_list() {
    let base = Instant::now();
    let now = base + ms(1500);
    let mut added = recent();
    added.torrents[0].hash_string = Some("00000000000000000000000000000000000000aa".to_owned());
    added.torrents[0].id = Some(TorrentId(8));
    let mut engine = loaded(base);
    engine.due_requests(now);

    engine.handle_response(now, &Request::RecentlyActive, Ok(Response::Torrents(added)));

    assert_eq!(engine.due_requests(now), []);
    assert_eq!(engine.store.len(), LISTED + 1);
}

#[test]
fn the_engine_keeps_the_records_it_could_not_show() {
    let base = Instant::now();
    let mut engine = connected(base);
    let mut broken = list();
    broken.torrents[1].hash_string = Some("not-a-hash".to_owned());

    engine.handle_response(base, &Request::FullList, Ok(Response::Torrents(broken)));

    assert_eq!(engine.store.len(), LISTED - 1);
    assert_eq!(
        engine.skipped,
        SkippedSet(HashSet::from([SkippedRecord {
            hash_prefix: Some("not-a-ha".to_owned()),
            reason: SkipReason::InvalidHash,
        }]))
    );
    engine.handle_response(base, &Request::FullList, Ok(answer(&Request::FullList)));
    assert_eq!(engine.skipped, SkippedSet::default());
}

#[test]
fn refresh_while_connected_reads_everything_now() {
    let base = Instant::now();
    let mut engine = loaded(base);
    let now = base + ms(700);

    assert_eq!(engine.handle_command(now, Command::Refresh), []);

    assert_eq!(engine.connection, ConnectionState::Connected);
    assert_eq!(
        engine.due_requests(now),
        [
            Request::SessionGet,
            Request::FullList,
            Request::SessionStats
        ]
    );
}

#[test]
fn refresh_or_reconnect_while_disconnected_connects_now() {
    let base = Instant::now();
    for command in [Command::Refresh, Command::Reconnect] {
        let mut engine = dropped(base, base);
        engine.due_requests(base + secs(1));
        engine.handle_response(base + secs(1), &Request::CheckServer, Err(timeout()));
        assert_eq!(engine.connection, reconnecting(2, ConnectionError::Timeout));

        let now = base + secs(1) + ms(100);
        assert_eq!(engine.handle_command(now, command.clone()), []);

        assert_eq!(
            engine.connection,
            ConnectionState::Connecting,
            "{command:?}"
        );
        assert_eq!(engine.due_requests(now), [Request::CheckServer]);
        // The backoff starts over.
        engine.handle_response(now, &Request::CheckServer, Err(timeout()));
        assert_eq!(engine.connection, reconnecting(1, ConnectionError::Timeout));
        assert_eq!(engine.next_wakeup(now), now + secs(1));
    }
}

#[test]
fn reconnect_while_connected_checks_the_server_again() {
    let base = Instant::now();
    let mut engine = loaded(base);
    engine.handle_command(base, Command::Run(ActionId(14), stop_first()));

    let events = engine.handle_command(base, Command::Reconnect);

    assert_eq!(events, [finished(14, Err(ActionError::NotConnected))]);
    assert_eq!(engine.connection, ConnectionState::Connecting);
    assert_eq!(engine.due_requests(base), [Request::CheckServer]);
}

#[test]
fn shutdown_changes_nothing() {
    let base = Instant::now();
    let mut engine = loaded(base);
    let before = engine.clone();

    assert_eq!(engine.handle_command(base, Command::Shutdown), []);

    assert_eq!(engine, before);
}

#[test]
fn next_wakeup_is_the_earliest_due_request() {
    let base = Instant::now();
    let engine = Engine::new(EngineConfig::default(), base);
    assert_eq!(engine.next_wakeup(base), base);

    let mut engine = loaded(base);
    assert_eq!(engine.next_wakeup(base + ms(100)), base + ms(1500));
    // An overdue tick is due now, not in the past.
    assert_eq!(engine.next_wakeup(base + secs(5)), base + secs(5));
    engine.handle_command(base + ms(100), Command::Select(Some(first())));
    assert_eq!(engine.next_wakeup(base + ms(100)), base + ms(100));

    let at = base + ms(1500);
    let engine = dropped(base, at);
    assert_eq!(engine.next_wakeup(at), at + secs(1));
}

#[test]
fn next_wakeup_is_a_minute_away_when_nothing_is_scheduled() {
    let base = Instant::now();
    let mut engine = Engine::new(EngineConfig::default(), base);
    engine.due_requests(base);
    engine.handle_response(
        base,
        &Request::CheckServer,
        Ok(Response::Server(version("5.0.0"))),
    );

    let now = base + secs(5);
    assert_eq!(engine.next_wakeup(now), now + secs(60));
}
