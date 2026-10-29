//! What the user can do to the selected torrents: the actions the keys and the toolbar run, which
//! of them a request becomes, and the actions sent and not yet answered.
//!
//! A request (`TorrentRequest`) is what the user asked for; `decide` turns it into the kind of
//! action to send (`ActionKind`), or into nothing when the selection does not allow it. Space is a
//! request of its own, which becomes a start or a pause depending on the targets. Every entry point
//! (a key, a toolbar button, later the context menu) goes through `decide`, so a disabled action
//! does nothing from any of them.
//!
//! Each action sent is kept (`PendingActions`) under its [`ActionId`], with its kind and the
//! torrents it named, until the service says how it went. While it waits, the same action on the
//! same torrents is not sent again: a second press of its key or a double click would only repeat
//! it. A success shows nothing (the next snapshot shows the change); a failure is worded from the
//! kind, the count and the error.

use std::collections::HashMap;

use gpui_kit::{SharedString, actions};
use x4_core::{Action, ActionError, ActionId, Availability, Row, ToggleKind, toggle_kind};
use x4_rpc::HashString;

use crate::i18n::action_failure_text;

actions!(
    x4,
    [
        /// Starts the selected torrents, or queues them when the download queue is full.
        StartTorrents,
        /// Pauses the selected torrents.
        PauseTorrents,
        /// Starts the selected torrents at once, ahead of the queue.
        StartTorrentsNow,
        /// Pauses the selected torrents when any of them runs, and starts them otherwise.
        ToggleTorrents,
        /// Checks the downloaded data of the selected torrents.
        VerifyTorrents,
        /// Asks the trackers of the selected torrents for more peers.
        ReannounceTorrents,
    ]
);

/// What the user asked to do to the selected torrents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TorrentRequest {
    /// Start.
    Start,
    /// Pause.
    Pause,
    /// Start now, ahead of the queue.
    StartNow,
    /// Space: pause when any target runs, start otherwise.
    Toggle,
    /// Verify the local data.
    Verify,
    /// Ask the trackers for more peers.
    Reannounce,
}

/// An action on torrents that is sent to the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ActionKind {
    /// Start, or queue.
    Start,
    /// Pause (the daemon's "stop").
    Pause,
    /// Start now, ahead of the queue.
    StartNow,
    /// Verify the local data.
    Verify,
    /// Ask the trackers for more peers.
    Reannounce,
}

impl ActionKind {
    /// The action to send for the torrents `hashes`.
    pub(crate) fn action(self, hashes: Vec<HashString>) -> Action {
        match self {
            Self::Start => Action::Start(hashes),
            Self::Pause => Action::Stop(hashes),
            Self::StartNow => Action::StartNow(hashes),
            Self::Verify => Action::Verify(hashes),
            Self::Reannounce => Action::Reannounce(hashes),
        }
    }

    /// Whether `availability` allows it.
    fn allowed(self, availability: Availability) -> bool {
        match self {
            Self::Start => availability.start,
            Self::Pause => availability.pause,
            Self::StartNow => availability.start_now,
            Self::Verify => availability.verify,
            Self::Reannounce => availability.reannounce,
        }
    }

    /// A short name for logs.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Pause => "pause",
            Self::StartNow => "start now",
            Self::Verify => "verify",
            Self::Reannounce => "reannounce",
        }
    }
}

/// The action that `request` becomes for `targets`, the selected rows that are shown, or `None`
/// when `availability` (of the same targets) does not allow it. Space becomes a start when every
/// target is stopped and a pause otherwise.
pub(crate) fn decide(
    request: TorrentRequest,
    targets: &[&Row],
    availability: Availability,
) -> Option<ActionKind> {
    let kind = match request {
        TorrentRequest::Start => ActionKind::Start,
        TorrentRequest::Pause => ActionKind::Pause,
        TorrentRequest::StartNow => ActionKind::StartNow,
        TorrentRequest::Verify => ActionKind::Verify,
        TorrentRequest::Reannounce => ActionKind::Reannounce,
        TorrentRequest::Toggle => match toggle_kind(targets)? {
            ToggleKind::Start => ActionKind::Start,
            ToggleKind::Pause => ActionKind::Pause,
        },
    };
    kind.allowed(availability).then_some(kind)
}

