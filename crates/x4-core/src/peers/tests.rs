//! Tests of building a [`PeerList`], cleaning what peers chose, finding countries and naming flag
//! letters.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr};

use x4_rpc::testing::fixtures::TORRENT_GET_PEERS;
use x4_rpc::{PeerRecord, PeersFromRecord, TorrentRecord};

use super::*;
use crate::test_support::fixture_result;

const HASH: &str = "0123456789abcdef0123456789abcdef01234567";
/// The hash of the torrent in the peers fixture.
const FIXTURE_HASH: &str = "0000000000000000000000000000000000000005";

fn hash() -> HashString {
    HashString::parse(HASH).unwrap()
}

fn ip(text: &str) -> IpAddr {
    text.parse().unwrap()
}

fn code(letters: &str) -> CountryCode {
    CountryCode::new(letters).unwrap()
}

fn record(address: &str, port: u16) -> PeerRecord {
    PeerRecord {
        address: address.to_owned(),
        port,
        ..PeerRecord::default()
    }
}

/// An answer holding one torrent, `HASH`, with these peers and origins.
fn answer(
    peers: Option<Vec<PeerRecord>>,
    peers_from: Option<PeersFromRecord>,
) -> TorrentGetResponse {
    TorrentGetResponse {
        torrents: vec![TorrentRecord {
            hash_string: Some(HASH.to_owned()),
            peers,
            peers_from,
            ..TorrentRecord::default()
        }],
        removed: Vec::new(),
    }
}

/// The only peer of an answer made of `record`.
fn built(record: PeerRecord) -> Peer {
    let list = build_peers(&hash(), &answer(Some(vec![record]), None)).unwrap();
    assert_eq!(list.peers.len(), 1);
    list.peers.into_iter().next().unwrap()
}

fn fixture() -> TorrentGetResponse {
    serde_json::from_value(fixture_result(TORRENT_GET_PEERS)).unwrap()
}

#[test]
fn the_fixture_builds_a_list_with_its_peers_and_origins() {
    let list = build_peers(&HashString::parse(FIXTURE_HASH).unwrap(), &fixture()).unwrap();

    assert_eq!(list.hash.as_str(), FIXTURE_HASH);
    assert_eq!(list.peers.len(), 20);
    assert_eq!(
        list.sources,
        PeerSources {
            tracker: 1,
            dht: 4,
            pex: 15,
            ..PeerSources::default()
        }
    );
    assert!(list.peers.iter().all(|peer| peer.ip.is_some()));
    assert!(list.peers.iter().all(|peer| peer.country.is_none()));
    assert!(
        list.peers
            .iter()
            .all(|peer| (0.0..=1.0).contains(&peer.progress))
    );

    // The first peer of the fixture: a partial leecher we upload to over uTP.
    let first = list.peers.first().unwrap();
    assert_eq!(first.ip, Some(ip("192.0.2.1")));
    assert_eq!(first.address, "192.0.2.1");
    assert_eq!(first.port, 63081);
    assert_eq!(first.client, "Client 1");
    assert_eq!(first.flags, "TUEX");
    assert!((first.progress - 0.185_284_67).abs() < 1e-6);
    assert_eq!((first.rate_down, first.rate_up), (0, 165_840));
    assert_eq!(
        first.traits,
        PeerTraits {
            encrypted: true,
            utp: true,
            choking_us: true,
            interested_in_us: true,
            ..PeerTraits::default()
        }
    );
}

#[test]
fn a_torrent_that_is_not_in_the_answer_has_no_list() {
    let other = HashString::parse(&"f".repeat(40)).unwrap();

    assert_eq!(build_peers(&other, &answer(Some(vec![]), None)), None);
    assert_eq!(
        build_peers(
            &hash(),
            &TorrentGetResponse {
                torrents: Vec::new(),
                removed: Vec::new(),
            }
        ),
        None
    );
}

