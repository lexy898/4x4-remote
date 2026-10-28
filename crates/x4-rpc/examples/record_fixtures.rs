//! Records scrubbed fixtures from a real Transmission daemon into `tests/fixtures/`.
//!
//! Run it with `just record-fixtures`; `X4_LIVE_URL` names the daemon's RPC endpoint. It calls
//! read-only methods only (`session_get`, `session_stats`, `free_space`, `torrent_get`). Every
//! response is scrubbed by [`Scrubber`] and nothing is written unless no original string survived.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::{env, fs};

use serde_json::{Value, json};
use x4_rpc::testing::scrub::Scrubber;
use x4_rpc::{Client, DEFAULT_TIMEOUT, RpcError, UreqTransport};

/// Fields of the app's torrent list request.
const LIST_FIELDS: [&str; 26] = [
    "id",
    "hash_string",
    "name",
    "status",
    "error",
    "error_string",
    "percent_done",
    "metadata_percent_complete",
    "rate_download",
    "rate_upload",
    "eta",
    "upload_ratio",
    "total_size",
    "size_when_done",
    "left_until_done",
    "peers_connected",
    "peers_getting_from_us",
    "peers_sending_to_us",
    "download_dir",
    "added_date",
    "done_date",
    "queue_position",
    "sequential_download",
    "recheck_progress",
    "is_finished",
    "labels",
];

/// Fields of the app's torrent details request.
const DETAILS_FIELDS: [&str; 15] = [
    "id",
    "hash_string",
    "comment",
    "creator",
    "date_created",
    "magnet_link",
    "is_private",
    "piece_count",
    "piece_size",
    "pieces",
    "files",
    "file_stats",
    "downloaded_ever",
    "uploaded_ever",
    "total_size",
];

/// Fields of the app's session request.
const SESSION_FIELDS: [&str; 11] = [
    "version",
    "rpc_version_semver",
    "download_dir",
    "alt_speed_enabled",
    "alt_speed_down",
    "alt_speed_up",
    "speed_limit_down",
    "speed_limit_down_enabled",
    "speed_limit_up",
    "speed_limit_up_enabled",
    "sequential_download",
];

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

    let session = call(
        &mut client,
        "session_get",
        json!({ "fields": SESSION_FIELDS }),
    )?;
    let download_dir = session
        .get("download_dir")
        .and_then(Value::as_str)
        .ok_or(RecordError::Shape {
            method: "session_get",
        })?
        .to_owned();
    let session_stats = call(&mut client, "session_stats", json!({}))?;
    let free_space = call(&mut client, "free_space", json!({ "path": download_dir }))?;
    let list = call(&mut client, "torrent_get", json!({ "fields": LIST_FIELDS }))?;
    let recent = call(
        &mut client,
        "torrent_get",
        json!({ "fields": LIST_FIELDS, "ids": "recently_active" }),
    )?;
    // The details fixture should cover multi-file data, which the list fields cannot tell.
    let file_counts = call(
        &mut client,
        "torrent_get",
        json!({ "fields": ["id", "file_count"] }),
    )?;
    let details_id = details_torrent_id(&file_counts)?;
    let details = call(
        &mut client,
        "torrent_get",
        json!({ "fields": DETAILS_FIELDS, "ids": [details_id] }),
    )?;

    // The torrent list goes first, so torrent numbers follow the list order.
    let mut fixtures = vec![
        ("torrent_get_list", envelope(&list)),
        ("torrent_get_recent", envelope(&recent)),
        ("torrent_get_details", envelope(&details)),
        ("session_get", envelope(&session)),
        ("session_stats", envelope(&session_stats)),
        ("free_space", envelope(&free_space)),
    ];
    let mut scrubber = Scrubber::new();
    for (_, fixture) in &mut fixtures {
        scrubber.scrub(fixture);
    }
    // Only the counts are reported: the leaked strings are the very data to keep private.
    let mut leaked_in = Vec::new();
    let mut leaked_count = 0;
    for (name, fixture) in &fixtures {
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

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    fs::create_dir_all(&dir).map_err(|source| RecordError::Write {
        path: dir.clone(),
        source,
    })?;
    let mut out = io::stdout().lock();
    for (name, fixture) in &fixtures {
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

/// Calls the read-only `method` and returns its `result`.
fn call(
    client: &mut Client<UreqTransport>,
    method: &'static str,
    params: Value,
) -> Result<Value, RecordError> {
    client
        .call_raw(method, Some(params))
        .map_err(|source| RecordError::Rpc { method, source })
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
