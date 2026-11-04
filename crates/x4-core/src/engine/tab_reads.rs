//! The reads of the Peers and the Trackers tab: which tab is shown, when the selected torrent's
//! peers or trackers are read, and what the answers change.
//!
//! The tab shown decides the read: the peers every
//! [`EngineConfig::peers_interval`](super::EngineConfig::peers_interval) while the Peers tab is
//! shown, the trackers every
//! [`EngineConfig::trackers_interval`](super::EngineConfig::trackers_interval) while the Trackers
//! tab is, each counted from the answer, and nothing for the other tabs. They go out only for the
//! one selected torrent, while the window is visible and once its full read has gone out, so not
//! while the selection moves; a tab is read at once when it opens, after a reconnect and after a
//! refresh, and the trackers once more right after a reannounce of the torrent. The trackers are
//! merged into the details, so they also wait for the details to arrive.
//!
//! The peers are not part of the details: the engine keeps the last list it sent as
//! [`Event::Peers`], sends a list only when it differs from that one, and drops it with
//! [`Event::PeersCleared`] when the tab is left or the selection changes.

use std::sync::Arc;
use std::time::Instant;

use x4_rpc::{HashString, TorrentGetResponse};

use super::schedule::Timer;
use super::{DetailsRead, Engine, Event, PollKind, Request, Visibility};
use crate::action::Action;
use crate::details;
use crate::peers::build_peers;
use crate::ui_state::DetailsTab;

/// When the reads of the tabs are due.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct TabReads {
    /// The last read of the peers.
    peers: Timer,
    /// The last read of the trackers.
    trackers: Timer,
    /// The batch the last read of the trackers went out in.
    trackers_batch: Option<u64>,
}

impl Engine {
    /// Shows the tab `tab` from now on; leaving the Peers tab drops the peers. The tab opened is
    /// read at once.
    pub(super) fn show_tab(&mut self, tab: DetailsTab) -> Vec<Event> {
        if tab == self.tab {
            return Vec::new();
        }
        self.tab = tab;
        match tab {
            DetailsTab::Peers => self.tab_reads.peers = Timer::default(),
            DetailsTab::Trackers => self.tab_reads.trackers = Timer::default(),
            DetailsTab::General | DetailsTab::Files => {}
        }
        self.drop_peers()
    }

    /// Another torrent, or none, is selected: the tab shown is read as soon as its full read has
    /// gone out, and the peers of the torrent before go.
    pub(super) fn tabs_selected(&mut self) -> Vec<Event> {
        self.tab_reads = TabReads::default();
        self.refused.answered(PollKind::Peers);
        self.refused.answered(PollKind::Trackers);
        self.drop_peers()
    }

    /// The daemon was connected to again: the tab shown is read with the full read. The peers
    /// shown stay until then.
    pub(super) fn tabs_connected(&mut self) {
        self.tab_reads = TabReads::default();
    }

    /// The user asked for everything to be read again: the tab shown too.
    pub(super) fn tabs_refresh(&mut self) {
        self.tab_reads.peers.force();
        self.tab_reads.trackers.force();
    }

    /// `action`, handed out in batch `sent_in`, was carried out. A reannounce of the selected
    /// torrent while the Trackers tab is shown reads the trackers at once, unless a read of them
    /// went out with the action or after it, which already shows it.
    pub(super) fn tabs_after_action(&mut self, action: &Action, sent_in: Option<u64>) {
        let reannounced = matches!(action, Action::Reannounce(hashes)
            if self.selected.as_ref().is_some_and(|hash| hashes.contains(hash)));
        let seen = sent_in
            .zip(self.tab_reads.trackers_batch)
            .is_some_and(|(action_batch, read_batch)| read_batch >= action_batch);
        if reannounced && self.tab == DetailsTab::Trackers && !seen {
            self.tab_reads.trackers.force();
        }
    }

    /// The read of the tab shown that is due at `now`, if any, which is then counted as sent.
    pub(super) fn due_tab_read(&mut self, now: Instant) -> Option<Request> {
        let kind = self.tab_read_due(now)?;
        let hash = self.selected.clone()?;
        let reads = &mut self.tab_reads;
        match kind {
            DetailsRead::Peers => reads.peers.sent(now),
            DetailsRead::Trackers => {
                reads.trackers.sent(now);
                reads.trackers_batch = Some(self.batch);
            }
            DetailsRead::Full | DetailsRead::Progress | DetailsRead::ProgressAndTrackers => {}
        }
        Some(Request::Details { hash, kind })
    }