#[test]
fn the_torrent_is_found_by_its_hash_among_others_in_any_case() {
    let mut response = answer(Some(vec![record("192.0.2.1", 1)]), None);
    response.torrents.insert(
        0,
        TorrentRecord {
            hash_string: Some("f".repeat(40)),
            peers: Some(vec![record("192.0.2.9", 9), record("192.0.2.8", 8)]),
            ..TorrentRecord::default()
        },
    );
    response.torrents.push(TorrentRecord::default());
    if let Some(wanted) = response.torrents.get_mut(1) {
        wanted.hash_string = Some(HASH.to_uppercase());
    }

    let list = build_peers(&hash(), &response).unwrap();

    assert_eq!(list.hash, hash());
    assert_eq!(list.peers.len(), 1);
    assert_eq!(list.peers.first().map(|peer| peer.port), Some(1));
}

#[test]
fn a_record_without_peers_or_origins_is_an_empty_list() {
    let list = build_peers(&hash(), &answer(None, None)).unwrap();

    assert_eq!(list.hash, hash());
    assert_eq!(list.peers, []);
    assert_eq!(list.sources, PeerSources::default());
}

#[test]
fn the_seven_origins_are_named_for_what_they_are() {
    let list = build_peers(
        &hash(),
        &answer(
            Some(Vec::new()),
            Some(PeersFromRecord {
                from_cache: 1,
                from_dht: 2,
                from_incoming: 3,
                from_lpd: 4,
                from_ltep: 5,
                from_pex: 6,
                from_tracker: 7,
            }),
        ),
    )
    .unwrap();

    assert_eq!(
        list.sources,
        PeerSources {
            tracker: 7,
            dht: 2,
            pex: 6,
            lpd: 4,
            incoming: 3,
            ltep: 5,
            resume: 1,
        }
    );
}

#[test]
fn the_wire_flags_are_renamed_for_what_they_say_about_the_peer() {
    // Each flag is set alone, so a swapped pair shows.
    let with = |set: fn(&mut PeerRecord)| {
        let mut record = record("192.0.2.1", 1);
        set(&mut record);
        built(record).traits
    };

    assert_eq!(
        with(|r| r.is_encrypted = true),
        PeerTraits {
            encrypted: true,
            ..PeerTraits::default()
        }
    );
    assert_eq!(
        with(|r| r.is_utp = true),
        PeerTraits {
            utp: true,
            ..PeerTraits::default()
        }
    );
    assert_eq!(
        with(|r| r.is_incoming = true),
        PeerTraits {
            incoming: true,
            ..PeerTraits::default()
        }
    );
    assert_eq!(
        with(|r| r.client_is_choked = true),
        PeerTraits {
            choking_us: true,
            ..PeerTraits::default()
        }
    );
    assert_eq!(
        with(|r| r.peer_is_choked = true),
        PeerTraits {
            choked_by_us: true,
            ..PeerTraits::default()
        }
    );
    assert_eq!(
        with(|r| r.peer_is_interested = true),
        PeerTraits {
            interested_in_us: true,
            ..PeerTraits::default()
        }
    );
    assert_eq!(
        with(|r| r.client_is_interested = true),
        PeerTraits {
            we_are_interested: true,
            ..PeerTraits::default()
        }
    );
    // "Downloading from" and "uploading to" are not traits: the rates say what flows.
    assert_eq!(
        with(|r| {
            r.is_downloading_from = true;
            r.is_uploading_to = true;
        }),
        PeerTraits::default()
    );
}

#[test]
fn the_rates_and_the_flag_string_come_through() {
    let peer = built(PeerRecord {
        rate_to_client: 1_500_000,
        rate_to_peer: 48_000,
        flag_str: "TODUEI".to_owned(),
        ..record("192.0.2.1", 6881)
    });

    assert_eq!((peer.rate_down, peer.rate_up), (1_500_000, 48_000));
    assert_eq!(peer.flags, "TODUEI");
}

#[test]
fn an_ipv4_mapped_ipv6_address_is_the_ipv4_address_it_stands_for() {
    let peer = built(record("::ffff:192.0.2.7", 6881));

    assert_eq!(peer.ip, Some(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7))));
    assert_eq!(peer.address, "::ffff:192.0.2.7", "the text stays as sent");
}

