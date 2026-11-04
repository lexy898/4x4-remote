//! Records scrubbed fixtures from a real Transmission daemon into `tests/fixtures/`.
//!
//! Run it with `just record-fixtures`; `X4_LIVE_URL` names the daemon's RPC endpoint. It makes
//! nine calls, a second apart because the daemon may be a small machine, and only read-only ones
//! (`session_get`, `session_stats`, `free_space`, `torrent_get`). The peers and the trackers are
//! those of the torrent with the most connected peers in the list. Every response is scrubbed by
//! [`Scrubber`] and nothing is written unless no original string survived and every peer address
//! is a documentation address.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;
use std::{env, fs, thread};

use serde_json::{Value, json};
use x4_rpc::testing::scrub::{Scrubber, stray_addresses};
use x4_rpc::{Client, DEFAULT_TIMEOUT, RpcError, SESSION_FIELDS, TorrentField, UreqTransport};

/// How long to wait after each call, so that a small daemon is asked at a modest rate.
const PAUSE: Duration = Duration::from_secs(1);

#[derive(Debug, thiserror::Error)]
enum RecordError {
    #[error("set X4_LIVE_URL to the daemon's RPC endpoint, e.g. http://host:9091/transmission/rpc")]
    MissingUrl,
    #[error("{method} failed: {source}")]
    Rpc {
        method: &'static str,
        source: RpcError,
    },
    #[error("the {method} response is not shaped as expected")]
    Shape { method: &'static str },
    #[error("the daemon has no torrents to record")]
    NoTorrents,
    #[error("no torrent has connected peers to record; try again when one has")]
    NoPeers,
    #[error("{count} peer address(es) are outside the documentation ranges; nothing was written")]
    StrayAddresses { count: usize },
    #[error("{count} sensitive string(s) survived scrubbing in {fixtures:?}; nothing was written")]
    Leaked { count: usize, fixtures: Vec<String> },
    #[error("cannot encode {name}: {source}")]
    Encode {
        name: String,
        source: serde_json::Error,
    },
    #[error("cannot write {}: {source}", path.display())]
    Write { path: PathBuf, source: io::Error },
    #[error("cannot print progress: {0}")]
    Progress(#[from] io::Error),
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // A failed write to stderr leaves nowhere else to report to.
            drop(writeln!(io::stderr(), "record_fixtures: {error}"));
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), RecordError> {
    let url = env::var("X4_LIVE_URL").map_err(|_| RecordError::MissingUrl)?;
    let mut client = Client::new(UreqTransport::new(&url, DEFAULT_TIMEOUT));

    let mut fixtures = read_fixtures(&mut client)?;
    scrub_and_check(&mut fixtures)?;
    write_fixtures(&fixtures)
}

/// Makes the nine calls and returns the responses as the fixtures to write, in the order the
/// scrubber numbers their torrents.
fn read_fixtures(
    client: &mut Client<UreqTransport>,
) -> Result<Vec<(&'static str, Value)>, RecordError> {
    let session = call(client, "session_get", json!({ "fields": SESSION_FIELDS }))?;
    let download_dir = session
        .get("download_dir")
        .and_then(Value::as_str)
        .ok_or(RecordError::Shape {
            method: "session_get",
        })?
        .to_owned();
    let session_stats = call(client, "session_stats", json!({}))?;
    let free_space = call(client, "free_space", json!({ "path": download_dir }))?;
    let list = call(
        client,
        "torrent_get",
        json!({ "fields": TorrentField::LIST }),
    )?;
    // Peers come and go: say so before the other reads if there are none to record.
    let peers_id = busiest_torrent_id(&list)?;
    let recent = call(
        client,
        "torrent_get",
        json!({ "fields": TorrentField::LIST, "ids": "recently_active" }),
    )?;
    // The details fixture should cover multi-file data, which the list fields cannot tell.
    let file_counts = call(
        client,
        "torrent_get",
        json!({ "fields": ["id", "file_count"] }),
    )?;
    let details_id = details_torrent_id(&file_counts)?;
    let details = call(
        client,
        "torrent_get",
        json!({ "fields": TorrentField::DETAILS_FULL, "ids": [details_id] }),
    )?;
    // The two reads of the Peers and Trackers tabs, for the torrent that has the most peers.
    let peers = call(
        client,
        "torrent_get",
        json!({ "fields": TorrentField::PEERS, "ids": [peers_id] }),
    )?;
    if !has_peers(&peers) {
        return Err(RecordError::NoPeers);
    }
    let trackers = call(
        client,
        "torrent_get",
        json!({ "fields": TorrentField::TRACKERS, "ids": [peers_id] }),
    )?;

    // The torrent list goes first, so torrent numbers follow the list order.
    Ok(vec![
        ("torrent_get_list", envelope(&list)),
        ("torrent_get_recent", envelope(&recent)),
        ("torrent_get_details", envelope(&details)),
        ("torrent_get_peers", envelope(&peers)),
        ("torrent_get_trackers", envelope(&trackers)),
        ("session_get", envelope(&session)),
        ("session_stats", envelope(&session_stats)),
        ("free_space", envelope(&free_space)),
    ])
}

/// Scrubs the fixtures with one scrubber and refuses to go on if a sensitive string or a peer
/// address outside the documentation ranges is left in any of them.
fn scrub_and_check(fixtures: &mut [(&'static str, Value)]) -> Result<(), RecordError> {
    let mut scrubber = Scrubber::new();
    for (_, fixture) in fixtures.iter_mut() {
        scrubber.scrub(fixture);
    }
    // Only the counts are reported: the leaked strings are the very data to keep private.
    let mut leaked_in = Vec::new();
    let mut leaked_count = 0;
    for (name, fixture) in fixtures.iter() {
        let leaked = scrubber.leaked(fixture);
        if !leaked.is_empty() {
            leaked_count += leaked.len();
            leaked_in.push((*name).to_owned());
        }
    }
    if leaked_count > 0 {
        return Err(RecordError::Leaked {
            count: leaked_count,
            fixtures: leaked_in,
        });
    }
    // Whatever the scrubber did with the addresses, none but a documentation one may be written.
    let stray: usize = fixtures
        .iter()
        .map(|(_, fixture)| stray_addresses(fixture))
        .sum();
    if stray > 0 {
        return Err(RecordError::StrayAddresses { count: stray });
    }
    Ok(())
}

/// Writes the fixtures to `tests/fixtures/`.
fn write_fixtures(fixtures: &[(&'static str, Value)]) -> Result<(), RecordError> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    fs::create_dir_all(&dir).map_err(|source| RecordError::Write {
        path: dir.clone(),
        source,
    })?;
    let mut out = io::stdout().lock();
    for (name, fixture) in fixtures {
        let mut text =
            serde_json::to_string_pretty(fixture).map_err(|source| RecordError::Encode {
                name: (*name).to_owned(),
                source,
            })?;
        text.push('\n');
        let path = dir.join(format!("{name}.json"));
        fs::write(&path, text).map_err(|source| RecordError::Write {
            path: path.clone(),
            source,
        })?;
        writeln!(out, "wrote {}", path.display())?;
    }
    Ok(())
}

/// Calls the read-only `method` and returns its `result`, then waits [`PAUSE`].
fn call(
    client: &mut Client<UreqTransport>,
    method: &'static str,
    params: Value,
) -> Result<Value, RecordError> {
    let result = client
        .call_raw(method, Some(params))
        .map_err(|source| RecordError::Rpc { method, source })?;
    thread::sleep(PAUSE);
    Ok(result)
}

/// The id of the torrent of the list with the most connected peers.
fn busiest_torrent_id(list: &Value) -> Result<u64, RecordError> {
    let torrents = list
        .get("torrents")
        .and_then(Value::as_array)
        .ok_or(RecordError::Shape {
            method: "torrent_get",
        })?;
    torrents
        .iter()
        .filter_map(|torrent| {
            let peers = torrent.get("peers_connected").and_then(Value::as_u64)?;
            let id = torrent.get("id").and_then(Value::as_u64)?;
            (peers > 0).then_some((peers, id))
        })
        .max_by_key(|(peers, _)| *peers)
        .map(|(_, id)| id)
        .ok_or(RecordError::NoPeers)
}

/// Whether the `torrent_get` result `peers` lists at least one peer.
fn has_peers(peers: &Value) -> bool {
    peers
        .get("torrents")
        .and_then(Value::as_array)
        .and_then(|torrents| torrents.first())
        .and_then(|torrent| torrent.get("peers"))
        .and_then(Value::as_array)
        .is_some_and(|peers| !peers.is_empty())
}

/// The id of the first torrent with more than one file, or of the first torrent.
fn details_torrent_id(file_counts: &Value) -> Result<u64, RecordError> {
    let torrents = file_counts
        .get("torrents")
        .and_then(Value::as_array)
        .ok_or(RecordError::Shape {
            method: "torrent_get",
        })?;
    let multi_file = torrents.iter().find(|torrent| {
        torrent
            .get("file_count")
            .and_then(Value::as_u64)
            .is_some_and(|count| count > 1)
    });
    multi_file
        .or_else(|| torrents.first())
        .ok_or(RecordError::NoTorrents)?
        .get("id")
        .and_then(Value::as_u64)
        .ok_or(RecordError::Shape {
            method: "torrent_get",
        })
}

/// The full JSON-RPC response around a `result`, as a fixture stores it.
fn envelope(result: &Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": 1, "result": result })
}
