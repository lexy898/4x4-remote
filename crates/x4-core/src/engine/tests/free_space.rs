//! Tests of the free space of a folder the user is choosing ([`Command::FreeSpace`]): one call
//! at a time, the newest question only, a walk up to the nearest parent that exists, and nothing
//! sent while the daemon is not connected.

use super::*;

use crate::engine::folder_space::MIN_GAP;

fn dir(path: &str) -> DownloadDir {
    DownloadDir::new(path).unwrap()
}

fn ask(path: &str) -> Command {
    Command::FreeSpace(dir(path))
}

fn folder(path: &str) -> Request {
    Request::FolderFreeSpace(dir(path))
}

fn space(path: &str, answer: FreeSpaceAnswer) -> Event {
    Event::FreeSpace {
        path: dir(path),
        answer,
    }
}

/// An engine that loaded at `base` and polls nothing else for an hour, so that the folder calls
/// of a test are its only requests.
fn quiet(base: Instant) -> Engine {
    let hour = secs(3600);
    let config = EngineConfig {
        visible_interval: hour,
        hidden_interval: hour,
        full_refresh_interval: hour,
        session_interval: hour,
        free_space_interval: hour,
        ..EngineConfig::default()
    };
    let mut engine = Engine::new(config, base);
    while !exchange(&mut engine, base).0.is_empty() {}
    assert_eq!(engine.connection, ConnectionState::Connected);
    engine
}

/// What the daemon answers for a folder that does not exist (live, 4.1.3).
fn no_such_folder() -> RpcError {
    RpcError::Server {
        code: 5,
        message: "system error".to_owned(),
        detail: Some("No such file or directory".to_owned()),
    }
}

#[test]
fn free_space_of_a_folder_that_exists_is_read_once() {
    let base = Instant::now();
    let now = base + ms(500);
    let mut engine = loaded(base);

    assert_eq!(command(&mut engine, now, ask("/data/films")), []);
    assert_eq!(engine.next_wakeup(now), now);
    assert_eq!(hand_out(&mut engine, now), [folder("/data/films")]);
    let events = respond(
        &mut engine,
        now,
        &folder("/data/films"),
        Ok(free_space("/data/films", 7_000)),
    );

    assert_eq!(
        events,
        [space("/data/films", FreeSpaceAnswer::Free { bytes: 7_000 })]
    );
    assert_eq!(hand_out(&mut engine, now), []);
    assert_eq!(engine.connection, ConnectionState::Connected);
}

#[test]
fn free_space_of_a_missing_folder_walks_up_to_its_parent() {
    let base = Instant::now();
    let mut now = base + ms(500);
    let mut engine = quiet(base);

    command(&mut engine, now, ask("/data/new/a/b"));
    let mut calls = Vec::new();
    for path in ["/data/new/a/b", "/data/new/a"] {
        let due = hand_out(&mut engine, now);
        assert_eq!(due, [folder(path)]);
        calls.extend(due);
        assert_eq!(
            respond(&mut engine, now, &folder(path), Err(no_such_folder())),
            []
        );
        now += MIN_GAP;
    }
    let due = hand_out(&mut engine, now);
    assert_eq!(due, [folder("/data/new")]);
    calls.extend(due);
    let events = respond(
        &mut engine,
        now,
        &folder("/data/new"),
        Ok(free_space("/data/new", 5_000)),
    );

    assert_eq!(
        events,
        [space(
            "/data/new/a/b",
            FreeSpaceAnswer::Missing {
                parent: Some(dir("/data/new")),
                free: Some(5_000),
            }
        )]
    );
    assert_eq!(calls.len(), 3);
    now += MIN_GAP;
    assert_eq!(hand_out(&mut engine, now), []);

    // No parent within three calls: the question ends there, with none found.
    command(&mut engine, now, ask("/x/y/z/w"));
    for path in ["/x/y/z/w", "/x/y/z"] {
        assert_eq!(hand_out(&mut engine, now), [folder(path)]);
        assert_eq!(
            respond(&mut engine, now, &folder(path), Err(no_such_folder())),
            []
        );
        now += MIN_GAP;
    }
    assert_eq!(hand_out(&mut engine, now), [folder("/x/y")]);
    let events = respond(&mut engine, now, &folder("/x/y"), Err(no_such_folder()));
    assert_eq!(
        events,
        [space(
            "/x/y/z/w",
            FreeSpaceAnswer::Missing {
                parent: None,
                free: None,
            }
        )]
    );
    now += MIN_GAP;
    assert_eq!(hand_out(&mut engine, now), []);

    // The root has no parent to ask.
    command(&mut engine, now, ask("/"));
    assert_eq!(hand_out(&mut engine, now), [folder("/")]);
    let events = respond(&mut engine, now, &folder("/"), Err(no_such_folder()));
    assert_eq!(
        events,
        [space(
            "/",
            FreeSpaceAnswer::Missing {
                parent: None,
                free: None,
            }
        )]
    );
    now += MIN_GAP;
    assert_eq!(hand_out(&mut engine, now), []);
    assert_eq!(engine.connection, ConnectionState::Connected);
}

