//! The peers of the made-up torrents, in the shapes `torrent_get` sends `peers` and `peers_from`,
//! and the countries of their addresses for the demo's own country database.
//!
//! A torrent is connected to as many peers as its `peers_connected` says, and who they are is
//! worked out from its id whenever a read asks for them: the same addresses, ports, clients,
//! progress and connections on every read and every run. Only what flows moves: at each read the
//! torrent's rates are shared out among as many peers as its `peers_sending_to_us` and
//! `peers_getting_from_us` say, so that the peers' rates add up to the torrent's, and the choke
//! and interest flags follow the rates. Of a torrent that downloads, every third peer is a seed,
//! which takes nothing from it; the peers of a complete torrent all download from it.
//!
//! Every address is in a range reserved for documentation: the three IPv4 ranges of RFC 5737 and
//! `2001:db8::/32` of RFC 3849. [`country_blocks`] puts each block of them in a country, in the
//! region EU or in none, so that the Peers tab shows flags, mostly white ones among them, and the
//! globe of a region and of an address without a country.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Map, Value, json};
use x4_core::CountryCode;

use super::{Fields, Kind, Lcg, Torrent};

/// The random stream of a torrent's peers.
const PEERS_STREAM: u8 = 6;
/// The first three bytes of each IPv4 range for documentation, whose hosts 1 to 254 the peers use.
const V4_RANGES: [[u8; 3]; 3] = [[192, 0, 2], [198, 51, 100], [203, 0, 113]];
/// The hosts of each IPv4 range the peers use.
const V4_HOSTS: u32 = 254;
/// How many IPv4 addresses the peers can have.
const V4_SLOTS: u32 = V4_HOSTS * 3;
/// The step between the IPv4 addresses of a torrent's peers; it shares no factor with
/// [`V4_SLOTS`] (2 × 3 × 127), so up to that many peers have addresses of their own.
const V4_STEP: u32 = 97;
/// The size of the IPv4 blocks [`country_blocks`] puts in countries, as a prefix length: eight
/// blocks of 32 addresses in each range.
const V4_BLOCK_PREFIX: u8 = 27;
/// The country of each IPv4 block, range by range; `None` is no block, an address of nowhere.
/// JP, KR and FI have mostly white flags; EU is a region, which has no country.
const V4_COUNTRIES: [Option<&str>; 24] = [
    Some("US"),
    Some("DE"),
    Some("JP"),
    Some("KR"),
    Some("LT"),
    Some("FI"),
    Some("GB"),
    Some("NL"),
    Some("BR"),
    Some("CA"),
    Some("SE"),
    Some("TR"),
    Some("PL"),
    Some("RU"),
    Some("IT"),
    Some("EU"),
    Some("AU"),
    Some("IN"),
    Some("CN"),
    Some("CH"),
    Some("ES"),
    Some("MX"),
    Some("PT"),
    None,
];
/// The country of each IPv6 block `2001:db8:N::/48`, from N = 1.
const V6_COUNTRIES: [Option<&str>; 6] = [
    Some("JP"),
    Some("DE"),
    Some("FI"),
    Some("US"),
    Some("KR"),
    None,
];
/// The prefix length of an IPv6 block.
const V6_BLOCK_PREFIX: u8 = 48;
/// How many of every 100 peers have an IPv6 address.
const V6_SHARE: u32 = 15;
/// What the peers call themselves; one says nothing.
const CLIENTS: [&str; 13] = [
    "Transmission 4.0.6",
    "Transmission 4.1.0",
    "qBittorrent 4.6.7",
    "qBittorrent 5.0.4",
    "Deluge 2.1.1",
    "libtorrent (Rasterbar) 2.0.11",
    "µTorrent 3.6.0",
    "BitTorrent 7.11.0",
    "Tixati 3.31",
    "BiglyBT 3.8.0.0",
    "aria2 1.37.0",
    "rTorrent 0.9.8",
    "",
];
/// The port most peers listen on, Transmission's default.
const USUAL_PORT: u32 = 51413;
/// Where the ports of incoming connections start: they are ephemeral.
const EPHEMERAL_PORTS: u32 = 49152;

