//! `torrent_add`, as Transmission 4.1 answers it (`torrentAdd`, T/rpcimpl.cc): the checks come in
//! its order, then the content is read, and a torrent the daemon already has, found by its info
//! hash, is reported and left alone.
//!
//! The content is read with the app's own readers, `x4_core::parse_torrent` for a `.torrent` file
//! and `x4_rpc::MagnetLink` for a magnet link, so that the names, the files and their order are
//! the ones the app expects. The real daemon also reads a bare info hash, a URL and a path on its
//! own disk from `filename`; this one reads only a magnet link, which is all the app ever sends.
//!
//! The torrent counts in whole KiB, like every torrent of this daemon, so each file size is
//! rounded up to a KiB, and a torrent of more than 4 TiB is refused.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};
use x4_core::parse_torrent;
use x4_rpc::MagnetLink;

use super::files::{FileOps, Plan};
use super::refusal::{PATH_NOT_ABSOLUTE, Refusal, UNRECOGNIZED_INFO};
use super::torrent::{NewTorrent, Torrent};
use super::{INVALID_PARAMS, State};

/// What the content of a `torrent_add` says about the torrent.
struct Content {
    hash: String,
    name: String,
    plan: Plan,
    total_kib: u32,
}

/// The daemon's refusal of content it cannot read, which says no more than that.
fn unrecognized() -> Refusal {
    Refusal::daemon(UNRECOGNIZED_INFO, None, None)
}

impl Content {
    /// What a magnet link says: its hash, its name (or the hash), and no files yet.
    fn of_magnet(text: &str) -> Result<Self, Refusal> {
        let link = MagnetLink::parse(text).map_err(|error| {
            tracing::debug!(%error, "the synthetic daemon cannot read the magnet link");
            unrecognized()
        })?;
        let hash = link.hash().to_string();
        Ok(Self {
            name: link.name().map_or_else(|| hash.clone(), str::to_owned),
            hash,
            plan: Plan::magnet(),
            total_kib: 0,
        })
    }

    /// What a `.torrent` file says, given as base64: its hash, its name and its files in the
    /// metainfo's order, padding included. A file the reader could number in more than one way
    /// has one file, named like the torrent and with no size.
    fn of_metainfo(base64: &str) -> Result<Self, Refusal> {
        let bytes = STANDARD.decode(base64).map_err(|error| {
            tracing::debug!(%error, "the synthetic daemon cannot decode the metainfo");
            unrecognized()
        })?;
        let parsed = parse_torrent(&bytes).map_err(|error| {
            tracing::debug!(%error, "the synthetic daemon cannot read the metainfo");
            unrecognized()
        })?;
        let hash = parsed.hash.to_string();
        let name = if parsed.name.is_empty() {
            hash.clone()
        } else {
            parsed.name
        };
        let files = match parsed.files {
            Some(files) => files
                .into_iter()
                .map(|file| (file.path, file.length))
                .collect(),
            None => vec![(name.clone(), parsed.total_size.unwrap_or(0))],
        };
        let (plan, total_kib) = Plan::listed(files).ok_or_else(|| {
            // Real Transmission has no such limit, so this one says what it is.
            Refusal::daemon(
                UNRECOGNIZED_INFO,
                Some("the synthetic daemon holds torrents of up to 4 TiB"),
                None,
            )
        })?;
        Ok(Self {
            hash,
            name,
            plan,
            total_kib,
        })
    }
}

impl State {
    /// The answer to `torrent_add`: `torrent_added` with the new torrent's id, name and hash, or
    /// `torrent_duplicate` with those of the torrent already there, which nothing in the request
    /// then changes.
    ///
    /// The parameters are read as the daemon reads them: one of the wrong type is as good as
    /// missing. `filename` wins over `metainfo`.
    ///
    /// # Errors
    ///
    /// -32602 for no `filename` and no `metainfo`, 3 for a `download_dir` that is not absolute,
    /// and 4 for content that is not a torrent or a magnet link the daemon can add.
    pub(super) fn torrent_add(&mut self, params: Option<&Value>) -> Result<Value, Refusal> {
        let field = |key: &str| params.and_then(|params| params.get(key));
        let text = |key: &str| field(key).and_then(Value::as_str).unwrap_or_default();
        let (filename, metainfo) = (text("filename"), text("metainfo"));
        if filename.is_empty() && metainfo.is_empty() {
            return Err(Refusal::daemon(
                INVALID_PARAMS,
                Some("no filename or metainfo specified"),
                None,
            ));
        }
        let dir = field("download_dir").and_then(Value::as_str);
        if dir.is_some_and(|dir| !dir.starts_with('/')) {
            return Err(Refusal::daemon(
                PATH_NOT_ABSOLUTE,
                Some("download directory path is not absolute"),
                None,
            ));
        }
        let content = if filename.is_empty() {
            Content::of_metainfo(metainfo)?
        } else {
            Content::of_magnet(filename)?
        };
        if let Some(existing) = self.torrents.iter().find(|t| t.hash == content.hash) {
            tracing::debug!(torrent = %content.hash, "the synthetic daemon already has the torrent");
            return Ok(json!({ "torrent_duplicate": existing.summary() }));
        }

        let id = self.next_id;
        self.next_id = id.saturating_add(1);
        let mut torrent = Torrent::from_add(NewTorrent {
            id,
            hash: content.hash,
            name: content.name,
            plan: content.plan,
            total_kib: content.total_kib,
            dir: dir.map(str::to_owned),
            // The session starts the torrents it is given (`start_added_torrents`).
            paused: field("paused").and_then(Value::as_bool).unwrap_or(false),
        });
        let lists = FileOps::for_add(params, torrent.files().file_count());
        torrent.set(
            &lists,
            field("sequential_download").and_then(Value::as_bool),
        );
        tracing::debug!(torrent = %torrent.hash, id, "the synthetic daemon added a torrent");
        let summary = torrent.summary();
        self.torrents.push(torrent);
        Ok(json!({ "torrent_added": summary }))
    }
}