#[test]
fn an_ipv6_address_stays_ipv6() {
    let peer = built(record("2001:db8::1", 6881));

    assert_eq!(peer.ip, Some(ip("2001:db8::1")));
    assert!(peer.ip.is_some_and(|ip| ip.is_ipv6()));
}

#[test]
fn an_address_that_is_no_ip_keeps_its_row_without_an_ip() {
    for text in [
        "Invalid address",
        "",
        "192.0.2.300",
        "192.0.2.1:6881",
        "[::1]",
    ] {
        let peer = built(PeerRecord {
            client_name: "Client".to_owned(),
            ..record(text, 6881)
        });

        assert_eq!(peer.ip, None, "{text:?}");
        assert_eq!(peer.address, text);
        assert_eq!(peer.port, 6881);
        assert_eq!(peer.client, "Client");
    }
}

#[test]
fn progress_is_clamped_to_the_unit_interval() {
    let progress = |raw: f64| {
        built(PeerRecord {
            progress: raw,
            ..record("192.0.2.1", 1)
        })
        .progress
    };

    assert!(progress(0.0).abs() < f32::EPSILON);
    assert!((progress(0.25) - 0.25).abs() < f32::EPSILON);
    assert!((progress(1.0) - 1.0).abs() < f32::EPSILON);
    assert!((progress(1.7) - 1.0).abs() < f32::EPSILON);
    assert!((progress(f64::INFINITY) - 1.0).abs() < f32::EPSILON);
    assert!(progress(-0.2).abs() < f32::EPSILON);
    assert!(progress(f64::NEG_INFINITY).abs() < f32::EPSILON);
    assert!(progress(f64::NAN).abs() < f32::EPSILON);
}

fn client(name: &str) -> String {
    built(PeerRecord {
        client_name: name.to_owned(),
        ..record("192.0.2.1", 1)
    })
    .client
}

#[test]
fn the_three_shapes_of_a_client_name_pass_unchanged() {
    assert_eq!(client("Name/1.2.3"), "Name/1.2.3");
    assert_eq!(client("Name 1.2.3"), "Name 1.2.3");
    assert_eq!(client("k%F5%91,?%FEX%04"), "k%F5%91,?%FEX%04");
    assert_eq!(client(""), "");
}

#[test]
fn a_ten_kilobyte_client_name_with_control_and_bidi_characters_is_cleaned_and_clipped() {
    // The right-to-left override would make the rest of the row read backwards.
    let hostile = format!(
        "\u{202E}evil\u{0}\u{7}\u{1b}[31m {}",
        "A\u{202D}\u{2066}\u{200F}\u{7}b ".repeat(2500)
    );
    assert!(hostile.len() > 10_000);

    let cleaned = client(&hostile);

    assert_eq!(cleaned.chars().count(), MAX_CLIENT_CHARS);
    assert!(cleaned.starts_with("evil[31m Ab Ab"), "{cleaned:?}");
    assert!(!cleaned.ends_with(' '));
    assert!(
        cleaned.chars().all(|c| !c.is_control()
            && !('\u{202A}'..='\u{202E}').contains(&c)
            && !('\u{2066}'..='\u{2069}').contains(&c)
            && c != '\u{200F}'),
        "{cleaned:?}"
    );
}

#[test]
fn bidi_marks_and_embeddings_are_removed_and_other_text_is_kept() {
    assert_eq!(
        client("a\u{202A}b\u{202B}c\u{202C}d\u{202D}e\u{202E}f"),
        "abcdef"
    );
    assert_eq!(client("a\u{2066}b\u{2067}c\u{2068}d\u{2069}e"), "abcde");
    assert_eq!(client("a\u{200E}b\u{200F}c\u{61C}d"), "abcd");
    assert_eq!(client("Клиент ✓ 日本語"), "Клиент ✓ 日本語");
}

