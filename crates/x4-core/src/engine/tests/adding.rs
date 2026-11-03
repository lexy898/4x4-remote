//! Tests of adding a torrent: its outcome reaches the UI as [`Event::AddFinished`], never as
//! [`Event::ActionFinished`], and an add that gets no answer within its long timeout leaves the
//! connection as it is.

use x4_rpc::{AddOutcome, AddedTorrent};

use super::*;

/// The hash of the torrent the daemon reports for an add.
const ADDED: &str = "00000000000000000000000000000000000000aa";

fn added_hash() -> HashString {
    HashString::parse(ADDED).unwrap()
}

fn added_torrent() -> AddedTorrent {
    AddedTorrent {
        id: TorrentId(99),
        name: "New torrent".to_owned(),
        hash: added_hash(),
    }
}

fn add_finished(id: u64, result: Result<Added, AddError>) -> Event {
    Event::AddFinished {
        id: ActionId(id),
        result,
    }
}

fn add_request(id: u64) -> Request {
    Request::Action(ActionId(id), Action::Add(magnet()))
}

/// An engine that loaded at `base` and has handed out the add `id` at `now`.
fn adding(base: Instant, now: Instant, id: u64) -> Engine {
    let mut engine = loaded(base);
    let run = Command::Run(ActionId(id), Action::Add(magnet()));
    assert_eq!(command(&mut engine, now, run), []);
    assert_eq!(hand_out(&mut engine, now), [add_request(id)]);
    engine
}

fn server_error(code: i64, message: &str, detail: Option<&str>) -> RpcError {
    RpcError::Server {
        code,
        message: message.to_owned(),
        detail: detail.map(str::to_owned),
    }
}

#[test]
fn an_add_answers_with_the_torrent_it_added_or_found() {
    let base = Instant::now();
    let now = base + ms(500);

    let mut engine = adding(base, now, 30);
    let events = respond(
        &mut engine,
        now,
        &add_request(30),
        Ok(Response::Added(AddOutcome::Added(added_torrent()))),
    );
    assert_eq!(
        events,
        [add_finished(
            30,
            Ok(Added {
                hash: added_hash(),
                name: "New torrent".to_owned(),
                duplicate: false,
            })
        )]
    );
    // The new torrent shows in the next tick, which comes at once.
    assert_eq!(
        hand_out(&mut engine, now),
        [Request::RecentlyActive, Request::SessionStats]
    );

    let mut engine = adding(base, now, 31);
    let events = respond(
        &mut engine,
        now,
        &add_request(31),
        Ok(Response::Added(AddOutcome::Duplicate(added_torrent()))),
    );
    assert_eq!(
        events,
        [add_finished(
            31,
            Ok(Added {
                hash: added_hash(),
                name: "New torrent".to_owned(),
                duplicate: true,
            })
        )]
    );
    // The daemon changed nothing, so nothing is read sooner than usual.
    assert_eq!(hand_out(&mut engine, now), []);
    assert_eq!(engine.connection, ConnectionState::Connected);
}

#[test]
fn an_add_that_times_out_is_unconfirmed_and_keeps_the_connection() {
    let base = Instant::now();
    let now = base + ms(500);
    let mut engine = adding(base, now, 32);
    engine.take_snapshot();

    let events = respond(&mut engine, now, &add_request(32), Err(timeout()));

    assert_eq!(events, [add_finished(32, Err(AddError::Unconfirmed))]);
    assert_eq!(engine.connection, ConnectionState::Connected);
    assert_eq!(engine.take_snapshot(), None);
    // No reconnect and no server check: the polls go on as before, and they tell whether the
    // torrent was added.
    assert_eq!(hand_out(&mut engine, now), []);
    assert_eq!(
        hand_out(&mut engine, base + ms(1500)),
        [Request::RecentlyActive, Request::SessionStats]
    );
}

#[test]
fn only_an_add_that_times_out_keeps_the_connection() {
    let add = add_request(1);
    let others = [
        Request::CheckServer,
        Request::SessionGet,
        Request::SessionStats,
        Request::FreeSpace(DOWNLOAD_DIR.to_owned()),
        Request::FolderFreeSpace(DownloadDir::new(DOWNLOAD_DIR).unwrap()),
        Request::FullList,
        Request::RecentlyActive,
        Request::Details {
            hash: first(),
            kind: DetailsRead::Full,
        },
        Request::Action(ActionId(1), stop_first()),
        Request::MagnetLinks(ActionId(1), vec![first()]),
    ];
    let timeout = timeout();
    let unreachable = RpcError::Transport(TransportError::Connect("refused".to_owned()));
    let broken = RpcError::Transport(TransportError::Io("reset".to_owned()));
    let refused = server_error(4, "unrecognized info", None);

    assert_eq!(Failure::of(&add, &timeout), Failure::Unanswered);
    for request in &others {
        assert_eq!(
            Failure::of(request, &timeout),
            Failure::Unusable,
            "{request:?}"
        );
    }
    // Only running out of time leaves the outcome open; a call that never reached the daemon or
    // broke off says the daemon cannot be used, as for any other call.
    assert_eq!(Failure::of(&add, &unreachable), Failure::Unusable);
    assert_eq!(Failure::of(&add, &broken), Failure::Unusable);
    assert_eq!(Failure::of(&add, &refused), Failure::Refused);
}

