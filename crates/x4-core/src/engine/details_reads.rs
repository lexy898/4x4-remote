//! When the selected torrent's details are read, in full or only for their progress, and what
//! the answers change.
//!
//! A full read goes out once a new selection has stayed for
//! [`EngineConfig::details_settle`](super::EngineConfig::details_settle), after connecting, and
//! when the torrent's files changed, as when a magnet link got its metadata. A progress read goes
//! out every [`EngineConfig::details_interval`](super::EngineConfig::details_interval) while the
//! torrent downloads or verifies, and otherwise with each whole list; also at once after an
//! action on the torrent and when the window shows again. The progress read that goes out with a
//! whole list, about once a minute, also reads the trackers, which change too slowly for the
//! reads every few seconds; a downloading torrent's read joins that batch for it. Nothing is read
//! while the window is hidden or nothing is selected.

use std::sync::Arc;
use std::time::Instant;

use x4_rpc::{HashString, TorrentGetResponse, TorrentRecord};

use super::schedule::Timer;
use super::{DetailsRead, Engine, Event, PollKind, Request, Visibility};
use crate::action::Action;
use crate::details::{self, Built, DetailsIssue};
use crate::state::DisplayState;

/// When the selected torrent's details are read next, and how.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct DetailsReads {
    /// From when a full read is due; `None` when none is wanted.
    full_at: Option<Instant>,
    /// A progress read is wanted at once.
    progress_now: bool,
    /// The last details read of either kind; the progress cadence counts from it.
    timer: Timer,
    /// The batch the last details read went out in.
    batch: Option<u64>,
    /// How many files the last full read found; a progress read that counts another number means
    /// the files changed.
    file_count: Option<u32>,
}

impl Engine {
    /// Shows the details of `hash` from `now` on, or none; returns the events that causes: the
    /// peers of the torrent selected before are cleared.
    pub(super) fn select(&mut self, now: Instant, hash: Option<HashString>) -> Vec<Event> {
        if hash == self.selected {
            return Vec::new();
        }
        // The old torrent's details must not show as the new one's.
        self.details = None;
        self.details_issues.clear();
        self.refused.answered(PollKind::Details);
        // Read only once the selection stays, so that moving through the list with the arrow
        // keys reads nothing on the way.
        let settled = now.checked_add(self.config.details_settle).unwrap_or(now);
        self.details_reads = DetailsReads {
            full_at: hash.is_some().then_some(settled),
            ..DetailsReads::default()
        };
        self.selected = hash;
        self.tabs_selected()
    }

    /// The daemon was connected to at `now`: anything may have changed while it was away, so the
    /// details are read in full at once.
    pub(super) fn details_connected(&mut self, now: Instant) {
        let reads = &mut self.details_reads;
        reads.full_at = self.selected.is_some().then_some(now);
        reads.progress_now = false;
        reads.timer = Timer::default();
        reads.batch = None;
    }

    /// The window becomes `visibility`; called before it is noted. Nothing was read while it was
    /// hidden, so showing it again reads the progress once.
    pub(super) fn details_visibility(&mut self, visibility: Visibility) {
        if self.visibility == Visibility::Hidden
            && visibility == Visibility::Visible
            && self.selected.is_some()
        {
            self.details_reads.progress_now = true;
        }
    }

    /// The user asked at `now` for everything to be read again: the selected torrent's details
    /// are read in full, as the metainfo and the files may have changed too. A selection that has
    /// not settled yet keeps waiting.
    pub(super) fn details_refresh(&mut self, now: Instant) {
        if self.selected.is_some() {
            self.details_reads.full_at.get_or_insert(now);
        }
    }

    /// `action`, handed out in batch `sent_in`, was carried out. When it may have changed the
    /// selected torrent's details, their progress is read at once, unless a details read went
    /// out with the action or after it: polls follow actions in a batch, so that read shows it.
    pub(super) fn details_after_action(&mut self, action: &Action, sent_in: Option<u64>) {
        let changes = self
            .selected
            .as_ref()
            .is_some_and(|hash| action.changes_details_of(hash));
        let seen = sent_in
            .zip(self.details_reads.batch)
            .is_some_and(|(action_batch, read_batch)| read_batch >= action_batch);
        if changes && !seen {
            self.details_reads.progress_now = true;
        }
    }

    /// The details read due at `now`, if any, which is then counted as sent; `full_list` tells
    /// whether a whole list goes out in this batch.
    pub(super) fn due_details(&mut self, now: Instant, full_list: bool) -> Option<Request> {
        let kind = self.details_read_due(now, full_list)?;
        let hash = self.selected.clone()?;
        let reads = &mut self.details_reads;
        // A full read brings the progress too.
        reads.full_at = None;
        reads.progress_now = false;
        reads.timer.sent(now);
        reads.batch = Some(self.batch);
        Some(Request::Details { hash, kind })
    }

