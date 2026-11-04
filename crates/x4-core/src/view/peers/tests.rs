//! Tests of the order of the rows, the summary and the reasons for an empty tab.

use std::net::IpAddr;

use x4_rpc::HashString;

use super::*;
use crate::peers::{Peer, PeerSources};
use crate::state::QueueKind;

fn code(letters: &str) -> CountryCode {
    CountryCode::new(letters).unwrap()
}

/// An idle, unremarkable peer at `address` and `port`.
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

fn list(peers: Vec<Peer>) -> PeerList {
    PeerList {
        hash: HashString::parse(&"a".repeat(40)).unwrap(),
        peers,
        sources: PeerSources::default(),
    }
}

/// The keys of the rows as `address:port`, which says which peer is where.
fn order(list: &PeerList, sort: PeerSort) -> Vec<String> {
    peer_rows(list, sort)
        .into_iter()
        .map(|row| format!("{}:{}", row.key.0, row.key.1))
        .collect()
}

fn default_order(list: &PeerList) -> Vec<String> {
    order(list, PeerSort::default())
}

const KIB: u64 = 1024;

#[test]
fn the_default_order_puts_the_fastest_senders_first_then_the_fastest_receivers() {
    let list = list(vec![
        Peer {
            rate_down: 5 * KIB,
            ..peer("192.0.2.1", 1)
        },
        Peer {
            rate_down: 50 * KIB,
            ..peer("192.0.2.2", 1)
        },
        Peer {
            rate_up: 90 * KIB,
            ..peer("192.0.2.3", 1)
        },
        Peer {
            rate_down: 5 * KIB,
            rate_up: KIB,
            ..peer("192.0.2.4", 1)
        },
        peer("192.0.2.5", 1),
    ]);

    assert_eq!(
        default_order(&list),
        [
            "192.0.2.2:1",
            "192.0.2.4:1",
            "192.0.2.1:1",
            "192.0.2.3:1",
            "192.0.2.5:1"
        ]
    );
}

#[test]
fn rates_are_compared_in_whole_kib_per_second() {
    // 1.2 and 1.9 KiB/s are both 1: the higher address does not jump ahead by being faster.
    let jitter = |low: u64, high: u64| {
        list(vec![
            Peer {
                rate_down: low,
                ..peer("192.0.2.1", 1)
            },
            Peer {
                rate_down: high,
                ..peer("192.0.2.9", 1)
            },
        ])
    };
    let wobbly = jitter(KIB * 12 / 10, KIB * 19 / 10);
    let swapped = jitter(KIB * 19 / 10, KIB * 12 / 10);
    assert_eq!(default_order(&wobbly), ["192.0.2.1:1", "192.0.2.9:1"]);
    assert_eq!(default_order(&swapped), ["192.0.2.1:1", "192.0.2.9:1"]);

    // The edge between 0 and 1 KiB/s, and between 1 and 2.
    let edge = list(vec![
        Peer {
            rate_down: KIB - 1,
            ..peer("192.0.2.1", 1)
        },
        Peer {
            rate_down: KIB,
            ..peer("192.0.2.2", 1)
        },
        Peer {
            rate_down: 2 * KIB,
            ..peer("192.0.2.3", 1)
        },
        Peer {
            rate_down: 2 * KIB - 1,
            ..peer("192.0.2.4", 1)
        },
    ]);
    assert_eq!(
        default_order(&edge),
        ["192.0.2.3:1", "192.0.2.2:1", "192.0.2.4:1", "192.0.2.1:1"]
    );

    // The same wobble in the rate we send.
    let uploads = list(vec![
        Peer {
            rate_up: KIB * 19 / 10,
            ..peer("192.0.2.1", 1)
        },
        Peer {
            rate_up: KIB * 12 / 10,
            ..peer("192.0.2.2", 1)
        },
    ]);
    assert_eq!(default_order(&uploads), ["192.0.2.1:1", "192.0.2.2:1"]);
}

#[test]
fn when_everything_is_idle_the_seeds_come_first() {
    let list = list(vec![
        Peer {
            progress: 0.0,
            ..peer("192.0.2.1", 1)
        },
        Peer {
            progress: 0.4,
            ..peer("192.0.2.2", 1)
        },
        Peer {
            progress: 1.0,
            ..peer("192.0.2.9", 1)
        },
        Peer {
            progress: 0.9,
            ..peer("192.0.2.3", 1)
        },
        Peer {
            progress: 1.0,
            ..peer("192.0.2.5", 1)
        },
    ]);

    assert_eq!(
        default_order(&list),
        [
            "192.0.2.5:1",
            "192.0.2.9:1",
            "192.0.2.3:1",
            "192.0.2.2:1",
            "192.0.2.1:1"
        ]
    );
}

