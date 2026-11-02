//! What the user can do to the selected torrents: the actions the keys and the toolbar run, which
//! of them a request becomes, and the actions sent and not yet answered.
//!
//! A request (`TorrentRequest`) is what the user asked for; `decide` turns it into the kind of
//! action to send (`ActionKind`), or into nothing when the selection does not allow it. Space is a
//! request of its own, which becomes a start or a pause depending on the targets. Every entry point
//! (a key, a toolbar button, an item of the list's context menu) goes through `decide`, so a
//! disabled action does nothing from any of them.
//!
//! The turtle and the speed limits change the daemon's session rather than torrents; they are
//! kinds of action too (`ActionKind::AltSpeed` and `ActionKind::SpeedLimit`, the latter made only
//! from what `x4-core`'s `limit_edit` decided), sent and kept the same way, with no torrents
//! named.
//!
//! The Files tab of the details panel changes one torrent: whether its files are downloaded, their
//! priority, and whether its pieces come in order. Those are kinds of action too
//! (`ActionKind::FilesWanted`, `ActionKind::FilePriority` and `ActionKind::Sequential`), but no
//! request becomes one: the panel hands the main view a `FileRequest`, whose action on files the
//! drawn `FileTree` of `x4-core` built, so that the torrent, the files and the new value come from
//! one read of the details. A file action counts as the same action again when it is of the same
//! kind on the same torrent and the same files.
//!
//! Removing is not a request: [`RemoveTorrents`] and [`RemoveTorrentsWithData`] only open a dialog
//! that asks first (the `removal` module), and the dialog's own button sends the removal of the
//! torrents it named when it opened. Copying is not one either: [`CopyHashes`] and
//! [`CopyMagnetLinks`] write to the clipboard (the `clipboard` module) and change nothing on the
//! server.
//!
//! Each action sent is kept (`PendingActions`) under its [`ActionId`], with its kind and the
//! torrents it named, until the service says how it went. While the action sent last waits, the
//! same action on the same torrents is not sent again: a second press of its key or a double click
//! would only repeat it. An older one does not count: after Pause and then Start, Pause again is
//! the user's newest choice and goes. A success shows nothing (the next snapshot shows the
//! change); a failure is worded from the kind, the count and the error. The reads of magnet links
//! take their ids from the same counter, so that no id is ever given twice, but they are not
//! actions: one read between two presses of Pause does not let the second pause go.

use std::collections::HashMap;

use gpui_kit::{SharedString, actions};
use x4_core::{
    Action, ActionError, ActionId, Availability, Direction, FileChange, KbPerSecond, LimitEdit,
    Row, ToggleKind, toggle_kind,
};
use x4_rpc::{FilePriority, HashString, RemoveData};

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
        /// Asks whether to remove the selected torrents from the list, keeping their files; only
        /// the dialog's own button removes them.
        RemoveTorrents,
        /// Asks whether to remove the selected torrents and delete their files from the server;
        /// only the dialog's own danger button does that.
        RemoveTorrentsWithData,
        /// Copies the magnet links of the selected torrents, one per line, once the daemon has
        /// sent them.
        CopyMagnetLinks,
        /// Copies the info hashes of the selected torrents, one per line; it needs no daemon.
        CopyHashes,
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

/// An action that is sent to the daemon: on torrents, or on the daemon's speed settings.
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
    /// Remove from the list, keeping the downloaded files on the server or deleting them.
    Remove(RemoveData),
    /// Switch the alternative speed limits (the turtle) on (`true`) or off.
    AltSpeed(bool),
    /// Set the normal speed limit of `direction` to `limit`, switched on, or switch it off
    /// (`None`). Made by [`ActionKind::speed_limit`] only, from the edit `x4-core` decided.
    SpeedLimit {
        /// The limit changed.
        direction: Direction,
        /// The new limit; `None` switches it off.
        limit: Option<KbPerSecond>,
    },
    /// Download files of one torrent (`true`) or skip them. Made by [`FileRequest`] only.
    FilesWanted(bool),
    /// Give files of one torrent this priority. Made by [`FileRequest`] only.
    FilePriority(FilePriority),
    /// Download the pieces of one torrent in order (`true`) or not. Made by [`FileRequest`] only.
    Sequential(bool),
}

impl ActionKind {
    /// The action that sends `edit` of the speed limit of `direction`, as `x4-core`'s
    /// `limit_edit` decided it; `None` when the edit leaves the limit as it is.
    pub(crate) fn speed_limit(direction: Direction, edit: LimitEdit) -> Option<Self> {
        let limit = match edit {
            LimitEdit::Unchanged => return None,
            LimitEdit::Set(kbps) => Some(kbps),
            LimitEdit::Off => None,
        };
        Some(Self::SpeedLimit { direction, limit })
    }

