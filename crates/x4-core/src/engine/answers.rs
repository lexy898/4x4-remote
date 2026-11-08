//! What the daemon's answers to the polls change in the engine's state.

use std::collections::HashSet;
use std::time::Instant;

use x4_rpc::{FreeSpace, SessionInfo, SessionStats, TorrentGetResponse, TorrentId};

use super::{Engine, Event, PollKind, Request, Response, unexpected};
use crate::model::Torrent;
use crate::snapshot::SessionSummary;
use crate::store::{Converted, SkippedRecord, Store, torrents_from_records};

impl Engine {
    /// Takes the answer to a poll that is still current; an answer of the wrong kind counts as
    /// an invalid response.
    pub(super) fn poll_answered(
        &mut self,
        now: Instant,
        poll: &Request,
        response: Response,
    ) -> Vec<Event> {
        let events = match (poll, response) {
            (Request::SessionGet, Response::Session(info)) => {
                self.session_read(&info);
                Vec::new()
            }
            (Request::SessionStats, Response::Stats(stats)) => {
                self.stats_read(&stats);
                Vec::new()
            }
            (Request::FreeSpace(path), Response::FreeSpace(space)) => {
                self.free_space_read(path, &space);
                Vec::new()
            }
            (Request::FullList, Response::Torrents(list)) => {
                self.list_read(list, ListKind::Full);
                Vec::new()
            }
            (Request::RecentlyActive | Request::ListOne(_), Response::Torrents(list)) => {
                self.list_read(list, ListKind::Recent);
                Vec::new()
            }
            (Request::Details { hash, kind }, Response::Details(answer)) => {
                self.details_read(now, hash, *kind, answer)
            }
            (poll, other) => return self.poll_failed(now, poll, &unexpected(poll, &other)),
        };
        if let Some(kind) = poll.poll_kind() {
            self.refused.answered(kind);
        }
        events
    }

    fn session_read(&mut self, info: &SessionInfo) {
        let moved = self
            .session
            .as_ref()
            .is_some_and(|known| known.download_dir != info.download_dir);
        if moved && self.free_space_bytes.is_some() {
            // Measured in the old directory; the new one is measured next.
            self.free_space_bytes = None;
            self.snapshot_changed = true;
        }
        let session = Some(SessionSummary {
            lists_empty_files: self.lists_empty_files,
            ..SessionSummary::from(info)
        });
        if session != self.session {
            self.session = session;
            self.snapshot_changed = true;
        }
        if self.download_dir.as_deref() != Some(info.download_dir.as_str()) {
            self.download_dir = Some(info.download_dir.clone());
            self.free_space_timer.force();
        }
    }

    fn stats_read(&mut self, stats: &SessionStats) {
        let rates = (stats.download_speed, stats.upload_speed);
        if rates != (self.rate_down, self.rate_up) {
            (self.rate_down, self.rate_up) = rates;
            self.snapshot_changed = true;
        }
    }

    fn free_space_read(&mut self, path: &str, space: &FreeSpace) {
        if self.download_dir.as_deref() != Some(path) {
            // Asked before the download directory changed.
            return;
        }
        // The server reports -1 for a size it could not read.
        let bytes = u64::try_from(space.size_bytes).ok();
        if bytes != self.free_space_bytes {
            self.free_space_bytes = bytes;
            self.snapshot_changed = true;
        }
    }

    fn list_read(&mut self, answer: TorrentGetResponse, kind: ListKind) {
        let Converted { torrents, skipped } = torrents_from_records(answer.torrents);
        if self.skipped.update(skipped, kind) {
            self.skipped.log();
        }
        match kind {
            ListKind::Full => self.replace_all(torrents),
            ListKind::Recent => self.apply_recent(torrents, &answer.removed),
        }
    }

    fn replace_all(&mut self, torrents: Vec<Torrent>) {
        let mut store = Store::new();
        store.replace_all(torrents);
        if store != self.store {
            self.store = store;
            self.snapshot_changed = true;
        }
        // Also for an empty list: from now on, no torrents means the daemon has none.
        if !self.listed {
            self.listed = true;
            self.snapshot_changed = true;
        }
    }

    fn apply_recent(&mut self, torrents: Vec<Torrent>, removed: &[TorrentId]) {
        if torrents.iter().any(|torrent| self.renumbered(torrent)) {
            tracing::info!("the daemon renumbered its torrents; reading the whole list");
            self.full_list_timer.force();
        }
        let changed = removed
            .iter()
            .any(|&id| self.store.hash_for_id(id).is_some())
            || torrents
                .iter()
                .any(|torrent| self.store.get(&torrent.hash) != Some(torrent));
        self.store.apply_recent(torrents, removed);
        if changed {
            self.snapshot_changed = true;
        }
    }

