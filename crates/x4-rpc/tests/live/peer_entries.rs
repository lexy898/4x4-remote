//! The read-only test of real peer entries: it looks for a few of the daemon's own torrents that
//! have connected peers, reads their peers as the Peers tab does, and checks that the answers
//! parse into [`TorrentRecord`] and [`PeerRecord`] with every key sent under its name and of its
//! kind, and that the counts agree. A key the daemon names otherwise would not fail the parse,
//! which takes a missing key as its default, so the test looks at the answer as sent too.
//!
//! It changes nothing: `torrent_get` only, of a few torrents at a time. What it reports are counts
//! and the JSON kinds of the keys, never an address, a client name or a torrent's name or hash;
//! its failures name a key and a kind, never a value. Peers come and go, so a daemon with none
//! at the moment passes without checking anything and says so.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::net::IpAddr;

use serde_json::Value;
use x4_rpc::{
    Client, HashString, PeerRecord, Protocol, ServerVersion, TorrentField, TorrentRecord,
    UreqTransport,
};

use crate::tracker_entries::{Kind, hash_of, read, selection_of};

/// How many of the daemon's torrents with peers the test reads, at most.
pub(crate) const WITH_PEERS: usize = 3;
/// How many torrents one look for peers asks about: the daemon runs on a Raspberry Pi.
const LOOK_BATCH: usize = 10;
/// How many torrents the test looks at in all before it settles for what it found.
const LOOK_LIMIT: usize = 50;
/// What a look for peers reads of a few torrents.
const LOOK_FIELDS: &[TorrentField] = &[
    TorrentField::Id,
    TorrentField::HashString,
    TorrentField::PeersConnected,
];
/// The letters of the daemon's `flag_str`.
const FLAG_LETTERS: &str = "TODdUuK?EHXI";

/// The keys of a `peers` entry, with the kinds each may be sent as. `progress` is a real, which
/// the daemon may write without a fraction; `peer_id` and `bytes_to_*` are not read by
/// [`PeerRecord`] but belong to the entry.
const PEER_KEYS: [(&str, &[Kind]); 19] = [
    ("address", &[Kind::Text]),
    ("port", &[Kind::Integer]),
    ("client_name", &[Kind::Text]),
    ("peer_id", &[Kind::Text]),
    ("flag_str", &[Kind::Text]),
    ("progress", &[Kind::Fraction, Kind::Integer]),
    ("rate_to_client", &[Kind::Integer]),
    ("rate_to_peer", &[Kind::Integer]),
    ("bytes_to_client", &[Kind::Integer]),
    ("bytes_to_peer", &[Kind::Integer]),
    ("is_encrypted", &[Kind::Bool]),
    ("is_utp", &[Kind::Bool]),
    ("is_incoming", &[Kind::Bool]),
    ("is_downloading_from", &[Kind::Bool]),
    ("is_uploading_to", &[Kind::Bool]),
    ("client_is_choked", &[Kind::Bool]),
    ("client_is_interested", &[Kind::Bool]),
    ("peer_is_choked", &[Kind::Bool]),
    ("peer_is_interested", &[Kind::Bool]),
];

/// The keys of [`PEER_KEYS`] that the daemon `server` does not send: `peer_id` and `bytes_to_*`
/// came with the JSON-RPC of Transmission 4.1, after every daemon of the legacy protocol.
pub(crate) fn keys_not_sent(server: &ServerVersion) -> &'static [&'static str] {
    if server.protocol == Protocol::Legacy {
        &["peer_id", "bytes_to_client", "bytes_to_peer"]
    } else {
        &[]
    }
}

/// The keys of the `peers_from` object.
const FROM_KEYS: [&str; 7] = [
    "from_cache",
    "from_dht",
    "from_incoming",
    "from_lpd",
    "from_ltep",
    "from_pex",
    "from_tracker",
];

/// What the test saw in an answer: counts and kinds, nothing else.
#[derive(Debug, Default)]
pub(crate) struct Tally {
    torrents: usize,
    /// The peer entries.
    pub(crate) peers: usize,
    ipv4: usize,
    ipv6: usize,
    /// The peers that moved data in either direction when the answer was made.
    moving: usize,
    /// The kinds each key of [`PEER_KEYS`] was sent as.
    kinds: BTreeMap<&'static str, BTreeSet<Kind>>,
}

impl fmt::Display for Tally {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} torrents, {} peers ({} IPv4, {} IPv6, {} moving data); keys:",
            self.torrents, self.peers, self.ipv4, self.ipv6, self.moving
        )?;
        for (key, _) in PEER_KEYS {
            let kinds: Vec<&str> = self
                .kinds
                .get(key)
                .into_iter()
                .flatten()
                .map(|kind| kind.name())
                .collect();
            write!(formatter, " {key} {}", kinds.join("/"))?;
        }
        Ok(())
    }
}

