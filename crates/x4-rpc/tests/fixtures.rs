//! The recorded fixtures: their envelope, that every string in them is scrubbed, and that a client
//! reads them the way later tests will serve them.

use serde_json::Value;
use x4_rpc::Client;
use x4_rpc::testing::fixtures::{
    FREE_SPACE, SESSION_GET, SESSION_STATS, TORRENT_GET_DETAILS, TORRENT_GET_LIST,
    TORRENT_GET_RECENT,
};
use x4_rpc::testing::{self, FakeTransport};

const ALL: [(&str, &str); 6] = [
    ("session_get", SESSION_GET),
    ("session_stats", SESSION_STATS),
    ("free_space", FREE_SPACE),
    ("torrent_get_list", TORRENT_GET_LIST),
    ("torrent_get_recent", TORRENT_GET_RECENT),
    ("torrent_get_details", TORRENT_GET_DETAILS),
];

/// Null for text that is not JSON; `every_fixture_is_a_full_response_envelope` reports that case.
fn parse(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or(Value::Null)
}

/// `n` in `prefix` + `n` + `suffix`: all digits, no leading zero.
fn numbered(text: &str, prefix: &str, suffix: &str) -> bool {
    text.strip_prefix(prefix)
        .and_then(|rest| rest.strip_suffix(suffix))
        .is_some_and(|n| {
            !n.is_empty() && !n.starts_with('0') && n.bytes().all(|b| b.is_ascii_digit())
        })
}

fn is_fake_hash(text: &str) -> bool {
    text.len() == 40
        && text.starts_with("000000000000000000000000000000")
        && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Whether `text` is something the scrubber writes (or an allowlisted constant) under `key`.
fn is_scrubbed(key: &str, text: &str) -> bool {
    match key {
        "jsonrpc" | "version" | "rpc_version_semver" => true,
        "pieces" => text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=')),
        "hash_string" => is_fake_hash(text),
        "name" => {
            numbered(text, "Torrent ", "")
                || numbered(text, "Torrent ", ".bin")
                || text.split_once("/file ").is_some_and(|(torrent, file)| {
                    numbered(torrent, "Torrent ", "") && numbered(file, "", ".bin")
                })
        }
        "magnet_link" => text
            .strip_prefix("magnet:?xt=urn:btih:")
            .and_then(|rest| rest.split_once("&dn=Torrent+"))
            .is_some_and(|(hash, number)| is_fake_hash(hash) && numbered(number, "", "")),
        "download_dir" | "path" => text == "/data/torrents/video",
        // The metainfo's texts, and those of `tracker_stats`: URLs, host names and what the
        // trackers answered.
        "comment"
        | "creator"
        | "source"
        | "announce"
        | "scrape"
        | "host"
        | "sitename"
        | "last_announce_result"
        | "last_scrape_result" => text == "redacted",
        "error_string" => text.is_empty() || text == "redacted error",
        "labels" => text == "label",
        _ => false,
    }
}

fn unscrubbed_strings(key: &str, value: &Value, found: &mut Vec<String>) {
    match value {
        Value::String(text) if !is_scrubbed(key, text) => found.push(format!("{key}: {text}")),
        Value::Array(items) => {
            for item in items {
                unscrubbed_strings(key, item, found);
            }
        }
        Value::Object(object) => {
            for (key, item) in object {
                unscrubbed_strings(key, item, found);
            }
        }
        _ => {}
    }
}

#[test]
fn every_fixture_is_a_full_response_envelope() {
    for (name, text) in ALL {
        let value = serde_json::from_str::<Value>(text);
        assert!(value.is_ok(), "{name} is not JSON: {value:?}");
        let value = parse(text);

        assert_eq!(value["jsonrpc"], "2.0", "{name}");
        assert_eq!(value["id"], 1, "{name}");
        assert!(value["result"].is_object(), "{name} has no result object");
    }
}

#[test]
fn every_string_in_the_fixtures_is_scrubbed() {
    for (name, text) in ALL {
        let mut found = Vec::new();

        unscrubbed_strings("", &parse(text), &mut found);

        assert!(
            found.is_empty(),
            "{name} holds unscrubbed strings: {found:?}"
        );
    }
}

#[test]
fn list_fixtures_cover_several_torrents_and_the_details_cover_several_files() {
    let list = parse(TORRENT_GET_LIST);
    let torrents = list["result"]["torrents"].as_array().expect("torrents");
    assert!(torrents.len() > 1, "the list needs several torrents");
    let hashes: std::collections::HashSet<_> = torrents
        .iter()
        .map(|torrent| torrent["hash_string"].as_str())
        .collect();
    assert_eq!(hashes.len(), torrents.len(), "hashes are unique");

    let recent = parse(TORRENT_GET_RECENT);
    assert!(recent["result"]["torrents"].is_array());
    assert!(recent["result"]["removed"].is_array());

    let details = parse(TORRENT_GET_DETAILS);
    let torrent = &details["result"]["torrents"][0];
    let files = torrent["files"].as_array().expect("files");
    assert!(files.len() > 1, "the details torrent needs several files");
    assert_eq!(
        torrent["file_count"].as_u64(),
        u64::try_from(files.len()).ok()
    );
    for per_file in ["priorities", "wanted"] {
        assert_eq!(
            torrent[per_file].as_array().map(Vec::len),
            Some(files.len()),
            "{per_file}"
        );
    }
    assert!(
        torrent["tracker_stats"]
            .as_array()
            .is_some_and(|trackers| !trackers.is_empty()),
        "the details torrent needs a tracker"
    );
}

#[test]
fn a_client_reads_each_fixture_result_through_the_fake_transport() {
    for (name, text) in ALL {
        let result = parse(text)["result"].clone();
        let mut client = Client::new(FakeTransport::scripted(vec![Ok(testing::ok(
            result.clone(),
        ))]));

        let read = client.call_raw("session_get", None);

        assert_eq!(read, Ok(result), "{name}");
    }
}
