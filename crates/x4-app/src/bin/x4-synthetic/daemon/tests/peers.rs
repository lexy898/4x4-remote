//! The peers of the synthetic torrents through the real client: who each running torrent is
//! connected to, how its rates are shared out among them, where they were found, and their
//! countries in the demo's own country database.

use std::collections::{BTreeSet, HashMap};
use std::net::IpAddr;

use x4_core::{CountryCode, CountryLookup as _, DetailsTab, PeerList};
use x4_rpc::testing::fixtures::TORRENT_GET_PEERS;
use x4_rpc::{PeerRecord, PeersFromRecord, Status};

use super::support::*;
use super::*;

/// Every torrent with the fields of the list and of the Peers tab, read at once, so that the
/// peers' rates and the torrent's are of the same moment.
fn with_peers(client: &mut Client<SyntheticDaemon>) -> Vec<TorrentRecord> {
    let fields: Vec<TorrentField> = (TorrentField::LIST.iter())
        .chain(TorrentField::PEERS)
        .copied()
        .collect();
    client
        .torrent_get(&GetIds::All, &fields)
        .expect("the client parses the answer")
        .torrents
}

fn peers_of(record: &TorrentRecord) -> &[PeerRecord] {
    record.peers.as_deref().expect("the peers")
}

/// Whether `ip` is in a range reserved for documentation: 192.0.2.0/24, 198.51.100.0/24 and
/// 203.0.113.0/24 (RFC 5737), and `2001:db8::/32` (RFC 3849).
fn is_documentation(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => matches!(
            v4.octets(),
            [192, 0, 2, _] | [198, 51, 100, _] | [203, 0, 113, _]
        ),
        IpAddr::V6(v6) => v6.segments()[..2] == [0x2001, 0x0db8],
    }
}

/// The peers of a record by address and port, with what stays the same from read to read.
fn identities(record: &TorrentRecord) -> BTreeSet<(String, u16, String, bool, bool, bool)> {
    peers_of(record)
        .iter()
        .map(|peer| {
            (
                peer.address.clone(),
                peer.port,
                peer.client_name.clone(),
                peer.is_encrypted,
                peer.is_utp,
                peer.is_incoming,
            )
        })
        .collect()
}

fn sum(from: PeersFromRecord) -> u32 {
    from.from_cache
        + from.from_dht
        + from.from_incoming
        + from.from_lpd
        + from.from_ltep
        + from.from_pex
        + from.from_tracker
}

#[test]
fn running_torrents_have_peers_at_documentation_addresses_whose_rates_add_up() {
    let mut client = client(Activity::Ticking);
    let records = with_peers(&mut client);

    let mut connected = 0;
    for record in &records {
        let peers = peers_of(record);
        assert_eq!(
            Some(u32::try_from(peers.len()).expect("fits")),
            record.peers_connected,
            "as many peers as the list says"
        );
        if peers.is_empty() {
            continue;
        }
        connected += 1;
        assert_ne!(
            record.status,
            Some(Status::Stopped),
            "a stopped torrent has peers"
        );
        let keys: BTreeSet<(&str, u16)> = peers
            .iter()
            .map(|peer| (peer.address.as_str(), peer.port))
            .collect();
        assert_eq!(keys.len(), peers.len(), "each address and port once");
        for peer in peers {
            let ip: IpAddr = peer.address.parse().expect("an IP address");
            assert!(is_documentation(ip), "{ip}");
            assert!(peer.port > 0);
            assert!((0.0..=1.0).contains(&peer.progress), "{}", peer.progress);
            // The flags follow from the rates: a peer sends to us only unchoked and wanted.
            assert_eq!(
                peer.rate_to_client > 0,
                peer.is_downloading_from,
                "{peer:?}"
            );
            assert_eq!(peer.rate_to_peer > 0, peer.is_uploading_to, "{peer:?}");
            if peer.rate_to_peer > 0 {
                assert!(peer.progress < 1.0, "a seed takes nothing from us");
            }
        }
        let down: u64 = peers.iter().map(|peer| peer.rate_to_client).sum();
        let up: u64 = peers.iter().map(|peer| peer.rate_to_peer).sum();
        assert_eq!(
            (Some(down), Some(up)),
            (record.rate_download, record.rate_upload)
        );
        let count = |rate: fn(&PeerRecord) -> u64| {
            u32::try_from(peers.iter().filter(|&peer| rate(peer) > 0).count()).expect("fits")
        };
        assert_eq!(
            Some(count(|peer| peer.rate_to_client)),
            record.peers_sending_to_us
        );
        assert_eq!(
            Some(count(|peer| peer.rate_to_peer)),
            record.peers_getting_from_us
        );
    }
    assert!(connected > 500, "{connected} torrents with peers");

    // Peers of every kind of client, a peer that does not say, IPv4 and IPv6, and seeds.
    let all: Vec<&PeerRecord> = records.iter().flat_map(peers_of).collect();
    let clients: BTreeSet<&str> = all.iter().map(|peer| peer.client_name.as_str()).collect();
    assert!(clients.len() > 8 && clients.contains(""), "{clients:?}");
    assert!(all.iter().any(|peer| peer.address.contains(':')));
    assert!(
        all.iter()
            .any(|peer| (peer.progress - 1.0).abs() < f64::EPSILON)
    );
    for has in [
        |peer: &&PeerRecord| peer.is_encrypted,
        |peer: &&PeerRecord| !peer.is_encrypted,
        |peer: &&PeerRecord| peer.is_utp,
        |peer: &&PeerRecord| peer.is_incoming,
    ] {
        assert!(all.iter().any(has));
    }
}