/// How a peer was first found, the key of `peers_from` it counts in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Tracker,
    Dht,
    Pex,
    Lpd,
    Ltep,
    Cache,
    Incoming,
}

impl Source {
    /// Every source, in the order of the keys of `peers_from`.
    const ALL: [Self; 7] = [
        Self::Cache,
        Self::Dht,
        Self::Incoming,
        Self::Lpd,
        Self::Ltep,
        Self::Pex,
        Self::Tracker,
    ];

    /// The key of `peers_from`.
    fn key(self) -> &'static str {
        match self {
            Self::Cache => "from_cache",
            Self::Dht => "from_dht",
            Self::Incoming => "from_incoming",
            Self::Lpd => "from_lpd",
            Self::Ltep => "from_ltep",
            Self::Pex => "from_pex",
            Self::Tracker => "from_tracker",
        }
    }

    /// The source of an outgoing connection for a roll from 0 to 99.
    fn pick(roll: u32) -> Self {
        match roll {
            0..45 => Self::Tracker,
            45..70 => Self::Dht,
            70..90 => Self::Pex,
            90..94 => Self::Ltep,
            94..96 => Self::Lpd,
            _ => Self::Cache,
        }
    }
}

/// What stays the same about a peer from read to read.
struct Peer {
    address: IpAddr,
    port: u16,
    client: &'static str,
    /// The share of the torrent it has, in thousandths.
    progress: u32,
    encrypted: bool,
    utp: bool,
    incoming: bool,
    source: Source,
    /// What it has sent us and we have sent it since it connected, in bytes.
    sent: (u64, u64),
}

impl Peer {
    fn is_seed(&self) -> bool {
        self.progress == 1000
    }
}

/// Whether the peer at `index` of a torrent of `kind` is a seed: every third peer of a torrent
/// that lacks data, none of a complete one, whose peers come for its data.
fn is_seed(kind: Kind, index: u32) -> bool {
    kind.incomplete() && index.is_multiple_of(3)
}

/// How many of `peers` peers of a torrent of `kind` can take data from it: those that are no
/// seeds ([`is_seed`]).
pub(super) fn leechers(kind: Kind, peers: u32) -> u32 {
    if kind.incomplete() {
        peers - peers.div_ceil(3)
    } else {
        peers
    }
}

impl Torrent {
    /// Adds `peers` and `peers_from` to `record` when `fields` names them; a read of the list
    /// does not, and would otherwise make every torrent's peers.
    pub(super) fn add_peers(&self, fields: &Fields, record: &mut Map<String, Value>) {
        if !fields.asks_any(&["peers", "peers_from"]) {
            return;
        }
        let peers = self.peers();
        if fields.asks("peers") {
            let down = self.shares(&peers, self.rate_down, self.peers_sending_to_us, |_| true);
            let up = self.shares(&peers, self.rate_up, self.peers_getting_from_us, |peer| {
                !peer.is_seed()
            });
            let entries: Vec<Value> = (peers.iter().zip(down).zip(up))
                .enumerate()
                .map(|(index, ((peer, down), up))| self.peer_entry(index, peer, down, up))
                .collect();
            record.insert("peers".to_owned(), Value::Array(entries));
        }
        if fields.asks("peers_from") {
            let from: Map<String, Value> = Source::ALL
                .iter()
                .map(|&source| {
                    let count = peers.iter().filter(|peer| peer.source == source).count();
                    (source.key().to_owned(), json!(count))
                })
                .collect();
            record.insert("peers_from".to_owned(), Value::Object(from));
        }
    }

