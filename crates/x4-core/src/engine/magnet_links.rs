//! The one-shot read of magnet links: the answer, in the order the UI asked for.
//!
//! A magnet link carries the name and every tracker, so it is read only when the user copies
//! it, never with the polls.

use std::collections::HashMap;
use std::time::Instant;

use x4_rpc::{HashString, RpcError, TorrentGetResponse};

use super::{Engine, Event};
use crate::action::{ActionError, ActionId};
use crate::snapshot::ConnectionError;

impl Engine {
    /// Takes the answer to the read of `hashes`' magnet links at `now`. It is a read, so a
    /// success brings no poll forward; a failure says what it says about the connection, as an
    /// action's does.
    pub(super) fn magnet_links_read(
        &mut self,
        now: Instant,
        id: ActionId,
        hashes: &[HashString],
        result: Result<TorrentGetResponse, RpcError>,
    ) -> Vec<Event> {
        match result {
            Ok(answer) => {
                let links = links_in_order(hashes, answer);
                tracing::debug!(
                    asked = hashes.len(),
                    read = links.len(),
                    "magnet links read"
                );
                vec![Event::MagnetLinks {
                    id,
                    result: Ok(links),
                }]
            }
            Err(error) => {
                tracing::error!(%error, "reading magnet links failed");
                let mut events = vec![Event::MagnetLinks {
                    id,
                    result: Err(ActionError::Rpc(ConnectionError::from(&error))),
                }];
                events.extend(self.call_failed(now, &error));
                events
            }
        }
    }
}

/// The magnet links of `answer` in the order of `hashes`, each torrent once.
///
/// Records without a magnet link, and records whose hash is missing or not valid, are left out;
/// a debug line counts them.
fn links_in_order(hashes: &[HashString], answer: TorrentGetResponse) -> Vec<(HashString, String)> {
    let mut links = HashMap::with_capacity(answer.torrents.len());
    let mut without_link = 0_usize;
    let mut without_hash = 0_usize;
    for record in answer.torrents {
        let hash = record
            .hash_string
            .as_deref()
            .and_then(|raw| HashString::parse(raw).ok());
        match (hash, record.magnet_link.filter(|link| !link.is_empty())) {
            (Some(hash), Some(link)) => {
                links.insert(hash, link);
            }
            (Some(_), None) => without_link += 1,
            (None, _) => without_hash += 1,
        }
    }
    if without_link > 0 || without_hash > 0 {
        tracing::debug!(
            without_link,
            without_hash,
            "left out torrents of a magnet link answer"
        );
    }
    hashes
        .iter()
        .filter_map(|hash| links.remove_entry(hash))
        .collect()
}
