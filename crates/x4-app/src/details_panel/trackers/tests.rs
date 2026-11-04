//! The tests of the Trackers tab: what it holds as the tab, the panel and the target change, what
//! its rows say, how they are keyed, and that the times on them keep moving.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{AppContext as _, Pixels, Role, TestAppContext, WindowHandle, size};
use x4_core::test_support::{details, torrent, tracker as neutral_tracker};
use x4_core::{
    DetailsTab, DetailsTarget, NextAnnounce, Torrent, TorrentDetails, TrackerAnswer,
    TrackerOutcome, TrackerStat,
};
use x4_rpc::{HashString, TrackerId, TrackerState};

use super::*;
use crate::app;
use crate::sidebar::SIDEBAR_WIDTH;

/// The time every test calls now, so that no text depends on the clock.
fn fixed_now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_793_784_667)
}

/// A moment `minutes` before the fixed now.
fn ago(minutes: u64) -> SystemTime {
    fixed_now() - Duration::from_secs(minutes * 60)
}

/// A moment `minutes` after the fixed now.
fn within(minutes: u64) -> SystemTime {
    fixed_now() + Duration::from_secs(minutes * 60)
}

/// What a tracker answered, `minutes` ago.
fn answer(outcome: TrackerOutcome, result: &str, minutes: u64) -> TrackerAnswer {
    TrackerAnswer {
        outcome,
        result: result.to_owned(),
        at: Some(ago(minutes)),
    }
}

/// A tracker with the id `id` that answered its last announce 24 minutes ago with 30 peers, and
/// has its next announce in 27 minutes.
fn working(id: u32, host: &str) -> TrackerStat {
    TrackerStat {
        id: TrackerId(id),
        announce_state: TrackerState::Waiting,
        next: NextAnnounce::At(within(27)),
        last_announce: Some(answer(TrackerOutcome::Succeeded, "Success", 24)),
        last_announce_peers: Some(30),
        seeders: Some(5),
        leechers: Some(12),
        downloads: Some(1_234),
        ..neutral_tracker(host)
    }
}

/// The details of `torrent`, with these trackers.
fn details_of(torrent: &Torrent, trackers: Vec<TrackerStat>) -> Arc<TorrentDetails> {
    Arc::new(TorrentDetails {
        trackers,
        ..details(&torrent.hash, &[])
    })
}

/// Three trackers of `torrent`, in tiers 0, 0 and 1.
fn three(torrent: &Torrent) -> Arc<TorrentDetails> {
    details_of(
        torrent,
        vec![
            working(7, "a.example.org:6969"),
            TrackerStat {
                is_backup: true,
                last_announce: None,
                last_announce_peers: None,
                next: NextAnnounce::Idle,
                seeders: None,
                leechers: None,
                downloads: None,
                ..working(3, "b.example.org:6969")
            },
            TrackerStat {
                tier: 1,
                ..working(5, "c.example.org:443")
            },
        ],
    )
}

/// A panel on the tab `tab`, about no torrent yet, whose clock stands at the fixed now.
fn panel_on(tab: DetailsTab, cx: &mut TestAppContext) -> WindowHandle<DetailsPanel> {
    cx.update(app::init);
    let panel =
        cx.add_window(|window, cx| DetailsPanel::new(tab, Rc::new(RefCell::default()), window, cx));
    change(panel, cx, |panel, _| panel.trackers.clock = fixed_now);
    panel
}

/// A panel on the Trackers tab about `torrent`.
fn trackers_of(torrent: &Torrent, cx: &mut TestAppContext) -> WindowHandle<DetailsPanel> {
    let panel = panel_on(DetailsTab::Trackers, cx);
    show(panel, torrent, cx);
    panel
}

/// Makes the panel about `torrent`.
fn show(panel: WindowHandle<DetailsPanel>, torrent: &Torrent, cx: &mut TestAppContext) {
    change(panel, cx, |panel, cx| {
        panel.show(DetailsTarget::One(torrent.hash.clone()), Some(torrent), cx);
    });
}