#[test]
fn equal_peers_are_ordered_by_the_bytes_of_the_address_and_then_the_port() {
    let list = list(vec![
        Peer {
            ..peer("not an ip", 1)
        },
        Peer {
            ..peer("2001:db8::1", 1)
        },
        Peer {
            ..peer("192.0.2.10", 2)
        },
        Peer {
            ..peer("192.0.2.9", 5)
        },
        Peer {
            ..peer("192.0.2.10", 1)
        },
        Peer {
            ..peer("::ffff:192.0.2.2", 7)
        },
        Peer {
            ..peer("another one", 1)
        },
    ]);

    // By text "192.0.2.10" would come before "192.0.2.9".
    assert_eq!(
        default_order(&list),
        [
            "::ffff:192.0.2.2:7",
            "192.0.2.9:5",
            "192.0.2.10:1",
            "192.0.2.10:2",
            "2001:db8::1:1",
            "another one:1",
            "not an ip:1",
        ]
    );
}

#[test]
fn the_default_order_does_not_depend_on_the_order_of_the_daemon() {
    let peers = vec![
        Peer {
            rate_down: 9 * KIB,
            ..peer("192.0.2.1", 1)
        },
        Peer {
            progress: 1.0,
            ..peer("192.0.2.2", 1)
        },
        Peer {
            rate_up: 3 * KIB,
            ..peer("192.0.2.3", 1)
        },
        peer("192.0.2.4", 1),
    ];
    let mut reversed = peers.clone();
    reversed.reverse();

    assert_eq!(default_order(&list(peers)), default_order(&list(reversed)));
}

#[test]
fn the_default_order_ignores_the_direction() {
    let list = list(vec![
        Peer {
            rate_down: 9 * KIB,
            ..peer("192.0.2.1", 1)
        },
        peer("192.0.2.2", 1),
    ]);

    assert_eq!(
        order(
            &list,
            PeerSort {
                column: None,
                descending: true
            }
        ),
        default_order(&list)
    );
}

/// Five peers that every column tells apart, with ties for the columns to break:
///
/// | peer      | country | client    | progress | down       | up         |
/// |:--|:--|:--|:--|:--|:--|
/// | `.1`      | DE      | `bravo`   | 0.5      | 3000       | 0          |
/// | `.2`      | DE      | `Alpha`   | 0.5      | 0          | 9000       |
/// | `.3`      | FR      | (none)    | 1.0      | 0          | 0          |
/// | `.4`      | (none)  | `charlie` | 0.1      | 3000       | 5000       |
/// | `2001::`  | AU      | `alpha`   | 0.9      | 100        | 0          |
///
/// Their default order is `.4`, `.1`, `.2`, `.3`, `2001::`.
fn five() -> PeerList {
    list(vec![
        Peer {
            country: Some(code("DE")),
            client: "bravo".to_owned(),
            progress: 0.5,
            rate_down: 3000,
            ..peer("192.0.2.1", 1)
        },
        Peer {
            country: Some(code("DE")),
            client: "Alpha".to_owned(),
            progress: 0.5,
            rate_up: 9000,
            ..peer("192.0.2.2", 2)
        },
        Peer {
            country: Some(code("FR")),
            progress: 1.0,
            ..peer("192.0.2.3", 3)
        },
        Peer {
            client: "charlie".to_owned(),
            progress: 0.1,
            rate_down: 3000,
            rate_up: 5000,
            ..peer("192.0.2.4", 4)
        },
        Peer {
            country: Some(code("AU")),
            client: "alpha".to_owned(),
            progress: 0.9,
            rate_down: 100,
            ..peer("2001:db8::1", 5)
        },
    ])
}

fn sorted(column: PeerColumn, descending: bool) -> Vec<String> {
    order(
        &five(),
        PeerSort {
            column: Some(column),
            descending,
        },
    )
}

const P1: &str = "192.0.2.1:1";
const P2: &str = "192.0.2.2:2";
const P3: &str = "192.0.2.3:3";
const P4: &str = "192.0.2.4:4";
const P5: &str = "2001:db8::1:5";

#[test]
fn the_fixture_peers_have_the_default_order_the_column_tests_rely_on() {
    assert_eq!(default_order(&five()), [P4, P1, P2, P3, P5]);
}

