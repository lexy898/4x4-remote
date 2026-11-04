//! Tests of the reads of the Peers and the Trackers tab: the selected torrent's peers every 3 s and
//! its trackers every 5 s, only while their tab is shown, the window is visible and the selection
//! has settled, and what their answers change.

use super::*;

use crate::ui_state::DetailsTab;

/// A read of the peers of `hash`.
fn peers(hash: HashString) -> Request {
    Request::Details {
        hash,
        kind: DetailsRead::Peers,
    }
}

/// A read of the trackers of `hash`.
fn trackers(hash: HashString) -> Request {
    Request::Details {
        hash,
        kind: DetailsRead::Trackers,
    }
}

/// The read of the tab `tab`.
fn tab_read(tab: DetailsTab, hash: HashString) -> Request {
    match tab {
        DetailsTab::Peers => peers(hash),
        DetailsTab::Trackers => trackers(hash),
        DetailsTab::General | DetailsTab::Files => panic!("{tab:?} has no read of its own"),
    }
}

/// Whether `request` reads the peers or the trackers of a tab.
fn is_tab_read(request: &Request) -> bool {
    matches!(
        request,
        Request::Details {
            kind: DetailsRead::Peers | DetailsRead::Trackers,
            ..
        }
    )
}

/// The reads of the tabs in `sent`, with when each went out.
fn tab_reads(sent: &[(Instant, Request)]) -> Vec<(Instant, Request)> {
    sent.iter()
        .filter(|(_, request)| is_tab_read(request))
        .cloned()
        .collect()
}

fn peer_lists(events: &[Event]) -> Vec<Arc<PeerList>> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Peers(list) => Some(Arc::clone(list)),
            _ => None,
        })
        .collect()
}

/// An engine with the first torrent in `status` and the tab `tab` shown, selected at `base` and
/// read once the selection settled: the full read, then the read of the tab. Returns it and when
/// the read of the tab went out.
fn opened(base: Instant, status: Status, tab: DetailsTab) -> (Engine, Instant) {
    let mut engine = loaded_as(base, status);
    assert_eq!(command(&mut engine, base, Command::ShowTab(tab)), []);
    command(&mut engine, base, Command::Select(Some(first())));
    let at = base + ms(250);
    let (due, _) = exchange_as(&mut engine, at, status);
    match tab {
        // The peers need no details: they go right after the full read.
        DetailsTab::Peers => assert_eq!(due, [full(first()), peers(first())]),
        // The trackers are merged into the details, so they wait for the full read's answer.
        DetailsTab::Trackers => {
            assert_eq!(due, [full(first())]);
            assert_eq!(engine.next_wakeup(at), at);
            assert_eq!(exchange_as(&mut engine, at, status).0, [trackers(first())]);
        }
        DetailsTab::General | DetailsTab::Files => {}
    }
    (engine, at)
}

#[test]
fn the_default_config_reads_peers_every_3s_and_trackers_every_5s() {
    let config = EngineConfig::default();

    assert_eq!(config.peers_interval, secs(3));
    assert_eq!(config.trackers_interval, secs(5));
}

#[test]
fn each_tab_read_asks_for_the_fields_of_its_tab() {
    assert_eq!(DetailsRead::Peers.fields(), TorrentField::PEERS);
    assert_eq!(DetailsRead::Trackers.fields(), TorrentField::TRACKERS);
    assert_eq!(peers(first()).name(), "peers");
    assert_eq!(trackers(first()).name(), "trackers");
}

#[test]
fn a_tab_is_read_at_once_when_the_selection_has_settled() {
    for tab in [DetailsTab::Peers, DetailsTab::Trackers] {
        let base = Instant::now();
        let mut engine = loaded_as(base, Status::Seeding);
        command(&mut engine, base, Command::ShowTab(tab));
        command(&mut engine, base, Command::Select(Some(first())));

        // Moving through the list reads nothing, the tab neither.
        assert_eq!(hand_out(&mut engine, base + ms(100)), [], "{tab:?}");
        assert_eq!(
            engine.next_wakeup(base + ms(100)),
            base + ms(250),
            "{tab:?}"
        );

        let (_, read_at) = opened(base, Status::Seeding, tab);
        assert_eq!(read_at, base + ms(250), "{tab:?}");
    }
}

