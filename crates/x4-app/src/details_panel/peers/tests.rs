//! The tests of the Peers tab: what it holds as the tab, the panel and the target change, what
//! its rows say, and how a click on a header orders them.

use std::cell::RefCell;
use std::net::IpAddr;
use std::rc::Rc;

use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};
use gpui_kit::{AppContext as _, Role, TestAppContext, WindowHandle, size};
use x4_core::test_support::torrent;
use x4_core::{
    CountryCode, DetailsTab, DetailsTarget, DisplayState, Peer, PeerSources, PeerTraits, Torrent,
};

use super::*;
use crate::app;

/// The places of the columns the tests click.
const ADDRESS: usize = 1;
const UP: usize = 5;
const TRAITS: usize = 6;

/// An idle peer at `address` and `port`, with nothing else said about it.
fn peer(address: &str, port: u16) -> Peer {
    Peer {
        ip: address.parse::<IpAddr>().ok().map(|ip| ip.to_canonical()),
        address: address.to_owned(),
        port,
        client: String::new(),
        progress: 0.0,
        rate_down: 0,
        rate_up: 0,
        traits: PeerTraits::default(),
        flags: String::new(),
        country: None,
    }
}

/// The peers of `torrent`, as the service sends them.
fn list_of(torrent: &Torrent, peers: Vec<Peer>) -> Arc<PeerList> {
    Arc::new(PeerList {
        hash: torrent.hash.clone(),
        peers,
        sources: PeerSources {
            tracker: 2,
            dht: 1,
            ..PeerSources::default()
        },
    })
}

/// Three peers of `torrent`: the busiest first, `.9` sends to us, `.10` receives from us and
/// `.2` is idle.
fn three(torrent: &Torrent) -> Arc<PeerList> {
    list_of(
        torrent,
        vec![
            peer("192.0.2.2", 3),
            Peer {
                rate_up: 4096,
                ..peer("192.0.2.10", 2)
            },
            Peer {
                rate_down: 8192,
                ..peer("192.0.2.9", 1)
            },
        ],
    )
}

/// A panel on the tab `tab`, about no torrent yet.
fn panel_on(tab: DetailsTab, cx: &mut TestAppContext) -> WindowHandle<DetailsPanel> {
    cx.update(app::init);
    cx.add_window(|window, cx| DetailsPanel::new(tab, Rc::new(RefCell::default()), window, cx))
}