#[test]
fn sorting_by_country_puts_peers_without_one_last_and_keeps_the_default_order_among_equals() {
    assert_eq!(sorted(PeerColumn::Country, false), [P5, P1, P2, P3, P4]);
    // Only the countries reverse: P1 stays before P2.
    assert_eq!(sorted(PeerColumn::Country, true), [P4, P3, P1, P2, P5]);
}

#[test]
fn sorting_by_address_goes_by_the_ip_with_ipv4_first() {
    assert_eq!(sorted(PeerColumn::Address, false), [P1, P2, P3, P4, P5]);
    assert_eq!(sorted(PeerColumn::Address, true), [P5, P4, P3, P2, P1]);

    let numbers = list(vec![
        peer("192.0.2.10", 1),
        peer("junk", 1),
        peer("192.0.2.9", 1),
        peer("::1", 1),
        peer("also junk", 1),
    ]);
    let column = |descending| {
        order(
            &numbers,
            PeerSort {
                column: Some(PeerColumn::Address),
                descending,
            },
        )
    };
    assert_eq!(
        column(false),
        [
            "192.0.2.9:1",
            "192.0.2.10:1",
            "::1:1",
            "also junk:1",
            "junk:1"
        ]
    );
    assert_eq!(
        column(true),
        [
            "junk:1",
            "also junk:1",
            "::1:1",
            "192.0.2.10:1",
            "192.0.2.9:1"
        ]
    );
}

#[test]
fn sorting_by_the_same_address_goes_by_the_port() {
    let numbers = list(vec![
        peer("192.0.2.1", 30),
        peer("192.0.2.1", 4),
        peer("192.0.2.1", 200),
    ]);
    let column = |descending| {
        order(
            &numbers,
            PeerSort {
                column: Some(PeerColumn::Address),
                descending,
            },
        )
    };

    assert_eq!(
        column(false),
        ["192.0.2.1:4", "192.0.2.1:30", "192.0.2.1:200"]
    );
    // The port is a tiebreak, which does not reverse.
    assert_eq!(
        column(true),
        ["192.0.2.1:4", "192.0.2.1:30", "192.0.2.1:200"]
    );
}

#[test]
fn sorting_by_client_ignores_case_and_puts_empty_names_last() {
    // "Alpha" and "alpha" are equal; the default order puts P2 before P5.
    assert_eq!(sorted(PeerColumn::Client, false), [P2, P5, P1, P4, P3]);
    assert_eq!(sorted(PeerColumn::Client, true), [P3, P4, P1, P2, P5]);
}

#[test]
fn sorting_by_progress_keeps_the_default_order_among_equals() {
    assert_eq!(sorted(PeerColumn::Progress, false), [P4, P1, P2, P5, P3]);
    assert_eq!(sorted(PeerColumn::Progress, true), [P3, P5, P1, P2, P4]);
}

#[test]
fn sorting_by_the_download_rate() {
    assert_eq!(sorted(PeerColumn::Down, false), [P2, P3, P5, P4, P1]);
    assert_eq!(sorted(PeerColumn::Down, true), [P4, P1, P5, P2, P3]);
}

#[test]
fn sorting_by_the_upload_rate() {
    assert_eq!(sorted(PeerColumn::Up, false), [P1, P3, P5, P4, P2]);
    assert_eq!(sorted(PeerColumn::Up, true), [P2, P4, P1, P3, P5]);
}

#[test]
fn a_chosen_rate_column_compares_exact_rates_not_whole_kibs() {
    let list = list(vec![
        Peer {
            rate_down: KIB * 12 / 10,
            ..peer("192.0.2.1", 1)
        },
        Peer {
            rate_down: KIB * 19 / 10,
            ..peer("192.0.2.9", 1)
        },
    ]);
    let down = |descending| {
        order(
            &list,
            PeerSort {
                column: Some(PeerColumn::Down),
                descending,
            },
        )
    };

    assert_eq!(down(true), ["192.0.2.9:1", "192.0.2.1:1"]);
    assert_eq!(down(false), ["192.0.2.1:1", "192.0.2.9:1"]);
}

#[test]
fn every_sort_keeps_every_peer_once() {
    let list = five();
    let mut expected: Vec<String> = [P1, P2, P3, P4, P5].map(str::to_owned).to_vec();
    expected.sort();
    let columns = [
        None,
        Some(PeerColumn::Country),
        Some(PeerColumn::Address),
        Some(PeerColumn::Client),
        Some(PeerColumn::Progress),
        Some(PeerColumn::Down),
        Some(PeerColumn::Up),
    ];
    for column in columns {
        for descending in [false, true] {
            let mut keys = order(&list, PeerSort { column, descending });
            keys.sort();
            assert_eq!(keys, expected, "{column:?} {descending}");
        }
    }
}

