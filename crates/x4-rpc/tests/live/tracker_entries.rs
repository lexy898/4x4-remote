//! The read-only test of real tracker entries: it reads the trackers of a few of the daemon's own
//! torrents, with the fields the details panel reads, and checks that the answers parse into
//! [`TorrentRecord`] and [`TrackerStatRecord`] with every key they read sent under its name. A key
//! the daemon names otherwise would not fail the parse, which takes a missing key as its default,
//! so the test looks at the answer as sent too.
//!
//! It changes nothing: `torrent_get` only, of a few torrents at a time, and `files` only of the
//! torrents it chose. What it reports are counts and the JSON kinds of the keys, never a torrent's
//! name or hash, nor a tracker's host, address or text; its failures name a key and a kind, never
//! a value.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde_json::{Map, Value};
use x4_rpc::{
    Client, GetIds, HashString, RpcError, TorrentField, TorrentGetResponse, TorrentRecord,
    TorrentSelection, TrackerStatRecord, UreqTransport,
};

/// How many of the daemon's torrents with trackers the test reads the details of, at most.
pub(crate) const WITH_TRACKERS: usize = 3;
/// How many torrents one look for trackers asks about: the daemon runs on a Raspberry Pi.
const LOOK_BATCH: usize = 10;
/// How many torrents the test looks at in all before it settles for what it found.
const LOOK_LIMIT: usize = 50;
/// The most files a chosen torrent may have, so that its full read stays as small as most.
const MAX_FILES: u32 = 500;
/// What the list reads of every torrent.
const LIST_FIELDS: &[TorrentField] = &[TorrentField::Id, TorrentField::HashString];
/// What a look for trackers reads of a few torrents.
const LOOK_FIELDS: &[TorrentField] = &[
    TorrentField::Id,
    TorrentField::HashString,
    TorrentField::FileCount,
    TorrentField::TrackerStats,
];

/// The JSON kind of a value, which the test reports instead of the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Kind {
    Missing,
    Null,
    Bool,
    Integer,
    Fraction,
    Text,
    List,
    Object,
}

impl Kind {
    /// The kind of `value`; [`Kind::Missing`] when there is none.
    fn of(value: Option<&Value>) -> Self {
        match value {
            None => Self::Missing,
            Some(Value::Null) => Self::Null,
            Some(Value::Bool(_)) => Self::Bool,
            Some(Value::Number(number)) if number.is_i64() || number.is_u64() => Self::Integer,
            Some(Value::Number(_)) => Self::Fraction,
            Some(Value::String(_)) => Self::Text,
            Some(Value::Array(_)) => Self::List,
            Some(Value::Object(_)) => Self::Object,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Null => "null",
            Self::Bool => "bool",
            Self::Integer => "integer",
            Self::Fraction => "fraction",
            Self::Text => "text",
            Self::List => "list",
            Self::Object => "object",
        }
    }
}

/// The keys of a `tracker_stats` entry that [`TrackerStatRecord`] reads, with the kind each must
/// be sent as.
const TRACKER_KEYS: [(&str, Kind); 11] = [
    ("host", Kind::Text),
    ("has_announced", Kind::Bool),
    ("last_announce_succeeded", Kind::Bool),
    ("last_announce_timed_out", Kind::Bool),
    ("last_announce_result", Kind::Text),
    ("has_scraped", Kind::Bool),
    ("last_scrape_succeeded", Kind::Bool),
    ("last_scrape_timed_out", Kind::Bool),
    ("last_scrape_result", Kind::Text),
    ("seeder_count", Kind::Integer),
    ("leecher_count", Kind::Integer),
];

/// The daemon's answer to a `torrent_get`: as sent, and as the client parses it.
pub(crate) struct Answer {
    raw: Value,
    parsed: TorrentGetResponse,
}

/// What the test saw in an answer: counts and kinds, nothing else.
#[derive(Debug, Default)]
pub(crate) struct Tally {
    torrents: usize,
    files: usize,
    /// The tracker entries.
    pub(crate) entries: usize,
    /// The entries of trackers announced to since the torrent started, and of those scraped.
    announced: usize,
    scraped: usize,
    /// The kinds each key of [`TRACKER_KEYS`] was sent as.
    kinds: BTreeMap<&'static str, BTreeSet<Kind>>,
}