/// A panel on the Peers tab about `torrent`.
fn peers_of(torrent: &Torrent, cx: &mut TestAppContext) -> WindowHandle<DetailsPanel> {
    let panel = panel_on(DetailsTab::Peers, cx);
    change(panel, cx, |panel, cx| {
        panel.show(DetailsTarget::One(torrent.hash.clone()), Some(torrent), cx);
    });
    panel
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

/// What the tab says instead of the peers, if it says anything.
fn empty_text(panel: WindowHandle<DetailsPanel>, cx: &mut TestAppContext) -> Option<String> {
    drawn(panel, cx, |window| {
        window
            .try_find(PEERS_EMPTY_ID)
            .and_then(|note| note.label().map(str::to_owned))
    })
}

/// The names of the rows drawn, top to bottom: each peer's address and port.
fn drawn_rows(panel: WindowHandle<DetailsPanel>, cx: &mut TestAppContext) -> Vec<String> {
    drawn(panel, cx, |window| {
        window
            .find_all_by_role(Role::Row)
            .iter()
            .filter_map(|row| row.label().map(str::to_owned))
            .collect()
    })
}

/// What the tab holds: whether it has a list, how many rows and the room it keeps for them, and
/// the sort.
fn held(
    panel: WindowHandle<DetailsPanel>,
    cx: &mut TestAppContext,
) -> (bool, usize, usize, PeerSort) {
    panel
        .update(cx, |panel, _, _| {
            let peers = &panel.peers;
            (
                peers.list.is_some(),
                peers.rows.len(),
                peers.rows.capacity(),
                peers.sort,
            )
        })
        .expect("the panel is open")
}

/// No list, no rows and no room for them, and the default order.
const NOTHING: (bool, usize, usize, PeerSort) = (
    false,
    0,
    0,
    PeerSort {
        column: None,
        descending: false,
    },
);

#[gpui_kit::test]
fn the_tab_says_a_dash_until_the_first_list_comes(cx: &mut TestAppContext) {
    let film = torrent("film");
    let panel = peers_of(&film, cx);
    assert_eq!(empty_text(panel, cx).as_deref(), Some("—"));
    assert_eq!(drawn_rows(panel, cx), [] as [String; 0]);

    change(panel, cx, |panel, cx| panel.take_peers(three(&film), cx));

    assert_eq!(empty_text(panel, cx), None);
    assert_eq!(
        drawn_rows(panel, cx),
        [
            "192.0.2.9, port 1",
            "192.0.2.10, port 2",
            "192.0.2.2, port 3"
        ]
    );
}

#[gpui_kit::test]
fn the_peers_of_another_torrent_are_never_shown(cx: &mut TestAppContext) {
    let (film, show) = (torrent("film"), torrent("show"));
    let panel = peers_of(&film, cx);

    // A list of a torrent the panel does not show is not taken.
    change(panel, cx, |panel, cx| panel.take_peers(three(&show), cx));
    assert_eq!(empty_text(panel, cx).as_deref(), Some("—"));
    assert_eq!(held(panel, cx), NOTHING);

    // The panel's own list shows, and goes the moment the panel is about another torrent: not
    // one frame shows the old rows under the new torrent.
    change(panel, cx, |panel, cx| panel.take_peers(three(&film), cx));
    assert_eq!(drawn_rows(panel, cx).len(), 3);
    change(panel, cx, |panel, cx| {
        panel.show(DetailsTarget::One(show.hash.clone()), Some(&show), cx);
    });
    assert_eq!(empty_text(panel, cx).as_deref(), Some("—"));
    assert_eq!(drawn_rows(panel, cx), [] as [String; 0]);

    // A read of the old torrent that comes late is dropped too.
    change(panel, cx, |panel, cx| panel.take_peers(three(&film), cx));
    assert_eq!(empty_text(panel, cx).as_deref(), Some("—"));
    change(panel, cx, |panel, cx| panel.take_peers(three(&show), cx));
    assert_eq!(drawn_rows(panel, cx).len(), 3);

    // Several torrents: none of their peers.
    change(panel, cx, |panel, cx| {
        panel.show(DetailsTarget::Many(2), None, cx);
    });
    change(panel, cx, |panel, cx| panel.take_peers(three(&show), cx));
    assert!(!held(panel, cx).0);
}

#[gpui_kit::test]
fn another_tab_takes_no_peers(cx: &mut TestAppContext) {
    let film = torrent("film");
    let panel = panel_on(DetailsTab::General, cx);
    change(panel, cx, |panel, cx| {
        panel.show(DetailsTarget::One(film.hash.clone()), Some(&film), cx);
        panel.take_peers(three(&film), cx);
    });
    assert_eq!(held(panel, cx), NOTHING);

    change(panel, cx, |panel, cx| panel.set_tab(DetailsTab::Peers, cx));
    assert_eq!(empty_text(panel, cx).as_deref(), Some("—"));
}

#[gpui_kit::test]
fn leaving_the_tab_drops_the_rows_and_the_order(cx: &mut TestAppContext) {
    let film = torrent("film");
    let panel = peers_of(&film, cx);
    change(panel, cx, |panel, cx| {
        panel.take_peers(three(&film), cx);
        panel.sort_peers(PeerColumn::Address, cx);
    });
    let (listed, rows, _, sort) = held(panel, cx);
    assert!(listed);
    assert_eq!((rows, sort.column), (3, Some(PeerColumn::Address)));

    change(panel, cx, |panel, cx| {
        panel.set_tab(DetailsTab::General, cx);
    });
    assert_eq!(held(panel, cx), NOTHING, "nothing held");

    // Back on the tab, it waits for the next list, in the default order.
    change(panel, cx, |panel, cx| panel.set_tab(DetailsTab::Peers, cx));
    assert_eq!(empty_text(panel, cx).as_deref(), Some("—"));
    change(panel, cx, |panel, cx| panel.take_peers(three(&film), cx));
    assert_eq!(
        drawn_rows(panel, cx).first().map(String::as_str),
        Some("192.0.2.9, port 1")
    );
}

#[gpui_kit::test]
fn a_hidden_panel_holds_no_peers(cx: &mut TestAppContext) {
    let film = torrent("film");
    let panel = peers_of(&film, cx);
    change(panel, cx, |panel, cx| {
        panel.take_peers(three(&film), cx);
        panel.sort_peers(PeerColumn::Client, cx);
    });

    change(panel, cx, |panel, _| panel.hide());
    assert_eq!(held(panel, cx), NOTHING);
    // A list that was on its way when the panel hid is not kept either.
    change(panel, cx, |panel, cx| panel.take_peers(three(&film), cx));
    assert_eq!(held(panel, cx), NOTHING);

    change(panel, cx, |panel, cx| {
        panel.show(DetailsTarget::One(film.hash.clone()), Some(&film), cx);
    });
    assert_eq!(empty_text(panel, cx).as_deref(), Some("—"));
}

#[gpui_kit::test]
fn cleared_peers_go_and_the_order_stays(cx: &mut TestAppContext) {
    let film = torrent("film");
    let panel = peers_of(&film, cx);
    change(panel, cx, |panel, cx| {
        panel.take_peers(three(&film), cx);
        panel.sort_peers(PeerColumn::Address, cx);
    });

    change(panel, cx, DetailsPanel::clear_peers);

    assert_eq!(empty_text(panel, cx).as_deref(), Some("—"));
    let (listed, rows, room, sort) = held(panel, cx);
    assert_eq!((listed, rows, room), (false, 0, 0));
    assert_eq!(sort.column, Some(PeerColumn::Address));
}

#[gpui_kit::test]
fn a_click_on_a_header_sorts_by_it_and_a_second_click_reverses(cx: &mut TestAppContext) {
    let film = torrent("film");
    let panel = peers_of(&film, cx);
    change(panel, cx, |panel, cx| panel.take_peers(three(&film), cx));
    let first_row = |cx: &mut TestAppContext| drawn_rows(panel, cx).first().cloned();
    let click_header = |column: usize, cx: &mut TestAppContext| {
        drawn(panel, cx, |_| ());
        cx.update_window(panel.into(), |_, window, cx| {
            window.within(PEER_HEADER_ID).click(("column", column), cx);
        })
        .expect("the panel is open");
        cx.run_until_parked();
    };
    // The busiest first.
    assert_eq!(first_row(cx).as_deref(), Some("192.0.2.9, port 1"));

    // The addresses by their bytes, smallest first, then largest first.
    click_header(ADDRESS, cx);
    assert_eq!(first_row(cx).as_deref(), Some("192.0.2.2, port 3"));
    let mark = drawn(panel, cx, |window| {
        window
            .within(PEER_HEADER_ID)
            .within(("column", ADDRESS))
            .find("sort-mark")
            .label()
            .map(str::to_owned)
    });
    assert_eq!(mark.as_deref(), Some("sorted ascending"));
    click_header(ADDRESS, cx);
    assert_eq!(first_row(cx).as_deref(), Some("192.0.2.10, port 2"));

    // The upload rate starts largest first; smallest first, the busiest of the two that send us
    // nothing comes first.
    click_header(UP, cx);
    assert_eq!(first_row(cx).as_deref(), Some("192.0.2.10, port 2"));
    click_header(UP, cx);
    assert_eq!(
        drawn_rows(panel, cx),
        [
            "192.0.2.9, port 1",
            "192.0.2.2, port 3",
            "192.0.2.10, port 2"
        ]
    );

    // The column of the connection's icons does not sort: a click on it leaves the order and the
    // mark as they were, and it gets no mark of its own.
    click_header(TRAITS, cx);
    assert_eq!(
        drawn_rows(panel, cx),
        [
            "192.0.2.9, port 1",
            "192.0.2.2, port 3",
            "192.0.2.10, port 2"
        ]
    );
    let (up_mark, traits_mark) = drawn(panel, cx, |window| {
        let mark = |window: &mut gpui_kit::Window, column: usize| {
            window
                .within(PEER_HEADER_ID)
                .within(("column", column))
                .try_find("sort-mark")
                .map(|mark| mark.label().map(str::to_owned))
        };
        (mark(window, UP), mark(window, TRAITS))
    });
    assert_eq!(up_mark, Some(Some("sorted ascending".to_owned())));
    assert_eq!(traits_mark, None);
}

#[gpui_kit::test]
fn an_empty_tab_says_why(cx: &mut TestAppContext) {
    for (state, text) in [
        (DisplayState::Paused, "The torrent is paused"),
        (DisplayState::Downloading, "No peers connected"),
        (DisplayState::Seeding, "No peers connected"),
        (DisplayState::Error, "No peers connected"),
    ] {
        let film = Torrent {
            state,
            ..torrent("film")
        };
        let panel = peers_of(&film, cx);
        change(panel, cx, |panel, cx| {
            panel.take_peers(list_of(&film, Vec::new()), cx);
        });
        assert_eq!(empty_text(panel, cx).as_deref(), Some(text), "{state:?}");
        assert!(
            drawn(panel, cx, |window| window
                .try_find(PEERS_SUMMARY_ID)
                .is_none()),
            "no summary without peers: {state:?}"
        );
    }
}

#[gpui_kit::test]
fn the_summary_line_shows_each_part_on_its_own(cx: &mut TestAppContext) {
    let film = torrent("film");
    let panel = peers_of(&film, cx);
    change(panel, cx, |panel, cx| panel.take_peers(three(&film), cx));

    let parts = drawn(panel, cx, |window| {
        let summary = window.find(PEERS_SUMMARY_ID);
        let parts: Vec<String> = (0..6usize)
            .filter_map(|index| {
                window
                    .within(PEERS_SUMMARY_ID)
                    .try_find(("summary-part", index))
                    .and_then(|part| part.label().map(str::to_owned))
            })
            .collect();
        (summary.bounds().size.height > px(0.), parts)
    });

    assert_eq!(
        parts,
        (
            true,
            [
                "3 peers",
                "2 from trackers",
                "1 from DHT",
                "1 sending to us",
                "1 receiving from us"
            ]
            .map(str::to_owned)
            .to_vec()
        )
    );
}

#[gpui_kit::test]
fn on_a_narrow_panel_the_summary_wraps_between_its_parts(cx: &mut TestAppContext) {
    let film = torrent("film");
    let panel = peers_of(&film, cx);
    change(panel, cx, |panel, cx| panel.take_peers(three(&film), cx));
    // On a wide panel the summary is one line.
    let (one_line, wide) = drawn(panel, cx, |window| {
        (
            window.find(("summary-part", 0usize)).bounds().size.height,
            window.find(PEERS_SUMMARY_ID).bounds().size.height,
        )
    });
    cx.simulate_window_resize(panel.into(), size(px(240.), px(400.)));

    drawn(panel, cx, |window| {
        let summary = window.find(PEERS_SUMMARY_ID).bounds();
        assert!(
            summary.size.height >= wide + one_line,
            "{:?}, one line {wide:?}",
            summary.size.height
        );
        // No part is cut: each lies whole inside the line, on one line of its own.
        for index in 0..5usize {
            let part = window
                .within(PEERS_SUMMARY_ID)
                .find(("summary-part", index))
                .bounds();
            assert!(
                summary.left() <= part.left()
                    && part.right() <= summary.right()
                    && summary.top() <= part.top()
                    && part.bottom() <= summary.bottom(),
                "part {index} at {part:?} outside {summary:?}"
            );
            assert_eq!(part.size.height, one_line, "part {index}");
        }
    });
}

/// The label of the cell `cell` of the row of `key`, the first of its address and port.
fn cell_label(
    panel: WindowHandle<DetailsPanel>,
    key: (&str, u16),
    cell: &'static str,
    cx: &mut TestAppContext,
) -> Option<String> {
    let row = row_id(&(key.0.to_owned(), key.1), 0);
    drawn(panel, cx, |window| {
        window
            .within(row)
            .try_find(cell)
            .and_then(|cell| cell.label().map(str::to_owned))
    })
}

#[gpui_kit::test]
fn a_row_draws_what_x4_core_decided(cx: &mut TestAppContext) {
    let film = Torrent {
        state: DisplayState::Downloading,
        ..torrent("film")
    };
    let panel = peers_of(&film, cx);
    let germany = CountryCode::new("DE").expect("a code");
    change(panel, cx, |panel, cx| {
        panel.take_peers(
            list_of(
                &film,
                vec![
                    Peer {
                        country: Some(germany),
                        client: "Client/1.2".to_owned(),
                        progress: 0.57,
                        rate_down: 1536,
                        traits: PeerTraits {
                            encrypted: true,
                            utp: true,
                            incoming: true,
                            ..PeerTraits::default()
                        },
                        ..peer("2001:db8::1", 51413)
                    },
                    peer("192.0.2.1", 6881),
                ],
            ),
            cx,
        );
    });
    let busy = ("2001:db8::1", 51413);
    let idle = ("192.0.2.1", 6881);

    assert_eq!(
        cell_label(panel, busy, "peer-country", cx).as_deref(),
        Some("DE")
    );
    assert_eq!(
        cell_label(panel, busy, "peer-address", cx).as_deref(),
        Some("2001:db8::1, port 51413")
    );
    assert_eq!(
        cell_label(panel, busy, "peer-client", cx).as_deref(),
        Some("Client/1.2")
    );
    assert_eq!(
        cell_label(panel, busy, "peer-progress", cx).as_deref(),
        Some("57%")
    );
    assert_eq!(
        cell_label(panel, busy, "peer-down", cx).as_deref(),
        Some("1.5 KiB/s")
    );
    assert_eq!(
        cell_label(panel, busy, "peer-up", cx).as_deref(),
        Some("0 B/s")
    );
    for (icon, word) in [
        ("peer-encrypted", "Encrypted connection"),
        ("peer-utp", "Connected over µTP, not TCP"),
        ("peer-incoming", "The peer connected to us"),
    ] {
        assert_eq!(
            cell_label(panel, busy, icon, cx).as_deref(),
            Some(word),
            "{icon}"
        );
        assert_eq!(
            cell_label(panel, idle, icon, cx),
            None,
            "{icon} of the idle peer"
        );
    }

    // A peer without a country has the globe, and one without a client name a dash.
    assert_eq!(
        cell_label(panel, idle, "peer-country", cx).as_deref(),
        Some("Unknown country")
    );
    assert_eq!(
        cell_label(panel, idle, "peer-client", cx).as_deref(),
        Some("—")
    );
    assert_eq!(
        cell_label(panel, idle, "peer-progress", cx).as_deref(),
        Some("0%")
    );
}

#[test]
fn a_row_is_keyed_by_its_address_and_port_and_repeats_get_their_own_ids() {
    let key = |address: &str, port: u16| (address.to_owned(), port);
    let ids = |keys: &[(String, u16)]| -> Vec<ElementId> {
        let rows: Vec<PeerRow> = keys
            .iter()
            .map(|(address, port)| PeerRow::from(&peer(address, *port)))
            .collect();
        row_ids(&rows)
    };
    let (a, b) = (key("192.0.2.1", 1), key("192.0.2.2", 1));

    // The same rows in another order keep their ids.
    let first = ids(&[a.clone(), b.clone()]);
    let resorted = ids(&[b.clone(), a.clone()]);
    assert_eq!(first.first(), resorted.get(1));
    assert_eq!(first.get(1), resorted.first());
    assert_ne!(first.first(), first.get(1));

    // The daemon can send the same text and port twice ("Invalid address"): never the same id.
    let twice = key("Invalid address", 6881);
    let repeated = ids(&[twice.clone(), twice.clone(), a.clone()]);
    assert_ne!(repeated.first(), repeated.get(1));
    assert_eq!(repeated.first(), Some(&row_id(&twice, 0)));
    assert_eq!(repeated.get(1), Some(&row_id(&twice, 1)));
    // An address with a space and a number cannot pass for a repeat of another.
    assert_ne!(row_id(&key("1 x", 6881), 0), row_id(&key("x", 6881), 1));
}