/// Runs `change` on the panel.
fn change(
    panel: WindowHandle<DetailsPanel>,
    cx: &mut TestAppContext,
    change: impl FnOnce(&mut DetailsPanel, &mut Context<'_, DetailsPanel>),
) {
    panel
        .update(cx, |panel, _, cx| change(panel, cx))
        .expect("the panel is open");
}

/// Hands the panel the details a read brought.
fn take(panel: WindowHandle<DetailsPanel>, details: Arc<TorrentDetails>, cx: &mut TestAppContext) {
    change(panel, cx, |panel, cx| panel.take_details(details, cx));
}

/// Draws the panel and runs `look` on what was drawn.
fn drawn<R>(
    panel: WindowHandle<DetailsPanel>,
    cx: &mut TestAppContext,
    look: impl FnOnce(&mut gpui_kit::Window) -> R,
) -> R {
    cx.update_window(panel.into(), |_, window, cx| {
        window.render_frame(cx);
        look(window)
    })
    .expect("the panel is open")
}

/// What the tab says instead of the trackers, if it says anything.
fn empty_text(panel: WindowHandle<DetailsPanel>, cx: &mut TestAppContext) -> Option<String> {
    drawn(panel, cx, |window| {
        window
            .try_find(TRACKERS_EMPTY_ID)
            .and_then(|note| note.label().map(str::to_owned))
    })
}

/// The names of the rows drawn, top to bottom: each tracker's address and tier.
fn drawn_rows(panel: WindowHandle<DetailsPanel>, cx: &mut TestAppContext) -> Vec<String> {
    drawn(panel, cx, |window| {
        window
            .find_all_by_role(Role::Row)
            .iter()
            .filter_map(|row| row.label().map(str::to_owned))
            .collect()
    })
}

/// What the tab holds: whether it has details, how many rows and the room it keeps for them, and
/// whether the clock that redraws the times runs.
fn held(panel: WindowHandle<DetailsPanel>, cx: &mut TestAppContext) -> (bool, usize, usize, bool) {
    panel
        .update(cx, |panel, _, _| {
            let trackers = &panel.trackers;
            (
                trackers.hash.is_some(),
                trackers.rows.len(),
                trackers.rows.capacity(),
                trackers.refresh.is_some(),
            )
        })
        .expect("the panel is open")
}

/// No details, no rows, no room for them and no clock.
const NOTHING: (bool, usize, usize, bool) = (false, 0, 0, false);

#[gpui_kit::test]
fn the_tab_says_a_dash_until_the_details_come(cx: &mut TestAppContext) {
    let film = torrent("film");
    let panel = trackers_of(&film, cx);
    assert_eq!(empty_text(panel, cx).as_deref(), Some("—"));
    assert_eq!(drawn_rows(panel, cx), [] as [String; 0]);

    take(panel, three(&film), cx);

    assert_eq!(empty_text(panel, cx), None);
    // In the daemon's order, tier by tier, which is not the order of the ids; the port 443 is
    // implied by the scheme, so it is not shown.
    assert_eq!(
        drawn_rows(panel, cx),
        [
            "a.example.org:6969, tier 1",
            "b.example.org:6969, tier 1",
            "c.example.org, tier 2"
        ]
    );
}

#[gpui_kit::test]
fn a_torrent_without_trackers_says_so(cx: &mut TestAppContext) {
    let film = torrent("film");
    let panel = trackers_of(&film, cx);

    take(panel, details_of(&film, Vec::new()), cx);
    assert_eq!(
        empty_text(panel, cx).as_deref(),
        Some("This torrent has no trackers (DHT and PEX only)")
    );
    assert_eq!(drawn_rows(panel, cx), [] as [String; 0]);

    // A private torrent may not use DHT or peer exchange either.
    let private = Arc::new(TorrentDetails {
        is_private: true,
        ..(*details_of(&film, Vec::new())).clone()
    });
    take(panel, private, cx);
    assert_eq!(
        empty_text(panel, cx).as_deref(),
        Some("This torrent has no trackers, and a private torrent cannot use DHT or PEX")
    );

    // Trackers added since are shown, and the words go.
    take(panel, three(&film), cx);
    assert_eq!(empty_text(panel, cx), None);
    assert_eq!(drawn_rows(panel, cx).len(), 3);
}

#[gpui_kit::test]
fn the_trackers_of_another_torrent_are_never_shown(cx: &mut TestAppContext) {
    let (film, show_) = (torrent("film"), torrent("show"));
    let panel = trackers_of(&film, cx);

    // Details of a torrent the panel does not show are not taken.
    take(panel, three(&show_), cx);
    assert_eq!(empty_text(panel, cx).as_deref(), Some("—"));
    assert_eq!(held(panel, cx), NOTHING);

    // The panel's own show, and go the moment the panel is about another torrent: not one frame
    // shows the old rows under the new torrent.
    take(panel, three(&film), cx);
    assert_eq!(drawn_rows(panel, cx).len(), 3);
    show(panel, &show_, cx);
    assert_eq!(empty_text(panel, cx).as_deref(), Some("—"));
    assert_eq!(drawn_rows(panel, cx), [] as [String; 0]);
    assert_eq!(held(panel, cx), NOTHING);

    // A read of the old torrent that comes late is dropped too.
    take(panel, three(&film), cx);
    assert_eq!(empty_text(panel, cx).as_deref(), Some("—"));
    take(panel, three(&show_), cx);
    assert_eq!(drawn_rows(panel, cx).len(), 3);

    // Several torrents: none of their trackers.
    change(panel, cx, |panel, cx| {
        panel.show(DetailsTarget::Many(2), None, cx);
    });
    take(panel, three(&show_), cx);
    assert_eq!(held(panel, cx), NOTHING);
}

#[gpui_kit::test]
fn another_tab_shows_no_trackers_and_opening_the_tab_shows_those_the_panel_has(
    cx: &mut TestAppContext,
) {
    let film = torrent("film");
    let panel = panel_on(DetailsTab::General, cx);
    show(panel, &film, cx);
    take(panel, three(&film), cx);
    assert_eq!(held(panel, cx), NOTHING);

    // The details are the panel's already: no wait for a read.
    change(panel, cx, |panel, cx| {
        panel.set_tab(DetailsTab::Trackers, cx);
    });
    assert_eq!(empty_text(panel, cx), None);
    assert_eq!(drawn_rows(panel, cx).len(), 3);
    let (listed, rows, _, _) = held(panel, cx);
    assert_eq!((listed, rows), (true, 3));
}

#[gpui_kit::test]
fn leaving_the_tab_or_hiding_the_panel_drops_the_rows(cx: &mut TestAppContext) {
    let film = torrent("film");
    let panel = trackers_of(&film, cx);
    take(panel, three(&film), cx);
    assert_eq!(held(panel, cx), (true, 3, 3, true));

    for other in [DetailsTab::General, DetailsTab::Files, DetailsTab::Peers] {
        change(panel, cx, |panel, cx| panel.set_tab(other, cx));
        assert_eq!(held(panel, cx), NOTHING, "{other:?}");
        change(panel, cx, |panel, cx| {
            panel.set_tab(DetailsTab::Trackers, cx);
        });
        assert_eq!(held(panel, cx), (true, 3, 3, true), "back from {other:?}");
    }

    change(panel, cx, |panel, _| panel.hide());
    assert_eq!(held(panel, cx), NOTHING);
    // A read that was on its way when the panel hid is not kept either.
    take(panel, three(&film), cx);
    assert_eq!(held(panel, cx), NOTHING);

    show(panel, &film, cx);
    assert_eq!(empty_text(panel, cx).as_deref(), Some("—"));
    take(panel, three(&film), cx);
    assert_eq!(drawn_rows(panel, cx).len(), 3);
}

#[gpui_kit::test]
fn trackers_that_change_between_reads_change_the_rows(cx: &mut TestAppContext) {
    let film = torrent("film");
    let panel = trackers_of(&film, cx);
    take(panel, three(&film), cx);
    assert_eq!(drawn_rows(panel, cx).len(), 3);

    take(
        panel,
        details_of(&film, vec![working(7, "a.example.org:6969")]),
        cx,
    );

    assert_eq!(drawn_rows(panel, cx), ["a.example.org:6969, tier 1"]);
    assert_eq!(held(panel, cx), (true, 1, 1, true));
}

/// The label of the cell `cell` of the row of the tracker `id`, the first of its id.
fn cell_label(
    panel: WindowHandle<DetailsPanel>,
    id: u32,
    cell: &'static str,
    cx: &mut TestAppContext,
) -> Option<String> {
    let row = row_id(TrackerId(id), 0);
    drawn(panel, cx, |window| {
        window
            .within(row)
            .try_find(cell)
            .and_then(|cell| cell.label().map(str::to_owned))
    })
}

#[gpui_kit::test]
fn a_working_tracker_draws_what_x4_core_decided(cx: &mut TestAppContext) {
    let film = torrent("film");
    let panel = trackers_of(&film, cx);
    take(
        panel,
        details_of(
            &film,
            vec![TrackerStat {
                sitename: "example".to_owned(),
                ..working(7, "203.0.113.7:6969")
            }],
        ),
        cx,
    );
    let label = |cell: &'static str, cx: &mut TestAppContext| cell_label(panel, 7, cell, cx);

    assert_eq!(label("tracker-dot", cx).as_deref(), Some("Working"));
    assert_eq!(label("tracker-tier", cx).as_deref(), Some("1"));
    assert_eq!(
        label("tracker-host", cx).as_deref(),
        Some("203.0.113.7:6969")
    );
    assert_eq!(label("tracker-sitename", cx).as_deref(), Some("example"));
    // The line under the address: the status, how many peers the last announce brought and how
    // long ago it was, each on its own.
    assert_eq!(label("tracker-status", cx).as_deref(), Some("Working"));
    assert_eq!(label("tracker-peers", cx).as_deref(), Some("30 peers"));
    assert_eq!(label("tracker-ago", cx).as_deref(), Some("24 m ago"));
    assert_eq!(label("tracker-scrape", cx), None);
    assert_eq!(label("tracker-last", cx).as_deref(), Some("24 m ago"));
    assert_eq!(label("tracker-next", cx).as_deref(), Some("in 27 m"));
    assert_eq!(label("tracker-seeders", cx).as_deref(), Some("5"));
    assert_eq!(label("tracker-leechers", cx).as_deref(), Some("12"));
    assert_eq!(label("tracker-downloads", cx).as_deref(), Some("1,234"));
}

#[gpui_kit::test]
fn a_tracker_that_has_said_nothing_shows_dashes_and_no_line_of_numbers(cx: &mut TestAppContext) {
    let film = torrent("film");
    let panel = trackers_of(&film, cx);
    take(
        panel,
        details_of(&film, vec![neutral_tracker("silent.example.org:80")]),
        cx,
    );
    let label = |cell: &'static str, cx: &mut TestAppContext| cell_label(panel, 0, cell, cx);

    assert_eq!(
        label("tracker-host", cx).as_deref(),
        Some("silent.example.org")
    );
    assert_eq!(label("tracker-sitename", cx), None);
    assert_eq!(
        label("tracker-dot", cx).as_deref(),
        Some("Not contacted yet")
    );
    assert_eq!(
        label("tracker-status", cx).as_deref(),
        Some("Not contacted yet")
    );
    assert_eq!(label("tracker-peers", cx), None);
    assert_eq!(label("tracker-ago", cx), None);
    for cell in [
        "tracker-last",
        "tracker-next",
        "tracker-seeders",
        "tracker-leechers",
        "tracker-downloads",
    ] {
        assert_eq!(label(cell, cx).as_deref(), Some("—"), "{cell}");
    }
}

#[gpui_kit::test]
fn each_status_says_itself_and_the_next_announce_says_why_it_is_not_a_time(
    cx: &mut TestAppContext,
) {
    let film = torrent("film");
    let panel = trackers_of(&film, cx);
    let queued = TrackerStat {
        announce_state: TrackerState::Queued,
        next: NextAnnounce::Queued,
        ..working(1, "queued.example.org:80")
    };
    let backup = TrackerStat {
        is_backup: true,
        last_announce: None,
        last_announce_peers: None,
        next: NextAnnounce::Idle,
        ..working(2, "backup.example.org:80")
    };
    let announcing = TrackerStat {
        announce_state: TrackerState::Active,
        next: NextAnnounce::Idle,
        ..working(3, "now.example.org:80")
    };
    let timed_out = TrackerStat {
        last_announce: Some(answer(TrackerOutcome::TimedOut, "", 3)),
        ..working(4, "slow.example.org:80")
    };
    take(
        panel,
        details_of(&film, vec![queued, backup, announcing, timed_out]),
        cx,
    );
    let label =
        |id: u32, cell: &'static str, cx: &mut TestAppContext| cell_label(panel, id, cell, cx);

    assert_eq!(label(1, "tracker-status", cx).as_deref(), Some("Working"));
    assert_eq!(label(1, "tracker-next", cx).as_deref(), Some("queued"));
    assert_eq!(label(2, "tracker-status", cx).as_deref(), Some("Backup"));
    assert_eq!(label(2, "tracker-next", cx).as_deref(), Some("—"));
    assert_eq!(
        label(3, "tracker-status", cx).as_deref(),
        Some("Announcing")
    );
    assert_eq!(label(3, "tracker-next", cx).as_deref(), Some("—"));
    // The daemon sent no text for the timeout, so the row words it itself, and no peers came.
    assert_eq!(label(4, "tracker-status", cx).as_deref(), Some("Timed out"));
    assert_eq!(label(4, "tracker-peers", cx), None);
    assert_eq!(label(4, "tracker-ago", cx).as_deref(), Some("3 m ago"));
}

#[gpui_kit::test]
fn a_failed_scrape_is_a_note_beside_a_working_tracker(cx: &mut TestAppContext) {
    let film = torrent("film");
    let panel = trackers_of(&film, cx);
    // The tracker reported no counts either, so the note says why they are missing.
    take(
        panel,
        details_of(
            &film,
            vec![TrackerStat {
                last_scrape: Some(answer(TrackerOutcome::Failed, "refused", 20)),
                seeders: None,
                leechers: None,
                downloads: None,
                ..working(7, "a.example.org:6969")
            }],
        ),
        cx,
    );

    // The announce works, so the tracker still says it does; the scrape's trouble comes after.
    assert_eq!(
        cell_label(panel, 7, "tracker-dot", cx).as_deref(),
        Some("Working")
    );
    assert_eq!(
        cell_label(panel, 7, "tracker-status", cx).as_deref(),
        Some("Working")
    );
    assert_eq!(
        cell_label(panel, 7, "tracker-scrape", cx).as_deref(),
        Some("No peer counts")
    );
    assert_eq!(
        cell_label(panel, 7, "tracker-seeders", cx).as_deref(),
        Some("—")
    );
}

#[gpui_kit::test]
fn a_failed_scrape_says_nothing_beside_the_counts_the_tracker_gave(cx: &mut TestAppContext) {
    let film = torrent("film");
    let panel = trackers_of(&film, cx);
    // The daemon took the counts from the announces: "No peer counts" would be false.
    take(
        panel,
        details_of(
            &film,
            vec![
                TrackerStat {
                    last_scrape: Some(answer(TrackerOutcome::Failed, "refused", 20)),
                    ..working(7, "a.example.org:6969")
                },
                TrackerStat {
                    last_scrape: Some(answer(TrackerOutcome::TimedOut, "", 20)),
                    seeders: None,
                    ..working(8, "b.example.org:6969")
                },
            ],
        ),
        cx,
    );

    assert_eq!(
        cell_label(panel, 7, "tracker-seeders", cx).as_deref(),
        Some("5")
    );
    assert_eq!(cell_label(panel, 7, "tracker-scrape", cx), None);
    assert_eq!(
        cell_label(panel, 8, "tracker-leechers", cx).as_deref(),
        Some("12")
    );
    assert_eq!(cell_label(panel, 8, "tracker-scrape", cx), None);
}

#[gpui_kit::test]
fn a_failure_shows_the_servers_text_cleaned_and_on_one_line(cx: &mut TestAppContext) {
    let film = torrent("film");
    let panel = trackers_of(&film, cx);
    let failed = |id: u32, text: &str| TrackerStat {
        last_announce: Some(answer(TrackerOutcome::Failed, text, 3)),
        last_announce_peers: Some(0),
        ..working(id, "a.example.org:6969")
    };
    // Control and direction characters, line breaks and runs of spaces; and a text of 10 KB.
    let hostile = "\u{202E}Torrent\r\n  not\tregistered\u{7}\u{1b}[0m  ";
    let endless = "x".repeat(10_000);
    take(
        panel,
        details_of(
            &film,
            vec![
                failed(1, "Unregistered torrent"),
                failed(2, hostile),
                failed(3, &endless),
                failed(4, ""),
            ],
        ),
        cx,
    );
    let label = |id: u32, cx: &mut TestAppContext| cell_label(panel, id, "tracker-status", cx);

    assert_eq!(label(1, cx).as_deref(), Some("Unregistered torrent"));
    assert_eq!(label(2, cx).as_deref(), Some("Torrent not registered[0m"));
    let long = label(3, cx).expect("a label");
    assert!(
        (1..=256).contains(&long.chars().count()),
        "{} characters",
        long.chars().count()
    );
    // The daemon said nothing, so the row says it failed; no peers came of it.
    assert_eq!(label(4, cx).as_deref(), Some("Announce failed"));
    assert_eq!(cell_label(panel, 1, "tracker-peers", cx), None);
    assert_eq!(
        cell_label(panel, 1, "tracker-ago", cx).as_deref(),
        Some("3 m ago")
    );

    // However long the text, the line is one line: it is cut at the edge of its cell, and the
    // row keeps its height.
    let (short_height, long_height, cell_right, line_right) = drawn(panel, cx, |window| {
        let bounds = |id: u32, cell: &'static str, window: &mut gpui_kit::Window| {
            window.within(row_id(TrackerId(id), 0)).find(cell).bounds()
        };
        let short = bounds(1, "tracker-status", window);
        let long = bounds(3, "tracker-status", window);
        let cell = bounds(3, "tracker-name", window);
        (
            short.size.height,
            long.size.height,
            cell.right(),
            long.right(),
        )
    });
    assert_eq!(short_height, long_height);
    assert!(
        line_right <= cell_right,
        "{line_right:?} past {cell_right:?}"
    );
}

