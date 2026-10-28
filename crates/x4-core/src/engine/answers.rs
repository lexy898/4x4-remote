//! What the daemon's answers to the polls change in the engine's state.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use x4_rpc::{FreeSpace, HashString, SessionInfo, SessionStats, TorrentGetResponse, TorrentId};

use super::{Engine, Event, Request, Response, unexpected};
use crate::details::{self, Built, DetailsIssue};
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
        match (poll, response) {
            (Request::SessionGet, Response::Session(info)) => self.session_read(&info),
            (Request::SessionStats, Response::Stats(stats)) => self.stats_read(&stats),
            (Request::FreeSpace(path), Response::FreeSpace(space)) => {
                self.free_space_read(path, &space);
            }
            (Request::FullList, Response::Torrents(list)) => self.list_read(list, ListKind::Full),
            (Request::RecentlyActive, Response::Torrents(list)) => {
                self.list_read(list, ListKind::Recent);
            }
            (Request::Details(hash), Response::Details(answer)) => {
                return self.details_read(hash, answer);
            }
            (poll, other) => return self.poll_failed(now, poll, &unexpected(poll, &other)),
        }
        Vec::new()
    }

    fn session_read(&mut self, info: &SessionInfo) {
        let session = Some(SessionSummary::from(info));
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

    fn details_read(&mut self, hash: &HashString, answer: TorrentGetResponse) -> Vec<Event> {
        if answer.torrents.is_empty() {
            // Removed since it was selected; the list shows that.
            return Vec::new();
        }
        let Some(record) = details::find_record(hash, answer) else {
            self.note_details_issues(hash, vec![DetailsIssue::NoMatchingRecord]);
            return Vec::new();
        };
        let Built { details, issues } = details::build(hash.clone(), record);
        self.note_details_issues(hash, issues);
        if self.details.as_deref() == Some(&details) {
            return Vec::new();
        }
        let details = Arc::new(details);
        self.details = Some(Arc::clone(&details));
        vec![Event::Details(details)]
    }

    /// Logs what is wrong with the selected torrent's details when that changes, not on every
    /// tick.
    fn note_details_issues(&mut self, hash: &HashString, issues: Vec<DetailsIssue>) {
        if issues == self.details_issues {
            return;
        }
        if !issues.is_empty() {
            tracing::warn!(torrent = %hash, ?issues, "the daemon sent incomplete torrent details");
        }
        self.details_issues = issues;
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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::store::SkipReason;

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