#[test]
fn only_the_newest_free_space_question_is_asked() {
    let base = Instant::now();
    let mut now = base + ms(500);
    let mut engine = quiet(base);

    // Questions that came before anything was sent: only the last one goes out.
    for path in ["/a", "/b", "/c"] {
        assert_eq!(command(&mut engine, now, ask(path)), []);
    }
    assert_eq!(hand_out(&mut engine, now), [folder("/c")]);
    assert_eq!(
        respond(&mut engine, now, &folder("/c"), Ok(free_space("/c", 1))),
        [space("/c", FreeSpaceAnswer::Free { bytes: 1 })]
    );

    // Questions that come while a call is out wait for it, and only the last one goes next;
    // the answer for the replaced question is dropped.
    now += MIN_GAP;
    command(&mut engine, now, ask("/d"));
    assert_eq!(hand_out(&mut engine, now), [folder("/d")]);
    command(&mut engine, now, ask("/e"));
    command(&mut engine, now, ask("/f"));
    // Nothing is due but the polls, an hour after loading.
    assert_eq!(engine.next_wakeup(now), base + secs(3600));
    now += MIN_GAP;
    assert_eq!(hand_out(&mut engine, now), []);
    assert_eq!(
        respond(&mut engine, now, &folder("/d"), Ok(free_space("/d", 2))),
        []
    );
    now += MIN_GAP;
    assert_eq!(engine.next_wakeup(now), now);
    assert_eq!(hand_out(&mut engine, now), [folder("/f")]);
    assert_eq!(
        respond(&mut engine, now, &folder("/f"), Ok(free_space("/f", 3))),
        [space("/f", FreeSpaceAnswer::Free { bytes: 3 })]
    );

    // The same question again, before or while it is asked, is asked once.
    now += MIN_GAP;
    command(&mut engine, now, ask("/g"));
    command(&mut engine, now, ask("/g"));
    assert_eq!(hand_out(&mut engine, now), [folder("/g")]);
    command(&mut engine, now, ask("/g"));
    now += MIN_GAP;
    assert_eq!(hand_out(&mut engine, now), []);
    assert_eq!(
        respond(&mut engine, now, &folder("/g"), Ok(free_space("/g", 4))),
        [space("/g", FreeSpaceAnswer::Free { bytes: 4 })]
    );
    now += MIN_GAP;
    assert_eq!(hand_out(&mut engine, now), []);
}

#[test]
fn free_space_is_not_asked_while_disconnected() {
    let base = Instant::now();

    // Connecting: answered at once, and nothing but the server check goes out.
    let mut engine = Engine::new(EngineConfig::default(), base);
    assert_eq!(
        command(&mut engine, base, ask("/data")),
        [space("/data", FreeSpaceAnswer::Unknown)]
    );
    assert_eq!(hand_out(&mut engine, base), [Request::CheckServer]);

    // Reconnecting: the same.
    let at = base + ms(1500);
    let mut engine = dropped(base, at);
    assert_eq!(
        command(&mut engine, at, ask("/data")),
        [space("/data", FreeSpaceAnswer::Unknown)]
    );
    assert_eq!(hand_out(&mut engine, at + secs(1)), [Request::CheckServer]);

    // A question waiting when the connection drops is answered then.
    let mut engine = loaded(base);
    command(&mut engine, base, ask("/data"));
    let events = respond(&mut engine, base, &Request::SessionStats, Err(timeout()));
    assert_eq!(events, [space("/data", FreeSpaceAnswer::Unknown)]);
    assert_eq!(hand_out(&mut engine, base), []);

    // A call handed out before the connection dropped is no longer sent, and an answer that
    // comes anyway changes nothing.
    let mut engine = loaded(base);
    command(&mut engine, base, ask("/data"));
    assert_eq!(hand_out(&mut engine, base), [folder("/data")]);
    assert!(engine.is_current(&folder("/data")));
    let events = respond(&mut engine, base, &Request::SessionStats, Err(timeout()));
    assert_eq!(events, [space("/data", FreeSpaceAnswer::Unknown)]);
    assert!(!engine.is_current(&folder("/data")));
    assert_eq!(
        respond(
            &mut engine,
            base,
            &folder("/data"),
            Ok(free_space("/data", 1))
        ),
        []
    );
}