#[test]
fn a_row_is_keyed_by_its_tracker_id_and_repeats_get_their_own_ids() {
    let rows = |ids: &[u32]| -> Vec<ElementId> {
        let hash = HashString::parse(&"ab".repeat(20)).unwrap();
        let trackers = ids
            .iter()
            .map(|&id| TrackerStat {
                id: TrackerId(id),
                ..neutral_tracker("a.example.org:80")
            })
            .collect();
        let rows = tracker_rows(&TorrentDetails {
            trackers,
            ..details(&hash, &[])
        });
        row_ids(&rows)
    };

    // The same rows in another order keep their ids.
    let first = rows(&[7, 3]);
    let reordered = rows(&[3, 7]);
    assert_eq!(first.first(), reordered.get(1));
    assert_eq!(first.get(1), reordered.first());
    assert_ne!(first.first(), first.get(1));

    // An id the daemon sent in a form we cannot read is read as 0, so two rows can share one:
    // never the same element id.
    let repeated = rows(&[0, 0, 5, 0]);
    assert_eq!(repeated.first(), Some(&row_id(TrackerId(0), 0)));
    assert_eq!(repeated.get(1), Some(&row_id(TrackerId(0), 1)));
    assert_eq!(repeated.get(2), Some(&row_id(TrackerId(5), 0)));
    assert_eq!(repeated.get(3), Some(&row_id(TrackerId(0), 2)));
    let mut unique = repeated.clone();
    unique.dedup();
    assert_eq!(unique.len(), repeated.len());
    // The first row of an id is not the second row of another: the number cannot be mistaken for
    // part of the id.
    assert_ne!(row_id(TrackerId(1), 10), row_id(TrackerId(11), 0));
    assert_ne!(row_id(TrackerId(1), 1), row_id(TrackerId(11), 0));
}