    /// The peers the torrent is connected to, in the daemon's order.
    fn peers(&self) -> Vec<Peer> {
        let mut random = Lcg::stream(self.id, PEERS_STREAM);
        let start = random.below(V4_SLOTS);
        let clients = u32::try_from(CLIENTS.len()).unwrap_or(1);
        (0..self.peers_connected)
            .map(|index| {
                // A fixed number of draws per peer, so that a peer is the same whatever the count.
                let v6 = random.below(100) < V6_SHARE;
                let v6_block = random.between(1, 7);
                let port_roll = random.below(100);
                let port_value = random.between(1024, 65536);
                let client = CLIENTS
                    .get(usize::try_from(random.below(clients)).unwrap_or(0))
                    .copied()
                    .unwrap_or_default();
                let progress = random.between(1, 1000);
                let encrypted = random.below(100) < 70;
                let utp = random.below(100) < 40;
                let incoming = random.below(100) < 25;
                let source = Source::pick(random.below(100));
                let sent = u64::from(random.below(1 << 20)) * 64;
                Peer {
                    address: if v6 {
                        IpAddr::V6(Ipv6Addr::new(
                            0x2001,
                            0x0db8,
                            u16::try_from(v6_block).unwrap_or(1),
                            0,
                            0,
                            0,
                            0,
                            u16::try_from(index + 1).unwrap_or(u16::MAX),
                        ))
                    } else {
                        v4_address((start + index * V4_STEP) % V4_SLOTS)
                    },
                    port: u16::try_from(match (incoming, port_roll) {
                        (true, _) => EPHEMERAL_PORTS + port_value % (65536 - EPHEMERAL_PORTS),
                        (false, 0..30) => USUAL_PORT,
                        (false, 30..40) => 6881,
                        (false, _) => port_value,
                    })
                    .unwrap_or(u16::MAX),
                    client,
                    progress: if is_seed(self.kind, index) {
                        1000
                    } else {
                        progress
                    },
                    encrypted,
                    utp,
                    incoming,
                    source: if incoming { Source::Incoming } else { source },
                    sent: (sent, sent / 3),
                }
            })
            .collect()
    }

    /// `rate` shared out among `takers` of `peers`, the first of them from a place that moves with
    /// the rate, among those `can_take`; the others get 0. Each taker gets at least 1 byte per
    /// second while the rate is that high, and the shares add up to the rate.
    fn shares(
        &self,
        peers: &[Peer],
        rate: u32,
        takers: u32,
        can_take: fn(&Peer) -> bool,
    ) -> Vec<u64> {
        let mut shares = vec![0; peers.len()];
        let count = u32::try_from(peers.len()).unwrap_or(u32::MAX);
        if rate == 0 || count == 0 {
            return shares;
        }
        let key = rate ^ self.id.rotate_left(16);
        let start = key % count;
        let chosen: Vec<usize> = (0..count)
            .map(|offset| usize::try_from((start + offset) % count).unwrap_or(0))
            .filter(|&index| peers.get(index).is_some_and(can_take))
            .take(usize::try_from(takers.min(rate)).unwrap_or(0))
            .collect();
        let weights: Vec<u64> = chosen.iter().map(|&index| weight(key, index)).collect();
        let total: u64 = weights.iter().sum();
        let taken = u64::try_from(chosen.len()).unwrap_or(u64::MAX);
        let rate = u64::from(rate);
        if total == 0 {
            return shares;
        }
        let mut given = 0;
        for (&index, &weight) in chosen.iter().zip(&weights) {
            let share = 1 + (rate - taken) * weight / total;
            if let Some(slot) = shares.get_mut(index) {
                *slot = share;
            }
            given += share;
        }
        if let Some(&first) = chosen.first()
            && let Some(slot) = shares.get_mut(first)
        {
            *slot += rate - given;
        }
        shares
    }

