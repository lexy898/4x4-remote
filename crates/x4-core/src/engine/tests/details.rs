//! Tests of the details reads: when the selected torrent's details are read in full or only for
//! their progress, and what the answers change. Time is virtual and every answer comes at once,
//! which makes the request counts an upper bound.

use x4_rpc::{Status, TorrentField};

use super::*;

mod tabs;

/// A full read of `hash`.
fn full(hash: HashString) -> Request {
    Request::Details {
        hash,
        kind: DetailsRead::Full,
    }
}

/// A progress read of `hash`.
fn progress(hash: HashString) -> Request {
    Request::Details {
        hash,
        kind: DetailsRead::Progress,
    }
}

/// A progress read of `hash` that also reads the trackers.
fn progress_and_trackers(hash: HashString) -> Request {
    Request::Details {
        hash,
        kind: DetailsRead::ProgressAndTrackers,
    }
}

/// Whether the details read `request` asks for the trackers.
fn asks_for_trackers(request: &Request) -> bool {
    match request {
        Request::Details { kind, .. } => kind.fields().contains(&TorrentField::TrackerStats),
        _ => false,
    }
}

/// The details fixture as a full read of the second torrent.
fn second_full_details() -> TorrentGetResponse {
    let mut answer = full_details();
    for record in &mut answer.torrents {
        record.hash_string = Some(SECOND.to_owned());
    }
    answer
}

fn is_details(request: &Request) -> bool {
    matches!(request, Request::Details { .. })
}

/// The details requests of `sent`, with when each went out.
fn details_reads(sent: &[(Instant, Request)]) -> Vec<(Instant, Request)> {
    sent.iter()
        .filter(|(_, request)| is_details(request))
        .cloned()
        .collect()
}

/// `list` with the first torrent in `status`, downloading at 1 MiB/s when it downloads; the
/// fixtures have it seeding.
fn with_first(mut list: TorrentGetResponse, status: Status) -> TorrentGetResponse {
    for record in &mut list.torrents {
        if record.hash_string.as_deref() == Some(FIRST) {
            record.status = Some(status);
            let rate = if status == Status::Downloading {
                1 << 20
            } else {
                0
            };
            record.rate_download = Some(rate);
        }
    }
    list
}

/// What the fixture daemon answers while the first torrent is in `status`.
fn answer_as(status: Status, request: &Request) -> Response {
    match request {
        Request::FullList => Response::Torrents(with_first(list(), status)),
        Request::RecentlyActive => Response::Torrents(with_first(recent(), status)),
        other => answer(other),
    }
}

/// Hands out what is due at `now` and answers all of it at once, with the first torrent in
/// `status`; returns the requests and the events their answers caused.
fn exchange_as(engine: &mut Engine, now: Instant, status: Status) -> (Vec<Request>, Vec<Event>) {
    let due = engine.due_requests(now);
    let mut events = Vec::new();
    for request in &due {
        events.extend(engine.handle_response(now, request, Ok(answer_as(status, request))));
    }
    (due, events)
}

/// An engine that connected at `base` and read what it reads on connecting, with the first
/// torrent in `status`.
fn loaded_as(base: Instant, status: Status) -> Engine {
    let mut engine = connected(base);
    while !exchange_as(&mut engine, base, status).0.is_empty() {}
    engine
}

/// An engine with the first torrent in `status`, selected at `base` and read in full once the
/// selection settled; returns it and when that read went out.
fn settled(base: Instant, status: Status) -> (Engine, Instant) {
    let mut engine = loaded_as(base, status);
    command(&mut engine, base, Command::Select(Some(first())));
    let at = base + ms(250);
    let (due, events) = exchange_as(&mut engine, at, status);
    assert_eq!(due, [full(first())]);
    assert_eq!(details_events(&events).len(), 1);
    (engine, at)
}

/// Runs the engine from `from` until before `until`, waking when it asks and answering at once
/// with the first torrent in `status`; returns every request with when it went out.
fn run(
    engine: &mut Engine,
    from: Instant,
    until: Instant,
    status: Status,
) -> Vec<(Instant, Request)> {
    let mut sent = Vec::new();
    let mut now = from;
    let mut turns = 0_u32;
    while now < until {
        let (due, _) = exchange_as(engine, now, status);
        sent.extend(due.into_iter().map(|request| (now, request)));
        now = engine.next_wakeup(now);
        turns += 1;
        assert!(turns < 100_000, "the engine never rests");
    }
    sent
}