/// Hands out the call about `path` at `now`, which must be due, and moves `now` on by the gap
/// the next call waits; returns when the call went out.
fn call(engine: &mut Engine, now: &mut Instant, path: &str) -> Instant {
    assert_eq!(hand_out(engine, *now), [folder(path)]);
    let at = *now;
    *now += MIN_GAP;
    at
}

#[test]
fn a_free_space_read_that_fails_is_unknown() {
    let base = Instant::now();
    let mut now = base + ms(500);
    let refused = RpcError::Server {
        code: 3,
        message: "path is not absolute".to_owned(),
        detail: Some("directory path is not absolute".to_owned()),
    };
    let mut engine = quiet(base);

    // Refused: unknown, and the connection stays.
    command(&mut engine, now, ask("/data"));
    let at = call(&mut engine, &mut now, "/data");
    assert_eq!(
        respond(&mut engine, at, &folder("/data"), Err(refused.clone())),
        [space("/data", FreeSpaceAnswer::Unknown)]
    );
    assert_eq!(engine.connection, ConnectionState::Connected);

    // A size the daemon could not read is no size.
    command(&mut engine, at, ask("/data"));
    let at = call(&mut engine, &mut now, "/data");
    assert_eq!(
        respond(
            &mut engine,
            at,
            &folder("/data"),
            Ok(free_space("/data", -1))
        ),
        [space("/data", FreeSpaceAnswer::Unknown)]
    );

    // Known to be missing, but its parent could not be read: missing, with no parent found.
    command(&mut engine, at, ask("/data/new"));
    let at = call(&mut engine, &mut now, "/data/new");
    respond(&mut engine, at, &folder("/data/new"), Err(no_such_folder()));
    let at = call(&mut engine, &mut now, "/data");
    assert_eq!(
        respond(&mut engine, at, &folder("/data"), Err(refused)),
        [space(
            "/data/new",
            FreeSpaceAnswer::Missing {
                parent: None,
                free: None,
            }
        )]
    );

    // A parent that exists but whose size could not be read.
    command(&mut engine, at, ask("/data/new"));
    let at = call(&mut engine, &mut now, "/data/new");
    respond(&mut engine, at, &folder("/data/new"), Err(no_such_folder()));
    let at = call(&mut engine, &mut now, "/data");
    assert_eq!(
        respond(
            &mut engine,
            at,
            &folder("/data"),
            Ok(free_space("/data", -1))
        ),
        [space(
            "/data/new",
            FreeSpaceAnswer::Missing {
                parent: Some(dir("/data")),
                free: None,
            }
        )]
    );
    assert_eq!(engine.connection, ConnectionState::Connected);

    // No answer: unknown, and the daemon cannot be used, as after any call that gets none.
    command(&mut engine, at, ask("/data"));
    let at = call(&mut engine, &mut now, "/data");
    assert_eq!(
        respond(&mut engine, at, &folder("/data"), Err(timeout())),
        [space("/data", FreeSpaceAnswer::Unknown)]
    );
    assert_eq!(engine.connection, reconnecting(1, ConnectionError::Timeout));
}

#[test]
fn a_free_space_question_goes_before_the_polls_and_leaves_the_status_bar_alone() {
    let base = Instant::now();
    let mut engine = loaded(base);
    let before = engine.take_snapshot().unwrap();
    let tick = base + ms(1500);

    command(&mut engine, tick, ask("/data/films"));
    let due = hand_out(&mut engine, tick);
    assert_eq!(
        due,
        [
            folder("/data/films"),
            Request::RecentlyActive,
            Request::SessionStats
        ]
    );
    respond(
        &mut engine,
        tick,
        &folder("/data/films"),
        Ok(free_space("/data/films", 1)),
    );

    // The status bar shows the free space of the session's download directory only.
    assert!(before.free_space_bytes.is_some_and(|bytes| bytes != 1));
    assert_eq!(engine.take_snapshot(), None);
}

