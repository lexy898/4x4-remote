//! The peers connected to a torrent: what the Peers tab is built from.
//!
//! [`build_peers`] turns the `peers` and `peers_from` of a `torrent_get` answer into a
//! [`PeerList`]. The wire's names for the choke and interest flags point two opposite ways, so
//! they are renamed here for what they say about the peer ([`PeerTraits`]). What a peer chose for
//! itself is cleaned: its client name loses control and bidirectional-text characters and is
//! clipped, and an address that is not an IP keeps its row without an [`Peer::ip`]. [`locate`]
//! then fills in the countries, asking a [`CountryLookup`] about public addresses only, and
//! [`peer_flag`] names the letters of a peer's flag string.
//!
//! How the list is ordered and summarized is decided in [`crate::view`].

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;

use x4_rpc::{HashString, PeerRecord, PeersFromRecord, TorrentGetResponse};

use crate::details::has_hash;
use crate::geoip::{CountryCode, CountryLookup};

/// The peers connected to one torrent when the daemon was asked, and where they came from.
#[derive(Debug, Clone, PartialEq)]
pub struct PeerList {
    /// The torrent the peers belong to. A view that shows another torrent drops the list.
    pub hash: HashString,
    /// The connected peers in the order the daemon gave them, which is not stable between reads.
    pub peers: Vec<Peer>,
    /// How many of the peers were first found in each way.
    pub sources: PeerSources,
}

/// One connected peer.
#[derive(Debug, Clone, PartialEq)]
pub struct Peer {
    /// The peer's IP address, with an IPv4-mapped IPv6 address unwrapped to the IPv4 one it
    /// stands for; `None` when the daemon's text is no IP address (it sends `Invalid address`).
    pub ip: Option<IpAddr>,
    /// The address as the daemon wrote it: no port and no brackets.
    pub address: String,
    /// The peer's port; an ephemeral one for an incoming connection.
    pub port: u16,
    /// What the peer calls itself, cleaned: no control or bidirectional-text characters, runs of
    /// whitespace collapsed, at most [`MAX_CLIENT_CHARS`] characters. Empty when it did not say.
    pub client: String,
    /// The share of the torrent the peer has, 0.0 to 1.0.
    pub progress: f32,
    /// Bytes per second from the peer to us, averaged over the last two seconds.
    pub rate_down: u64,
    /// Bytes per second from us to the peer, averaged over the last two seconds.
    pub rate_up: u64,
    /// How the connection is made and who chokes and wants what.
    pub traits: PeerTraits,
    /// The letters of the daemon's flag string, as sent; see [`peer_flag`].
    pub flags: String,
    /// The peer's country; `None` until [`locate`] finds it, and after that for an address that
    /// has none.
    pub country: Option<CountryCode>,
}

/// What the daemon says about the connection to a peer and the state of the exchange with it.
///
/// `choking_us` and the like are named for what they tell about the peer, not as the wire does
/// (`client_is_choked` is easy to read the wrong way round). `choking_us` and `interested_in_us`
/// say that the peer chokes us and wants our data; they do not say that bytes flow: the rates do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the daemon's flags for one peer, each one an independent fact"
)]
pub struct PeerTraits {
    /// The connection is encrypted.
    pub encrypted: bool,
    /// The connection runs over uTP, not TCP.
    pub utp: bool,
    /// The peer connected to us.
    pub incoming: bool,
    /// The peer chokes us: it sends us nothing for now.
    pub choking_us: bool,
    /// We choke the peer: we send it nothing for now.
    pub choked_by_us: bool,
    /// The peer wants pieces we have.
    pub interested_in_us: bool,
    /// We want pieces the peer has.
    pub we_are_interested: bool,
}

/// How many of the connected peers were first found in each way. A peer is counted once, by the
/// way it was found first, so the counts add up to the number of peers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PeerSources {
    /// Named by a tracker.
    pub tracker: u32,
    /// Found through the DHT.
    pub dht: u32,
    /// Named by another peer (peer exchange).
    pub pex: u32,
    /// Found on the local network.
    pub lpd: u32,
    /// Connected to us on their own.
    pub incoming: u32,
    /// Named in a peer's extension handshake.
    pub ltep: u32,
    /// Remembered in the resume file of an earlier run (the daemon's `from_cache`).
    pub resume: u32,
}

/// How many characters of a peer's client name are kept.
pub const MAX_CLIENT_CHARS: usize = 64;

/// What a letter of a peer's flag string says. The daemon sets them in this order: `T`, `O`, `D`
/// or `d`, `U` or `u`, `K`, `?`, `E`, `H`, `X`, `I`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PeerFlag {
    /// `T`: connected over uTP.
    Utp,
    /// `O`: the swarm's optimistic unchoke.
    OptimisticUnchoke,
    /// `D`: we are interested and the peer has unchoked us.
    Downloading,
    /// `d`: we would download if the peer let us.
    WantsToDownload,
    /// `U`: the peer is interested and we have unchoked it.
    Uploading,
    /// `u`: we would upload if the peer asked.
    WantsToUpload,
    /// `K`: the peer has unchoked us, but we are not interested.
    UnchokedUsNotInterested,
    /// `?`: we unchoked the peer, but it is not interested.
    UnchokedByUsNotInterested,
    /// `E`: the connection is encrypted.
    Encrypted,
    /// `H`: first found through the DHT.
    FoundByDht,
    /// `X`: first found through peer exchange.
    FoundByPex,
    /// `I`: an incoming connection.
    Incoming,
}