/// An action sent and not answered yet: what it was and how many torrents it named, which is
/// what a failure says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PendingAction {
    /// What the action does.
    pub(crate) kind: ActionKind,
    /// How many torrents it named.
    pub(crate) count: usize,
}

/// A failed action, as its notification shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failure {
    /// What the action did; `None` when that is not known.
    pub(crate) kind: Option<ActionKind>,
    /// What the notification says.
    pub(crate) text: SharedString,
}

/// An action sent and not answered yet, with the torrents it named.
#[derive(Debug)]
struct Sent {
    action: PendingAction,
    /// The hashes of its torrents, sorted, so that the same torrents in another order compare
    /// equal.
    targets: Vec<HashString>,
}

/// The actions sent and not answered yet, by id, and the next id to give out.
#[derive(Debug, Default)]
pub(crate) struct PendingActions {
    /// The id of the previous action; the first is 1.
    last_id: u64,
    pending: HashMap<ActionId, Sent>,
}

impl PendingActions {
    /// Keeps the action `kind` on the torrents `hashes` until it is answered, under a new id,
    /// which it returns. Returns `None`, and keeps nothing, when the same action on the same
    /// torrents, in any order, is still waiting for its answer.
    pub(crate) fn start(&mut self, kind: ActionKind, hashes: &[HashString]) -> Option<ActionId> {
        let mut targets = hashes.to_vec();
        targets.sort_unstable();
        if self
            .pending
            .values()
            .any(|sent| sent.action.kind == kind && sent.targets == targets)
        {
            return None;
        }
        self.last_id = self.last_id.wrapping_add(1);
        let id = ActionId(self.last_id);
        let action = PendingAction {
            kind,
            count: targets.len(),
        };
        self.pending.insert(id, Sent { action, targets });
        Some(id)
    }

    /// Forgets the action `id`, which the service has answered with `result`, and returns the
    /// failure: `None` on success. The answer to an action that is not pending (which never
    /// happens) is worded without its kind.
    pub(crate) fn finish(
        &mut self,
        id: ActionId,
        result: &Result<(), ActionError>,
    ) -> Option<Failure> {
        let pending = self.pending.remove(&id).map(|sent| sent.action);
        let kind = pending.map(|pending| pending.kind);
        let action = kind.map_or("unknown", ActionKind::name);
        let count = pending.map_or(0, |pending| pending.count);
        if pending.is_none() {
            tracing::warn!(?id, "an action was answered that was not pending");
        }
        match result {
            Ok(()) => {
                tracing::debug!(?id, action, count, "the daemon accepted a torrent action");
                None
            }
            Err(error) => {
                tracing::error!(?id, action, count, %error, "torrent action failed");
                Some(Failure {
                    kind,
                    text: action_failure_text(pending, error),
                })
            }
        }
    }

    /// How many actions are waiting for their answer.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod tests {
    use x4_core::test_support::torrent;
    use x4_core::{ConnectionError, DisplayState, QueueKind, Torrent, availability};

    use super::*;

    fn row(state: DisplayState) -> Row {
        Row::from(&Torrent {
            state,
            ..torrent("a")
        })
    }

    /// What `request` becomes for rows in `states`, with the daemon `connected` or not.
    fn decided(
        request: TorrentRequest,
        states: &[DisplayState],
        connected: bool,
    ) -> Option<ActionKind> {
        let rows: Vec<Row> = states.iter().copied().map(row).collect();
        let targets: Vec<&Row> = rows.iter().collect();
        decide(request, &targets, availability(connected, &targets))
    }

    #[test]
    fn each_request_becomes_its_action_when_allowed() {
        let mixed = [DisplayState::Paused, DisplayState::Downloading];
        for (request, kind) in [
            (TorrentRequest::Start, ActionKind::Start),
            (TorrentRequest::Pause, ActionKind::Pause),
            (TorrentRequest::StartNow, ActionKind::StartNow),
            (TorrentRequest::Verify, ActionKind::Verify),
            (TorrentRequest::Reannounce, ActionKind::Reannounce),
            (TorrentRequest::Toggle, ActionKind::Pause),
        ] {
            assert_eq!(decided(request, &mixed, true), Some(kind), "{request:?}");
            assert_eq!(decided(request, &mixed, false), None, "{request:?} offline");
            assert_eq!(
                decided(request, &[], true),
                None,
                "{request:?} without targets"
            );
        }
    }

