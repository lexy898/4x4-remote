//! The recorded fixtures: their envelope, that every string in them is scrubbed, and that a client
//! reads them the way later tests will serve them.

use std::collections::BTreeSet;
use std::net::IpAddr;

use serde_json::Value;
use x4_rpc::Client;
use x4_rpc::testing::fixtures::{
    FREE_SPACE, SESSION_GET, SESSION_STATS, TORRENT_GET_DETAILS, TORRENT_GET_LIST,
    TORRENT_GET_PEERS, TORRENT_GET_RECENT, TORRENT_GET_TRACKERS,
};
use x4_rpc::testing::{self, FakeTransport};

/// The letters of a peer's `flag_str`.
const FLAG_LETTERS: &str = "TODdUuK?EHXI";

const ALL: [(&str, &str); 8] = [
    ("session_get", SESSION_GET),
    ("session_stats", SESSION_STATS),
    ("free_space", FREE_SPACE),
    ("torrent_get_list", TORRENT_GET_LIST),
    ("torrent_get_recent", TORRENT_GET_RECENT),
    ("torrent_get_details", TORRENT_GET_DETAILS),
    ("torrent_get_peers", TORRENT_GET_PEERS),
    ("torrent_get_trackers", TORRENT_GET_TRACKERS),
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
        // The metainfo's texts.
        "comment" | "creator" | "source" | "peer_id" => text == "redacted",
        "error_string" => text.is_empty() || text == "redacted error",
        "labels" => text == "label",
        // The peers: a documentation address, a numbered client, the daemon's flag letters.
        "address" => text == "redacted" || is_documentation_address(text),
        "client_name" => text.is_empty() || numbered(text, "Client ", ""),
        "flag_str" => {
            text == "redacted" || text.chars().all(|letter| FLAG_LETTERS.contains(letter))
        }
        // The trackers: a numbered example host with the port, the daemon's own answers. The
        // hand-written `torrent_get_details` holds `redacted` for the texts.
        "host" => {
            text == "redacted"
                || text.rsplit_once(':').is_some_and(|(host, port)| {
                    numbered(host, "tracker", ".example.org") && numbered(port, "", "")
                })
        }
        "announce" => {
            text == "redacted" || numbered(text, "https://tracker", ".example.org/announce")
        }
        "scrape" => {
            text.is_empty()
                || text == "redacted"
                || numbered(text, "https://tracker", ".example.org/scrape")
        }
        "sitename" => text == "redacted" || text == "example",
        "last_announce_result" | "last_scrape_result" => matches!(
            text,
            "" | "Success"
                | "Could not connect to tracker"
                | "Tracker did not respond"
                | "redacted"
        ),
        _ => false,
    }
}

/// Whether `text` is an IP address in a range reserved for documentation (RFC 5737, RFC 3849).
fn is_documentation_address(text: &str) -> bool {
    match text.parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => {
            let [a, b, c, host] = address.octets();
            matches!([a, b, c], [192, 0, 2] | [198, 51, 100] | [203, 0, 113])
                && host != 0
                && host != 255
        }
        Ok(IpAddr::V6(address)) => address.segments()[..2] == [0x2001, 0x0db8],
        Err(_) => false,
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

/// The one torrent of a recorded `torrent_get` fixture.
fn the_torrent(text: &str) -> Value {
    let result = parse(text);
    let torrents = result
        .get("result")
        .and_then(|result| result.get("torrents"));
    let torrents = torrents
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(torrents.len(), 1, "one torrent");
    torrents.into_iter().next().unwrap_or_default()
}

#[test]
fn the_peers_fixture_has_peers_at_documentation_addresses_and_origins_that_add_up() {
    let torrent = the_torrent(TORRENT_GET_PEERS);

    let peers = torrent["peers"].as_array().expect("peers");
    assert!(!peers.is_empty(), "the peers fixture needs a peer");
    for peer in peers {
        let address = peer["address"].as_str().expect("address");
        assert!(is_documentation_address(address), "address {address}");
        let flags = peer["flag_str"].as_str().expect("flag_str");
        assert!(
            flags.chars().all(|letter| FLAG_LETTERS.contains(letter)),
            "flag_str {flags}"
        );
        // The daemon sends all 19 keys, and the fixture is its exact answer.
        assert_eq!(
            peer.as_object().map(serde_json::Map::len),
            Some(19),
            "{peer}"
        );
    }
    let addresses: BTreeSet<&str> = peers
        .iter()
        .filter_map(|peer| peer["address"].as_str())
        .collect();
    assert_eq!(addresses.len(), peers.len(), "one entry per peer");
    let from = torrent["peers_from"].as_object().expect("peers_from");
    assert_eq!(from.len(), 7);
    let total: u64 = from.values().filter_map(Value::as_u64).sum();
    assert_eq!(Some(total), u64::try_from(peers.len()).ok());
}

#[test]
fn the_trackers_fixture_has_unique_ids_and_tiers_in_order() {
    let torrent = the_torrent(TORRENT_GET_TRACKERS);

    let trackers = torrent["tracker_stats"].as_array().expect("tracker_stats");
    assert!(!trackers.is_empty(), "the trackers fixture needs a tracker");
    let ids: BTreeSet<u64> = trackers
        .iter()
        .filter_map(|tracker| tracker["id"].as_u64())
        .collect();
    assert_eq!(ids.len(), trackers.len(), "tracker ids are unique");
    let tiers: Vec<u64> = trackers
        .iter()
        .filter_map(|tracker| tracker["tier"].as_u64())
        .collect();
    assert_eq!(tiers.len(), trackers.len());
    assert!(tiers.is_sorted(), "tiers do not go back: {tiers:?}");
    for tracker in trackers {
        // The daemon sends all 28 keys, and the fixture is its exact answer.
        assert_eq!(
            tracker.as_object().map(serde_json::Map::len),
            Some(28),
            "{tracker}"
        );
    }
}

#[test]
fn the_peers_and_trackers_fixtures_are_the_answers_to_the_tab_reads() {
    let peers = the_torrent(TORRENT_GET_PEERS);
    let trackers = the_torrent(TORRENT_GET_TRACKERS);

    let keys = |torrent: &Value| -> BTreeSet<String> {
        torrent
            .as_object()
            .map(|keys| keys.keys().cloned().collect())
            .unwrap_or_default()
    };
    let expect = |names: &[&str]| -> BTreeSet<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    };
    assert_eq!(
        keys(&peers),
        expect(&["id", "hash_string", "peers", "peers_from"])
    );
    assert_eq!(
        keys(&trackers),
        expect(&["id", "hash_string", "tracker_stats"])
    );
    assert_eq!(peers["hash_string"], trackers["hash_string"]);
}
