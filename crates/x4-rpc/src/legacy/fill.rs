//! What a legacy `torrent_get` is asked for in place of what 4x4 reads, and how the answer is
//! made to carry what 4x4 reads: the legacy daemons serve neither the per-torrent
//! `bytes_completed` array, nor the pieces of a file, nor (before 4.0) `file_count`, nor `wanted`
//! as booleans. Each is computed exactly as the daemon of Transmission 4.1 does, or left absent.

use serde_json::{Map, Value};

use super::Fill;
use super::names::torrent_field_since;
use crate::error::RpcError;

/// The per-file arrays of a torrent, in the order `file_count` is counted from them.
const PER_FILE: [&str; 5] = [
    "priorities",
    "wanted",
    "files",
    "file_stats",
    "bytes_completed",
];

/// `fields` (snake case) as a legacy daemon of the RPC version `rpc_version` is to be asked for
/// them, and what the answer needs filled in. Nothing is added that a field already asked for
/// gives.
pub(super) fn plan(fields: &mut Vec<String>, rpc_version: u32) -> Fill {
    let has = |fields: &[String], name: &str| fields.iter().any(|field| field == name);
    let mut fill = Fill::default();
    if has(fields, "bytes_completed") {
        fields.retain(|field| field != "bytes_completed");
        fill.bytes_from_file_stats = true;
        if !has(fields, "file_stats") {
            fields.push("file_stats".to_owned());
            fill.added.push("file_stats");
        }
    }
    if has(fields, "files") {
        fill.piece_spans = true;
        for name in ["piece_size", "piece_count"] {
            if !has(fields, name) {
                fields.push(name.to_owned());
                fill.added.push(name);
            }
        }
    }
    fill.file_count = has(fields, "file_count");
    // A daemon without `file-count` (3.00) needs a per-file array to count: the most compact.
    let served = torrent_field_since("file_count").is_none_or(|since| since <= rpc_version);
    if fill.file_count && !served && !PER_FILE.iter().any(|name| has(fields, name)) {
        fields.push("priorities".to_owned());
        fill.added.push("priorities");
    }
    fill
}

/// Fills in each torrent of the `torrents` of a `torrent_get` answer, in JSON-RPC's names.
///
/// # Errors
///
/// [`RpcError::InvalidResponse`] when the files of a torrent have no piece spans in the answer and
/// none can be computed.
pub(super) fn apply(fill: &Fill, arguments: &mut Value) -> Result<(), RpcError> {
    let Some(Value::Array(torrents)) = arguments.get_mut("torrents") else {
        return Ok(());
    };
    for torrent in torrents {
        if let Value::Object(torrent) = torrent {
            fill_torrent(fill, torrent)?;
        }
    }
    Ok(())
}

fn fill_torrent(fill: &Fill, torrent: &mut Map<String, Value>) -> Result<(), RpcError> {
    wanted_as_bools(torrent);
    if fill.bytes_from_file_stats
        && let Some(Value::Array(stats)) = torrent.get("file_stats")
    {
        let completed = stats
            .iter()
            .map(|stat| stat.get("bytes_completed").cloned().unwrap_or(Value::Null))
            .collect();
        torrent.insert("bytes_completed".to_owned(), Value::Array(completed));
    }
    if fill.piece_spans {
        add_piece_spans(torrent)?;
    }
    if fill.file_count && !torrent.contains_key("file_count") {
        let count = PER_FILE
            .iter()
            .find_map(|name| torrent.get(*name).and_then(Value::as_array))
            .map(Vec::len);
        if let Some(count) = count {
            torrent.insert("file_count".to_owned(), Value::from(count));
        }
    }
    for name in &fill.added {
        torrent.remove(*name);
    }
    Ok(())
}

/// Legacy daemons send `wanted` as `0` and `1`, the top-level array and each file's stat too.
fn wanted_as_bools(torrent: &mut Map<String, Value>) {
    let to_bool = |value: &mut Value| {
        if let Some(number) = value.as_f64() {
            *value = Value::Bool(number != 0.0);
        }
    };
    if let Some(Value::Array(wanted)) = torrent.get_mut("wanted") {
        wanted.iter_mut().for_each(to_bool);
    }
    if let Some(Value::Array(stats)) = torrent.get_mut("file_stats") {
        for stat in stats {
            if let Some(wanted) = stat.get_mut("wanted") {
                to_bool(wanted);
            }
        }
    }
}

