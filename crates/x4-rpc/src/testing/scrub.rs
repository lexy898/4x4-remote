//! Scrubbing of recorded Transmission responses, so fixtures hold structure and numbers but no
//! names, paths, hashes, trackers or magnet links of a real daemon.
//!
//! The rules are an allowlist: a string survives only under the keys in [`KEEP_KEYS`]; every
//! other string is replaced, so a field nobody thought about is scrubbed too.

use std::collections::{BTreeSet, HashMap};

use serde_json::{Map, Value};

/// Keys whose string values are kept verbatim: protocol constants and the piece bitfield.
const KEEP_KEYS: [&str; 4] = ["jsonrpc", "version", "rpc_version_semver", "pieces"];

const FAKE_DIR: &str = "/data/torrents/video";
const REDACTED: &str = "redacted";
const REDACTED_ERROR: &str = "redacted error";
const LABEL: &str = "label";
const BTIH_PREFIX: &str = "urn:btih:";

/// An original is searched for inside longer strings only from this length on: shorter ones
/// would match fake values by chance (a one-letter label is a substring of "label").
const MIN_EMBEDDED_LEN: usize = 6;

/// Replaces sensitive strings in Transmission responses and checks that none survived.
///
/// One scrubber is shared by all fixtures of a recording, so the same torrent gets the same
/// number, name and hash in every fixture. Torrents are numbered from 1 in order of first
/// appearance.
#[derive(Debug, Default)]
pub struct Scrubber {
    /// Real hash (lowercase) to torrent number; the fake hash is derived from the number.
    hashes: HashMap<String, usize>,
    /// Real name to torrent number, for torrents that come without a hash.
    names: HashMap<String, usize>,
    torrent_count: usize,
    /// Every sensitive string that was replaced.
    originals: BTreeSet<String>,
}

impl Scrubber {
    /// A scrubber that has seen nothing yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Scrubs `value` in place.
    pub fn scrub(&mut self, value: &mut Value) {
        self.walk(None, value);
    }

    /// The original sensitive strings that are still present in `scrubbed`, as a whole value or,
    /// for longer ones, inside another string; empty when the scrubbing is complete.
    #[must_use]
    pub fn leaked(&self, scrubbed: &Value) -> Vec<String> {
        let mut present = Vec::new();
        collect_strings(scrubbed, &mut present);
        self.originals
            .iter()
            .filter(|original| {
                present.iter().any(|text| {
                    text == *original
                        || (original.len() >= MIN_EMBEDDED_LEN && text.contains(original.as_str()))
                })
            })
            .cloned()
            .collect()
    }