#[gpui_kit::test]
fn rows_keep_their_ids_when_the_daemon_reorders_the_trackers(cx: &mut TestAppContext) {
    let film = torrent("film");
    let panel = trackers_of(&film, cx);
    take(panel, three(&film), cx);
    let host_of = |id: u32, cx: &mut TestAppContext| cell_label(panel, id, "tracker-host", cx);
    assert_eq!(host_of(7, cx).as_deref(), Some("a.example.org:6969"));
    assert_eq!(host_of(5, cx).as_deref(), Some("c.example.org"));

    // After an error the daemon rotates a tier; the same tracker is found under the same id.
    take(
        panel,
        details_of(
            &film,
            vec![
                working(5, "c.example.org:443"),
                working(3, "b.example.org:6969"),
                working(7, "a.example.org:6969"),
            ],
        ),
        cx,
    );

    assert_eq!(host_of(7, cx).as_deref(), Some("a.example.org:6969"));
    assert_eq!(host_of(5, cx).as_deref(), Some("c.example.org"));
    assert_eq!(
        drawn_rows(panel, cx),
        [
            "c.example.org, tier 1",
            "b.example.org:6969, tier 1",
            "a.example.org:6969, tier 1"
        ]
    );
}

#[gpui_kit::test]
fn trackers_that_share_an_id_all_get_a_row(cx: &mut TestAppContext) {
    let film = torrent("film");
    let panel = trackers_of(&film, cx);
    take(
        panel,
        details_of(
            &film,
            vec![
                working(0, "a.example.org:6969"),
                working(0, "b.example.org:6969"),
            ],
        ),
        cx,
    );

    assert_eq!(
        drawn_rows(panel, cx),
        ["a.example.org:6969, tier 1", "b.example.org:6969, tier 1"]
    );
}