#[test]
fn a_row_holds_what_the_view_prints() {
    let rows = peer_rows(
        &list(vec![Peer {
            country: Some(code("DE")),
            client: "Name/1.2".to_owned(),
            progress: 0.25,
            rate_down: 1500,
            rate_up: 0,
            traits: PeerTraits {
                encrypted: true,
                utp: true,
                ..PeerTraits::default()
            },
            flags: "TE".to_owned(),
            ..peer("::ffff:192.0.2.7", 51413)
        }]),
        PeerSort::default(),
    );

    assert_eq!(
        rows,
        [PeerRow {
            key: ("::ffff:192.0.2.7".to_owned(), 51413),
            country: Some(code("DE")),
            address: "::ffff:192.0.2.7".to_owned(),
            client: "Name/1.2".to_owned(),
            progress: 0.25,
            rate_down: 1500,
            rate_up: 0,
            traits: PeerTraits {
                encrypted: true,
                utp: true,
                ..PeerTraits::default()
            },
            percent: 25,
            transferring_down: true,
            transferring_up: false,
        }]
    );
}

#[test]
fn a_row_shows_whole_percents_and_100_only_when_the_peer_is_complete() {
    let percents = |progress: &[f32]| -> Vec<u8> {
        let peers = progress
            .iter()
            .enumerate()
            .map(|(index, &progress)| Peer {
                progress,
                ..peer("192.0.2.1", u16::try_from(index).unwrap())
            })
            .collect();
        let sort = PeerSort {
            column: Some(PeerColumn::Progress),
            descending: false,
        };
        peer_rows(&list(peers), sort)
            .iter()
            .map(|row| row.percent)
            .collect()
    };

    // The daemon keeps a peer's progress as an `f32`: 57 of 100 pieces is 0.569_999_99.
    assert_eq!(
        percents(&[0.0, 0.29, 0.57, 0.634, 0.999_99, 1.0]),
        [0, 29, 57, 63, 99, 100]
    );
}

#[test]
fn transferring_is_decided_by_the_rates_not_by_the_flags() {
    let rows = peer_rows(
        &list(vec![
            // Unchoked and interested both ways, but nothing flows.
            Peer {
                flags: "TDUE".to_owned(),
                ..peer("192.0.2.1", 1)
            },
            Peer {
                rate_down: 1,
                ..peer("192.0.2.2", 1)
            },
            Peer {
                rate_up: 1,
                ..peer("192.0.2.3", 1)
            },
        ]),
        PeerSort {
            column: Some(PeerColumn::Address),
            descending: false,
        },
    );

    let flows: Vec<(bool, bool)> = rows
        .iter()
        .map(|row| (row.transferring_down, row.transferring_up))
        .collect();
    assert_eq!(flows, [(false, false), (true, false), (false, true)]);
}

#[test]
fn an_empty_list_has_no_rows() {
    assert_eq!(peer_rows(&list(Vec::new()), PeerSort::default()), []);
}

fn sources(sources: PeerSources) -> PeerList {
    PeerList {
        sources,
        ..list(Vec::new())
    }
}

#[test]
fn the_summary_lists_the_sources_with_peers_by_count() {
    let summary = peers_summary(&sources(PeerSources {
        tracker: 1,
        dht: 4,
        pex: 15,
        ..PeerSources::default()
    }));

    assert_eq!(
        summary.sources,
        [
            (PeerSource::Pex, 15),
            (PeerSource::Dht, 4),
            (PeerSource::Tracker, 1)
        ]
    );
}

#[test]
fn sources_with_equal_counts_keep_the_order_of_the_enum() {
    let summary = peers_summary(&sources(PeerSources {
        tracker: 2,
        dht: 2,
        pex: 3,
        lpd: 2,
        incoming: 3,
        ltep: 2,
        resume: 2,
    }));

    assert_eq!(
        summary.sources,
        [
            (PeerSource::Pex, 3),
            (PeerSource::Incoming, 3),
            (PeerSource::Tracker, 2),
            (PeerSource::Dht, 2),
            (PeerSource::Lpd, 2),
            (PeerSource::Ltep, 2),
            (PeerSource::Resume, 2),
        ]
    );
}

#[test]
fn a_torrent_with_no_sources_has_none_listed() {
    assert_eq!(peers_summary(&sources(PeerSources::default())).sources, []);
    assert_eq!(peers_summary(&list(Vec::new())), PeersSummary::default());
}