    /// Scrubs the strings below `value`; `key` is the object key it sits under (array items
    /// inherit the key of their array).
    fn walk(&mut self, key: Option<&str>, value: &mut Value) {
        match value {
            Value::String(text) => self.scrub_string(key, text),
            Value::Array(items) => {
                for item in items {
                    match item {
                        Value::Object(object) if key == Some("torrents") => {
                            self.scrub_torrent(object);
                        }
                        _ => self.walk(key, item),
                    }
                }
            }
            Value::Object(object) if object.get("hash_string").is_some_and(Value::is_string) => {
                self.scrub_torrent(object);
            }
            Value::Object(object) => {
                for (key, item) in object {
                    self.walk(Some(key), item);
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }

    fn scrub_string(&mut self, key: Option<&str>, text: &mut String) {
        let replacement = match key {
            Some(key) if KEEP_KEYS.contains(&key) => return,
            Some("download_dir" | "path") => FAKE_DIR,
            // An empty error string means "no error", which a fixture has to keep saying.
            Some("error_string") if text.is_empty() => return,
            Some("error_string") => REDACTED_ERROR,
            Some("labels") => LABEL,
            _ => REDACTED,
        };
        self.replace(text, replacement.to_owned());
    }

    /// Scrubs one torrent object: its identity strings follow its number, the rest is scrubbed
    /// by key like everything else.
    fn scrub_torrent(&mut self, torrent: &mut Map<String, Value>) {
        let name = string_field(torrent, "name");
        let hash = string_field(torrent, "hash_string")
            .or_else(|| string_field(torrent, "magnet_link").and_then(|magnet| btih(&magnet)));
        let number = self.torrent_number(hash.as_deref(), name.as_deref());
        let file_count = torrent
            .get("files")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        for (key, value) in torrent {
            match (key.as_str(), value) {
                ("hash_string", Value::String(text)) => self.replace(text, fake_hash(number)),
                ("name", Value::String(text)) => self.replace(text, format!("Torrent {number}")),
                ("magnet_link", Value::String(text)) => {
                    let magnet = if btih(text).is_some() {
                        format!(
                            "magnet:?xt={BTIH_PREFIX}{}&dn=Torrent+{number}",
                            fake_hash(number)
                        )
                    } else {
                        REDACTED.to_owned()
                    };
                    self.replace(text, magnet);
                }
                ("files", Value::Array(files)) => {
                    for (index, file) in files.iter_mut().enumerate() {
                        self.scrub_file(file, number, index + 1, file_count);
                    }
                }
                (key, value) => self.walk(Some(key), value),
            }
        }
    }

    /// `position` is the 1-based index of the file in its torrent.
    fn scrub_file(&mut self, file: &mut Value, number: usize, position: usize, file_count: usize) {
        let Value::Object(file) = file else {
            self.walk(Some("files"), file);
            return;
        };
        for (key, value) in file {
            match (key.as_str(), value) {
                ("name", Value::String(text)) => {
                    let name = if file_count == 1 {
                        format!("Torrent {number}.bin")
                    } else {
                        format!("Torrent {number}/file {position}.bin")
                    };
                    self.replace(text, name);
                }
                (key, value) => self.walk(Some(key), value),
            }
        }
    }

    /// The number of the torrent with this hash (or, without a hash, this name); a torrent seen
    /// for the first time gets the next one. A new hash never reuses the number of a known name,
    /// because two torrents may share a name but must not share a hash.
    fn torrent_number(&mut self, hash: Option<&str>, name: Option<&str>) -> usize {
        let hash = hash.map(str::to_ascii_lowercase);
        let known = match &hash {
            Some(hash) => self.hashes.get(hash),
            None => name.and_then(|name| self.names.get(name)),
        };
        let number = known.copied().unwrap_or_else(|| {
            self.torrent_count += 1;
            self.torrent_count
        });
        if let Some(hash) = hash {
            self.originals.insert(hash.clone());
            self.hashes.insert(hash, number);
        }
        if let Some(name) = name {
            self.originals.insert(name.to_owned());
            self.names.insert(name.to_owned(), number);
        }
        number
    }

    /// Overwrites `text` and remembers the original, unless there was nothing to hide.
    fn replace(&mut self, text: &mut String, replacement: String) {
        if !text.is_empty() && *text != replacement {
            self.originals.insert(std::mem::take(text));
        }
        *text = replacement;
    }
}

/// The 40-hex-digit fake hash of torrent `number`: `000…0001`, `000…0002`, and so on.
fn fake_hash(number: usize) -> String {
    format!("{number:040x}")
}

/// The info hash of a magnet link, as written in it.
fn btih(magnet: &str) -> Option<String> {
    let start = magnet.find(BTIH_PREFIX)? + BTIH_PREFIX.len();
    let rest = magnet.get(start..)?;
    let hash = rest.split('&').next().unwrap_or(rest);
    (!hash.is_empty()).then(|| hash.to_owned())
}

fn string_field(object: &Map<String, Value>, key: &str) -> Option<String> {
    object.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// Every string below `value`: values and object keys.
fn collect_strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => out.push(text.clone()),
        Value::Array(items) => {
            for item in items {
                collect_strings(item, out);
            }
        }
        Value::Object(object) => {
            for (key, item) in object {
                out.push(key.clone());
                collect_strings(item, out);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const HASH: &str = "0123456789abcdef0123456789abcdef01234567";
    const OTHER_HASH: &str = "fedcba9876543210fedcba9876543210fedcba98";
    const FAKE_HASH_1: &str = "0000000000000000000000000000000000000001";
    const FAKE_HASH_2: &str = "0000000000000000000000000000000000000002";

    /// Every string a real response would carry that must not reach a fixture.
    const SENSITIVE: [&str; 13] = [
        HASH,
        "Real.Show.S01.1080p",
        "Real.Show.S01.1080p/e01.mkv",
        "Real.Show.S01.1080p/sub/e02.mkv",
        "/mnt/real/disk/video",
        "https://tracker.example/announce",
        "great release, see https://real.example",
        "RealCreator 1.0",
        "tv-archive",
        "private-label",
        "Tracker gave an error: real.example",
        "mystery value",
        "/mnt/real/disk/video/Real.Show.S01.1080p",
    ];

    fn multi_file_torrent() -> Value {
        json!({
            "id": 7,
            "hash_string": HASH,
            "name": "Real.Show.S01.1080p",
            "magnet_link": format!(
                "magnet:?xt=urn:btih:{HASH}&dn=Real.Show.S01.1080p&tr=https%3A%2F%2Ftracker.example%2Fannounce"
            ),
            "comment": "great release, see https://real.example",
            "creator": "RealCreator 1.0",
            "labels": ["tv-archive", "private-label"],
            "download_dir": "/mnt/real/disk/video",
            "error_string": "Tracker gave an error: real.example",
            "unknown_key": "mystery value",
            "trackers": [{ "announce": "https://tracker.example/announce", "tier": 0 }],
            "files": [
                { "name": "Real.Show.S01.1080p/e01.mkv", "length": 100, "bytes_completed": 40 },
                { "name": "Real.Show.S01.1080p/sub/e02.mkv", "length": 200, "bytes_completed": 0 }
            ],
            "file_stats": [
                { "bytes_completed": 40, "wanted": true, "priority": 0 },
                { "bytes_completed": 0, "wanted": false, "priority": -1 }
            ],
            "total_size": 300,
            "rate_upload": 1234,
            "percent_done": 0.5,
            "is_private": true,
            "pieces": "AAEC/w==",
        })
    }

    fn single_file_torrent() -> Value {
        json!({
            "id": 8,
            "hash_string": OTHER_HASH,
            "name": "Real.Movie.2020.mkv",
            "files": [{ "name": "Real.Movie.2020.mkv", "length": 500, "bytes_completed": 500 }],
            "error_string": "",
            "labels": [],
        })
    }

    fn response(torrents: &[Value]) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "torrents": torrents, "removed": [3, 4] }
        })
    }

    fn scrubbed(value: Value) -> Value {
        let mut value = value;
        Scrubber::new().scrub(&mut value);
        value
    }

    #[test]
    fn replaces_every_string_outside_allowlist() {
        let mut value = response(&[multi_file_torrent()]);
        value["result"]["version"] = json!("4.1.3 (abcdef0123)");
        value["result"]["rpc_version_semver"] = json!("6.0.0");

        let scrubbed = scrubbed(value);

        let text = scrubbed.to_string();
        for original in SENSITIVE {
            assert!(!text.contains(original), "{original:?} survived in {text}");
        }
        let torrent = &scrubbed["result"]["torrents"][0];
        assert_eq!(torrent["comment"], "redacted");
        assert_eq!(torrent["creator"], "redacted");
        assert_eq!(torrent["unknown_key"], "redacted");
        assert_eq!(torrent["download_dir"], "/data/torrents/video");
        assert_eq!(torrent["error_string"], "redacted error");
        assert_eq!(torrent["labels"], json!(["label", "label"]));
        assert_eq!(torrent["trackers"][0]["announce"], "redacted");
        assert_eq!(scrubbed["jsonrpc"], "2.0");
        assert_eq!(scrubbed["result"]["version"], "4.1.3 (abcdef0123)");
        assert_eq!(scrubbed["result"]["rpc_version_semver"], "6.0.0");
        assert_eq!(torrent["pieces"], "AAEC/w==");
    }

    #[test]
    fn same_hash_maps_to_same_fake_everywhere() {
        let mut scrubber = Scrubber::new();
        let mut list = response(&[multi_file_torrent(), single_file_torrent()]);
        let mut details = response(&[single_file_torrent()]);

        scrubber.scrub(&mut list);
        scrubber.scrub(&mut details);

        let first = &list["result"]["torrents"][0];
        assert_eq!(first["hash_string"], FAKE_HASH_1);
        assert_eq!(
            first["magnet_link"],
            format!("magnet:?xt=urn:btih:{FAKE_HASH_1}&dn=Torrent+1")
        );
        assert_eq!(first["name"], "Torrent 1");
        let second = &list["result"]["torrents"][1];
        assert_eq!(second["hash_string"], FAKE_HASH_2);
        assert_eq!(second["name"], "Torrent 2");
        // The same torrent in another response keeps its fake identity.
        let again = &details["result"]["torrents"][0];
        assert_eq!(again["hash_string"], FAKE_HASH_2);
        assert_eq!(again["name"], "Torrent 2");
    }

    #[test]
    fn files_are_named_after_their_torrent() {
        let scrubbed = scrubbed(response(&[multi_file_torrent(), single_file_torrent()]));

        let multi = &scrubbed["result"]["torrents"][0]["files"];
        assert_eq!(multi[0]["name"], "Torrent 1/file 1.bin");
        assert_eq!(multi[1]["name"], "Torrent 1/file 2.bin");
        assert_eq!(multi[1]["length"], 200);
        let single = &scrubbed["result"]["torrents"][1]["files"];
        assert_eq!(single[0]["name"], "Torrent 2.bin");
    }

    #[test]
    fn torrents_without_a_hash_string_are_scrubbed_too() {
        let mut scrubber = Scrubber::new();
        let mut value = response(&[json!({
            "id": 9,
            "name": "Real.Show.S01.1080p",
            "labels": ["tv-archive"],
            "magnet_link": format!("magnet:?xt=urn:btih:{HASH}&dn=Real.Show.S01.1080p"),
        })]);

        scrubber.scrub(&mut value);

        let torrent = &value["result"]["torrents"][0];
        assert_eq!(torrent["name"], "Torrent 1");
        assert_eq!(torrent["labels"], json!(["label"]));
        assert_eq!(
            torrent["magnet_link"],
            format!("magnet:?xt=urn:btih:{FAKE_HASH_1}&dn=Torrent+1")
        );
        assert_eq!(scrubber.leaked(&value), Vec::<String>::new());
    }

    #[test]
    fn empty_error_string_and_labels_stay_empty() {
        let scrubbed = scrubbed(response(&[single_file_torrent()]));

        let torrent = &scrubbed["result"]["torrents"][0];
        assert_eq!(torrent["error_string"], "");
        assert_eq!(torrent["labels"], json!([]));
    }

    #[test]
    fn path_keys_name_the_fake_directory() {
        let scrubbed = scrubbed(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "path": "/mnt/real/disk/video", "size_bytes": 10, "total_size": 20 }
        }));

        assert_eq!(scrubbed["result"]["path"], "/data/torrents/video");
        assert_eq!(scrubbed["result"]["size_bytes"], 10);
    }

    #[test]
    fn keeps_numbers_and_pieces() {
        let scrubbed = scrubbed(response(&[multi_file_torrent()]));

        let torrent = &scrubbed["result"]["torrents"][0];
        assert_eq!(torrent["total_size"], 300);
        assert_eq!(torrent["rate_upload"], 1234);
        assert_eq!(torrent["percent_done"], 0.5);
        assert_eq!(torrent["id"], 7);
        assert_eq!(torrent["is_private"], true);
        assert_eq!(torrent["pieces"], "AAEC/w==");
        assert_eq!(torrent["file_stats"][1]["priority"], -1);
        assert_eq!(scrubbed["result"]["removed"], json!([3, 4]));
    }

    #[test]
    fn complete_scrubbing_leaks_nothing() {
        let mut scrubber = Scrubber::new();
        let mut value = response(&[multi_file_torrent(), single_file_torrent()]);

        scrubber.scrub(&mut value);

        assert_eq!(scrubber.leaked(&value), Vec::<String>::new());
    }

    #[test]
    fn leaked_reports_surviving_originals() {
        let mut scrubber = Scrubber::new();
        let mut value = response(&[multi_file_torrent()]);
        scrubber.scrub(&mut value);

        value["result"]["torrents"][0]["unknown_key"] = json!("mystery value");
        value["result"]["torrents"][0]["note"] = json!(format!("hash {HASH} in a sentence"));

        assert_eq!(
            scrubber.leaked(&value),
            vec![HASH.to_owned(), "mystery value".to_owned()]
        );
    }
}