#[test]
fn select_reads_full_details_once_after_it_settles() {
    let base = Instant::now();
    let mut engine = loaded(base);

    command(&mut engine, base + ms(100), Command::Select(Some(first())));
    assert_eq!(hand_out(&mut engine, base + ms(200)), []);
    // The selection moves on before the first torrent settled, as with the arrow keys.
    command(&mut engine, base + ms(300), Command::Select(Some(second())));
    assert_eq!(hand_out(&mut engine, base + ms(350)), []);

    let settled = base + ms(550);
    assert_eq!(engine.next_wakeup(base + ms(350)), settled);
    assert_eq!(hand_out(&mut engine, settled), [full(second())]);
    let events = respond(
        &mut engine,
        settled,
        &full(second()),
        Ok(Response::Details(second_full_details())),
    );
    assert_eq!(details_events(&events).len(), 1);
    // Neither the first torrent nor the second again until the whole list comes round.
    let sent = run(
        &mut engine,
        base + ms(550),
        base + secs(59),
        Status::Seeding,
    );
    assert_eq!(details_reads(&sent), []);
}

#[test]
fn progress_is_read_every_3s_only_while_downloading_or_verifying() {
    for status in [Status::Downloading, Status::Checking] {
        let base = Instant::now();
        let (mut engine, read_at) = settled(base, status);

        let sent = run(&mut engine, read_at, base + secs(30), status);

        let expected: Vec<(Instant, Request)> = (1..=9)
            .map(|n| (read_at + secs(3 * n), progress(first())))
            .collect();
        assert_eq!(details_reads(&sent), expected, "{status:?}");
    }

    // Once the torrent seeds, its progress waits for the whole list.
    let base = Instant::now();
    let (mut engine, read_at) = settled(base, Status::Downloading);
    run(&mut engine, read_at, base + secs(30), Status::Downloading);
    let sent = run(
        &mut engine,
        base + secs(30),
        base + secs(59),
        Status::Seeding,
    );
    assert_eq!(details_reads(&sent), []);
}

#[test]
fn a_seeding_selection_is_read_with_the_full_list_only() {
    let base = Instant::now();
    let (mut engine, read_at) = settled(base, Status::Seeding);

    let sent = run(&mut engine, read_at, base + secs(181), Status::Seeding);

    let full_lists: Vec<Instant> = sent
        .iter()
        .filter(|(_, request)| *request == Request::FullList)
        .map(|(at, _)| *at)
        .collect();
    assert_eq!(
        full_lists,
        [base + secs(60), base + secs(120), base + secs(180)]
    );
    let with_the_lists: Vec<(Instant, Request)> = full_lists
        .iter()
        .map(|&at| (at, progress_and_trackers(first())))
        .collect();
    assert_eq!(details_reads(&sent), with_the_lists);
}

#[test]
fn nothing_is_read_while_hidden_or_with_no_selection() {
    let base = Instant::now();
    let until = base + secs(181);

    // Nothing selected.
    let mut engine = loaded_as(base, Status::Downloading);
    let sent = run(&mut engine, base, until, Status::Downloading);
    assert_eq!(details_reads(&sent), []);

    // Selected while hidden: the selection settles, but nothing is read and nothing wakes for it.
    let mut engine = loaded_as(base, Status::Downloading);
    command(
        &mut engine,
        base,
        Command::SetVisibility(Visibility::Hidden),
    );
    command(&mut engine, base, Command::Select(Some(first())));
    assert_eq!(engine.next_wakeup(base + ms(300)), base + secs(10));
    let sent = run(&mut engine, base, until, Status::Downloading);
    assert_eq!(details_reads(&sent), []);

    // Read, then hidden.
    let (mut engine, read_at) = settled(base, Status::Downloading);
    command(
        &mut engine,
        read_at,
        Command::SetVisibility(Visibility::Hidden),
    );
    let sent = run(&mut engine, read_at, until, Status::Downloading);
    assert_eq!(details_reads(&sent), []);

    // Read, then deselected, as when the panel closes.
    let (mut engine, read_at) = settled(base, Status::Downloading);
    command(&mut engine, read_at, Command::Select(None));
    let sent = run(&mut engine, read_at, until, Status::Downloading);
    assert_eq!(details_reads(&sent), []);
}

