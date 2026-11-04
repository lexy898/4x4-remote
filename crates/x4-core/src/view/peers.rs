//! What the Peers tab shows: the peers in the order the user chose, a summary of where they came
//! from, and why the list is empty.

use std::cmp::{Ordering, Reverse};

use super::percent::percent_f32;
use crate::geoip::CountryCode;
use crate::peers::{Peer, PeerList, PeerTraits};
use crate::state::DisplayState;

/// A column of the Peers tab that a click on its header sorts by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PeerColumn {
    /// The country; peers without one come after those with one.
    Country,
    /// The IP address, by its bytes, IPv4 before IPv6; a text that is no IP address comes last.
    Address,
    /// The client name, ignoring case; peers without one come last.
    Client,
    /// How much of the torrent the peer has.
    Progress,
    /// The rate at which the peer sends to us.
    Down,
    /// The rate at which we send to the peer.
    Up,
}

/// How the peers are ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PeerSort {
    /// The column to sort by, or `None` for the default order: the busiest peers first.
    pub column: Option<PeerColumn>,
    /// Whether the greatest value of the column comes first. The default order has one direction
    /// and ignores this.
    pub descending: bool,
}

impl PeerSort {
    /// Handles a click on the header of `column`: the column the peers are sorted by already
    /// flips its direction. Any other column starts largest first if it is
    /// [`PeerColumn::Progress`], [`PeerColumn::Down`] or [`PeerColumn::Up`], and smallest first
    /// (A to Z) otherwise, as the torrent list does.
    pub fn toggle(&mut self, column: PeerColumn) {
        if self.column == Some(column) {
            self.descending = !self.descending;
        } else {
            self.column = Some(column);
            self.descending = match column {
                PeerColumn::Progress | PeerColumn::Down | PeerColumn::Up => true,
                PeerColumn::Country | PeerColumn::Address | PeerColumn::Client => false,
            };
        }
    }
}

/// One peer as the Peers tab draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct PeerRow {
    /// What identifies the row between reads: the address text and the port. The order of the
    /// daemon's list is not stable and neither is the peer id.
    pub key: (String, u16),
    /// The peer's country, if known.
    pub country: Option<CountryCode>,
    /// The address as the daemon wrote it.
    pub address: String,
    /// The client's name, cleaned; empty when the peer did not say.
    pub client: String,
    /// How much of the torrent the peer has, 0.0 to 1.0, for the bar.
    pub progress: f32,
    /// The same in whole percent, rounded down: 100 only for a peer that has every piece.
    pub percent: u8,
    /// Bytes per second from the peer to us.
    pub rate_down: u64,
    /// Bytes per second from us to the peer.
    pub rate_up: u64,
    /// How the connection is made and who chokes and wants what.
    pub traits: PeerTraits,
    /// Whether the peer sends us data now (its rate is above zero).
    pub transferring_down: bool,
    /// Whether we send the peer data now (its rate is above zero).
    pub transferring_up: bool,
}

/// The rows of the Peers tab, in the order `sort` asks for.
///
/// The default order puts the busiest peers first: by the rate at which they send to us, then
/// the rate at which we send to them (both in whole KiB/s, so that a rate wobbling by under a
/// KiB/s does not shuffle the rows between reads), then the more complete peers (when everything
/// is idle the seeds come first), then by address and port. A chosen column orders by its own
/// value, and what it cannot tell apart falls back to that default order, in either direction.
#[must_use]
pub fn peer_rows(list: &PeerList, sort: PeerSort) -> Vec<PeerRow> {
    let mut peers: Vec<&Peer> = list.peers.iter().collect();
    peers.sort_by(|a, b| compare(sort, a, b));
    peers.into_iter().map(PeerRow::from).collect()
}

impl From<&Peer> for PeerRow {
    fn from(peer: &Peer) -> Self {
        Self {
            key: (peer.address.clone(), peer.port),
            country: peer.country,
            address: peer.address.clone(),
            client: peer.client.clone(),
            progress: peer.progress,
            percent: percent_f32(peer.progress),
            rate_down: peer.rate_down,
            rate_up: peer.rate_up,
            traits: peer.traits,
            transferring_down: peer.rate_down > 0,
            transferring_up: peer.rate_up > 0,
        }
    }
}

/// How `sort` orders two peers. The chosen column reverses with the direction; the default order
/// breaks its ties, in the same way for both directions, so the rows do not jump about between
/// reads.
fn compare(sort: PeerSort, a: &Peer, b: &Peer) -> Ordering {
    let Some(column) = sort.column else {
        return busiest_first(a, b);
    };
    let by_column = by_column(column, a, b);
    let by_column = if sort.descending {
        by_column.reverse()
    } else {
        by_column
    };
    by_column.then_with(|| busiest_first(a, b))
}