/// Sets `begin_piece` and `end_piece` of the files that lack them.
fn add_piece_spans(torrent: &mut Map<String, Value>) -> Result<(), RpcError> {
    let piece_size = torrent.get("piece_size").and_then(Value::as_u64);
    let piece_count = torrent
        .get("piece_count")
        .and_then(Value::as_u64)
        .and_then(|count| u32::try_from(count).ok());
    let Some(Value::Array(files)) = torrent.get_mut("files") else {
        return Ok(());
    };
    if files.iter().all(|file| file.get("begin_piece").is_some()) {
        return Ok(());
    }
    let lengths: Option<Vec<u64>> = files
        .iter()
        .map(|file| file.get("length").and_then(Value::as_u64))
        .collect();
    let spans = lengths
        .zip(piece_size)
        .zip(piece_count)
        .and_then(|((lengths, size), count)| piece_spans(&lengths, size, count));
    let Some(spans) = spans else {
        return Err(RpcError::InvalidResponse(
            "torrent_get: files without a piece size".to_owned(),
        ));
    };
    for (file, (begin, end)) in files.iter_mut().zip(spans) {
        if let Value::Object(file) = file {
            file.insert("begin_piece".to_owned(), Value::from(begin));
            file.insert("end_piece".to_owned(), Value::from(end));
        }
    }
    Ok(())
}