/// Up to [`WITH_PEERS`] of the torrents `hashes` that have connected peers, the busiest of those
/// looked at, and how many torrents were looked at to find them. The test looks at [`LOOK_BATCH`]
/// at a time, at most [`LOOK_LIMIT`] in all, and stops once it has found enough.
pub(crate) fn with_peers(
    client: &mut Client<UreqTransport>,
    hashes: &[HashString],
) -> (Vec<HashString>, usize) {
    let mut found: Vec<(u32, HashString)> = Vec::new();
    let mut looked_at = 0;
    for batch in hashes.chunks(LOOK_BATCH).take(LOOK_LIMIT / LOOK_BATCH) {
        if found.len() >= WITH_PEERS {
            break;
        }
        let answer = read(client, &selection_of(batch), LOOK_FIELDS, "look for peers");
        looked_at += batch.len();
        for record in &answer.parsed.torrents {
            let connected = record.peers_connected.unwrap_or(0);
            if let (true, Some(hash)) = (connected > 0, hash_of(record)) {
                found.push((connected, hash));
            }
        }
    }
    found.sort_by_key(|(connected, _)| std::cmp::Reverse(*connected));
    let chosen = found
        .into_iter()
        .take(WITH_PEERS)
        .map(|(_, hash)| hash)
        .collect();
    (chosen, looked_at)
}

/// Reads the peers of the torrents `hashes`, as the Peers tab does, together with their
/// `peers_connected` in the same request, and checks the answer: it names those torrents and no
/// others; each torrent has as many peers as `peers_connected` says and its `peers_from` adds up
/// to the same; every key of every entry is sent, of its kind, and parsed to the value sent;
/// every address is an IP address, every `flag_str` holds the daemon's letters and every
/// `progress` is between 0 and 1. The keys `not_sent` ([`keys_not_sent`]) must be missing.
pub(crate) fn read_and_check(
    client: &mut Client<UreqTransport>,
    hashes: &[HashString],
    what: &str,
    not_sent: &[&str],
) -> Tally {
    let fields: Vec<TorrentField> = TorrentField::PEERS
        .iter()
        .copied()
        .chain([TorrentField::PeersConnected])
        .collect();
    let answer = read(client, &selection_of(hashes), &fields, what);
    let asked: BTreeSet<&str> = hashes.iter().map(HashString::as_str).collect();
    let named: BTreeSet<&str> = answer
        .parsed
        .torrents
        .iter()
        .filter_map(|record| record.hash_string.as_deref())
        .collect();
    assert!(
        named == asked && answer.parsed.torrents.len() == hashes.len(),
        "{what}: the answer has {} torrents for the {} asked, or other ones",
        answer.parsed.torrents.len(),
        hashes.len()
    );
    let raw_records = answer
        .raw
        .get("torrents")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut tally = Tally::default();
    for (raw, record) in raw_records.iter().zip(&answer.parsed.torrents) {
        tally.torrents += 1;
        check_torrent(raw, record, what, not_sent, &mut tally);
    }
    tally
}

/// Checks the peers of one torrent: `raw` is the record as the daemon sent it; the keys
/// `not_sent` must be missing.
fn check_torrent(
    raw: &Value,
    record: &TorrentRecord,
    what: &str,
    not_sent: &[&str],
    tally: &mut Tally,
) {
    let sent = raw
        .get("peers")
        .and_then(Value::as_array)
        .map(Vec::as_slice);
    let parsed = record.peers.as_deref();
    assert!(
        sent.is_some() && parsed.is_some(),
        "{what}: a torrent came without its peers list: sent {:?}",
        Kind::of(raw.get("peers"))
    );
    let (sent, parsed) = (sent.unwrap_or_default(), parsed.unwrap_or_default());
    assert_eq!(
        sent.len(),
        parsed.len(),
        "{what}: one parsed entry per sent"
    );
    // Both come from one request, which the single-threaded daemon answers from one moment.
    let connected = usize::try_from(record.peers_connected.unwrap_or(0)).ok();
    assert_eq!(
        Some(parsed.len()),
        connected,
        "{what}: as many peers as peers_connected says"
    );
    check_origins(raw, record, connected, what);
    let mut wrong: BTreeMap<&str, BTreeSet<Kind>> = BTreeMap::new();
    for (sent, parsed) in sent.iter().zip(parsed) {
        tally.peers += 1;
        match parsed.address.parse::<IpAddr>() {
            Ok(IpAddr::V4(_)) => tally.ipv4 += 1,
            Ok(IpAddr::V6(_)) => tally.ipv6 += 1,
            Err(_) => panic_on("an address that is no IP address", what),
        }
        tally.moving += usize::from(parsed.rate_to_client > 0 || parsed.rate_to_peer > 0);
        for (key, allowed) in PEER_KEYS {
            let kind = Kind::of(sent.get(key));
            tally.kinds.entry(key).or_default().insert(kind);
            let allowed = if not_sent.contains(&key) {
                &[Kind::Missing]
            } else {
                allowed
            };
            if !allowed.contains(&kind) {
                wrong.entry(key).or_default().insert(kind);
            }
        }
        let differ = keys_parsed_otherwise(sent, parsed);
        assert!(
            differ.is_empty(),
            "{what}: peer entry keys parsed to other values than sent: {differ:?}"
        );
        assert!(
            parsed
                .flag_str
                .chars()
                .all(|letter| FLAG_LETTERS.contains(letter)),
            "{what}: a flag_str with a letter outside {FLAG_LETTERS}"
        );
        assert!(
            (0.0..=1.0).contains(&parsed.progress),
            "{what}: a progress outside 0..=1"
        );
    }
    assert!(
        wrong.is_empty(),
        "{what}: peer entry keys missing or of another kind, with the kinds sent: {wrong:?}"
    );
}