/// The default order: the peers that send us the most first, then those we send the most to,
/// then the most complete ones, then by address and port.
///
/// The rates count in whole KiB/s: a rate averaged over two seconds wobbles by less than that
/// from read to read, and the rows should not trade places for it.
fn busiest_first(a: &Peer, b: &Peer) -> Ordering {
    let kib = |rate: u64| rate / 1024;
    kib(b.rate_down)
        .cmp(&kib(a.rate_down))
        .then_with(|| kib(b.rate_up).cmp(&kib(a.rate_up)))
        .then_with(|| b.progress.total_cmp(&a.progress))
        .then_with(|| by_address(a, b))
        .then_with(|| a.port.cmp(&b.port))
}

/// The order of one column, smallest first.
fn by_column(column: PeerColumn, a: &Peer, b: &Peer) -> Ordering {
    match column {
        PeerColumn::Country => some_first(a.country, b.country),
        PeerColumn::Address => by_address(a, b),
        PeerColumn::Client => by_client(&a.client, &b.client),
        PeerColumn::Progress => a.progress.total_cmp(&b.progress),
        PeerColumn::Down => a.rate_down.cmp(&b.rate_down),
        PeerColumn::Up => a.rate_up.cmp(&b.rate_up),
    }
}

/// By the bytes of the IP, so that `192.0.2.9` comes before `192.0.2.10` and IPv4 before IPv6;
/// text that is no IP comes after the addresses, in the order of the text.
fn by_address(a: &Peer, b: &Peer) -> Ordering {
    some_first(a.ip, b.ip).then_with(|| a.address.cmp(&b.address))
}

/// By the name ignoring case, without allocating; a peer with no name comes after the named ones.
fn by_client(a: &str, b: &str) -> Ordering {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => a
            .chars()
            .flat_map(char::to_lowercase)
            .cmp(b.chars().flat_map(char::to_lowercase)),
    }
}

/// Values before the lack of one.
fn some_first<T: Ord>(a: Option<T>, b: Option<T>) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => a.cmp(&b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// A way in which a peer was first found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PeerSource {
    /// Named by a tracker.
    Tracker,
    /// Found through the DHT.
    Dht,
    /// Named by another peer (peer exchange).
    Pex,
    /// Found on the local network.
    Lpd,
    /// Connected to us on their own.
    Incoming,
    /// Named in a peer's extension handshake.
    Ltep,
    /// Remembered in the resume file of an earlier run.
    Resume,
}

/// The line above the peers: how many there are, where they came from, who transfers.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PeersSummary {
    /// How many peers are connected.
    pub connected: u32,
    /// The ways peers were found, with how many each found: the greatest count first, those with
    /// none left out, ties in the order of [`PeerSource`].
    pub sources: Vec<(PeerSource, u32)>,
    /// How many peers send us data now (their rate is above zero).
    pub sending_to_us: u32,
    /// How many peers we send data to now (their rate is above zero).
    pub receiving_from_us: u32,
}

/// The summary of `list`.
#[must_use]
pub fn peers_summary(list: &PeerList) -> PeersSummary {
    let from = list.sources;
    // In the order of the enum, so that the stable sort by count leaves ties in that order.
    let mut sources: Vec<(PeerSource, u32)> = [
        (PeerSource::Tracker, from.tracker),
        (PeerSource::Dht, from.dht),
        (PeerSource::Pex, from.pex),
        (PeerSource::Lpd, from.lpd),
        (PeerSource::Incoming, from.incoming),
        (PeerSource::Ltep, from.ltep),
        (PeerSource::Resume, from.resume),
    ]
    .into_iter()
    .filter(|&(_, count)| count > 0)
    .collect();
    sources.sort_by_key(|&(_, count)| Reverse(count));

    // Whether data flows is told by the rates: the daemon's `D` and `U` flags only say that the
    // other side is unchoked and interested.
    let count = |flowing: fn(&Peer) -> bool| {
        let peers = list.peers.iter().filter(|peer| flowing(peer)).count();
        u32::try_from(peers).unwrap_or(u32::MAX)
    };
    PeersSummary {
        connected: u32::try_from(list.peers.len()).unwrap_or(u32::MAX),
        sources,
        sending_to_us: count(|peer| peer.rate_down > 0),
        receiving_from_us: count(|peer| peer.rate_up > 0),
    }
}

/// Why the Peers tab has no peers to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeersEmpty {
    /// The first answer has not come: nothing is known yet.
    Reading,
    /// The torrent is stopped, so it connects to no one.
    Paused,
    /// The torrent runs, but no peer is connected.
    NoneConnected,
}

/// Why the Peers tab shows no peers for a torrent in `state`, whose peers were last read as
/// `list`; `None` when there are peers to show.
#[must_use]
pub fn peers_empty(state: DisplayState, list: Option<&PeerList>) -> Option<PeersEmpty> {
    // Before the first answer nothing is known, so not even a stopped torrent is said to have no
    // peers yet: the same care as for the table's `EmptyTable::Waiting`.
    let Some(list) = list else {
        return Some(PeersEmpty::Reading);
    };
    if !list.peers.is_empty() {
        None
    } else if state == DisplayState::Paused {
        Some(PeersEmpty::Paused)
    } else {
        Some(PeersEmpty::NoneConnected)
    }
}

#[cfg(test)]
mod tests;