#[test]
fn opening_a_tab_reads_it_at_once() {
    for tab in [DetailsTab::Peers, DetailsTab::Trackers] {
        let base = Instant::now();
        let (mut engine, read_at) = opened(base, Status::Seeding, DetailsTab::General);
        let sent = run(&mut engine, read_at, base + secs(10), Status::Seeding);
        assert_eq!(tab_reads(&sent), [], "{tab:?}: not while General is shown");

        let now = base + secs(10);
        assert_eq!(
            command(&mut engine, now, Command::ShowTab(tab)),
            [],
            "{tab:?}"
        );

        assert_eq!(engine.next_wakeup(now), now, "{tab:?}");
        let due = hand_out(&mut engine, now);
        assert_eq!(
            due.into_iter().filter(is_tab_read).collect::<Vec<_>>(),
            [tab_read(tab, first())],
            "{tab:?}"
        );
    }
}

#[test]
fn peers_are_read_every_3s_and_trackers_every_5s() {
    for (tab, interval) in [(DetailsTab::Peers, 3), (DetailsTab::Trackers, 5)] {
        let base = Instant::now();
        let (mut engine, read_at) = opened(base, Status::Seeding, tab);

        let sent = run(&mut engine, read_at, read_at + secs(30), Status::Seeding);

        let expected: Vec<(Instant, Request)> = (1..30 / interval)
            .map(|n| (read_at + secs(interval * n), tab_read(tab, first())))
            .collect();
        assert_eq!(tab_reads(&sent), expected, "{tab:?}");
        // Seeding: the details themselves wait for the whole list, which is not due yet.
        assert_eq!(details_reads(&sent).len(), expected.len(), "{tab:?}");
    }
}

#[test]
fn the_next_tab_read_is_counted_from_the_answer() {
    for (tab, interval) in [(DetailsTab::Peers, 3), (DetailsTab::Trackers, 5)] {
        let base = Instant::now();
        let (mut engine, read_at) = opened(base, Status::Seeding, tab);
        let at = read_at + secs(interval);
        let due: Vec<Request> = hand_out(&mut engine, at)
            .into_iter()
            .filter(is_tab_read)
            .collect();
        assert_eq!(due, [tab_read(tab, first())], "{tab:?}");
        let answered = at + secs(2);

        respond(&mut engine, answered, &due[0], Ok(answer(&due[0])));

        // One interval from the answer, not from the send.
        let next = answered + secs(interval);
        assert!(
            !hand_out(&mut engine, answered + ms(interval * 1000 - 1))
                .iter()
                .any(is_tab_read),
            "{tab:?}"
        );
        assert!(
            hand_out(&mut engine, next).contains(&tab_read(tab, first())),
            "{tab:?}"
        );
    }
}

#[test]
fn no_tab_is_read_while_hidden_with_no_selection_or_with_another_tab() {
    for tab in [DetailsTab::Peers, DetailsTab::Trackers] {
        let base = Instant::now();
        let until = base + secs(181);

        // Nothing selected, as with no single selection or a closed panel.
        let mut engine = loaded_as(base, Status::Downloading);
        command(&mut engine, base, Command::ShowTab(tab));
        let sent = run(&mut engine, base, until, Status::Downloading);
        assert_eq!(tab_reads(&sent), [], "{tab:?}: nothing selected");

        // Selected while hidden: nothing is read, and nothing wakes for it.
        let mut engine = loaded_as(base, Status::Downloading);
        command(
            &mut engine,
            base,
            Command::SetVisibility(Visibility::Hidden),
        );
        command(&mut engine, base, Command::ShowTab(tab));
        command(&mut engine, base, Command::Select(Some(first())));
        assert_eq!(engine.next_wakeup(base + ms(300)), base + secs(10));
        let sent = run(&mut engine, base, until, Status::Downloading);
        assert_eq!(tab_reads(&sent), [], "{tab:?}: hidden");

        // Read, then hidden.
        let (mut engine, read_at) = opened(base, Status::Downloading, tab);
        command(
            &mut engine,
            read_at,
            Command::SetVisibility(Visibility::Hidden),
        );
        let sent = run(&mut engine, read_at, until, Status::Downloading);
        assert_eq!(tab_reads(&sent), [], "{tab:?}: read, then hidden");

        // Read, then deselected, as when the panel closes.
        let (mut engine, read_at) = opened(base, Status::Downloading, tab);
        command(&mut engine, read_at, Command::Select(None));
        let sent = run(&mut engine, read_at, until, Status::Downloading);
        assert_eq!(tab_reads(&sent), [], "{tab:?}: deselected");

        // Read, then another tab.
        for other in [DetailsTab::General, DetailsTab::Files] {
            let (mut engine, read_at) = opened(base, Status::Downloading, tab);
            command(&mut engine, read_at, Command::ShowTab(other));
            let sent = run(&mut engine, read_at, until, Status::Downloading);
            assert_eq!(tab_reads(&sent), [], "{tab:?}, then {other:?}");
            // The details are read as before.
            assert!(!details_reads(&sent).is_empty(), "{tab:?}, then {other:?}");
        }
    }
}