#[test]
fn a_reconnect_or_file_count_change_reads_full_again() {
    let base = Instant::now();

    // A reconnect reads the details in full, with everything else it reads on connecting.
    let (mut engine, _) = settled(base, Status::Seeding);
    let lost = base + ms(1500);
    respond(&mut engine, lost, &Request::SessionStats, Err(timeout()));
    let check = engine.next_wakeup(lost);
    assert_eq!(hand_out(&mut engine, check), [Request::CheckServer]);
    respond(
        &mut engine,
        check,
        &Request::CheckServer,
        Ok(answer(&Request::CheckServer)),
    );
    assert_eq!(
        hand_out(&mut engine, check),
        [
            Request::SessionGet,
            Request::FullList,
            Request::SessionStats,
            full(first())
        ]
    );

    // A magnet link read before its metadata came: no files, until a progress read counts them.
    let mut engine = loaded_as(base, Status::Downloading);
    command(&mut engine, base, Command::Select(Some(first())));
    let read_at = base + ms(250);
    assert_eq!(hand_out(&mut engine, read_at), [full(first())]);
    let magnet = TorrentGetResponse {
        torrents: vec![TorrentRecord {
            hash_string: Some(FIRST.to_owned()),
            file_count: Some(0),
            files: Some(Vec::new()),
            priorities: Some(Vec::new()),
            wanted: Some(Vec::new()),
            pieces: Some(String::new()),
            ..TorrentRecord::default()
        }],
        removed: Vec::new(),
    };
    let events = respond(
        &mut engine,
        read_at,
        &full(first()),
        Ok(Response::Details(magnet)),
    );
    let [without_files] = details_events(&events).try_into().unwrap();
    assert_eq!(without_files.files, []);

    let sent = run(&mut engine, read_at, base + secs(4), Status::Downloading);

    let counted = read_at + secs(3);
    assert_eq!(
        details_reads(&sent),
        [(counted, progress(first())), (counted, full(first()))]
    );
    assert_eq!(engine.details.as_ref().map(|d| d.files.len()), Some(10));
}

#[test]
fn a_successful_file_action_on_the_selected_torrent_reads_progress_soon() {
    let base = Instant::now();
    let (mut engine, _) = settled(base, Status::Seeding);
    let now = base + ms(1000);
    let unwant = Action::SetFilesWanted {
        hash: first(),
        files: vec![1],
        wanted: false,
    };
    command(&mut engine, now, Command::Run(ActionId(30), unwant.clone()));

    let (due, events) = exchange(&mut engine, now);

    assert_eq!(due, [Request::Action(ActionId(30), unwant)]);
    assert_eq!(events, [finished(30, Ok(()))]);
    assert_eq!(
        hand_out(&mut engine, now),
        [
            Request::RecentlyActive,
            Request::SessionStats,
            progress(first())
        ]
    );

    // An action on another torrent reads no details.
    let now = base + ms(2000);
    let other = Action::SetFilePriority {
        hash: second(),
        files: vec![0],
        priority: FilePriority::High,
    };
    command(&mut engine, now, Command::Run(ActionId(31), other));
    exchange(&mut engine, now);
    assert_eq!(
        hand_out(&mut engine, now),
        [Request::RecentlyActive, Request::SessionStats]
    );

    // A read that went out after the action in its batch already shows it.
    let (mut engine, read_at) = settled(base, Status::Downloading);
    let at = read_at + secs(3);
    let raise = Action::SetFilePriority {
        hash: first(),
        files: vec![2],
        priority: FilePriority::High,
    };
    command(&mut engine, at, Command::Run(ActionId(32), raise.clone()));
    let (due, events) = exchange_as(&mut engine, at, Status::Downloading);
    assert_eq!(due.first(), Some(&Request::Action(ActionId(32), raise)));
    assert!(due.contains(&progress(first())), "{due:?}");
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::ActionFinished { .. }))
            .count(),
        1
    );
    assert_eq!(hand_out(&mut engine, at), []);
}