    /// The entry of `peers` of `peer`, the one at `index`, which sends us `down` and gets `up`
    /// bytes per second. Its flags follow from the rates: a peer that sends chokes us no more and
    /// is wanted, one that gets is unchoked and wants.
    fn peer_entry(&self, index: usize, peer: &Peer, down: u64, up: u64) -> Value {
        let choking_us = down == 0;
        let we_are_interested = self.kind.incomplete();
        let choked_by_us = up == 0;
        let interested_in_us = !peer.is_seed();
        let downloading_from = we_are_interested && !choking_us;
        let uploading_to = interested_in_us && !choked_by_us;
        // The letters in the daemon's order: T, O, D or d, U or u, K, ?, E, H or X, I.
        let mut flags = String::new();
        let mut flag = |set: bool, letter: char| {
            if set {
                flags.push(letter);
            }
        };
        flag(peer.utp, 'T');
        flag(downloading_from, 'D');
        flag(!downloading_from && we_are_interested, 'd');
        flag(uploading_to, 'U');
        flag(!uploading_to && interested_in_us, 'u');
        flag(!choking_us && !we_are_interested, 'K');
        flag(!choked_by_us && !interested_in_us, '?');
        flag(peer.encrypted, 'E');
        flag(peer.source == Source::Dht, 'H');
        flag(peer.source == Source::Pex, 'X');
        flag(peer.incoming, 'I');
        // A peer id of 20 bytes: a client prefix, then the torrent and the peer.
        let peer_id = format!("-SY0100-{:06}{index:06}", self.id);
        json!({
            "address": peer.address.to_string(),
            "bytes_to_client": peer.sent.0,
            "bytes_to_peer": peer.sent.1,
            "client_is_choked": choking_us,
            "client_is_interested": we_are_interested,
            "client_name": peer.client,
            "flag_str": flags,
            "is_downloading_from": downloading_from,
            "is_encrypted": peer.encrypted,
            "is_incoming": peer.incoming,
            "is_uploading_to": uploading_to,
            "is_utp": peer.utp,
            "peer_id": STANDARD.encode(peer_id),
            "peer_is_choked": choked_by_us,
            "peer_is_interested": interested_in_us,
            "port": peer.port,
            "progress": f64::from(peer.progress) / 1000.0,
            "rate_to_client": down,
            "rate_to_peer": up,
        })
    }
}

/// The IPv4 address of `slot`, from 0 to [`V4_SLOTS`] − 1: the hosts 1 to 254 of each range in
/// turn.
fn v4_address(slot: u32) -> IpAddr {
    let range = V4_RANGES
        .get(usize::try_from(slot / V4_HOSTS).unwrap_or(0))
        .copied()
        .unwrap_or(V4_RANGES[0]);
    let host = u8::try_from(slot % V4_HOSTS + 1).unwrap_or(1);
    let [a, b, c] = range;
    IpAddr::V4(Ipv4Addr::new(a, b, c, host))
}

/// A weight from 1 to 64 of the peer at `index` in a share keyed by `key`.
fn weight(key: u32, index: usize) -> u64 {
    let index = u64::try_from(index).unwrap_or(u64::MAX);
    let mut random = Lcg(((u64::from(key) << 32) | index).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    // The first numbers of generators whose seeds are alike are alike.
    random.next();
    u64::from(random.below(64)) + 1
}

/// The blocks of the documentation ranges the peers' addresses are in, each with its country (or
/// the region EU), for a country database of the demo's own. The blocks without one are left
/// out, so their addresses have no country.
pub(crate) fn country_blocks() -> Vec<(IpAddr, u8, CountryCode)> {
    let v4 = V4_COUNTRIES
        .iter()
        .enumerate()
        .filter_map(|(block, country)| {
            let block = u32::try_from(block).ok()?;
            let range = V4_RANGES.get(usize::try_from(block / 8).ok()?)?;
            let [a, b, c] = *range;
            let start = u8::try_from(block % 8 * 32).ok()?;
            let network = IpAddr::V4(Ipv4Addr::new(a, b, c, start));
            Some((network, V4_BLOCK_PREFIX, CountryCode::new((*country)?)?))
        });
    let v6 = V6_COUNTRIES
        .iter()
        .enumerate()
        .filter_map(|(block, country)| {
            let block = u16::try_from(block + 1).ok()?;
            let network = IpAddr::V6(Ipv6Addr::new(0x2001, 0x0db8, block, 0, 0, 0, 0, 0));
            Some((network, V6_BLOCK_PREFIX, CountryCode::new((*country)?)?))
        });
    v4.chain(v6).collect()
}