#[test]
fn showing_the_window_again_reads_the_tab_that_fell_due_while_hidden() {
    let base = Instant::now();
    let (mut engine, read_at) = opened(base, Status::Seeding, DetailsTab::Peers);
    command(
        &mut engine,
        read_at,
        Command::SetVisibility(Visibility::Hidden),
    );
    run(&mut engine, read_at, base + secs(20), Status::Seeding);

    let shown = base + secs(20);
    command(
        &mut engine,
        shown,
        Command::SetVisibility(Visibility::Visible),
    );

    assert_eq!(engine.next_wakeup(shown), shown);
    assert!(hand_out(&mut engine, shown).contains(&peers(first())));
}

#[test]
fn the_peers_come_as_an_event_and_an_equal_list_is_not_sent_again() {
    let base = Instant::now();
    let mut engine = loaded_as(base, Status::Seeding);
    command(&mut engine, base, Command::ShowTab(DetailsTab::Peers));
    command(&mut engine, base, Command::Select(Some(first())));
    let at = base + ms(250);

    let (due, events) = exchange_as(&mut engine, at, Status::Seeding);

    assert_eq!(due, [full(first()), peers(first())]);
    let [list] = peer_lists(&events).try_into().unwrap();
    assert_eq!(list.hash, first());
    assert_eq!(list.peers.len(), 20);
    assert_eq!(
        list.sources.pex + list.sources.dht + list.sources.tracker,
        20
    );
    assert!(
        list.peers.iter().all(|peer| peer.country.is_none()),
        "the engine looks up no country"
    );
    // The peers are not part of the details.
    assert_eq!(details_events(&events).len(), 1);

    // The same peers again: no event.
    let at = at + secs(3);
    let (due, events) = exchange_as(&mut engine, at, Status::Seeding);
    assert!(due.contains(&peers(first())), "{due:?}");
    assert_eq!(peer_lists(&events), []);

    // A peer that sends faster: a new list.
    let at = at + secs(3);
    let due = hand_out(&mut engine, at);
    assert!(due.contains(&peers(first())), "{due:?}");
    let mut faster = peers_answer();
    faster.torrents[0].peers.as_mut().unwrap()[3].rate_to_client = 4096;
    let events = respond(
        &mut engine,
        at,
        &peers(first()),
        Ok(Response::Details(faster)),
    );
    let [list] = peer_lists(&events).try_into().unwrap();
    assert_eq!(list.peers[3].rate_down, 4096);
}

#[test]
fn leaving_the_tab_or_changing_the_selection_clears_the_peers() {
    let base = Instant::now();

    // Another tab: the peers go, once, and are not read again.
    let (mut engine, read_at) = opened(base, Status::Seeding, DetailsTab::Peers);
    let left = read_at + secs(1);
    assert_eq!(
        command(&mut engine, left, Command::ShowTab(DetailsTab::Files)),
        [Event::PeersCleared]
    );
    assert_eq!(
        command(&mut engine, left, Command::ShowTab(DetailsTab::General)),
        []
    );
    // Back on the tab, the same peers are news again: the UI dropped them.
    command(&mut engine, left, Command::ShowTab(DetailsTab::Peers));
    let (due, events) = exchange_as(&mut engine, left, Status::Seeding);
    assert_eq!(due, [peers(first())]);
    assert_eq!(peer_lists(&events).len(), 1);

    // Another torrent, and none, as when the panel closes.
    for selection in [Some(second()), None] {
        let (mut engine, read_at) = opened(base, Status::Seeding, DetailsTab::Peers);
        assert_eq!(
            command(&mut engine, read_at, Command::Select(selection.clone())),
            [Event::PeersCleared],
            "{selection:?}"
        );
        assert_eq!(
            command(&mut engine, read_at, Command::Select(Some(first()))),
            [],
            "{selection:?}: nothing to clear"
        );
    }

    // No peers sent yet: nothing to clear.
    let mut engine = loaded(base);
    command(&mut engine, base, Command::ShowTab(DetailsTab::Peers));
    command(&mut engine, base, Command::Select(Some(first())));
    assert_eq!(
        command(&mut engine, base, Command::ShowTab(DetailsTab::General)),
        []
    );
    assert_eq!(command(&mut engine, base, Command::Select(None)), []);
}