#[test]
fn a_failed_action_on_the_selected_torrent_reads_no_details() {
    let base = Instant::now();
    let (mut engine, _) = settled(base, Status::Seeding);
    let now = base + ms(1000);
    let unwant = Action::SetFilesWanted {
        hash: first(),
        files: vec![1],
        wanted: false,
    };
    command(&mut engine, now, Command::Run(ActionId(30), unwant.clone()));
    let due = hand_out(&mut engine, now);
    assert_eq!(due, [Request::Action(ActionId(30), unwant)]);
    let refusal = RpcError::Server {
        code: 1,
        message: "system error".to_owned(),
        detail: Some("torrent is busy".to_owned()),
    };

    let events = respond(&mut engine, now, &due[0], Err(refusal));

    assert_eq!(
        events,
        [finished(
            30,
            Err(ActionError::Rpc(ConnectionError::Server(
                "torrent is busy".to_owned()
            )))
        )]
    );
    // The action changed nothing, so neither the tick nor the details are read early.
    assert_eq!(hand_out(&mut engine, now), []);
    assert!(engine.next_wakeup(now) > now);
}

#[test]
fn becoming_visible_again_reads_progress_once() {
    let base = Instant::now();
    let (mut engine, read_at) = settled(base, Status::Seeding);
    command(
        &mut engine,
        read_at,
        Command::SetVisibility(Visibility::Hidden),
    );
    let sent = run(&mut engine, read_at, base + secs(50), Status::Seeding);
    assert_eq!(details_reads(&sent), []);

    let shown = base + secs(50);
    command(
        &mut engine,
        shown,
        Command::SetVisibility(Visibility::Visible),
    );
    assert_eq!(engine.next_wakeup(shown), shown);
    let sent = run(&mut engine, shown, base + secs(55), Status::Seeding);
    assert_eq!(details_reads(&sent), [(shown, progress(first()))]);

    // Being told again that the window is visible is not news.
    let again = base + secs(55);
    command(
        &mut engine,
        again,
        Command::SetVisibility(Visibility::Visible),
    );
    assert!(!hand_out(&mut engine, again).iter().any(is_details));
}

#[test]
fn progress_merges_into_the_last_full_details() {
    let base = Instant::now();
    let (mut engine, read_at) = settled(base, Status::Downloading);
    let whole = engine.details.clone().unwrap();
    let at = read_at + secs(3);
    let due = hand_out(&mut engine, at);
    assert!(due.contains(&progress(first())), "{due:?}");
    let mut answer = progress_details();
    let record = &mut answer.torrents[0];
    record.bytes_completed.as_mut().unwrap()[1] = 0;
    record.wanted.as_mut().unwrap()[2] = false;
    record.priorities.as_mut().unwrap()[0] = FilePriority::Low;
    // "f" in place of the leading "/": the first piece is missing, so the first file is not
    // playable from its start any more.
    record.pieces.as_mut().unwrap().replace_range(..1, "f");
    record.downloaded_ever = Some(whole.downloaded_ever + 1);

    let events = respond(
        &mut engine,
        at,
        &progress(first()),
        Ok(Response::Details(answer)),
    );

    let [merged] = details_events(&events).try_into().unwrap();
    assert_eq!(merged.files[1].bytes_completed, 0);
    assert!(!merged.files[2].wanted);
    assert_eq!(merged.files[0].priority, FilePriority::Low);
    assert_eq!(merged.files[0].playable_from_start, 0);
    assert_eq!(merged.files[3], whole.files[3]);
    assert_eq!(merged.downloaded_ever, whole.downloaded_ever + 1);
    // What only the full read brings stays as it was.
    let names = |details: &TorrentDetails| -> Vec<String> {
        details.files.iter().map(|file| file.name.clone()).collect()
    };
    assert_eq!(names(&merged), names(&whole));
    assert_eq!(merged.comment, whole.comment);
    assert_eq!(merged.source, whole.source);
    assert_eq!(merged.trackers, whole.trackers);
    assert_eq!(engine.details.as_deref(), Some(merged.as_ref()));
}