#[gpui_kit::test]
fn the_times_move_with_the_clock_and_the_tab_redraws_for_them(cx: &mut TestAppContext) {
    let film = torrent("film");
    let panel = trackers_of(&film, cx);
    take(
        panel,
        details_of(&film, vec![working(7, "a.example.org:6969")]),
        cx,
    );
    assert_eq!(
        cell_label(panel, 7, "tracker-next", cx).as_deref(),
        Some("in 27 m")
    );

    // A read that brings the same trackers sends nothing, so the tab draws again by itself: once
    // in 5 seconds, as long as it shows trackers.
    let redraws = Rc::new(Cell::new(0_u32));
    let entity = panel.entity(cx).expect("the panel is open");
    cx.update(|cx| {
        let redraws = Rc::clone(&redraws);
        cx.observe(&entity, move |_, _| redraws.set(redraws.get() + 1))
            .detach();
    });
    cx.executor().advance_clock(Duration::from_secs(4));
    cx.run_until_parked();
    assert_eq!(redraws.get(), 0, "not yet");
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(redraws.get(), 1);
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();
    assert_eq!(redraws.get(), 2);

    // The clock the times are counted from is read at each draw.
    change(panel, cx, |panel, _| {
        panel.trackers.clock = || fixed_now() + Duration::from_mins(10);
    });
    assert_eq!(
        cell_label(panel, 7, "tracker-next", cx).as_deref(),
        Some("in 17 m")
    );
    assert_eq!(
        cell_label(panel, 7, "tracker-last", cx).as_deref(),
        Some("34 m ago")
    );

    // Left, the tab stops drawing: nothing runs for a tab nobody sees.
    change(panel, cx, |panel, cx| {
        panel.set_tab(DetailsTab::General, cx);
    });
    let after_leaving = redraws.get();
    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();
    assert_eq!(redraws.get(), after_leaving);
}