#[test]
fn an_add_the_daemon_refuses_says_why_and_keeps_the_connection() {
    let base = Instant::now();
    let now = base + ms(500);
    for (error, why) in [
        (
            server_error(4, "unrecognized info", None),
            "unrecognized info",
        ),
        (
            server_error(
                3,
                "path is not absolute",
                Some("download directory path is not absolute"),
            ),
            "download directory path is not absolute",
        ),
    ] {
        let mut engine = adding(base, now, 33);

        let events = respond(&mut engine, now, &add_request(33), Err(error));

        assert_eq!(
            events,
            [add_finished(33, Err(AddError::Daemon(why.to_owned())))]
        );
        assert_eq!(engine.connection, ConnectionState::Connected);
        // A refused add changed nothing, so nothing is pulled forward.
        assert_eq!(hand_out(&mut engine, now), []);
    }
}

#[test]
fn an_add_that_cannot_reach_the_daemon_fails_and_reconnects() {
    let base = Instant::now();
    let now = base + ms(500);
    for error in [
        TransportError::Connect("connection refused".to_owned()),
        TransportError::Io("connection reset".to_owned()),
    ] {
        let mut engine = adding(base, now, 34);

        let events = respond(
            &mut engine,
            now,
            &add_request(34),
            Err(RpcError::Transport(error)),
        );

        assert_eq!(
            events,
            [add_finished(
                34,
                Err(AddError::Failed(ConnectionError::Unreachable))
            )]
        );
        assert_eq!(
            engine.connection,
            reconnecting(1, ConnectionError::Unreachable)
        );
    }
}

#[test]
fn an_add_answered_with_something_else_is_a_protocol_error() {
    let base = Instant::now();
    let now = base + ms(500);
    let mut engine = adding(base, now, 35);

    let events = respond(&mut engine, now, &add_request(35), Ok(Response::Done));

    assert!(
        matches!(
            events.as_slice(),
            [Event::AddFinished {
                id: ActionId(35),
                result: Err(AddError::Failed(ConnectionError::Protocol(_)))
            }]
        ),
        "{events:?}"
    );
    assert!(
        matches!(engine.connection, ConnectionState::Reconnecting { .. }),
        "{:?}",
        engine.connection
    );
}

#[test]
fn an_add_that_is_not_sent_is_answered_as_an_add() {
    let base = Instant::now();
    let run = |id| Command::Run(ActionId(id), Action::Add(magnet()));

    // Not connected: refused at once, never queued.
    let mut engine = Engine::new(EngineConfig::default(), base);
    assert_eq!(
        command(&mut engine, base, run(36)),
        [add_finished(36, Err(AddError::NotConnected))]
    );
    assert_eq!(hand_out(&mut engine, base), [Request::CheckServer]);

    // Queued, then the connection drops: refused with the actions, in the order they came.
    let mut engine = loaded(base);
    assert_eq!(command(&mut engine, base, run(37)), []);
    assert_eq!(
        command(&mut engine, base, Command::Run(ActionId(38), stop_first())),
        []
    );
    let events = respond(&mut engine, base, &Request::SessionStats, Err(timeout()));
    assert_eq!(
        events,
        [
            add_finished(37, Err(AddError::NotConnected)),
            finished(38, Err(ActionError::NotConnected)),
        ]
    );
    assert_eq!(hand_out(&mut engine, base), []);
}

#[test]
fn an_add_the_client_refuses_to_send_fails_and_keeps_the_connection() {
    let base = Instant::now();
    let now = base + ms(500);
    let mut engine = adding(base, now, 39);

    let events = respond(
        &mut engine,
        now,
        &add_request(39),
        Err(RpcError::MagnetWithFileChoices),
    );

    assert_eq!(
        events,
        [add_finished(
            39,
            Err(AddError::Failed(ConnectionError::Protocol(
                RpcError::MagnetWithFileChoices.to_string()
            )))
        )]
    );
    assert_eq!(engine.connection, ConnectionState::Connected);
}