#[test]
fn a_progress_answer_for_another_hash_is_ignored() {
    let base = Instant::now();
    let (mut engine, read_at) = settled(base, Status::Downloading);
    let before = engine.details.clone();
    let at = read_at + secs(3);
    hand_out(&mut engine, at);

    // An answer that holds only another torrent.
    let mut other = progress_details();
    other.torrents[0].hash_string = Some(SECOND.to_owned());
    other.torrents[0].bytes_completed = Some(vec![0; 10]);
    let events = respond(
        &mut engine,
        at,
        &progress(first()),
        Ok(Response::Details(other)),
    );
    assert_eq!(events, []);
    assert_eq!(engine.details, before);
    assert_eq!(engine.details_issues, [DetailsIssue::NoMatchingRecord]);

    // A late answer for the torrent selected before, once the second one's details are shown.
    command(&mut engine, at, Command::Select(Some(second())));
    let settle = at + ms(250);
    assert!(hand_out(&mut engine, settle).contains(&full(second())));
    respond(
        &mut engine,
        settle,
        &full(second()),
        Ok(Response::Details(second_full_details())),
    );
    let shown = engine.details.clone();
    assert_eq!(shown.as_ref().map(|details| &details.hash), Some(&second()));

    let events = respond(
        &mut engine,
        settle,
        &progress(first()),
        Ok(Response::Details(progress_details())),
    );

    assert_eq!(events, []);
    assert_eq!(engine.details, shown);
}

#[test]
fn the_progress_read_with_the_full_list_also_reads_trackers() {
    // A seeding torrent is read with the full list only. A downloading one is read every 3 s,
    // and the read joins the batch of the full list, so that its trackers are read too.
    for status in [Status::Seeding, Status::Downloading] {
        let base = Instant::now();
        let (mut engine, read_at) = settled(base, status);
        let whole = engine.details.clone().unwrap();
        assert_eq!(whole.trackers.len(), 1, "{status:?}");
        let listed = base + secs(60);
        run(&mut engine, read_at, listed, status);

        let due = hand_out(&mut engine, listed);

        assert!(due.contains(&Request::FullList), "{status:?}: {due:?}");
        let reads: Vec<Request> = due.into_iter().filter(is_details).collect();
        assert_eq!(reads, [progress_and_trackers(first())], "{status:?}");
        assert!(asks_for_trackers(&reads[0]), "{status:?}");
        // The trackers of the answer replace the last ones.
        let mut answer = progress_and_trackers_details();
        let trackers = answer.torrents[0].tracker_stats.as_mut().unwrap();
        trackers[0].seeder_count = Some(77);
        let mut backup = trackers[0].clone();
        backup.host = "backup.example:80".to_owned();
        trackers.push(backup);

        let events = respond(
            &mut engine,
            listed,
            &reads[0],
            Ok(Response::Details(answer)),
        );

        let [merged] = details_events(&events).try_into().unwrap();
        assert_eq!(merged.trackers.len(), 2, "{status:?}");
        assert_eq!(merged.trackers[0].seeders, Some(77), "{status:?}");
        assert_eq!(merged.trackers[1].host, "backup.example:80", "{status:?}");
        assert_ne!(merged.trackers, whole.trackers, "{status:?}");
        // The rest is merged as with any progress.
        assert_eq!(merged.files, whole.files, "{status:?}");
        assert_eq!(merged.comment, whole.comment, "{status:?}");
        assert_eq!(engine.details.as_deref(), Some(merged.as_ref()));
    }
}

#[test]
fn the_3s_progress_read_does_not_read_trackers() {
    let base = Instant::now();
    let (mut engine, read_at) = settled(base, Status::Downloading);

    let sent = run(&mut engine, read_at, base + secs(30), Status::Downloading);

    let reads = details_reads(&sent);
    assert_eq!(reads.len(), 9);
    for (at, request) in &reads {
        assert_eq!(request, &progress(first()), "at {at:?}");
        assert!(!asks_for_trackers(request), "at {at:?}");
    }
    // Nor does the read after an action, which goes out without a whole list.
    let (mut engine, _) = settled(base, Status::Seeding);
    let now = base + ms(1000);
    let raise = Action::SetFilePriority {
        hash: first(),
        files: vec![2],
        priority: FilePriority::High,
    };
    command(&mut engine, now, Command::Run(ActionId(40), raise));
    exchange(&mut engine, now);
    let reads: Vec<Request> = hand_out(&mut engine, now)
        .into_iter()
        .filter(is_details)
        .collect();
    assert_eq!(reads, [progress(first())]);
    assert!(!asks_for_trackers(&reads[0]));
}