#[test]
fn whitespace_is_collapsed_and_trimmed_and_other_controls_vanish() {
    assert_eq!(client("  a \t\n b\r\n\r\nc  "), "a b c");
    assert_eq!(client("a\u{a0}\u{a0}b"), "a b");
    // The next-line control is whitespace as well as a control: it separates words.
    assert_eq!(client("a\u{85}b"), "a b");
    assert_eq!(client("a\u{0}b\u{7f}c"), "abc");
    assert_eq!(client("a \u{0} b"), "a b");
    assert_eq!(client("\u{0}\t \n"), "");
}

#[test]
fn a_client_name_is_clipped_by_characters_and_never_ends_in_a_space() {
    // 64 two-byte characters fit exactly; a 65th goes.
    let wide = "é".repeat(MAX_CLIENT_CHARS + 1);
    assert_eq!(client(&wide), "é".repeat(MAX_CLIENT_CHARS));

    // A space that would be the last character is not kept.
    let before_space = format!("{} x", "a".repeat(MAX_CLIENT_CHARS - 1));
    assert_eq!(client(&before_space), "a".repeat(MAX_CLIENT_CHARS - 1));
    let after_space = format!("{} x", "a".repeat(MAX_CLIENT_CHARS - 2));
    assert_eq!(
        client(&after_space),
        format!("{} x", "a".repeat(MAX_CLIENT_CHARS - 2))
    );
    let exact = "a".repeat(MAX_CLIENT_CHARS);
    assert_eq!(client(&exact), exact);
}

#[test]
fn every_letter_the_daemon_writes_has_a_flag_and_no_other_has() {
    let letters = [
        ('T', PeerFlag::Utp),
        ('O', PeerFlag::OptimisticUnchoke),
        ('D', PeerFlag::Downloading),
        ('d', PeerFlag::WantsToDownload),
        ('U', PeerFlag::Uploading),
        ('u', PeerFlag::WantsToUpload),
        ('K', PeerFlag::UnchokedUsNotInterested),
        ('?', PeerFlag::UnchokedByUsNotInterested),
        ('E', PeerFlag::Encrypted),
        ('H', PeerFlag::FoundByDht),
        ('X', PeerFlag::FoundByPex),
        ('I', PeerFlag::Incoming),
    ];
    for (letter, flag) in letters {
        assert_eq!(peer_flag(letter), Some(flag), "{letter}");
    }
    for letter in ['Q', 'o', 't', 'e', ' ', '!', 'é', '\u{0}', '1'] {
        assert_eq!(peer_flag(letter), None, "{letter:?}");
    }
    let distinct: HashSet<PeerFlag> = letters.iter().map(|&(_, flag)| flag).collect();
    assert_eq!(distinct.len(), 12);
}

/// A lookup that knows a few addresses and records every address it is asked about.
#[derive(Default)]
struct Asked {
    known: HashMap<IpAddr, CountryCode>,
    asked: Vec<IpAddr>,
}

impl CountryLookup for Asked {
    fn country(&mut self, ip: IpAddr) -> Option<CountryCode> {
        self.asked.push(ip);
        self.known.get(&ip).copied()
    }
}

/// A lookup that answers for every address, so that any call shows as a country, and counts them.
struct Everything(usize);

impl CountryLookup for Everything {
    fn country(&mut self, _: IpAddr) -> Option<CountryCode> {
        self.0 += 1;
        CountryCode::new("US")
    }
}

fn list_of(addresses: &[&str]) -> PeerList {
    let peers = addresses
        .iter()
        .zip(1_u16..)
        .map(|(address, port)| record(address, port))
        .collect();
    build_peers(&hash(), &answer(Some(peers), None)).unwrap()
}

fn countries(list: &PeerList) -> Vec<Option<CountryCode>> {
    list.peers.iter().map(|peer| peer.country).collect()
}

#[test]
fn public_addresses_get_their_country_and_unknown_ones_none() {
    let mut list = list_of(&[
        "192.0.2.1",
        "198.51.100.2",
        "2001:db8::3",
        "::ffff:203.0.113.4",
    ]);
    let mut lookup = Asked {
        known: HashMap::from([
            (ip("192.0.2.1"), code("DE")),
            (ip("2001:db8::3"), code("NL")),
            (ip("203.0.113.4"), code("US")),
        ]),
        ..Asked::default()
    };
    let mut cache = HashMap::new();

    locate(&mut list, &mut lookup, &mut cache);

    assert_eq!(
        countries(&list),
        [Some(code("DE")), None, Some(code("NL")), Some(code("US"))]
    );
    // The mapped address is asked about as the IPv4 address it stands for.
    assert_eq!(
        lookup.asked,
        [
            ip("192.0.2.1"),
            ip("198.51.100.2"),
            ip("2001:db8::3"),
            ip("203.0.113.4")
        ]
    );
}