#[test]
fn a_tab_read_answered_after_its_tab_or_its_torrent_went_is_ignored() {
    let base = Instant::now();

    // The tab was left while the read was out.
    let (mut engine, read_at) = opened(base, Status::Seeding, DetailsTab::Peers);
    let at = read_at + secs(3);
    let due = hand_out(&mut engine, at);
    assert!(due.contains(&peers(first())), "{due:?}");
    command(&mut engine, at, Command::ShowTab(DetailsTab::General));
    assert!(!engine.is_current(&peers(first())));
    let mut changed = peers_answer();
    changed.torrents[0].peers.as_mut().unwrap().truncate(2);
    assert_eq!(
        respond(
            &mut engine,
            at,
            &peers(first()),
            Ok(Response::Details(changed.clone()))
        ),
        []
    );

    // Another torrent was selected while the read was out.
    let (mut engine, read_at) = opened(base, Status::Seeding, DetailsTab::Peers);
    let at = read_at + secs(3);
    hand_out(&mut engine, at);
    command(&mut engine, at, Command::Select(Some(second())));
    assert!(!engine.is_current(&peers(first())));
    assert_eq!(
        respond(
            &mut engine,
            at,
            &peers(first()),
            Ok(Response::Details(changed))
        ),
        []
    );

    // The trackers of a tab no longer shown are not merged.
    let (mut engine, read_at) = opened(base, Status::Seeding, DetailsTab::Trackers);
    let before = engine.details.clone();
    command(&mut engine, read_at, Command::ShowTab(DetailsTab::General));
    assert!(!engine.is_current(&trackers(first())));
    let mut answer = trackers_answer();
    answer.torrents[0].tracker_stats.as_mut().unwrap().pop();
    assert_eq!(
        respond(
            &mut engine,
            read_at,
            &trackers(first()),
            Ok(Response::Details(answer))
        ),
        []
    );
    assert_eq!(engine.details, before);
}

#[test]
fn a_trackers_answer_replaces_the_trackers_and_keeps_the_files() {
    let base = Instant::now();
    let mut engine = loaded_as(base, Status::Seeding);
    command(&mut engine, base, Command::ShowTab(DetailsTab::Trackers));
    command(&mut engine, base, Command::Select(Some(first())));
    let at = base + ms(250);
    let (due, events) = exchange_as(&mut engine, at, Status::Seeding);
    assert_eq!(due, [full(first())]);
    let [whole] = details_events(&events).try_into().unwrap();
    assert_eq!(whole.trackers.len(), 1);

    let (due, events) = exchange_as(&mut engine, at, Status::Seeding);

    assert_eq!(due, [trackers(first())]);
    let [merged] = details_events(&events).try_into().unwrap();
    assert_eq!(merged.trackers.len(), 2);
    assert_eq!(merged.trackers[0].host, "tracker2.example.org:80");
    assert_eq!(merged.trackers[1].tier, 1);
    // Only the trackers changed.
    assert_eq!(merged.files, whole.files);
    assert_eq!(merged.comment, whole.comment);
    assert_eq!(merged.downloaded_ever, whole.downloaded_ever);
    assert_eq!(
        TorrentDetails {
            trackers: whole.trackers.clone(),
            ..TorrentDetails::clone(&merged)
        },
        *whole
    );
    assert_eq!(engine.details.as_deref(), Some(merged.as_ref()));
    // The same trackers again change nothing.
    let at = at + secs(5);
    let (due, events) = exchange_as(&mut engine, at, Status::Seeding);
    assert!(due.contains(&trackers(first())), "{due:?}");
    assert_eq!(details_events(&events), []);
}