#[test]
fn a_progress_answer_without_trackers_keeps_the_last_trackers() {
    let base = Instant::now();
    // A progress read that asked for no trackers, and one that asked but was answered without.
    for (status, at) in [
        (Status::Downloading, base + ms(3250)),
        (Status::Seeding, base + secs(60)),
    ] {
        let (mut engine, read_at) = settled(base, status);
        let whole = engine.details.clone().unwrap();
        assert_eq!(whole.trackers.len(), 1, "{status:?}");
        run(&mut engine, read_at, at, status);
        let reads: Vec<Request> = hand_out(&mut engine, at)
            .into_iter()
            .filter(is_details)
            .collect();
        let [read] = <[Request; 1]>::try_from(reads).unwrap();
        let mut answer = progress_details();
        assert_eq!(answer.torrents[0].tracker_stats, None);
        answer.torrents[0].downloaded_ever = Some(whole.downloaded_ever + 1);

        let events = respond(&mut engine, at, &read, Ok(Response::Details(answer)));

        let [merged] = details_events(&events).try_into().unwrap();
        assert_eq!(merged.downloaded_ever, whole.downloaded_ever + 1);
        assert_eq!(merged.trackers, whole.trackers, "{status:?}");
    }
}

#[test]
fn refresh_reads_full_details() {
    let base = Instant::now();
    let (mut engine, _) = settled(base, Status::Seeding);
    let now = base + ms(1000);

    command(&mut engine, now, Command::Refresh);

    assert_eq!(engine.next_wakeup(now), now);
    let due = hand_out(&mut engine, now);
    assert!(due.contains(&Request::FullList), "{due:?}");
    // One read of the one selected torrent, and in full.
    let reads: Vec<Request> = due.into_iter().filter(is_details).collect();
    assert_eq!(reads, [full(first())]);
    // It is read once, not again with the next tick.
    assert_eq!(hand_out(&mut engine, now), []);
    let sent = run(&mut engine, now, now + secs(10), Status::Seeding);
    assert_eq!(details_reads(&sent), []);
}

#[test]
fn refresh_reads_no_details_without_a_selection_while_hidden_or_before_the_selection_settles() {
    let base = Instant::now();

    // Nothing is selected.
    let mut engine = loaded(base);
    command(&mut engine, base + ms(1000), Command::Refresh);
    let due = hand_out(&mut engine, base + ms(1000));
    assert!(!due.iter().any(is_details), "{due:?}");

    // The window is hidden: the full read waits until it shows.
    let (mut engine, _) = settled(base, Status::Seeding);
    command(
        &mut engine,
        base + ms(1000),
        Command::SetVisibility(Visibility::Hidden),
    );
    command(&mut engine, base + ms(1100), Command::Refresh);
    let due = hand_out(&mut engine, base + ms(1100));
    assert!(!due.iter().any(is_details), "{due:?}");
    command(
        &mut engine,
        base + ms(1200),
        Command::SetVisibility(Visibility::Visible),
    );
    let reads: Vec<Request> = hand_out(&mut engine, base + ms(1200))
        .into_iter()
        .filter(is_details)
        .collect();
    assert_eq!(reads, [full(first())]);

    // The selection has not settled: moving through the list reads nothing, a refresh or not.
    let mut engine = loaded(base);
    command(&mut engine, base + ms(100), Command::Select(Some(first())));
    command(&mut engine, base + ms(150), Command::Refresh);
    let due = hand_out(&mut engine, base + ms(150));
    assert!(!due.iter().any(is_details), "{due:?}");
    let reads: Vec<Request> = hand_out(&mut engine, base + ms(350))
        .into_iter()
        .filter(is_details)
        .collect();
    assert_eq!(reads, [full(first())]);
}

