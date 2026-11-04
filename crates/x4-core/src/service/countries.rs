//! The countries of the peers, filled in on the service thread before the peers reach the UI.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;

use super::OpenCountries;
use crate::engine::Event;
use crate::geoip::{CountryCode, GeoIp};
use crate::peers::locate;

/// The country databases and what they said about the peers shown.
pub(super) struct Countries {
    /// Opens the databases; taken by the first list of peers.
    open: Option<OpenCountries>,
    /// The databases, once a list of peers has opened them.
    geoip: Option<GeoIp>,
    /// The country of each address of the peers shown, `None` for one that has none.
    cache: HashMap<IpAddr, Option<CountryCode>>,
}

impl Countries {
    /// Databases that `open` opens when the first list of peers needs them.
    pub(super) fn new(open: OpenCountries) -> Self {
        Self {
            open: Some(open),
            geoip: None,
            cache: HashMap::new(),
        }
    }

    /// `event`, with the country of each peer filled in when it brings peers; the first peers
    /// open the databases. Cleared peers take their addresses out of the cache, which otherwise
    /// keeps only those of the last peers, so it never outgrows one torrent's peers.
    pub(super) fn locate(&mut self, event: Event) -> Event {
        match event {
            Event::Peers(mut list) => {
                let geoip = self.geoip.get_or_insert_with(|| {
                    self.open.take().map_or_else(GeoIp::none, |open| open())
                });
                // The engine keeps the list it sent, without countries, to tell the next one by.
                locate(Arc::make_mut(&mut list), geoip, &mut self.cache);
                Event::Peers(list)
            }
            Event::PeersCleared => {
                self.cache = HashMap::new();
                Event::PeersCleared
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::io::Cursor;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use x4_rpc::HashString;

    use super::*;
    use crate::geoip::{CountryDb, Edition, build_country_file};
    use crate::peers::{Peer, PeerList, PeerSources, PeerTraits};

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    fn code(letters: &str) -> CountryCode {
        CountryCode::new(letters).unwrap()
    }

    fn peer(address: &str) -> Peer {
        Peer {
            ip: Some(ip(address)),
            address: address.to_owned(),
            port: 51413,
            client: String::new(),
            progress: 1.0,
            rate_down: 0,
            rate_up: 0,
            traits: PeerTraits::default(),
            flags: String::new(),
            country: None,
        }
    }

    fn peers(addresses: &[&str]) -> Event {
        Event::Peers(Arc::new(PeerList {
            hash: HashString::parse(&"5".repeat(40)).unwrap(),
            peers: addresses.iter().map(|address| peer(address)).collect(),
            sources: PeerSources::default(),
        }))
    }

    /// Countries whose opener counts its calls and opens a database of the documentation ranges.
    fn counted() -> (Countries, Arc<AtomicUsize>) {
        let opened = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&opened);
        let countries = Countries::new(Box::new(move || {
            count.fetch_add(1, Ordering::Relaxed);
            let blocks = [
                (ip("192.0.2.0"), 24, code("GB")),
                (ip("198.51.100.0"), 24, code("US")),
            ];
            let file = build_country_file(Edition::V4, &blocks);
            let v4 = CountryDb::open(Box::new(Cursor::new(file)), Edition::V4).unwrap();
            GeoIp::new(Some(v4), None)
        }));
        (countries, opened)
    }

    fn countries_of(event: &Event) -> Vec<Option<&str>> {
        match event {
            Event::Peers(list) => list
                .peers
                .iter()
                .map(|peer| peer.country.as_ref().map(CountryCode::as_str))
                .collect(),
            other => panic!("not peers: {other:?}"),
        }
    }

    fn cached(countries: &Countries) -> HashSet<IpAddr> {
        countries.cache.keys().copied().collect()
    }

    #[test]
    fn the_first_peers_open_the_databases_and_get_their_countries() {
        let (mut countries, opened) = counted();

        let event = countries.locate(peers(&["192.0.2.1", "198.51.100.7", "203.0.113.9"]));

        assert_eq!(countries_of(&event), [Some("GB"), Some("US"), None]);
        assert_eq!(opened.load(Ordering::Relaxed), 1);
        let event = countries.locate(peers(&["198.51.100.8"]));
        assert_eq!(countries_of(&event), [Some("US")]);
        assert_eq!(opened.load(Ordering::Relaxed), 1, "opened once");
    }

    #[test]
    fn other_events_open_nothing_and_pass_unchanged() {
        let (mut countries, opened) = counted();

        assert_eq!(countries.locate(Event::PeersCleared), Event::PeersCleared);
        let refused = Event::FreeSpace {
            path: x4_rpc::DownloadDir::new("/data").unwrap(),
            answer: crate::engine::FreeSpaceAnswer::Unknown,
        };
        assert_eq!(countries.locate(refused.clone()), refused);

        assert_eq!(opened.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn the_cache_holds_only_the_addresses_of_the_peers_shown() {
        let (mut countries, _) = counted();

        countries.locate(peers(&["192.0.2.1", "198.51.100.7", "10.0.0.1"]));
        assert_eq!(
            cached(&countries),
            HashSet::from([ip("192.0.2.1"), ip("198.51.100.7")])
        );

        countries.locate(peers(&["198.51.100.7", "203.0.113.9"]));
        assert_eq!(
            cached(&countries),
            HashSet::from([ip("198.51.100.7"), ip("203.0.113.9")])
        );

        // Cleared peers leave nothing behind.
        countries.locate(Event::PeersCleared);
        assert_eq!(cached(&countries), HashSet::new());
    }

    #[test]
    fn a_missing_database_gives_no_countries() {
        let mut countries = Countries::new(Box::new(GeoIp::none));

        let event = countries.locate(peers(&["192.0.2.1"]));

        assert_eq!(countries_of(&event), [None]);
    }
}