    /// Which details read is due at `now`, if any; `full_list` tells whether a whole list goes out
    /// in the same batch.
    fn details_read_due(&self, now: Instant, full_list: bool) -> Option<DetailsRead> {
        let hash = self.selected.as_ref()?;
        if self.visibility != Visibility::Visible {
            return None;
        }
        let reads = &self.details_reads;
        if let Some(at) = reads.full_at {
            // A selection that has not settled yet waits, whatever else asks for a read.
            return (at <= now).then_some(DetailsRead::Full);
        }
        // A whole list catches what another client changed in the files, and a downloading
        // torrent's read joins it, so that the trackers are read once a minute whatever the
        // torrent does. The read replaces the next one on the 3 s timer, which is measured
        // from it.
        let wanted = reads.progress_now
            || full_list
            || (self.changing(hash) && reads.timer.is_due(now, self.config.details_interval));
        // Progress is merged into the last full read; without one, it is read in full again.
        wanted.then_some(if self.details.is_none() {
            DetailsRead::Full
        } else if full_list {
            DetailsRead::ProgressAndTrackers
        } else {
            DetailsRead::Progress
        })
    }

    /// When the next details read is due, if one is scheduled apart from the whole list.
    pub(super) fn next_details(&self, now: Instant) -> Option<Instant> {
        let hash = self.selected.as_ref()?;
        if self.visibility != Visibility::Visible {
            return None;
        }
        let reads = &self.details_reads;
        match reads.full_at {
            Some(at) => Some(at),
            None if reads.progress_now => Some(now),
            None if self.changing(hash) => reads.timer.due_at(now, self.config.details_interval),
            // The progress rides with the whole list, which has its own timer.
            None => None,
        }
    }

    /// The answer to a `kind` read came at `now`: the next read of its kind is measured from it,
    /// so a slow daemon rests a full interval.
    pub(super) fn details_answered_at(&mut self, kind: DetailsRead, now: Instant) {
        match kind {
            DetailsRead::Full | DetailsRead::Progress | DetailsRead::ProgressAndTrackers => {
                self.details_reads.timer.answered(now);
            }
            DetailsRead::Peers | DetailsRead::Trackers => self.tab_read_answered_at(kind, now),
        }
    }

    /// Whether a full read of the selected torrent is still to go out: the selection has not
    /// settled yet, or the details are to be read whole again. The tabs wait for it.
    pub(super) fn details_unsettled(&self) -> bool {
        self.details_reads.full_at.is_some()
    }

    /// Whether the torrent `hash` downloads or verifies, so its files change by the second.
    fn changing(&self, hash: &HashString) -> bool {
        self.store.get(hash).is_some_and(|torrent| {
            matches!(
                torrent.state,
                DisplayState::Downloading | DisplayState::Checking
            )
        })
    }

    /// Takes the answer, which came at `now`, to a `kind` read of the details of `hash`, which is
    /// still selected.
    pub(super) fn details_read(
        &mut self,
        now: Instant,
        hash: &HashString,
        kind: DetailsRead,
        answer: TorrentGetResponse,
    ) -> Vec<Event> {
        if answer.torrents.is_empty() {
            // Removed since it was selected; the list shows that.
            return Vec::new();
        }
        match kind {
            DetailsRead::Full => self.full_read(hash, answer),
            DetailsRead::Progress | DetailsRead::ProgressAndTrackers => {
                self.progress_read(now, hash, answer)
            }
            DetailsRead::Peers => self.peers_read(hash, &answer),
            DetailsRead::Trackers => self.trackers_read(hash, &answer),
        }
    }

    /// Takes a full read of the details of `hash`.
    fn full_read(&mut self, hash: &HashString, answer: TorrentGetResponse) -> Vec<Event> {
        let Some(record) = self.record_of(hash, answer) else {
            return Vec::new();
        };
        self.details_reads.file_count = details::file_count(&record);
        let built = details::build(hash.clone(), record);
        self.show_built(hash, built)
    }

    /// Takes a read of the progress of `hash`, which came at `now`, and merges it into the last
    /// full read; a progress that counts other files than that read has the details read in
    /// full again.
    fn progress_read(
        &mut self,
        now: Instant,
        hash: &HashString,
        answer: TorrentGetResponse,
    ) -> Vec<Event> {
        let Some(record) = self.record_of(hash, answer) else {
            return Vec::new();
        };
        let Some(last) = self.details.as_deref() else {
            tracing::debug!("ignored a details progress with no full details to merge into");
            return Vec::new();
        };
        if let (Some(counted), Some(known)) =
            (details::file_count(&record), self.details_reads.file_count)
            && counted != known
        {
            tracing::info!(
                torrent = %hash,
                files = counted,
                "the selected torrent's files changed; reading its details again"
            );
            self.details_reads.full_at = Some(now);
            return Vec::new();
        }
        let built = details::merge_progress(last, record);
        self.show_built(hash, built)
    }

    /// The record of `hash` in `answer`; `None`, noted as an issue, when it holds only others.
    fn record_of(
        &mut self,
        hash: &HashString,
        answer: TorrentGetResponse,
    ) -> Option<TorrentRecord> {
        let record = details::find_record(hash, answer);
        if record.is_none() {
            self.note_details_issues(hash, vec![DetailsIssue::NoMatchingRecord]);
        }
        record
    }

    /// Keeps the details of `hash` that a read `built`, and sends them when they changed.
    fn show_built(&mut self, hash: &HashString, built: Built) -> Vec<Event> {
        let Built { details, issues } = built;
        self.note_details_issues(hash, issues);
        if self.details.as_deref() == Some(&details) {
            return Vec::new();
        }
        let details = Arc::new(details);
        self.details = Some(Arc::clone(&details));
        vec![Event::Details(details)]
    }

    /// Logs what is wrong with the selected torrent's details when that changes, not on every
    /// read.
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