/// Every request of ten minutes of virtual time with the window visible and the first torrent,
/// in `status`, selected from the start; the server check before is left out.
fn ten_minutes_selected(status: Status) -> Vec<(Instant, Request)> {
    let base = Instant::now();
    let mut engine = connected(base);
    command(&mut engine, base, Command::Select(Some(first())));
    run(&mut engine, base, base + secs(600), status)
}

/// Requests per second over ten minutes.
fn per_second(sent: &[(Instant, Request)]) -> f64 {
    f64::from(u32::try_from(sent.len()).unwrap()) / 600.0
}

#[test]
fn request_rate_with_a_downloading_selection_is_at_most_1_75_per_second() {
    let sent = ten_minutes_selected(Status::Downloading);

    let rate = per_second(&sent);
    assert!(
        rate <= 1.75,
        "{} requests: {rate:.3} per second",
        sent.len()
    );
    let reads = details_reads(&sent);
    let fulls = reads
        .iter()
        .filter(|(_, request)| *request == full(first()))
        .count();
    assert_eq!(fulls, 1);
    // The progress really is read all along: every 3 s.
    assert!(reads.len() >= 199, "{} details reads", reads.len());
    // The trackers are read with the nine whole lists that follow the first one, and only then.
    let with_trackers = reads
        .iter()
        .filter(|(_, request)| *request == progress_and_trackers(first()))
        .count();
    assert_eq!(with_trackers, 9);
}

#[test]
fn request_rate_with_a_seeding_selection_is_at_most_1_45_per_second() {
    let sent = ten_minutes_selected(Status::Seeding);

    let rate = per_second(&sent);
    assert!(
        rate <= 1.45,
        "{} requests: {rate:.3} per second",
        sent.len()
    );
    // The full read, then the progress, and the trackers with it, with each of the nine whole
    // lists that follow.
    let reads = details_reads(&sent);
    let with_trackers = reads
        .iter()
        .filter(|(_, request)| *request == progress_and_trackers(first()))
        .count();
    assert_eq!((reads.len(), with_trackers), (10, 9));
}

#[test]
fn a_selection_made_before_connecting_is_read_on_connecting() {
    let base = Instant::now();
    let mut engine = Engine::new(EngineConfig::default(), base);
    command(&mut engine, base, Command::Select(Some(first())));
    assert_eq!(engine.due_requests(base), [Request::CheckServer]);

    respond(
        &mut engine,
        base,
        &Request::CheckServer,
        Ok(answer(&Request::CheckServer)),
    );

    assert_eq!(
        hand_out(&mut engine, base),
        [
            Request::SessionGet,
            Request::FullList,
            Request::SessionStats,
            full(first())
        ]
    );
}

#[test]
fn details_compute_playable() {
    let base = Instant::now();
    let mut engine = loaded(base);
    command(&mut engine, base, Command::Select(Some(first())));

    let (due, events) = exchange(&mut engine, base + ms(250));

    assert_eq!(due, [full(first())]);
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
    let (mut engine, read_at) = settled(base, Status::Downloading);

    let (due, events) = exchange_as(&mut engine, read_at + secs(3), Status::Downloading);

    assert!(due.contains(&progress(first())), "{due:?}");
    assert_eq!(details_events(&events), []);
}

#[test]
fn selecting_another_torrent_drops_the_details() {
    let base = Instant::now();
    let (mut engine, read_at) = settled(base, Status::Seeding);

    command(&mut engine, read_at, Command::Select(Some(second())));
    assert_eq!(engine.details, None);
    command(&mut engine, read_at, Command::Select(Some(first())));

    // The same details again are news: the UI dropped them with the selection.
    let (due, events) = exchange(&mut engine, read_at + ms(250));
    assert_eq!(due, [full(first())]);
    assert_eq!(details_events(&events).len(), 1);
}