/// Runs the engine as the worker does, from `now` until the question about `path` is answered:
/// it sleeps until each wakeup and answers every request at once, folders with `folder_answer`
/// and the polls like the fixture daemon. Returns when that was, and adds to `calls` the time of
/// each folder call.
fn run_until_answered(
    engine: &mut Engine,
    mut now: Instant,
    calls: &mut Vec<Instant>,
    folder_answer: impl Fn(&DownloadDir) -> Result<Response, RpcError>,
) -> Instant {
    for _ in 0..100 {
        now = engine.next_wakeup(now);
        let mut answered = false;
        for request in hand_out(engine, now) {
            let result = match &request {
                Request::FolderFreeSpace(folder) => {
                    calls.push(now);
                    folder_answer(folder)
                }
                poll => Ok(answer(poll)),
            };
            let events = respond(engine, now, &request, result);
            answered |= events
                .iter()
                .any(|event| matches!(event, Event::FreeSpace { .. }));
        }
        if answered {
            return now;
        }
    }
    panic!("no answer after 100 wakeups");
}

#[test]
fn free_space_calls_are_at_least_400_ms_apart() {
    let base = Instant::now();
    let mut engine = loaded(base);
    let mut now = base + ms(100);
    let mut calls = Vec::new();
    let answer = |folder: &DownloadDir| {
        if folder.as_str() == "/c/new" {
            Err(no_such_folder())
        } else {
            Ok(free_space(folder.as_str(), 1))
        }
    };

    // Each question comes as soon as the one before is answered; the missing folder's parent is
    // asked too.
    for path in ["/a", "/b", "/c/new", "/d"] {
        command(&mut engine, now, ask(path));
        now = run_until_answered(&mut engine, now, &mut calls, answer);
    }

    // The engine wakes for each call when the gap is over, not before and not later; the tick at
    // 1500 ms does not bring the last call forward.
    assert_eq!(
        calls,
        [100, 500, 900, 1300, 1700].map(|at| base + ms(at)),
        "calls at {:?}",
        calls
            .iter()
            .map(|at| at.duration_since(base))
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_chatty_ui_gets_at_most_one_free_space_call_every_400_ms() {
    let base = Instant::now();
    let mut engine = loaded(base);
    let mut calls = Vec::new();

    // A new folder every 50 ms for two seconds, each answered at once.
    for step in 1..=40_u64 {
        let now = base + ms(50 * step);
        command(&mut engine, now, ask(&format!("/typed/{step}")));
        while engine.next_wakeup(now) <= now {
            let due = hand_out(&mut engine, now);
            if due.is_empty() {
                break;
            }
            for request in due {
                let result = match &request {
                    Request::FolderFreeSpace(folder) => {
                        calls.push(now);
                        Ok(free_space(folder.as_str(), 1))
                    }
                    poll => Ok(answer(poll)),
                };
                respond(&mut engine, now, &request, result);
            }
        }
    }

    assert!(calls.len() <= 6, "{} calls in two seconds", calls.len());
    for pair in calls.windows(2) {
        assert!(
            pair[1].duration_since(pair[0]) >= ms(400),
            "calls {:?} apart",
            pair[1].duration_since(pair[0])
        );
    }
}

#[test]
fn the_gap_after_a_slow_free_space_answer_is_counted_from_the_answer() {
    let base = Instant::now();
    let mut engine = loaded(base);
    let now = base + ms(100);

    command(&mut engine, now, ask("/a"));
    assert_eq!(hand_out(&mut engine, now), [folder("/a")]);
    respond(
        &mut engine,
        now + ms(300),
        &folder("/a"),
        Ok(free_space("/a", 1)),
    );
    command(&mut engine, now + ms(300), ask("/b"));

    assert_eq!(engine.next_wakeup(now + ms(300)), now + ms(700));
    assert_eq!(hand_out(&mut engine, now + ms(699)), []);
    assert_eq!(hand_out(&mut engine, now + ms(700)), [folder("/b")]);
}