#[test]
fn the_peers_stay_while_their_rates_change() {
    let mut client = client(Activity::Ticking);
    let before = with_peers(&mut client);
    list(&mut client, &GetIds::RecentlyActive);
    let after = with_peers(&mut client);

    let mut moved = 0;
    for (then, now) in before.iter().zip(&after) {
        assert_eq!(then.hash_string, now.hash_string);
        assert_eq!(identities(then), identities(now));
        let rates = |record: &TorrentRecord| -> HashMap<(String, u16), (u64, u64)> {
            peers_of(record)
                .iter()
                .map(|peer| {
                    let key = (peer.address.clone(), peer.port);
                    (key, (peer.rate_to_client, peer.rate_to_peer))
                })
                .collect()
        };
        if rates(then) != rates(now) {
            moved += 1;
        }
    }
    assert!(moved > 200, "{moved} torrents whose peers' rates moved");
}

#[test]
fn one_torrent_is_busy_a_seeding_one_has_no_peers_and_stopped_ones_none() {
    let records = with_peers(&mut client(Activity::Ticking));

    let busy: Vec<&TorrentRecord> = records
        .iter()
        .filter(|record| peers_of(record).len() > 100)
        .collect();
    let [busy] = busy[..] else {
        panic!("one busy torrent, not {}", busy.len());
    };
    assert_eq!(busy.status, Some(Status::Downloading));
    assert_eq!(peers_of(busy).len(), 150);
    assert!(
        records.iter().any(|record| {
            record.status == Some(Status::Seeding) && peers_of(record).is_empty()
        }),
        "a seeding torrent nobody is connected to"
    );
    for record in &records {
        if record.status == Some(Status::Stopped) {
            assert_eq!(peers_of(record), [], "{:?}", record.name);
            assert_eq!(record.peers_from.map(sum), Some(0));
        }
    }
}

#[test]
fn every_peer_was_found_in_one_way() {
    let records = with_peers(&mut client(Activity::Static));
    let mut sources = PeersFromRecord::default();

    for record in &records {
        let from = record.peers_from.expect("where the peers came from");
        assert_eq!(
            usize::try_from(sum(from)).expect("fits"),
            peers_of(record).len()
        );
        let incoming = peers_of(record)
            .iter()
            .filter(|peer| peer.is_incoming)
            .count();
        assert_eq!(usize::try_from(from.from_incoming).expect("fits"), incoming);
        sources.from_tracker += from.from_tracker;
        sources.from_dht += from.from_dht;
        sources.from_pex += from.from_pex;
        sources.from_incoming += from.from_incoming;
    }
    assert!(
        sources.from_tracker > 0
            && sources.from_dht > 0
            && sources.from_pex > 0
            && sources.from_incoming > 0,
        "{sources:?}"
    );
}