#[test]
fn details_for_a_deselected_torrent_are_ignored() {
    let base = Instant::now();
    let mut engine = loaded(base);
    command(&mut engine, base, Command::Select(Some(first())));
    let due = engine.due_requests(base + ms(250));
    assert_eq!(due, [full(first())]);

    command(&mut engine, base, Command::Select(Some(second())));

    assert!(!engine.is_current(&due[0]));
    assert_eq!(respond(&mut engine, base, &due[0], Ok(answer(&due[0]))), []);
    assert_eq!(engine.details, None);
}

#[test]
fn an_empty_details_answer_sends_no_event() {
    let base = Instant::now();
    let mut engine = loaded(base);
    command(&mut engine, base, Command::Select(Some(first())));
    let due = engine.due_requests(base + ms(250));

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
    command(&mut engine, base, Command::Select(Some(first())));
    let other = TorrentGetResponse {
        torrents: vec![TorrentRecord {
            hash_string: Some(SECOND.to_owned()),
            ..TorrentRecord::default()
        }],
        removed: Vec::new(),
    };

    let events = engine.handle_response(base, &full(first()), Ok(Response::Details(other)));

    assert_eq!(events, []);
    assert_eq!(engine.details_issues, [DetailsIssue::NoMatchingRecord]);
    // A sound answer clears them.
    respond(
        &mut engine,
        base,
        &full(first()),
        Ok(Response::Details(full_details())),
    );
    assert_eq!(engine.details_issues, []);
}

#[test]
fn selecting_the_same_torrent_again_keeps_its_details() {
    let base = Instant::now();
    let (mut engine, read_at) = settled(base, Status::Seeding);

    let now = read_at + ms(200);
    assert_eq!(
        engine.handle_command(now, Command::Select(Some(first()))),
        []
    );

    assert!(engine.details.is_some());
    assert_eq!(hand_out(&mut engine, now), []);
    assert_eq!(hand_out(&mut engine, now + ms(250)), []);
}

#[test]
fn a_late_details_answer_moves_the_next_progress_read_but_not_the_tick() {
    let base = Instant::now();
    let (mut engine, read_at) = settled(base, Status::Downloading);
    let at = read_at + secs(3);
    let due = hand_out(&mut engine, at);
    assert_eq!(
        due,
        [
            Request::RecentlyActive,
            Request::SessionStats,
            progress(first())
        ]
    );
    for request in &due[..2] {
        respond(
            &mut engine,
            at,
            request,
            Ok(answer_as(Status::Downloading, request)),
        );
    }
    let answered = at + secs(2);
    respond(&mut engine, answered, &due[2], Ok(answer(&due[2])));

    assert_eq!(
        hand_out(&mut engine, at + ms(1500)),
        [Request::RecentlyActive, Request::SessionStats]
    );
    assert!(
        !hand_out(&mut engine, answered + ms(2999))
            .iter()
            .any(is_details)
    );
    assert!(hand_out(&mut engine, answered + secs(3)).contains(&progress(first())));
}

#[test]
fn a_new_selection_makes_a_details_refusal_news_again() {
    let base = Instant::now();
    let mut engine = loaded(base);
    command(&mut engine, base, Command::Select(Some(first())));
    let refusal = RpcError::Server {
        code: 1,
        message: "system error".to_owned(),
        detail: None,
    };

    respond(&mut engine, base, &full(first()), Err(refusal));
    assert_eq!(
        engine.refused,
        RefusedPolls(HashSet::from([PollKind::Details]))
    );

    command(&mut engine, base, Command::Select(Some(second())));
    assert_eq!(engine.refused, RefusedPolls::default());
}

#[test]
fn a_refused_full_read_is_tried_again_in_full() {
    let base = Instant::now();
    let mut engine = loaded_as(base, Status::Downloading);
    command(&mut engine, base, Command::Select(Some(first())));
    let read_at = base + ms(250);
    assert_eq!(hand_out(&mut engine, read_at), [full(first())]);
    let refusal = RpcError::Server {
        code: 1,
        message: "system error".to_owned(),
        detail: None,
    };
    respond(&mut engine, read_at, &full(first()), Err(refusal));

    // With nothing to merge into, the next read is a full one, at the progress cadence.
    let sent = run(&mut engine, read_at, base + secs(4), Status::Downloading);

    assert_eq!(details_reads(&sent), [(read_at + secs(3), full(first()))]);
    assert!(engine.details.is_some());
}