#[test]
fn private_loopback_link_local_and_unique_local_addresses_are_never_looked_up() {
    let local = [
        "10.1.2.3",
        "172.16.0.9",
        "192.168.1.7",
        "127.0.0.1",
        "169.254.10.20",
        "::1",
        "fc00::1",
        "fd12:3456::9",
        "fe80::1",
        "::ffff:192.168.1.7",
    ];
    let mut list = list_of(&local);
    let mut lookup = Everything(0);
    let mut cache = HashMap::new();

    locate(&mut list, &mut lookup, &mut cache);

    assert_eq!(lookup.0, 0, "no lookup for a local address");
    assert_eq!(countries(&list), vec![None; local.len()]);
    assert!(cache.is_empty(), "nothing to remember about them");
}

#[test]
fn a_peer_without_an_ip_has_no_country_and_no_lookup() {
    let mut list = list_of(&["Invalid address", "192.0.2.1"]);
    let mut lookup = Asked {
        known: HashMap::from([(ip("192.0.2.1"), code("DE"))]),
        ..Asked::default()
    };

    locate(&mut list, &mut lookup, &mut HashMap::new());

    assert_eq!(countries(&list), [None, Some(code("DE"))]);
    assert_eq!(lookup.asked, [ip("192.0.2.1")]);
}

#[test]
fn an_address_is_looked_up_once_while_it_stays_in_the_list() {
    let mut lookup = Asked {
        known: HashMap::from([(ip("192.0.2.1"), code("DE"))]),
        ..Asked::default()
    };
    let mut cache = HashMap::new();
    let mut first = list_of(&["192.0.2.1", "198.51.100.2"]);
    locate(&mut first, &mut lookup, &mut cache);
    assert_eq!(lookup.asked.len(), 2);

    // The same peers again, in another order, and with the same address on two ports.
    let mut second = list_of(&["198.51.100.2", "192.0.2.1", "192.0.2.1"]);
    locate(&mut second, &mut lookup, &mut cache);

    assert_eq!(
        lookup.asked.len(),
        2,
        "answered from the cache, also for 'no country'"
    );
    assert_eq!(
        countries(&second),
        [None, Some(code("DE")), Some(code("DE"))]
    );
}

#[test]
fn the_cache_is_pruned_to_the_addresses_in_the_list() {
    let mut lookup = Asked {
        known: HashMap::from([(ip("192.0.2.1"), code("DE")), (ip("192.0.2.2"), code("FR"))]),
        ..Asked::default()
    };
    let mut cache = HashMap::new();
    locate(
        &mut list_of(&["192.0.2.1", "192.0.2.2"]),
        &mut lookup,
        &mut cache,
    );
    assert_eq!(cache.len(), 2);

    locate(
        &mut list_of(&["192.0.2.2", "10.0.0.1"]),
        &mut lookup,
        &mut cache,
    );

    assert_eq!(
        cache,
        HashMap::from([(ip("192.0.2.2"), Some(code("FR")))]),
        "the peer that left is forgotten, a local address is never kept"
    );
    // A peer that left and came back is looked up again.
    locate(&mut list_of(&["192.0.2.1"]), &mut lookup, &mut cache);
    assert_eq!(
        lookup
            .asked
            .iter()
            .filter(|&&a| a == ip("192.0.2.1"))
            .count(),
        2
    );
}

#[test]
fn an_empty_list_empties_the_cache() {
    let mut cache = HashMap::from([(ip("192.0.2.1"), Some(code("DE")))]);

    locate(&mut list_of(&[]), &mut Asked::default(), &mut cache);

    assert!(cache.is_empty());
}
