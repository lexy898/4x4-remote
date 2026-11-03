//! The torrent methods of [`Client`] and their request parameters (rpc-spec §3.1–§3.5).
//!
//! The parameter structs here are the only place that decides what goes on the wire. They take
//! a [`TorrentSelection`] wherever an omitted `ids` would mean every torrent, and leave out
//! every empty file-index list, which would mean every file.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};

use crate::client::{ADD_TIMEOUT, Client};
use crate::download_dir::DownloadDir;
use crate::error::RpcError;
use crate::ids::{GetIds, TorrentSelection};
use crate::torrent::{
    AddOutcome, AddSource, AddedTorrent, RemoveData, TorrentAction, TorrentAdd, TorrentField,
    TorrentGetResponse, TorrentSet,
};
use crate::transport::Transport;

impl<T: Transport> Client<T> {
    /// Reads `fields` of the torrents `ids` names (`torrent_get`).
    ///
    /// Fields that were not requested are `None` in the records.
    ///
    /// # Errors
    ///
    /// Any [`RpcError`] of the call; [`RpcError::InvalidResponse`] when the result does not
    /// decode, for example a field of the wrong type.
    pub fn torrent_get(
        &mut self,
        ids: &GetIds,
        fields: &[TorrentField],
    ) -> Result<TorrentGetResponse, RpcError> {
        self.call("torrent_get", Some(&GetParams { ids, fields }))
    }

    /// Starts, stops, verifies or reannounces the selected torrents (`torrent_start`, …).
    ///
    /// # Errors
    ///
    /// Any [`RpcError`] of the call.
    pub fn torrent_action(
        &mut self,
        action: TorrentAction,
        ids: &TorrentSelection,
    ) -> Result<(), RpcError> {
        self.call(action.method(), Some(&IdsParams { ids }))
            .map(|IgnoredAny| ())
    }

    /// Changes the selected torrents (`torrent_set`).
    ///
    /// # Errors
    ///
    /// Any [`RpcError`] of the call.
    pub fn torrent_set(
        &mut self,
        ids: &TorrentSelection,
        set: &TorrentSet,
    ) -> Result<(), RpcError> {
        let params = SetParams {
            ids,
            sequential_download: set.sequential_download,
            files_wanted: &set.files_wanted,
            files_unwanted: &set.files_unwanted,
            priority_high: &set.priority_high,
            priority_normal: &set.priority_normal,
            priority_low: &set.priority_low,
        };
        self.call("torrent_set", Some(&params)).map(|IgnoredAny| ())
    }

    /// Removes the selected torrents, deleting their data when `data` is
    /// [`RemoveData::Delete`] (`torrent_remove`).
    ///
    /// # Errors
    ///
    /// Any [`RpcError`] of the call.
    pub fn torrent_remove(
        &mut self,
        ids: &TorrentSelection,
        data: RemoveData,
    ) -> Result<(), RpcError> {
        let params = RemoveParams {
            ids,
            delete_local_data: data == RemoveData::Delete,
        };
        self.call("torrent_remove", Some(&params))
            .map(|IgnoredAny| ())
    }

    /// Adds a torrent from a magnet link or metainfo (`torrent_add`).
    ///
    /// Adding a torrent the server already has is not an error: it reports
    /// [`AddOutcome::Duplicate`]. The call waits up to [`ADD_TIMEOUT`] for its answer, in place of
    /// the transport's own timeout.
    ///
    /// # Errors
    ///
    /// [`RpcError::MagnetWithFileChoices`], before anything is sent, when a magnet link comes
    /// with unwanted files or file priorities. Otherwise any [`RpcError`] of the call;
    /// [`RpcError::InvalidResponse`] when the result holds neither or both of `torrent_added`
    /// and `torrent_duplicate`. A [`TransportError::Timeout`](crate::TransportError::Timeout)
    /// leaves open whether the torrent was added.
    pub fn torrent_add(&mut self, add: &TorrentAdd) -> Result<AddOutcome, RpcError> {
        let (filename, metainfo) = match &add.source {
            AddSource::Magnet(link) => {
                let file_choices = [&add.files_unwanted, &add.priority_high, &add.priority_low];
                if file_choices.iter().any(|files| !files.is_empty()) {
                    return Err(RpcError::MagnetWithFileChoices);
                }
                (Some(link.as_str()), None)
            }
            AddSource::Metainfo(bytes) => (None, Some(STANDARD.encode(bytes))),
        };
        let params = AddParams {
            filename,
            metainfo,
            download_dir: add.download_dir.as_ref().map(DownloadDir::as_str),
            paused: add.paused,
            files_unwanted: &add.files_unwanted,
            priority_high: &add.priority_high,
            priority_low: &add.priority_low,
            sequential_download: add.sequential_download,
        };
        let result: AddResult =
            self.call_with_timeout("torrent_add", Some(&params), Some(ADD_TIMEOUT))?;
        match (result.torrent_added, result.torrent_duplicate) {
            (Some(added), None) => Ok(AddOutcome::Added(added)),
            (None, Some(duplicate)) => Ok(AddOutcome::Duplicate(duplicate)),
            _ => Err(RpcError::InvalidResponse(
                "torrent_add: expected exactly one of torrent_added and torrent_duplicate"
                    .to_owned(),
            )),
        }
    }
}

#[derive(Serialize)]
struct GetParams<'a> {
    /// Flattened: [`GetIds::All`] must leave `ids` out rather than send `null` or `[]`.
    #[serde(flatten)]
    ids: &'a GetIds,
    fields: &'a [TorrentField],
}

#[derive(Serialize)]
struct IdsParams<'a> {
    ids: &'a TorrentSelection,
}

#[derive(Serialize)]
struct SetParams<'a> {
    ids: &'a TorrentSelection,
    #[serde(skip_serializing_if = "Option::is_none")]
    sequential_download: Option<bool>,
    #[serde(skip_serializing_if = "<[usize]>::is_empty")]
    files_wanted: &'a [usize],
    #[serde(skip_serializing_if = "<[usize]>::is_empty")]
    files_unwanted: &'a [usize],
    #[serde(skip_serializing_if = "<[usize]>::is_empty")]
    priority_high: &'a [usize],
    #[serde(skip_serializing_if = "<[usize]>::is_empty")]
    priority_normal: &'a [usize],
    #[serde(skip_serializing_if = "<[usize]>::is_empty")]
    priority_low: &'a [usize],
}

#[derive(Serialize)]
struct RemoveParams<'a> {
    ids: &'a TorrentSelection,
    delete_local_data: bool,
}

#[derive(Serialize)]
struct AddParams<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    filename: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metainfo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    download_dir: Option<&'a str>,
    paused: bool,
    #[serde(skip_serializing_if = "<[usize]>::is_empty")]
    files_unwanted: &'a [usize],
    #[serde(skip_serializing_if = "<[usize]>::is_empty")]
    priority_high: &'a [usize],
    #[serde(skip_serializing_if = "<[usize]>::is_empty")]
    priority_low: &'a [usize],
    #[serde(skip_serializing_if = "Option::is_none")]
    sequential_download: Option<bool>,
}

/// Exactly one of the two is expected; missing ones are `None`.
#[derive(Deserialize)]
struct AddResult {
    torrent_added: Option<AddedTorrent>,
    torrent_duplicate: Option<AddedTorrent>,
}