#[test]
fn the_summary_counts_the_connected_peers_and_who_transfers_by_the_rates() {
    let list = list(vec![
        // Unchoked and interested but idle: not counted, whatever the flags say.
        Peer {
            flags: "TDUE".to_owned(),
            ..peer("192.0.2.1", 1)
        },
        Peer {
            rate_down: 10,
            ..peer("192.0.2.2", 1)
        },
        Peer {
            rate_down: 20,
            rate_up: 30,
            ..peer("192.0.2.3", 1)
        },
        Peer {
            rate_up: 1,
            ..peer("192.0.2.4", 1)
        },
        // Flowing without a single flag set.
        Peer {
            rate_down: 1,
            flags: String::new(),
            ..peer("192.0.2.5", 1)
        },
        peer("192.0.2.6", 1),
    ]);

    let summary = peers_summary(&list);

    assert_eq!(summary.connected, 6);
    assert_eq!(summary.sending_to_us, 3);
    assert_eq!(summary.receiving_from_us, 2);
}

#[test]
fn the_summary_counts_the_peers_of_the_list_not_the_sources() {
    // The daemon's origins may lag behind the list by a moment; the rows are what is shown.
    let list = PeerList {
        sources: PeerSources {
            dht: 9,
            ..PeerSources::default()
        },
        ..list(vec![peer("192.0.2.1", 1), peer("192.0.2.2", 1)])
    };

    let summary = peers_summary(&list);

    assert_eq!(summary.connected, 2);
    assert_eq!(summary.sources, [(PeerSource::Dht, 9)]);
}

#[test]
fn peers_to_show_leave_no_empty_reason() {
    let some = list(vec![peer("192.0.2.1", 1)]);

    for state in [
        DisplayState::Downloading,
        DisplayState::Seeding,
        DisplayState::Paused,
        DisplayState::Queued(QueueKind::Download),
        DisplayState::Checking,
        DisplayState::Error,
    ] {
        assert_eq!(peers_empty(state, Some(&some)), None, "{state:?}");
    }
}

#[test]
fn nothing_is_known_before_the_first_list_comes() {
    for state in [
        DisplayState::Downloading,
        DisplayState::Paused,
        DisplayState::Checking,
    ] {
        assert_eq!(
            peers_empty(state, None),
            Some(PeersEmpty::Reading),
            "{state:?}"
        );
    }
}

#[test]
fn an_empty_list_of_a_stopped_torrent_says_it_is_paused() {
    let none = list(Vec::new());

    assert_eq!(
        peers_empty(DisplayState::Paused, Some(&none)),
        Some(PeersEmpty::Paused)
    );
}

#[test]
fn an_empty_list_of_any_other_torrent_says_no_one_is_connected() {
    let none = list(Vec::new());

    for state in [
        DisplayState::Downloading,
        DisplayState::Seeding,
        DisplayState::Queued(QueueKind::Download),
        DisplayState::Queued(QueueKind::Seed),
        DisplayState::Checking,
        DisplayState::Error,
    ] {
        assert_eq!(
            peers_empty(state, Some(&none)),
            Some(PeersEmpty::NoneConnected),
            "{state:?}"
        );
    }
}

#[test]
fn a_header_click_sorts_by_its_column_and_a_second_click_reverses() {
    let mut sort = PeerSort::default();

    sort.toggle(PeerColumn::Address);
    assert_eq!(
        sort,
        PeerSort {
            column: Some(PeerColumn::Address),
            descending: false,
        }
    );
    sort.toggle(PeerColumn::Address);
    assert!(sort.descending, "reversed");
    sort.toggle(PeerColumn::Address);
    assert!(!sort.descending, "and back");
}

#[test]
fn the_rates_and_the_progress_start_largest_first_and_the_texts_smallest_first() {
    for (column, descending) in [
        (PeerColumn::Country, false),
        (PeerColumn::Address, false),
        (PeerColumn::Client, false),
        (PeerColumn::Progress, true),
        (PeerColumn::Down, true),
        (PeerColumn::Up, true),
    ] {
        let mut from_default = PeerSort::default();
        from_default.toggle(column);
        assert_eq!(
            from_default,
            PeerSort {
                column: Some(column),
                descending,
            },
            "{column:?} after the default order"
        );

        // From another column, in either direction, the column starts in its own direction.
        for other_descending in [false, true] {
            let other = if column == PeerColumn::Up {
                PeerColumn::Down
            } else {
                PeerColumn::Up
            };
            let mut from_other = PeerSort {
                column: Some(other),
                descending: other_descending,
            };
            from_other.toggle(column);
            assert_eq!(from_other, from_default, "{column:?} after {other:?}");
        }
    }
}
