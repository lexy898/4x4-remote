//! What the tests of the details share: reads of one torrent, a look at its pieces, and
//! `torrent_set` calls with the lists the client refuses to send.

use x4_rpc::{FilePriority, FileRecord, Pieces, Status};

use super::*;

/// The fields of a full details read and then `extra`.
pub(super) fn full_and(extra: &[TorrentField]) -> Vec<TorrentField> {
    let mut fields = TorrentField::DETAILS_FULL.to_vec();
    fields.extend_from_slice(extra);
    fields
}

/// The record of the torrent `hash`, with `fields`.
pub(super) fn read(
    client: &mut Client<SyntheticDaemon>,
    hash: &HashString,
    fields: &[TorrentField],
) -> TorrentRecord {
    client
        .torrent_get(&GetIds::Selection(selection(&[hash])), fields)
        .expect("the client parses the answer")
        .torrents
        .into_iter()
        .next()
        .expect("the torrent is there")
}

/// Every torrent's hash, status and file count.
pub(super) fn counts(client: &mut Client<SyntheticDaemon>) -> Vec<(HashString, Status, u32)> {
    let fields = [
        TorrentField::HashString,
        TorrentField::Status,
        TorrentField::FileCount,
    ];
    let records = client
        .torrent_get(&GetIds::All, &fields)
        .expect("the client parses the answer")
        .torrents;
    records
        .into_iter()
        .map(|record| {
            (
                HashString::parse(&record.hash_string.expect("a hash")).expect("a valid hash"),
                record.status.expect("a status"),
                record.file_count.expect("a file count"),
            )
        })
        .collect()
}

/// The hash of the first torrent whose status and file count `wanted` accepts.
pub(super) fn pick(
    client: &mut Client<SyntheticDaemon>,
    wanted: impl Fn(Status, u32) -> bool,
) -> HashString {
    counts(client)
        .into_iter()
        .find(|&(_, status, files)| wanted(status, files))
        .expect("a torrent of that kind")
        .0
}

/// The posted `torrent_get` of `hash` with `fields`, as the JSON the daemon sent.
pub(super) fn raw_get(daemon: &SyntheticDaemon, hash: &HashString, fields: &[&str]) -> Value {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "torrent_get",
        "params": { "ids": [hash.to_string()], "fields": fields },
    });
    post(daemon, &request.to_string())
}

/// The wanted switches and the priorities of the torrent `hash`, from a progress read.
pub(super) fn switches(
    client: &mut Client<SyntheticDaemon>,
    hash: &HashString,
) -> (Vec<bool>, Vec<FilePriority>) {
    let record = read(client, hash, TorrentField::DETAILS_PROGRESS);
    (
        record.wanted.expect("wanted"),
        record.priorities.expect("priorities"),
    )
}

/// A `torrent_set` of `ids` and `settings` as the daemon reads it, with the lists the client
/// refuses to send (an empty one) and the ones of a wrong type.
pub(super) fn set_raw(
    client: &mut Client<SyntheticDaemon>,
    ids: &[&HashString],
    settings: Value,
) -> Result<Value, RpcError> {
    let mut params = settings;
    params["ids"] = ids.iter().map(ToString::to_string).collect();
    client.call_raw("torrent_set", Some(params))
}

/// Whether the sequential download of `hash` is on.
pub(super) fn is_sequential(client: &mut Client<SyntheticDaemon>, hash: &HashString) -> bool {
    read(client, hash, &[TorrentField::SequentialDownload])
        .sequential_download
        .expect("the flag")
}

/// Turns the sequential download of `hash` on or off.
pub(super) fn set_sequential(client: &mut Client<SyntheticDaemon>, hash: &HashString, on: bool) {
    let set = TorrentSet {
        sequential_download: Some(on),
        ..TorrentSet::default()
    };
    client.torrent_set(&selection(&[hash]), &set).expect("set");
}

/// `part` as a share of `whole`, to KiB.
pub(super) fn share(part: u64, whole: u64) -> f64 {
    let kib = |bytes: u64| f64::from(u32::try_from(bytes / 1024).expect("fits"));
    kib(part) / kib(whole)
}

/// A torrent's pieces and files as a details read reports them.
pub(super) struct View {
    /// Which pieces are present.
    pub(super) have: Vec<bool>,
    pub(super) files: Vec<FileRecord>,
    pub(super) piece_size: u64,
    pub(super) total: u64,
    pub(super) left: u64,
    pub(super) percent_done: f64,
}

impl View {
    /// Reads the details of `hash`.
    pub(super) fn of(client: &mut Client<SyntheticDaemon>, hash: &HashString) -> Self {
        let extra = [
            TorrentField::TotalSize,
            TorrentField::LeftUntilDone,
            TorrentField::PercentDone,
        ];
        let record = read(client, hash, &full_and(&extra));
        let pieces =
            Pieces::decode_base64(record.pieces.as_deref().expect("pieces")).expect("base64");
        let count = record.piece_count.expect("a piece count");
        Self {
            have: (0..count).map(|piece| pieces.has(piece)).collect(),
            files: record.files.expect("files"),
            piece_size: record.piece_size.expect("a piece size"),
            total: record.total_size.expect("a total"),
            left: record.left_until_done.expect("left"),
            percent_done: record.percent_done.expect("percent done"),
        }
    }

    /// How many pieces are present.
    pub(super) fn present(&self) -> usize {
        self.have.iter().filter(|&&have| have).count()
    }

    /// The first piece that is missing.
    pub(super) fn first_missing(&self) -> usize {
        self.have
            .iter()
            .position(|&have| !have)
            .expect("a missing piece")
    }

    /// The bytes the present pieces hold; the last piece may be shorter than the others.
    pub(super) fn present_bytes(&self) -> u64 {
        self.have
            .iter()
            .enumerate()
            .filter(|&(_, &have)| have)
            .map(|(piece, _)| {
                let start = u64::try_from(piece).expect("fits") * self.piece_size;
                (start + self.piece_size).min(self.total) - start
            })
            .sum()
    }

    /// The pieces present now that were not in `before`; none may be gone.
    pub(super) fn gained_since(&self, before: &Self) -> Vec<usize> {
        assert_eq!(self.have.len(), before.have.len());
        let mut gained = Vec::new();
        for (piece, (&now, &then)) in self.have.iter().zip(&before.have).enumerate() {
            assert!(now || !then, "piece {piece} is gone");
            if now && !then {
                gained.push(piece);
            }
        }
        gained
    }

    /// How many bytes of each file the present pieces hold, worked out from the pieces alone.
    pub(super) fn covered(&self) -> Vec<u64> {
        let mut offset = 0;
        let mut covered = Vec::new();
        for file in &self.files {
            let (start, end) = (offset, offset + file.length);
            offset = end;
            let mut bytes = 0;
            for (piece, _) in self.have.iter().enumerate().filter(|&(_, &have)| have) {
                let piece = u64::try_from(piece).expect("fits");
                let from = (piece * self.piece_size).max(start);
                let to = ((piece + 1) * self.piece_size).min(self.total).min(end);
                bytes += to.saturating_sub(from);
            }
            covered.push(bytes);
        }
        covered
    }

    /// The bytes of each file as the daemon reports them.
    pub(super) fn reported(&self) -> Vec<u64> {
        self.files.iter().map(|file| file.bytes_completed).collect()
    }
}