impl fmt::Display for Tally {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} torrents, {} files, {} tracker entries ({} announced to, {} scraped); keys:",
            self.torrents, self.files, self.entries, self.announced, self.scraped
        )?;
        for (key, _) in TRACKER_KEYS {
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

/// The hashes of the daemon's torrents, from a read of nothing but their ids and hashes.
pub(crate) fn listed_hashes(client: &mut Client<UreqTransport>) -> Vec<HashString> {
    read(client, &GetIds::All, LIST_FIELDS, "list")
        .parsed
        .torrents
        .iter()
        .filter_map(hash_of)
        .collect()
}

/// Up to [`WITH_TRACKERS`] of the torrents `hashes` that have trackers and at most
/// [`MAX_FILES`] files, and how many torrents were looked at to find them. The test looks at
/// [`LOOK_BATCH`] at a time, at most [`LOOK_LIMIT`] in all, and stops once it has found enough
/// with a tracker that was announced to, so that real answers are read; those come first.
pub(crate) fn with_trackers(
    client: &mut Client<UreqTransport>,
    hashes: &[HashString],
) -> (Vec<HashString>, usize) {
    let mut announced = Vec::new();
    let mut others = Vec::new();
    let mut looked_at = 0;
    for batch in hashes.chunks(LOOK_BATCH).take(LOOK_LIMIT / LOOK_BATCH) {
        if announced.len() >= WITH_TRACKERS {
            break;
        }
        let answer = read(client, &selection_of(batch), LOOK_FIELDS, "look");
        looked_at += batch.len();
        for record in &answer.parsed.torrents {
            let (Some(hash), Some(entries)) = (hash_of(record), record.tracker_stats.as_deref())
            else {
                continue;
            };
            if entries.is_empty() || record.file_count.is_none_or(|count| count > MAX_FILES) {
                continue;
            }
            if entries.iter().any(|entry| entry.has_announced) {
                announced.push(hash);
            } else {
                others.push(hash);
            }
        }
    }
    let chosen = announced
        .into_iter()
        .chain(others)
        .take(WITH_TRACKERS)
        .collect();
    (chosen, looked_at)
}

/// Reads `fields` of the torrents `hashes`, as `what` in failures, and checks the answer: it
/// names those torrents and no others, each torrent's per-file arrays have one value per file,
/// and every tracker entry has the keys [`TrackerStatRecord`] reads, of their kinds, parsed to
/// the values sent, with a host.
pub(crate) fn read_and_check(
    client: &mut Client<UreqTransport>,
    hashes: &[HashString],
    fields: &[TorrentField],
    what: &str,
) -> Tally {
    let answer = read(client, &selection_of(hashes), fields, what);
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
        check_files(record, fields, what, &mut tally);
        check_trackers(raw, record, what, &mut tally);
    }
    tally
}

/// Checks that each per-file array of `record` that `fields` asks for has one value per file.
fn check_files(record: &TorrentRecord, fields: &[TorrentField], what: &str, tally: &mut Tally) {
    let files = record
        .file_count
        .and_then(|count| usize::try_from(count).ok());
    tally.files += files.unwrap_or(0);
    let arrays = [
        (TorrentField::Files, record.files.as_ref().map(Vec::len)),
        (
            TorrentField::BytesCompleted,
            record.bytes_completed.as_ref().map(Vec::len),
        ),
        (
            TorrentField::Priorities,
            record.priorities.as_ref().map(Vec::len),
        ),
        (TorrentField::Wanted, record.wanted.as_ref().map(Vec::len)),
    ];
    for (field, length) in arrays {
        if fields.contains(&field) {
            assert_eq!(length, files, "{what}: {field:?} has one value per file");
        }
    }
}

/// Checks each tracker entry of `record` against `raw`, the record as the daemon sent it.
fn check_trackers(raw: &Value, record: &TorrentRecord, what: &str, tally: &mut Tally) {
    let sent = raw
        .get("tracker_stats")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let parsed = record.tracker_stats.as_deref().unwrap_or_default();
    assert!(
        !parsed.is_empty(),
        "{what}: a torrent chosen for its trackers came without them"
    );
    assert_eq!(
        sent.len(),
        parsed.len(),
        "{what}: one parsed entry per sent"
    );
    let mut wrong: BTreeMap<&str, BTreeSet<Kind>> = BTreeMap::new();
    for (sent, parsed) in sent.iter().zip(parsed) {
        tally.entries += 1;
        tally.announced += usize::from(parsed.has_announced);
        tally.scraped += usize::from(parsed.has_scraped);
        for (key, expected) in TRACKER_KEYS {
            let kind = Kind::of(sent.get(key));
            tally.kinds.entry(key).or_default().insert(kind);
            if kind != expected {
                wrong.entry(key).or_default().insert(kind);
            }
        }
        let differ = keys_parsed_otherwise(sent, parsed);
        assert!(
            differ.is_empty(),
            "{what}: tracker entry keys parsed to other values than sent: {differ:?}"
        );
        assert!(
            !parsed.host.is_empty(),
            "{what}: a tracker entry without a host"
        );
    }
    assert!(
        wrong.is_empty(),
        "{what}: tracker entry keys missing or of another kind, with the kinds sent: {wrong:?}"
    );
}