    #[test]
    fn space_starts_only_when_every_target_is_stopped() {
        let stopped = [DisplayState::Paused, DisplayState::Error];
        assert_eq!(
            decided(TorrentRequest::Toggle, &stopped, true),
            Some(ActionKind::Start)
        );
        let queued = [DisplayState::Paused, DisplayState::Queued(QueueKind::Seed)];
        assert_eq!(
            decided(TorrentRequest::Toggle, &queued, true),
            Some(ActionKind::Pause)
        );
    }

    #[test]
    fn a_request_the_targets_do_not_allow_becomes_nothing() {
        let seeding = [DisplayState::Seeding];
        assert_eq!(decided(TorrentRequest::Start, &seeding, true), None);
        assert_eq!(decided(TorrentRequest::StartNow, &seeding, true), None);
        let paused = [DisplayState::Paused];
        assert_eq!(decided(TorrentRequest::Pause, &paused, true), None);
        assert_eq!(decided(TorrentRequest::Reannounce, &paused, true), None);
    }

    #[test]
    fn every_kind_sends_its_action_with_the_hashes_in_order() {
        let hashes = vec![torrent("b").hash, torrent("a").hash];
        assert_eq!(
            ActionKind::Start.action(hashes.clone()),
            Action::Start(hashes.clone())
        );
        assert_eq!(
            ActionKind::Pause.action(hashes.clone()),
            Action::Stop(hashes.clone())
        );
        assert_eq!(
            ActionKind::StartNow.action(hashes.clone()),
            Action::StartNow(hashes.clone())
        );
        assert_eq!(
            ActionKind::Verify.action(hashes.clone()),
            Action::Verify(hashes.clone())
        );
        assert_eq!(
            ActionKind::Reannounce.action(hashes.clone()),
            Action::Reannounce(hashes)
        );
    }

    fn hash(name: &str) -> HashString {
        torrent(name).hash
    }

    #[test]
    fn pending_actions_get_new_ids_and_are_forgotten_when_answered() {
        let mut pending = PendingActions::default();
        let first = pending.start(ActionKind::Start, &[hash("a"), hash("b")]);
        let second = pending.start(ActionKind::Verify, &[hash("a")]);
        let (Some(first), Some(second)) = (first, second) else {
            panic!("both actions are new: {first:?}, {second:?}");
        };
        assert_ne!(first, second);
        assert_eq!(pending.len(), 2);

        assert_eq!(
            pending.finish(first, &Ok(())),
            None,
            "a success says nothing"
        );
        assert_eq!(pending.len(), 1);
        let failure = pending.finish(second, &Err(ActionError::NotConnected));
        assert_eq!(
            failure,
            Some(Failure {
                kind: Some(ActionKind::Verify),
                text: "Could not verify the torrent: the server is not connected, so nothing \
                       was sent"
                    .into(),
            })
        );
        assert_eq!(pending.len(), 0);
    }

    #[test]
    fn the_same_action_on_the_same_torrents_waits_for_its_answer() {
        let mut pending = PendingActions::default();
        let Some(first) = pending.start(ActionKind::Pause, &[hash("a"), hash("b")]) else {
            panic!("the first pause is new");
        };
        assert_eq!(
            pending.start(ActionKind::Pause, &[hash("b"), hash("a")]),
            None,
            "the same torrents in another order"
        );
        assert!(
            pending
                .start(ActionKind::Verify, &[hash("a"), hash("b")])
                .is_some()
        );
        assert!(pending.start(ActionKind::Pause, &[hash("a")]).is_some());
        assert!(
            pending
                .start(ActionKind::Pause, &[hash("a"), hash("b"), hash("c")])
                .is_some()
        );
        assert_eq!(pending.len(), 4);

        pending.finish(first, &Ok(()));
        assert!(
            pending
                .start(ActionKind::Pause, &[hash("a"), hash("b")])
                .is_some(),
            "answered, it can go again"
        );
    }

    #[test]
    fn a_failure_of_an_unknown_action_is_still_shown() {
        let mut pending = PendingActions::default();
        let failure = pending.finish(
            ActionId(99),
            &Err(ActionError::Rpc(ConnectionError::Forbidden)),
        );
        assert_eq!(
            failure,
            Some(Failure {
                kind: None,
                text: "An action failed: access denied".into(),
            })
        );
    }
}