#[gpui_kit::test]
fn a_tab_without_trackers_does_not_redraw_for_them(cx: &mut TestAppContext) {
    let film = torrent("film");
    let panel = trackers_of(&film, cx);
    take(panel, details_of(&film, Vec::new()), cx);
    assert!(!held(panel, cx).3);
    let redraws = Rc::new(Cell::new(0_u32));
    let entity = panel.entity(cx).expect("the panel is open");
    cx.update(|cx| {
        let redraws = Rc::clone(&redraws);
        cx.observe(&entity, move |_, _| redraws.set(redraws.get() + 1))
            .detach();
    });

    cx.executor().advance_clock(Duration::from_secs(60));
    cx.run_until_parked();

    assert_eq!(redraws.get(), 0);
}

#[gpui_kit::test]
fn the_cells_line_up_under_their_headers_in_rows_of_44_px(cx: &mut TestAppContext) {
    let film = torrent("film");
    let panel = trackers_of(&film, cx);
    take(panel, three(&film), cx);
    cx.simulate_window_resize(panel.into(), size(px(900.), px(400.)));

    drawn(panel, cx, |window| {
        let header = |window: &mut gpui_kit::Window, column: usize| -> (Pixels, Pixels) {
            let bounds = window
                .within(TRACKER_HEADER_ID)
                .find(("column", column))
                .bounds();
            (bounds.left(), bounds.size.width)
        };
        let cells = [
            "tracker-dot",
            "tracker-tier",
            "tracker-name",
            "tracker-last",
            "tracker-next",
            "tracker-seeders",
            "tracker-leechers",
            "tracker-downloads",
        ];
        for (column, cell) in cells.into_iter().enumerate() {
            let (left, width) = header(window, column);
            for id in [7, 3, 5] {
                let bounds = window.within(row_id(TrackerId(id), 0)).find(cell).bounds();
                assert_eq!(bounds.left(), left, "{cell} of {id}");
                assert_eq!(bounds.size.width, width, "{cell} of {id}");
            }
        }
        for id in [7, 3, 5] {
            let row = window.find(row_id(TrackerId(id), 0)).bounds();
            assert_eq!(row.size.height, px(ROW_HEIGHT), "row {id}");
        }
        // Not sortable: no header has a sort mark, and none draws a pointer.
        assert!(
            window
                .within(TRACKER_HEADER_ID)
                .try_find("sort-mark")
                .is_none()
        );
    });
    assert_eq!(ROW_HEIGHT, 44.);
}