#[test]
fn the_trackers_wait_for_details_to_merge_into() {
    let base = Instant::now();
    let mut engine = loaded_as(base, Status::Seeding);
    command(&mut engine, base, Command::ShowTab(DetailsTab::Trackers));
    command(&mut engine, base, Command::Select(Some(first())));
    let at = base + ms(250);
    assert_eq!(hand_out(&mut engine, at), [full(first())]);
    let refusal = RpcError::Server {
        code: 1,
        message: "system error".to_owned(),
        detail: None,
    };
    respond(&mut engine, at, &full(first()), Err(refusal));

    // No details: no trackers read, and nothing wakes for one.
    let sent = run(&mut engine, at, base + secs(30), Status::Seeding);
    assert_eq!(tab_reads(&sent), []);
    // An answer that comes anyway is not merged into nothing.
    assert_eq!(
        respond(
            &mut engine,
            at,
            &trackers(first()),
            Ok(Response::Details(trackers_answer()))
        ),
        []
    );
    assert_eq!(engine.details, None);
}

#[test]
fn a_reannounce_of_the_selected_torrent_reads_its_trackers_again_at_once() {
    let base = Instant::now();
    let (mut engine, read_at) = opened(base, Status::Seeding, DetailsTab::Trackers);
    let now = read_at + secs(1);
    let reannounce = Action::Reannounce(vec![first()]);
    command(
        &mut engine,
        now,
        Command::Run(ActionId(50), reannounce.clone()),
    );

    let (due, events) = exchange(&mut engine, now);

    assert_eq!(due, [Request::Action(ActionId(50), reannounce)]);
    assert_eq!(events, [finished(50, Ok(()))]);
    let reads: Vec<Request> = hand_out(&mut engine, now)
        .into_iter()
        .filter(is_tab_read)
        .collect();
    assert_eq!(reads, [trackers(first())]);
    // Once: the next one keeps the interval, from that read.
    assert!(!hand_out(&mut engine, now + secs(4)).iter().any(is_tab_read));
    assert!(hand_out(&mut engine, now + secs(5)).contains(&trackers(first())));

    // A reannounce of another torrent, or one while the Peers tab is shown, reads no trackers.
    for (tab, hashes) in [
        (DetailsTab::Trackers, vec![second()]),
        (DetailsTab::Peers, vec![first()]),
    ] {
        let (mut engine, read_at) = opened(base, Status::Seeding, tab);
        let now = read_at + secs(1);
        command(
            &mut engine,
            now,
            Command::Run(ActionId(51), Action::Reannounce(hashes.clone())),
        );
        exchange(&mut engine, now);
        assert!(
            !hand_out(&mut engine, now).iter().any(is_tab_read),
            "{tab:?} {hashes:?}"
        );
    }

    // A read of the trackers that went out after the reannounce in its batch already shows it.
    let (mut engine, read_at) = opened(base, Status::Seeding, DetailsTab::Trackers);
    let now = read_at + secs(5);
    command(
        &mut engine,
        now,
        Command::Run(ActionId(53), Action::Reannounce(vec![first()])),
    );
    let (due, _) = exchange(&mut engine, now);
    assert_eq!(
        due.first(),
        Some(&Request::Action(
            ActionId(53),
            Action::Reannounce(vec![first()])
        ))
    );
    assert!(due.contains(&trackers(first())), "{due:?}");
    assert!(!hand_out(&mut engine, now).iter().any(is_tab_read));

    // A failed reannounce changed nothing.
    let (mut engine, read_at) = opened(base, Status::Seeding, DetailsTab::Trackers);
    let now = read_at + secs(1);
    command(
        &mut engine,
        now,
        Command::Run(ActionId(52), Action::Reannounce(vec![first()])),
    );
    let due = hand_out(&mut engine, now);
    let refusal = RpcError::Server {
        code: 1,
        message: "system error".to_owned(),
        detail: None,
    };
    respond(&mut engine, now, &due[0], Err(refusal));
    assert!(!hand_out(&mut engine, now).iter().any(is_tab_read));
}