    /// Which read of the tab shown is due at `now`, if any.
    fn tab_read_due(&self, now: Instant) -> Option<DetailsRead> {
        let (kind, at) = self.next_tab_read_of(now)?;
        (at <= now).then_some(kind)
    }

    /// When the next read of the tab shown is due, if one is wanted now.
    pub(super) fn next_tab_read(&self, now: Instant) -> Option<Instant> {
        self.next_tab_read_of(now).map(|(_, at)| at)
    }

    /// The next read of the tab shown and when it is due: only for a selected torrent, while the
    /// window is visible and once its full read has gone out; the trackers only once there are
    /// details to merge them into.
    fn next_tab_read_of(&self, now: Instant) -> Option<(DetailsRead, Instant)> {
        self.selected.as_ref()?;
        if self.visibility != Visibility::Visible || self.details_unsettled() {
            return None;
        }
        let reads = &self.tab_reads;
        match self.tab {
            DetailsTab::Peers => reads
                .peers
                .due_at(now, self.config.peers_interval)
                .map(|at| (DetailsRead::Peers, at)),
            DetailsTab::Trackers if self.details.is_some() => reads
                .trackers
                .due_at(now, self.config.trackers_interval)
                .map(|at| (DetailsRead::Trackers, at)),
            DetailsTab::Trackers | DetailsTab::General | DetailsTab::Files => None,
        }
    }

    /// The answer to a `kind` read of a tab came at `now`: the next one is measured from it.
    pub(super) fn tab_read_answered_at(&mut self, kind: DetailsRead, now: Instant) {
        match kind {
            DetailsRead::Peers => self.tab_reads.peers.answered(now),
            DetailsRead::Trackers => self.tab_reads.trackers.answered(now),
            DetailsRead::Full | DetailsRead::Progress | DetailsRead::ProgressAndTrackers => {}
        }
    }

    /// Whether the answer to a `kind` read is still wanted with the tab shown now: a read of the
    /// peers or the trackers only while its tab is.
    pub(super) fn tab_shows(&self, kind: DetailsRead) -> bool {
        match kind {
            DetailsRead::Peers => self.tab == DetailsTab::Peers,
            DetailsRead::Trackers => self.tab == DetailsTab::Trackers,
            DetailsRead::Full | DetailsRead::Progress | DetailsRead::ProgressAndTrackers => true,
        }
    }

    /// Takes the peers of `hash`, which is still selected, read while the Peers tab is shown;
    /// sends them when they differ from the last ones sent.
    pub(super) fn peers_read(
        &mut self,
        hash: &HashString,
        answer: &TorrentGetResponse,
    ) -> Vec<Event> {
        let Some(list) = build_peers(hash, answer) else {
            tracing::debug!(torrent = %hash, "a peers answer without the selected torrent");
            return Vec::new();
        };
        if self.peers.as_deref() == Some(&list) {
            return Vec::new();
        }
        let list = Arc::new(list);
        self.peers = Some(Arc::clone(&list));
        vec![Event::Peers(list)]
    }

    /// Takes the trackers of `hash`, which is still selected, read while the Trackers tab is
    /// shown: they replace those of its details, and the details go out again when that changed
    /// them.
    pub(super) fn trackers_read(
        &mut self,
        hash: &HashString,
        answer: &TorrentGetResponse,
    ) -> Vec<Event> {
        let Some(last) = self.details.as_deref() else {
            tracing::debug!(torrent = %hash, "ignored trackers with no details to merge into");
            return Vec::new();
        };
        let mut merged = last.clone();
        details::merge_trackers(&mut merged, answer);
        if merged == *last {
            return Vec::new();
        }
        let merged = Arc::new(merged);
        self.details = Some(Arc::clone(&merged));
        vec![Event::Details(merged)]
    }

    /// Lets go of the peers sent last; tells the UI so, if any were sent.
    fn drop_peers(&mut self) -> Vec<Event> {
        match self.peers.take() {
            Some(_) => vec![Event::PeersCleared],
            None => Vec::new(),
        }
    }
}