/// The keys of the tracker entry `sent` whose value `parsed` does not hold.
fn keys_parsed_otherwise(sent: &Value, parsed: &TrackerStatRecord) -> Vec<&'static str> {
    let text = |key: &str| sent.get(key).and_then(Value::as_str);
    let flag = |key: &str| sent.get(key).and_then(Value::as_bool);
    let count = |key: &str| sent.get(key).and_then(Value::as_i64);
    [
        ("host", text("host") == Some(parsed.host.as_str())),
        (
            "has_announced",
            flag("has_announced") == Some(parsed.has_announced),
        ),
        (
            "last_announce_succeeded",
            flag("last_announce_succeeded") == Some(parsed.last_announce_succeeded),
        ),
        (
            "last_announce_timed_out",
            flag("last_announce_timed_out") == Some(parsed.last_announce_timed_out),
        ),
        (
            "last_announce_result",
            text("last_announce_result") == Some(parsed.last_announce_result.as_str()),
        ),
        (
            "has_scraped",
            flag("has_scraped") == Some(parsed.has_scraped),
        ),
        (
            "last_scrape_succeeded",
            flag("last_scrape_succeeded") == Some(parsed.last_scrape_succeeded),
        ),
        (
            "last_scrape_timed_out",
            flag("last_scrape_timed_out") == Some(parsed.last_scrape_timed_out),
        ),
        (
            "last_scrape_result",
            text("last_scrape_result") == Some(parsed.last_scrape_result.as_str()),
        ),
        ("seeder_count", count("seeder_count") == parsed.seeder_count),
        (
            "leecher_count",
            count("leecher_count") == parsed.leecher_count,
        ),
    ]
    .into_iter()
    .filter(|(_, same)| !same)
    .map(|(key, _)| key)
    .collect()
}

/// Calls `torrent_get` for `ids` and `fields`, with the parameters the client's own
/// `torrent_get` sends, and parses the result as it does (`serde_json::from_value` of the
/// result); `what` names the read in failures. An answer that does not parse fails with the keys
/// that do not and the kinds of value sent for them, never the values.
#[expect(
    clippy::panic,
    reason = "a live test stops at the first failure, saying no more than kinds"
)]
fn read(
    client: &mut Client<UreqTransport>,
    ids: &GetIds,
    fields: &[TorrentField],
    what: &str,
) -> Answer {
    let mut params = match serde_json::to_value(ids) {
        Ok(Value::Object(params)) => params,
        Ok(_) => panic!("{what}: the ids do not encode as an object"),
        Err(error) => panic!("{what}: the ids do not encode: {error}"),
    };
    match serde_json::to_value(fields) {
        Ok(fields) => params.insert("fields".to_owned(), fields),
        Err(error) => panic!("{what}: the fields do not encode: {error}"),
    };
    let raw = match client.call_raw("torrent_get", Some(Value::Object(params))) {
        Ok(raw) => raw,
        // Its text may quote the answer.
        Err(RpcError::InvalidResponse(_)) => panic!("{what}: the answer is not JSON-RPC"),
        Err(error) => panic!("{what}: the read failed: {error}"),
    };
    // The error's text may quote a value; the keys and kinds say where it went wrong.
    let Ok(parsed) = serde_json::from_value::<TorrentGetResponse>(raw.clone()) else {
        panic!(
            "{what}: the answer does not parse; the keys that do not, with the kinds sent: {:?}",
            failing_keys(&raw)
        );
    };
    Answer { raw, parsed }
}

/// The keys of the records in the `torrent_get` result `raw` that do not parse on their own, with
/// the kinds of value sent for each; a key of a tracker entry is named `tracker_stats.<key>`.
fn failing_keys(raw: &Value) -> BTreeMap<String, BTreeSet<Kind>> {
    let mut failing: BTreeMap<String, BTreeSet<Kind>> = BTreeMap::new();
    let Some(records) = raw.get("torrents").and_then(Value::as_array) else {
        failing
            .entry("torrents".to_owned())
            .or_default()
            .insert(Kind::of(raw.get("torrents")));
        return failing;
    };
    for record in records.iter().filter_map(Value::as_object) {
        for (key, value) in record {
            if !parses_alone::<TorrentRecord>(key, value) {
                failing
                    .entry(key.clone())
                    .or_default()
                    .insert(Kind::of(Some(value)));
            }
        }
        let entries = record.get("tracker_stats").and_then(Value::as_array);
        for entry in entries.into_iter().flatten().filter_map(Value::as_object) {
            for (key, value) in entry {
                if !parses_alone::<TrackerStatRecord>(key, value) {
                    failing
                        .entry(format!("tracker_stats.{key}"))
                        .or_default()
                        .insert(Kind::of(Some(value)));
                }
            }
        }
    }
    failing
}

/// Whether an object of nothing but `key` and `value` parses as `T`.
fn parses_alone<T: serde::de::DeserializeOwned>(key: &str, value: &Value) -> bool {
    let mut object = Map::new();
    object.insert(key.to_owned(), value.clone());
    serde_json::from_value::<T>(Value::Object(object)).is_ok()
}

/// The hash of `record`, when it has a valid one.
fn hash_of(record: &TorrentRecord) -> Option<HashString> {
    record
        .hash_string
        .as_deref()
        .and_then(|hash| HashString::parse(hash).ok())
}

/// The torrents `hashes`, which are never none, as the ids of a read.
#[expect(
    clippy::panic,
    reason = "a live test stops at the first failure; callers pass at least one hash"
)]
fn selection_of(hashes: &[HashString]) -> GetIds {
    match TorrentSelection::from_hashes(hashes.iter().cloned()) {
        Ok(selection) => GetIds::Selection(selection),
        Err(error) => panic!("no torrents to read: {error}"),
    }
}