/// The widths the columns ask for, without the gaps and the room around the line: the fixed ones
/// whole, and the least of the flexible ones.
fn least_width(columns: &[Column<()>]) -> f32 {
    columns
        .iter()
        .map(|column| match column.width {
            Width::Fixed(width) | Width::Flex(width) => width,
        })
        .sum()
}

/// The details panel at its least: the window at its least, less the sidebar.
const PANEL_LEAST_WIDTH: f32 = x4_core::MIN_WINDOW_WIDTH - SIDEBAR_WIDTH;

#[test]
fn the_peers_column_is_left_in_the_line_under_the_address_because_the_columns_do_not_fit() {
    // Even without the Peers column the columns ask for more than the panel has at its least
    // width, so a ninth one fits all the less; the peers are in the line under the address.
    assert!(
        least_width(&columns()) > PANEL_LEAST_WIDTH,
        "{} of {PANEL_LEAST_WIDTH}",
        least_width(&columns())
    );
    assert_eq!(columns().len(), 8);
}

#[test]
fn no_column_sorts() {
    for column in columns() {
        assert_eq!(column.sort, None, "{}", column.name);
    }
}

#[test]
fn the_times_are_counted_from_the_wall_clock_unless_a_test_sets_another() {
    let before = SystemTime::now();
    let read = (TrackersTab::new().clock)();
    let after = SystemTime::now();

    assert!(
        before <= read && read <= after,
        "{before:?} {read:?} {after:?}"
    );
}