/// Checks that `peers_from` was sent with its seven integer keys and adds up to `connected`.
fn check_origins(raw: &Value, record: &TorrentRecord, connected: Option<usize>, what: &str) {
    let sent = raw.get("peers_from");
    let wrong: BTreeMap<&str, Kind> = FROM_KEYS
        .iter()
        .map(|key| (*key, Kind::of(sent.and_then(|from| from.get(*key)))))
        .filter(|(_, kind)| *kind != Kind::Integer)
        .collect();
    assert!(
        wrong.is_empty(),
        "{what}: peers_from keys missing or of another kind, with the kinds sent: {wrong:?}"
    );
    let from = record.peers_from.unwrap_or_default();
    let total = [
        from.from_cache,
        from.from_dht,
        from.from_incoming,
        from.from_lpd,
        from.from_ltep,
        from.from_pex,
        from.from_tracker,
    ]
    .into_iter()
    .map(u64::from)
    .sum::<u64>();
    assert_eq!(
        Some(total),
        connected.and_then(|count| u64::try_from(count).ok()),
        "{what}: peers_from adds up to peers_connected"
    );
}

/// The keys of the peer entry `sent` whose value `parsed` does not hold.
fn keys_parsed_otherwise(sent: &Value, parsed: &PeerRecord) -> Vec<&'static str> {
    let text = |key: &str| sent.get(key).and_then(Value::as_str);
    let flag = |key: &str| sent.get(key).and_then(Value::as_bool);
    let count = |key: &str| sent.get(key).and_then(Value::as_u64);
    [
        ("address", text("address") == Some(parsed.address.as_str())),
        ("port", count("port") == Some(u64::from(parsed.port))),
        (
            "client_name",
            text("client_name") == Some(parsed.client_name.as_str()),
        ),
        (
            "flag_str",
            text("flag_str") == Some(parsed.flag_str.as_str()),
        ),
        (
            "progress",
            sent.get("progress").and_then(Value::as_f64) == Some(parsed.progress),
        ),
        (
            "rate_to_client",
            count("rate_to_client") == Some(parsed.rate_to_client),
        ),
        (
            "rate_to_peer",
            count("rate_to_peer") == Some(parsed.rate_to_peer),
        ),
        (
            "is_encrypted",
            flag("is_encrypted") == Some(parsed.is_encrypted),
        ),
        ("is_utp", flag("is_utp") == Some(parsed.is_utp)),
        (
            "is_incoming",
            flag("is_incoming") == Some(parsed.is_incoming),
        ),
        (
            "is_downloading_from",
            flag("is_downloading_from") == Some(parsed.is_downloading_from),
        ),
        (
            "is_uploading_to",
            flag("is_uploading_to") == Some(parsed.is_uploading_to),
        ),
        (
            "client_is_choked",
            flag("client_is_choked") == Some(parsed.client_is_choked),
        ),
        (
            "client_is_interested",
            flag("client_is_interested") == Some(parsed.client_is_interested),
        ),
        (
            "peer_is_choked",
            flag("peer_is_choked") == Some(parsed.peer_is_choked),
        ),
        (
            "peer_is_interested",
            flag("peer_is_interested") == Some(parsed.peer_is_interested),
        ),
    ]
    .into_iter()
    .filter(|(_, same)| !same)
    .map(|(key, _)| key)
    .collect()
}

/// Fails the test with `problem`, which names no value.
#[expect(
    clippy::panic,
    reason = "a live test stops at the first failure, saying no more than the kind of problem"
)]
fn panic_on(problem: &str, what: &str) -> ! {
    panic!("{what}: {problem}")
}