/// The pieces `(begin, end)` (end exclusive) each file of `lengths` spans in a torrent of
/// `piece_count` pieces of `piece_size` bytes, as Transmission 4.1 serves them: a file of no
/// bytes spans the one piece at its offset, kept inside the torrent.
///
/// `Some(vec![])` for no files; `None` when there are files but `piece_size` or `piece_count` is
/// 0, or a number overflows.
pub(crate) fn piece_spans(
    lengths: &[u64],
    piece_size: u64,
    piece_count: u32,
) -> Option<Vec<(u32, u32)>> {
    if lengths.is_empty() {
        return Some(Vec::new());
    }
    if piece_size == 0 || piece_count == 0 {
        return None;
    }
    let last_piece = u64::from(piece_count - 1);
    let mut offset = 0_u64;
    let mut spans = Vec::with_capacity(lengths.len());
    for &length in lengths {
        let begin = (offset / piece_size).min(last_piece);
        let end = if length > 0 {
            offset.checked_add(length - 1)? / piece_size + 1
        } else {
            begin + 1
        };
        spans.push((u32::try_from(begin).ok()?, u32::try_from(end).ok()?));
        offset = offset.checked_add(length)?;
    }
    Some(spans)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::testing::fixtures::TORRENT_GET_DETAILS;

    #[test]
    fn piece_spans_match_what_transmission_4_1_serves() {
        let envelope: Value = serde_json::from_str(TORRENT_GET_DETAILS).unwrap_or_default();
        let torrents = envelope["result"]["torrents"].as_array().cloned();
        let mut checked = 0;
        for torrent in torrents.unwrap_or_default() {
            let (Some(files), Some(size), Some(count)) = (
                torrent["files"].as_array(),
                torrent["piece_size"].as_u64(),
                torrent["piece_count"].as_u64(),
            ) else {
                continue;
            };
            let lengths: Vec<u64> = files.iter().filter_map(|f| f["length"].as_u64()).collect();
            let served: Vec<(u64, u64)> = files
                .iter()
                .filter_map(|f| Some((f["begin_piece"].as_u64()?, f["end_piece"].as_u64()?)))
                .collect();
            let computed = piece_spans(&lengths, size, u32::try_from(count).unwrap_or(0));
            let computed: Option<Vec<(u64, u64)>> = computed.map(|spans| {
                spans
                    .into_iter()
                    .map(|(b, e)| (u64::from(b), u64::from(e)))
                    .collect()
            });
            assert_eq!(computed, Some(served));
            checked += 1;
        }
        assert!(checked > 0);
    }

    #[test]
    fn a_zero_length_file_spans_one_piece() {
        assert_eq!(
            piece_spans(&[16384, 0, 100], 16384, 2),
            Some(vec![(0, 1), (1, 2), (1, 2)])
        );
    }

    #[test]
    fn a_last_zero_length_file_at_a_piece_boundary_stays_in_range() {
        assert_eq!(
            piece_spans(&[32768, 0], 16384, 2),
            Some(vec![(0, 2), (1, 2)])
        );
    }

    #[test]
    fn a_magnet_without_metadata_has_no_spans() {
        assert_eq!(piece_spans(&[], 0, 0), Some(vec![]));
    }

    #[test]
    fn files_without_a_piece_size_have_no_spans() {
        assert_eq!(piece_spans(&[5], 0, 0), None);
    }

    #[test]
    fn spans_that_overflow_are_none() {
        assert_eq!(piece_spans(&[u64::MAX, 1], 1, u32::MAX), None);
    }

    #[test]
    fn plan_adds_only_what_is_missing_and_records_it() {
        let mut fields = vec![
            "id".to_owned(),
            "bytes_completed".to_owned(),
            "files".to_owned(),
        ];
        let fill = plan(&mut fields, 18);
        assert_eq!(
            fields,
            ["id", "files", "file_stats", "piece_size", "piece_count"]
        );
        assert!(fill.bytes_from_file_stats && fill.piece_spans);
        assert_eq!(fill.added, ["file_stats", "piece_size", "piece_count"]);

        let mut fields: Vec<String> = ["file_stats", "bytes_completed", "files", "piece_size"]
            .map(str::to_owned)
            .to_vec();
        let fill = plan(&mut fields, 18);
        assert_eq!(fill.added, ["piece_count"]);
    }

    #[test]
    fn file_count_alone_is_counted_from_the_priorities_below_rpc_17() {
        let planned = |fields: &[&str], rpc_version| {
            let mut fields: Vec<String> = fields.iter().map(|&field| field.to_owned()).collect();
            let fill = plan(&mut fields, rpc_version);
            (fields, fill)
        };

        let (fields, fill) = planned(&["id", "file_count"], 16);
        assert_eq!(fields, ["id", "file_count", "priorities"]);
        assert_eq!(fill.added, ["priorities"]);
        let mut answer = json!({ "torrents": [{ "id": 1, "priorities": [0, 0, 0] }] });
        assert_eq!(apply(&fill, &mut answer), Ok(()));
        assert_eq!(
            answer,
            json!({ "torrents": [{ "id": 1, "file_count": 3 }] })
        );

        // A daemon of rpc-version 17 sends file-count itself, and an array asked already counts.
        assert_eq!(planned(&["id", "file_count"], 17).0, ["id", "file_count"]);
        assert_eq!(
            planned(&["file_count", "wanted"], 16).0,
            ["file_count", "wanted"]
        );
        assert_eq!(
            planned(&["file_count", "bytes_completed"], 16).0,
            ["file_count", "file_stats"]
        );
    }

    #[test]
    fn file_count_comes_from_the_first_per_file_array_present() {
        let fill = Fill {
            file_count: true,
            ..Fill::default()
        };
        let mut answer = json!({ "torrents": [
            { "files": [{}, {}, {}] },
            { "wanted": [1], "files": [{}, {}] },
            { "file_count": 9, "wanted": [1] },
            { "id": 4 },
        ] });
        assert_eq!(apply(&fill, &mut answer), Ok(()));
        assert_eq!(answer["torrents"][0]["file_count"], 3);
        assert_eq!(answer["torrents"][1]["file_count"], 1);
        assert_eq!(answer["torrents"][2]["file_count"], 9);
        assert_eq!(answer["torrents"][3].get("file_count"), None);
    }

    #[test]
    fn files_without_a_piece_size_in_the_answer_are_invalid() {
        let fill = Fill {
            piece_spans: true,
            ..Fill::default()
        };
        let mut answer = json!({ "torrents": [{ "files": [{ "length": 5 }] }] });
        assert_eq!(
            apply(&fill, &mut answer),
            Err(RpcError::InvalidResponse(
                "torrent_get: files without a piece size".to_owned()
            ))
        );
    }
}