    /// The action to send for the torrents `hashes`. A change of the speed settings names no
    /// torrents, so it takes none.
    ///
    /// `None` for a change of one torrent's files or of its downloading in order: those are not
    /// made from a list of torrents, but come built, with their files, in a [`FileRequest`].
    pub(crate) fn action(self, hashes: Vec<HashString>) -> Option<Action> {
        Some(match self {
            Self::Start => Action::Start(hashes),
            Self::Pause => Action::Stop(hashes),
            Self::StartNow => Action::StartNow(hashes),
            Self::Verify => Action::Verify(hashes),
            Self::Reannounce => Action::Reannounce(hashes),
            Self::Remove(data) => Action::Remove { hashes, data },
            Self::AltSpeed(enabled) => Action::SetAltSpeed(enabled),
            Self::SpeedLimit { direction, limit } => Action::SetSpeedLimit { direction, limit },
            Self::FilesWanted(_) | Self::FilePriority(_) | Self::Sequential(_) => return None,
        })
    }

    /// Whether `availability` allows it. The speed settings are not an action on the selected
    /// torrents, which is all a request asks for, so no request becomes one of them.
    fn allowed(self, availability: Availability) -> bool {
        match self {
            Self::Start => availability.start,
            Self::Pause => availability.pause,
            Self::StartNow => availability.start_now,
            Self::Verify => availability.verify,
            Self::Reannounce => availability.reannounce,
            Self::Remove(_) => availability.remove,
            Self::AltSpeed(_)
            | Self::SpeedLimit { .. }
            | Self::FilesWanted(_)
            | Self::FilePriority(_)
            | Self::Sequential(_) => false,
        }
    }

    /// A short name for logs, which also marks its failure's notification: one notification
    /// per name. The turtle has one name whichever way it was switched, and each speed limit one;
    /// so do the changes of files and of downloading in order, whatever value they set.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Pause => "pause",
            Self::StartNow => "start now",
            Self::Verify => "verify",
            Self::Reannounce => "reannounce",
            Self::Remove(RemoveData::Keep) => "remove",
            Self::Remove(RemoveData::Delete) => "remove with data",
            Self::AltSpeed(_) => "alternative speed",
            Self::SpeedLimit {
                direction: Direction::Down,
                ..
            } => "download limit",
            Self::SpeedLimit {
                direction: Direction::Up,
                ..
            } => "upload limit",
            Self::FilesWanted(_) => "files wanted",
            Self::FilePriority(_) => "file priority",
            Self::Sequential(_) => "sequential",
        }
    }
}

/// What the details panel asks the main view to send for its torrent: a change of its files, as
/// the drawn file tree built it, or of its downloading in order.
///
/// It is made only from what was drawn: [`FileRequest::of_files`] reads back the action a
/// `FileTree` built, and [`FileRequest::sequential`] takes the torrent of the row the switch was
/// drawn for, so the torrent and the files cannot come from different reads.
///
/// Its parts are read only through its getters, so that the torrent, the files and the action
/// cannot be changed apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileRequest {
    /// What the action does.
    kind: ActionKind,
    /// The torrent it changes.
    hash: HashString,
    /// The files it names, sorted; none for the switch of downloading in order.
    files: Vec<usize>,
    /// The action itself, as it goes to the service.
    action: Action,
}

impl FileRequest {
    /// The request that sends `action`, which a `FileTree` built; `None` when `action` changes no
    /// files.
    pub(crate) fn of_files(action: Action) -> Option<Self> {
        let target = action.file_change()?;
        let kind = match target.change {
            FileChange::Wanted(wanted) => ActionKind::FilesWanted(wanted),
            FileChange::Priority(priority) => ActionKind::FilePriority(priority),
        };
        let hash = target.hash.clone();
        let mut files = target.files.to_vec();
        files.sort_unstable();
        Some(Self {
            kind,
            hash,
            files,
            action,
        })
    }

    /// The request that switches downloading the pieces of the torrent `hash` in order on
    /// (`enabled`) or off.
    pub(crate) fn sequential(hash: HashString, enabled: bool) -> Self {
        Self {
            kind: ActionKind::Sequential(enabled),
            hash: hash.clone(),
            files: Vec::new(),
            action: Action::SetSequential { hash, enabled },
        }
    }

    /// What the action does.
    pub(crate) fn kind(&self) -> ActionKind {
        self.kind
    }

    /// The torrent it changes.
    pub(crate) fn hash(&self) -> &HashString {
        &self.hash
    }