    /// Whether `torrent` shows that the daemon handed out its ids anew (it restarted): its hash
    /// is known under another id, or its id belongs to another hash.
    fn renumbered(&self, torrent: &Torrent) -> bool {
        self.store
            .get(&torrent.hash)
            .is_some_and(|known| known.id != torrent.id)
            || self
                .store
                .hash_for_id(torrent.id)
                .is_some_and(|known| known != &torrent.hash)
    }
}

/// Which `torrent_get` answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ListKind {
    /// Every torrent.
    Full,
    /// Only the recently active torrents.
    Recent,
}

/// The records the daemon sends that cannot be shown, kept so they are logged once, not on
/// every poll.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct SkippedSet(pub(super) HashSet<SkippedRecord>);

impl SkippedSet {
    /// Takes the records one list answer skipped; tells whether the set changed, which is when
    /// it is worth a log line.
    ///
    /// A full list says which records are skipped now. A recent list only adds to them: it
    /// leaves out the torrents that did not change, so it cannot tell that one was fixed.
    pub(super) fn update(&mut self, skipped: Vec<SkippedRecord>, kind: ListKind) -> bool {
        match kind {
            ListKind::Full => {
                let now: HashSet<SkippedRecord> = skipped.into_iter().collect();
                let changed = now != self.0;
                self.0 = now;
                changed
            }
            ListKind::Recent => {
                let before = self.0.len();
                self.0.extend(skipped);
                self.0.len() != before
            }
        }
    }

    /// Logs the set, without torrent names and with at most 8 characters of each hash.
    pub(super) fn log(&self) {
        if self.0.is_empty() {
            tracing::info!("every torrent the daemon reports can be shown again");
        } else {
            let records: Vec<&SkippedRecord> = self.0.iter().collect();
            tracing::warn!(
                count = records.len(),
                ?records,
                "the daemon reports torrents that cannot be shown"
            );
        }
    }
}

/// The kinds of poll the daemon refused the last time they were sent, so that a refusal that
/// repeats is logged as a warning once, not at every interval.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct RefusedPolls(pub(super) HashSet<PollKind>);

impl RefusedPolls {
    /// Notes that the daemon refused a poll of `kind`; tells whether that is news, worth a
    /// warning: the first refusal, or the first after the poll was answered again.
    pub(super) fn failed(&mut self, kind: PollKind) -> bool {
        self.0.insert(kind)
    }

    /// Notes that the daemon answered a poll of `kind`, so its next refusal is news again.
    pub(super) fn answered(&mut self, kind: PollKind) {
        self.0.remove(&kind);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::store::SkipReason;

    #[test]
    fn a_refused_poll_is_news_once_until_it_is_answered() {
        let mut refused = RefusedPolls::default();

        assert!(refused.failed(PollKind::FreeSpace));
        assert!(!refused.failed(PollKind::FreeSpace));
        assert!(refused.failed(PollKind::SessionStats));

        refused.answered(PollKind::FreeSpace);
        assert!(refused.failed(PollKind::FreeSpace));
        assert!(!refused.failed(PollKind::SessionStats));
    }

    #[test]
    fn skipped_records_are_reported_only_when_the_set_changes() {
        let bad = |prefix: &str| SkippedRecord {
            hash_prefix: Some(prefix.to_owned()),
            reason: SkipReason::InvalidHash,
        };
        let mut set = SkippedSet::default();

        assert!(set.update(vec![bad("aaaa")], ListKind::Full));
        assert!(!set.update(vec![bad("aaaa")], ListKind::Full));
        // A recent list leaves out the torrents that did not change; that fixes nothing.
        assert!(!set.update(Vec::new(), ListKind::Recent));
        assert!(!set.update(vec![bad("aaaa")], ListKind::Recent));
        assert!(set.update(vec![bad("bbbb")], ListKind::Recent));
        // A full list says which are skipped now.
        assert!(set.update(vec![bad("bbbb")], ListKind::Full));
        assert_eq!(set, SkippedSet(HashSet::from([bad("bbbb")])));
        assert!(set.update(Vec::new(), ListKind::Full));
        assert!(!set.update(Vec::new(), ListKind::Full));
    }
}