#[test]
fn a_failed_tab_read_is_a_poll_failure_like_the_others() {
    for (tab, kind, interval) in [
        (DetailsTab::Peers, PollKind::Peers, 3000),
        (DetailsTab::Trackers, PollKind::Trackers, 5000),
    ] {
        let base = Instant::now();
        let read = tab_read(tab, first());

        // No answer: the daemon cannot be used.
        let (mut engine, read_at) = opened(base, Status::Seeding, tab);
        let at = read_at + ms(interval);
        assert!(hand_out(&mut engine, at).contains(&read), "{tab:?}");
        let events = respond(&mut engine, at, &read, Err(timeout()));
        assert_eq!(events, [], "{tab:?}");
        assert_eq!(
            engine.connection,
            reconnecting(1, ConnectionError::Timeout),
            "{tab:?}"
        );

        // Refused: noted once, tried again at its interval, and the connection stays.
        let (mut engine, read_at) = opened(base, Status::Seeding, tab);
        let at = read_at + ms(interval);
        assert!(hand_out(&mut engine, at).contains(&read), "{tab:?}");
        let refusal = RpcError::Server {
            code: 1,
            message: "system error".to_owned(),
            detail: None,
        };
        respond(&mut engine, at, &read, Err(refusal));
        assert_eq!(engine.connection, ConnectionState::Connected, "{tab:?}");
        // Its own kind: an answer to a details read does not make its next refusal news.
        assert_eq!(
            engine.refused,
            RefusedPolls(HashSet::from([kind])),
            "{tab:?}"
        );
        assert!(
            !hand_out(&mut engine, at + ms(interval - 1))
                .iter()
                .any(is_tab_read),
            "{tab:?}"
        );
        assert!(
            hand_out(&mut engine, at + ms(interval)).contains(&read),
            "{tab:?}"
        );
    }
}

#[test]
fn refresh_and_reconnect_read_the_tab_again_with_the_full_read() {
    let base = Instant::now();

    let (mut engine, read_at) = opened(base, Status::Seeding, DetailsTab::Peers);
    let now = read_at + secs(1);
    command(&mut engine, now, Command::Refresh);
    let reads: Vec<Request> = hand_out(&mut engine, now)
        .into_iter()
        .filter(is_details)
        .collect();
    assert_eq!(reads, [full(first()), peers(first())]);

    let (mut engine, read_at) = opened(base, Status::Seeding, DetailsTab::Peers);
    let lost = read_at + secs(1);
    respond(&mut engine, lost, &Request::SessionStats, Err(timeout()));
    let check = engine.next_wakeup(lost);
    assert_eq!(hand_out(&mut engine, check), [Request::CheckServer]);
    respond(
        &mut engine,
        check,
        &Request::CheckServer,
        Ok(answer(&Request::CheckServer)),
    );
    let reads: Vec<Request> = hand_out(&mut engine, check)
        .into_iter()
        .filter(is_details)
        .collect();
    assert_eq!(reads, [full(first()), peers(first())]);
}

/// Every request of ten minutes of virtual time with the window visible, the first torrent, in
/// `status`, selected from the start and the tab `tab` shown; the server check before is left out.
fn ten_minutes_on(tab: DetailsTab, status: Status) -> Vec<(Instant, Request)> {
    let base = Instant::now();
    let mut engine = connected(base);
    command(&mut engine, base, Command::ShowTab(tab));
    command(&mut engine, base, Command::Select(Some(first())));
    run(&mut engine, base, base + secs(600), status)
}

#[test]
fn the_peers_tab_adds_one_read_every_3s_and_the_trackers_tab_one_every_5s() {
    for (tab, per_minute) in [(DetailsTab::Peers, 20), (DetailsTab::Trackers, 12)] {
        let sent = ten_minutes_on(tab, Status::Downloading);
        let reads = tab_reads(&sent).len();
        // Ten minutes, less the first quarter second the selection takes to settle.
        assert!(
            (per_minute * 10 - 1..=per_minute * 10).contains(&reads),
            "{tab:?}: {reads} reads"
        );
        let general = ten_minutes_selected(Status::Downloading).len();
        assert_eq!(sent.len(), general + reads, "{tab:?}");
    }
}