#[test]
fn the_peers_have_the_wire_shape_of_transmission() {
    let daemon = SyntheticDaemon::new(COUNT, Activity::Static);
    let hash = pick(&mut client(Activity::Static), |status, _| {
        status == Status::Downloading
    });
    let answer = raw_get(
        &daemon,
        &hash,
        &["id", "hash_string", "peers", "peers_from"],
    );
    let recorded: Value = serde_json::from_str(TORRENT_GET_PEERS).expect("the fixture");

    let keys = |answer: &Value, pointer: &str| -> BTreeSet<String> {
        answer
            .pointer(pointer)
            .and_then(Value::as_object)
            .unwrap_or_else(|| panic!("{pointer} in {answer}"))
            .keys()
            .cloned()
            .collect()
    };
    assert_eq!(
        keys(&answer, "/result/torrents/0"),
        keys(&recorded, "/result/torrents/0")
    );
    assert_eq!(
        keys(&answer, "/result/torrents/0/peers/0"),
        keys(&recorded, "/result/torrents/0/peers/0")
    );
    assert_eq!(
        keys(&answer, "/result/torrents/0/peers_from"),
        keys(&recorded, "/result/torrents/0/peers_from")
    );
}

#[test]
fn the_peers_are_sent_only_when_asked_for() {
    let daemon = SyntheticDaemon::new(COUNT, Activity::Static);
    let hash = pick(&mut client(Activity::Static), |status, _| {
        status == Status::Downloading
    });

    let answer = raw_get(&daemon, &hash, &["id", "name"]);
    let torrent = answer.pointer("/result/torrents/0").expect("a torrent");
    assert!(torrent.get("peers").is_none() && torrent.get("peers_from").is_none());
}

#[test]
fn the_demo_puts_its_peers_in_countries_a_region_and_none() {
    let records = with_peers(&mut client(Activity::Static));
    let mut geoip = crate::countries();

    let mut found: BTreeSet<Option<CountryCode>> = BTreeSet::new();
    for peer in records.iter().flat_map(peers_of) {
        let ip: IpAddr = peer.address.parse().expect("an IP address");
        found.insert(geoip.country(ip));
    }
    let code = |letters| CountryCode::new(letters).expect("a code");
    // Mostly white flags among them, for their edge; IPv6 addresses have countries too.
    for country in ["JP", "KR", "FI", "US", "DE"] {
        assert!(found.contains(&Some(code(country))), "{country}: {found:?}");
    }
    assert!(found.len() > 15, "{found:?}");
    // The region EU and the addresses no block holds have no country.
    assert!(found.contains(&None));
    assert_eq!(
        geoip.country("198.51.100.230".parse().expect("an address")),
        None,
        "EU"
    );
    assert_eq!(
        geoip.country("203.0.113.230".parse().expect("an address")),
        None,
        "nowhere"
    );
    assert_eq!(
        geoip.country("2001:db8:1::7".parse().expect("an address")),
        Some(code("JP"))
    );
}

/// An engine over the daemon, as the demo runs it, reads the peers of the selected torrent while
/// the Peers tab is shown, with their countries.
#[test]
fn the_service_reads_the_peers_of_the_selected_torrent_with_their_countries() {
    // The same activity as the service's: a ticking daemon draws other torrents than a static one
    // after the first that ticks.
    let records = with_peers(&mut client(Activity::Ticking));
    let busy = records
        .iter()
        .find(|record| peers_of(record).len() > 100)
        .expect("the busy torrent");
    let hash = HashString::parse(busy.hash_string.as_deref().expect("a hash")).expect("valid");
    let config = EngineConfig {
        details_settle: Duration::from_millis(10),
        peers_interval: Duration::from_millis(50),
        visible_interval: Duration::from_millis(50),
        ..EngineConfig::default()
    };
    let service = service::spawn(
        client(Activity::Ticking),
        config,
        Box::new(crate::countries),
    )
    .expect("the service starts");
    let (events, received) = crossbeam_channel::unbounded();
    let source = service.events().clone();
    let forward = thread::spawn(move || {
        while let Ok(event) = source.recv_blocking() {
            if events.send(event).is_err() {
                break;
            }
        }
    });
    let commands = service.commands();
    commands
        .send(Command::ShowTab(DetailsTab::Peers))
        .expect("the service listens");
    commands
        .send(Command::Select(Some(hash.clone())))
        .expect("the service listens");

    let list: std::sync::Arc<PeerList> = wait_for(&received, |event| match event {
        Event::Peers(list) => Some(list.clone()),
        _ => None,
    });
    service.shutdown();
    forward.join().expect("the forwarding thread ends");

    assert_eq!(list.hash, hash);
    assert_eq!(list.peers.len(), 150);
    let located = list
        .peers
        .iter()
        .filter(|peer| peer.country.is_some())
        .count();
    assert!(
        located > 100 && located < 150,
        "most peers have a country, some none: {located}"
    );
}