/// The flag a letter of a flag string stands for; `None` for a letter the daemon does not use.
#[must_use]
pub fn peer_flag(letter: char) -> Option<PeerFlag> {
    Some(match letter {
        'T' => PeerFlag::Utp,
        'O' => PeerFlag::OptimisticUnchoke,
        'D' => PeerFlag::Downloading,
        'd' => PeerFlag::WantsToDownload,
        'U' => PeerFlag::Uploading,
        'u' => PeerFlag::WantsToUpload,
        'K' => PeerFlag::UnchokedUsNotInterested,
        '?' => PeerFlag::UnchokedByUsNotInterested,
        'E' => PeerFlag::Encrypted,
        'H' => PeerFlag::FoundByDht,
        'X' => PeerFlag::FoundByPex,
        'I' => PeerFlag::Incoming,
        _ => return None,
    })
}

/// The peers of the torrent `hash` in a `torrent_get` answer read with the Peers fields, or
/// `None` when the answer holds no such torrent (it was removed meanwhile).
///
/// A torrent whose record lacks `peers` has no peers in the list, and one without `peers_from`
/// has no sources. The countries are empty: [`locate`] fills them.
#[must_use]
pub fn build_peers(hash: &HashString, response: &TorrentGetResponse) -> Option<PeerList> {
    let record = response
        .torrents
        .iter()
        .find(|record| has_hash(record, hash))?;
    Some(PeerList {
        hash: hash.clone(),
        peers: record
            .peers
            .iter()
            .flatten()
            .map(Peer::from_record)
            .collect(),
        sources: record.peers_from.map(PeerSources::from).unwrap_or_default(),
    })
}

impl Peer {
    /// The peer of one entry of the daemon's `peers`.
    fn from_record(record: &PeerRecord) -> Self {
        Self {
            // A mapped address (`::ffff:192.0.2.1`) is the IPv4 peer it names; the database and
            // the sort know it as such.
            ip: record
                .address
                .parse::<IpAddr>()
                .ok()
                .map(|ip| ip.to_canonical()),
            address: record.address.clone(),
            port: record.port,
            client: clean_client(&record.client_name),
            progress: unit_interval(record.progress),
            rate_down: record.rate_to_client,
            rate_up: record.rate_to_peer,
            traits: PeerTraits {
                encrypted: record.is_encrypted,
                utp: record.is_utp,
                incoming: record.is_incoming,
                choking_us: record.client_is_choked,
                choked_by_us: record.peer_is_choked,
                interested_in_us: record.peer_is_interested,
                we_are_interested: record.client_is_interested,
            },
            flags: record.flag_str.clone(),
            country: None,
        }
    }
}

impl From<PeersFromRecord> for PeerSources {
    fn from(record: PeersFromRecord) -> Self {
        Self {
            tracker: record.from_tracker,
            dht: record.from_dht,
            pex: record.from_pex,
            lpd: record.from_lpd,
            incoming: record.from_incoming,
            ltep: record.from_ltep,
            resume: record.from_cache,
        }
    }
}

/// `progress` held to 0.0 to 1.0; a value that is no number counts as 0.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the value is within 0 to 1, where an f32 loses only digits no bar can show"
)]
fn unit_interval(progress: f64) -> f32 {
    if progress.is_nan() {
        0.0
    } else {
        progress.clamp(0.0, 1.0) as f32
    }
}

/// Whether `ch` changes the direction in which text runs, which would make the rest of a row
/// read backwards or hide what follows.
fn is_bidi_control(ch: char) -> bool {
    matches!(
        ch,
        '\u{61C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// What a peer calls itself, made fit to print on one line: control and bidirectional-text
/// characters are dropped, a run of whitespace (a tab and a line break included) becomes one
/// space, the ends are trimmed and no more than [`MAX_CLIENT_CHARS`] characters are kept.
///
/// It stops reading at the limit, because a peer can send as much as it likes.
fn clean_client(raw: &str) -> String {
    let mut clean = String::new();
    let mut chars = 0;
    let mut space_pending = false;
    for ch in raw.chars() {
        if ch.is_whitespace() {
            space_pending = chars > 0;
        } else if !ch.is_control() && !is_bidi_control(ch) {
            if space_pending {
                // The space is kept only when the character after it fits too.
                if chars + 1 >= MAX_CLIENT_CHARS {
                    break;
                }
                clean.push(' ');
                chars += 1;
                space_pending = false;
            }
            if chars >= MAX_CLIENT_CHARS {
                break;
            }
            clean.push(ch);
            chars += 1;
        }
    }
    clean
}

/// Whether `ip` is on a network of its own, where no country can be told: a private, loopback,
/// link-local or unique-local address.
fn is_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_private() || ip.is_loopback() || ip.is_link_local(),
        IpAddr::V6(ip) => ip.is_loopback() || ip.is_unique_local() || ip.is_unicast_link_local(),
    }
}

/// Fills the [`Peer::country`] of each peer of `list` from `lookup`.
///
/// Addresses of a private, loopback, link-local or unique-local network are never looked up:
/// they have no country. `cache` keeps the answers between polls, so a peer that stays
/// connected is looked up once (also when the answer was "no country"), and it is pruned to the
/// addresses `list` holds, so it cannot grow beyond one list.
#[expect(
    clippy::implicit_hasher,
    reason = "the cache is the service's own map; a hasher parameter would only add noise"
)]
pub fn locate(
    list: &mut PeerList,
    lookup: &mut dyn CountryLookup,
    cache: &mut HashMap<IpAddr, Option<CountryCode>>,
) {
    let mut present = HashSet::new();
    for peer in &mut list.peers {
        peer.country = match peer.ip {
            Some(ip) if !is_local(ip) => {
                present.insert(ip);
                *cache.entry(ip).or_insert_with(|| lookup.country(ip))
            }
            _ => None,
        };
    }
    cache.retain(|ip, _| present.contains(ip));
}

#[cfg(test)]
mod tests;
