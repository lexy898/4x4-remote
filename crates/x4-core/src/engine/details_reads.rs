//! When the selected torrent's details are read, in full or only for their progress, and what
//! the answers change.
//!
//! A full read goes out once a new selection has stayed for
//! [`EngineConfig::details_settle`](super::EngineConfig::details_settle), after connecting, and
//! when the torrent's files changed, as when a magnet link got its metadata. A progress read goes
//! out every [`EngineConfig::details_interval`](super::EngineConfig::details_interval) while the
//! torrent downloads or verifies, and otherwise with each whole list; also at once after an
//! action on the torrent and when the window shows again. Nothing is read while the window is
//! hidden or nothing is selected.

use std::sync::Arc;
use std::time::Instant;

use x4_rpc::{HashString, TorrentGetResponse};

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
    /// Shows the details of `hash` from `now` on, or none.
    pub(super) fn select(&mut self, now: Instant, hash: Option<HashString>) {
        if hash == self.selected {
            return;
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

    /// The user asked for everything to be read again now.
    pub(super) fn details_refresh(&mut self) {
        if self.selected.is_some() {
            self.details_reads.progress_now = true;
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

    /// Which details read is due at `now`, if any.
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
        let wanted = reads.progress_now
            || if self.changing(hash) {
                reads.timer.is_due(now, self.config.details_interval)
            } else {
                // A whole list catches what another client changed in the files.
                full_list
            };
        // Progress is merged into the last full read; without one, it is read in full again.
        wanted.then_some(if self.details.is_some() {
            DetailsRead::Progress
        } else {
            DetailsRead::Full
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

    /// The answer to a details read came at `now`: the next progress read is measured from it,
    /// so a slow daemon rests a full interval.
    pub(super) fn details_answered_at(&mut self, now: Instant) {
        self.details_reads.timer.answered(now);
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
        let Some(record) = details::find_record(hash, answer) else {
            self.note_details_issues(hash, vec![DetailsIssue::NoMatchingRecord]);
            return Vec::new();
        };
        let Built { details, issues } = match kind {
            DetailsRead::Full => {
                self.details_reads.file_count = details::file_count(&record);
                details::build(hash.clone(), record)
            }
            DetailsRead::Progress => {
                let Some(last) = self.details.as_deref() else {
                    tracing::debug!(
                        "ignored a details progress with no full details to merge into"
                    );
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
                details::merge_progress(last, record)
            }
        };
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