    /// The files it names, sorted; none for the switch of downloading in order.
    pub(crate) fn files(&self) -> &[usize] {
        &self.files
    }

    /// The action itself, as it goes to the service.
    pub(crate) fn action(&self) -> &Action {
        &self.action
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
    /// How many torrents it named; for an action on files, how many files.
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

/// An action sent and not answered yet, with the torrents and the files it named.
#[derive(Debug)]
struct Sent {
    action: PendingAction,
    /// The hashes of its torrents, sorted, so that the same torrents in another order compare
    /// equal.
    targets: Vec<HashString>,
    /// The indices of the files of its torrent, sorted; none for an action on whole torrents.
    files: Vec<usize>,
}

/// The actions sent and not answered yet, by id, and the ids given out.
#[derive(Debug, Default)]
pub(crate) struct PendingActions {
    /// The id given out last, to an action or to a read of magnet links; the first is 1.
    last_id: u64,
    /// The id of the action sent last, which a request for the same action is checked against.
    /// Not `last_id`: that may be a read's.
    newest_action: Option<ActionId>,
    pending: HashMap<ActionId, Sent>,
}

impl PendingActions {
    /// A new id, never given before, for something sent to the service that is not an action,
    /// such as a read of magnet links. The action sent last stays the one a repeat is checked
    /// against.
    pub(crate) fn next_id(&mut self) -> ActionId {
        self.last_id = self.last_id.wrapping_add(1);
        ActionId(self.last_id)
    }

    /// Keeps the action `kind` on the torrents `hashes` until it is answered, under a new id,
    /// which it returns. Returns `None`, and keeps nothing, when the action sent last is the same
    /// action on the same torrents, in any order, and is still waiting for its answer. An older
    /// pending action does not count: the daemon runs the actions in order, so the newest choice
    /// must go last.
    pub(crate) fn start(&mut self, kind: ActionKind, hashes: &[HashString]) -> Option<ActionId> {
        let mut targets = hashes.to_vec();
        targets.sort_unstable();
        let count = targets.len();
        self.keep(kind, targets, Vec::new(), count)
    }

    /// [`PendingActions::start`] for the action `kind` on the files `files` of the torrent
    /// `hash`, or on the torrent alone when `files` is empty: it is the same action again only
    /// when it is of the same kind on the same torrent and the same files, in any order.
    pub(crate) fn start_files(
        &mut self,
        kind: ActionKind,
        hash: &HashString,
        files: &[usize],
    ) -> Option<ActionId> {
        let mut files = files.to_vec();
        files.sort_unstable();
        let count = files.len().max(1);
        self.keep(kind, vec![hash.clone()], files, count)
    }

    /// Keeps the action `kind` on `targets` and `files`, both sorted, which a failure counts as
    /// `count`, unless the action sent last is the same and still waits.
    fn keep(
        &mut self,
        kind: ActionKind,
        targets: Vec<HashString>,
        files: Vec<usize>,
        count: usize,
    ) -> Option<ActionId> {
        if self
            .newest_action
            .and_then(|newest| self.pending.get(&newest))
            .is_some_and(|last| {
                last.action.kind == kind && last.targets == targets && last.files == files
            })
        {
            return None;
        }
        let id = self.next_id();
        self.newest_action = Some(id);
        let action = PendingAction { kind, count };
        self.pending.insert(
            id,
            Sent {
                action,
                targets,
                files,
            },
        );
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
                tracing::debug!(?id, action, count, "the daemon accepted an action");
                None
            }
            // The engine has logged the failure at error, with its details; the user is told in a
            // notification.
            Err(error) => {
                tracing::debug!(?id, action, count, %error, "an action failed");
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
    use x4_core::test_support::{details, torrent};
    use x4_core::{
        ConnectionError, DisplayState, FileTree, NodeKey, QueueKind, Torrent, availability,
    };
    use x4_rpc::FilePriority;

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
            Some(Action::Start(hashes.clone()))
        );
        assert_eq!(
            ActionKind::Pause.action(hashes.clone()),
            Some(Action::Stop(hashes.clone()))
        );
        assert_eq!(
            ActionKind::StartNow.action(hashes.clone()),
            Some(Action::StartNow(hashes.clone()))
        );
        assert_eq!(
            ActionKind::Verify.action(hashes.clone()),
            Some(Action::Verify(hashes.clone()))
        );
        assert_eq!(
            ActionKind::Reannounce.action(hashes.clone()),
            Some(Action::Reannounce(hashes.clone()))
        );
        assert_eq!(
            ActionKind::Remove(RemoveData::Keep).action(hashes.clone()),
            Some(Action::Remove {
                hashes: hashes.clone(),
                data: RemoveData::Keep
            })
        );
        assert_eq!(
            ActionKind::Remove(RemoveData::Delete).action(hashes.clone()),
            Some(Action::Remove {
                hashes,
                data: RemoveData::Delete
            })
        );
    }

    #[test]
    fn a_speed_change_names_no_torrents_and_no_request_becomes_one() {
        let down = ActionKind::speed_limit(Direction::Down, LimitEdit::Set(KbPerSecond(524)));
        let up_off = ActionKind::speed_limit(Direction::Up, LimitEdit::Off);
        assert_eq!(
            down,
            Some(ActionKind::SpeedLimit {
                direction: Direction::Down,
                limit: Some(KbPerSecond(524)),
            })
        );
        assert_eq!(
            ActionKind::speed_limit(Direction::Down, LimitEdit::Unchanged),
            None,
            "an edit that changes nothing sends nothing"
        );
        let (Some(down), Some(up_off)) = (down, up_off) else {
            panic!("both edits change a limit: {down:?}, {up_off:?}");
        };
        assert_eq!(
            down.action(Vec::new()),
            Some(Action::SetSpeedLimit {
                direction: Direction::Down,
                limit: Some(KbPerSecond(524)),
            })
        );
        assert_eq!(
            up_off.action(Vec::new()),
            Some(Action::SetSpeedLimit {
                direction: Direction::Up,
                limit: None,
            })
        );
        assert_eq!(
            ActionKind::AltSpeed(true).action(Vec::new()),
            Some(Action::SetAltSpeed(true))
        );
        // Each limit has a notification of its own, the turtle one for both ways.
        assert_ne!(down.name(), up_off.name());
        assert_eq!(
            ActionKind::AltSpeed(true).name(),
            ActionKind::AltSpeed(false).name()
        );
        let everything = availability(true, &[&row(DisplayState::Paused)]);
        assert!(!down.allowed(everything));
        assert!(!ActionKind::AltSpeed(true).allowed(everything));
    }

    #[test]
    fn the_turtle_switched_the_same_way_again_waits_for_the_answer() {
        let mut pending = PendingActions::default();
        let Some(on) = pending.start(ActionKind::AltSpeed(true), &[]) else {
            panic!("the first switch goes");
        };
        assert_eq!(pending.start(ActionKind::AltSpeed(true), &[]), None);
        pending.finish(on, &Ok(()));
        assert!(pending.start(ActionKind::AltSpeed(true), &[]).is_some());
        assert!(
            pending.start(ActionKind::AltSpeed(false), &[]).is_some(),
            "the other way is a new choice"
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
    fn the_action_sent_last_is_not_sent_again_while_it_waits() {
        let mut pending = PendingActions::default();
        let both = [hash("a"), hash("b")];
        assert!(pending.start(ActionKind::Pause, &both).is_some());
        assert_eq!(
            pending.start(ActionKind::Pause, &[hash("b"), hash("a")]),
            None,
            "the same torrents in another order"
        );
        assert!(pending.start(ActionKind::Pause, &[hash("a")]).is_some());
        assert!(
            pending
                .start(ActionKind::Pause, &[hash("a"), hash("b"), hash("c")])
                .is_some()
        );

        // Pause, then Start, then Pause again: the older pause does not hold back the newest.
        assert!(pending.start(ActionKind::Start, &both).is_some());
        let Some(last) = pending.start(ActionKind::Pause, &both) else {
            panic!("the pause after a start goes");
        };
        assert_eq!(pending.start(ActionKind::Pause, &both), None);
        assert_eq!(pending.len(), 5);

        pending.finish(last, &Ok(()));
        assert!(
            pending.start(ActionKind::Pause, &both).is_some(),
            "answered, it can go again"
        );
    }

    #[test]
    fn an_id_for_a_read_is_new_and_leaves_the_repeat_check_alone() {
        let mut pending = PendingActions::default();
        let both = [hash("a"), hash("b")];
        let Some(pause) = pending.start(ActionKind::Pause, &both) else {
            panic!("the first pause goes");
        };
        let read = pending.next_id();
        assert_ne!(read, pause);

        assert_eq!(
            pending.start(ActionKind::Pause, &both),
            None,
            "the pause sent last still waits"
        );
        let Some(start) = pending.start(ActionKind::Start, &both) else {
            panic!("another action goes");
        };
        assert!(start != read && start != pause, "{start:?}");
        assert_eq!(pending.len(), 2, "a read is not kept as an action");
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

    fn tree(files: &[(&str, bool, FilePriority)]) -> FileTree {
        let files: Vec<(&str, u64, u64, bool, FilePriority)> = files
            .iter()
            .map(|&(name, wanted, priority)| (name, 1, 0, wanted, priority))
            .collect();
        FileTree::new(&details(&hash("a"), &files))
    }

    #[test]
    fn a_file_request_takes_its_kind_and_files_from_the_action_the_tree_built() {
        let tree = tree(&[
            ("T/b.mkv", true, FilePriority::High),
            ("T/a.mkv", true, FilePriority::Normal),
        ]);
        let folder = NodeKey::new("T");
        let toggle = tree
            .toggle_wanted_action(&folder)
            .map(FileRequest::of_files);
        let Some(Some(toggle)) = toggle else {
            panic!("a folder of files can be toggled: {toggle:?}");
        };
        assert_eq!(toggle.kind(), ActionKind::FilesWanted(false));
        assert_eq!(toggle.hash(), &hash("a"));
        assert_eq!(toggle.files, [0, 1]);
        assert_eq!(Some(toggle.action), tree.toggle_wanted_action(&folder));

        let step = tree
            .step_priority_action(&NodeKey::new("T/a.mkv"), true)
            .and_then(FileRequest::of_files);
        assert_eq!(
            step.map(|request| (request.kind(), request.files().to_vec())),
            Some((ActionKind::FilePriority(FilePriority::High), vec![1]))
        );
        // Only an action on files becomes a request on files.
        assert_eq!(FileRequest::of_files(Action::Stop(vec![hash("a")])), None);

        let sequential = FileRequest::sequential(hash("b"), true);
        assert_eq!(sequential.kind(), ActionKind::Sequential(true));
        assert_eq!(sequential.hash(), &hash("b"));
        assert_eq!(sequential.files, [] as [usize; 0]);
        assert_eq!(
            sequential.action,
            Action::SetSequential {
                hash: hash("b"),
                enabled: true
            }
        );
    }

    #[test]
    fn the_actions_on_one_torrent_are_made_with_their_request_only() {
        let everything = availability(true, &[&row(DisplayState::Paused)]);
        for kind in [
            ActionKind::FilesWanted(true),
            ActionKind::FilePriority(FilePriority::Low),
            ActionKind::Sequential(false),
        ] {
            assert!(!kind.allowed(everything), "{kind:?}");
            assert_eq!(kind.action(vec![hash("a")]), None, "{kind:?}");
        }
        // One notification for each, whichever value it set.
        assert_eq!(
            ActionKind::FilesWanted(true).name(),
            ActionKind::FilesWanted(false).name()
        );
        assert_eq!(
            ActionKind::FilePriority(FilePriority::Low).name(),
            ActionKind::FilePriority(FilePriority::High).name()
        );
        assert_eq!(
            ActionKind::Sequential(true).name(),
            ActionKind::Sequential(false).name()
        );
    }

    #[test]
    fn a_file_action_waits_for_its_answer_on_the_same_files_of_the_same_torrent() {
        let mut pending = PendingActions::default();
        let skip = ActionKind::FilesWanted(false);
        let Some(first) = pending.start_files(skip, &hash("a"), &[3, 1]) else {
            panic!("the first one goes");
        };
        assert_eq!(
            pending.start_files(skip, &hash("a"), &[1, 3]),
            None,
            "the same files in another order"
        );
        assert!(pending.start_files(skip, &hash("a"), &[1]).is_some());
        assert!(pending.start_files(skip, &hash("b"), &[1]).is_some());
        assert!(
            pending
                .start_files(ActionKind::FilesWanted(true), &hash("b"), &[1])
                .is_some()
        );
        assert!(
            pending
                .start(ActionKind::FilesWanted(true), &[hash("b")])
                .is_some(),
            "a request without files is another action"
        );
        assert_eq!(pending.len(), 5);

        let failure = pending.finish(first, &Err(ActionError::NotConnected));
        assert_eq!(
            failure.map(|failure| failure.kind),
            Some(Some(skip)),
            "a failure says what it was"
        );
    }

    /// The engine logs a failed action at error, with its details, and the user sees it in a
    /// notification: one error line is enough.
    #[test]
    fn a_failed_action_adds_no_second_error_line_to_the_log() {
        let mut pending = PendingActions::default();
        let Some(id) = pending.start(ActionKind::Pause, &[hash("a")]) else {
            panic!("the first pause goes");
        };

        let events = crate::test_log::events_of(|| {
            pending.finish(id, &Err(ActionError::Rpc(ConnectionError::Forbidden)));
        });

        assert_eq!(
            events,
            [(tracing::Level::DEBUG, "an action failed".to_owned())]
        );
    }
}
