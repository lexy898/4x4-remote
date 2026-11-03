//! The torrent to select once it shows in the list, after an add: its whole life, from the moment
//! the add is sent to its selection, the user's own choice or the next full list.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use x4_rpc::HashString;

use crate::action::ActionId;
use crate::snapshot::Snapshot;
use crate::view::ViewState;

/// How long a [`PendingSelection`] waits for its torrent: two full lists of the engine's default
/// schedule (one a minute), so that the next full list read after the add has come and gone,
/// however slow it was. The engine sends no snapshot for a full list that changed nothing, so the
/// list itself cannot be watched; its schedule can.
pub const PENDING_SELECTION_LIFE: Duration = Duration::from_secs(120);

/// What the user has chosen in the list so far, as counts that only grow: a new count says that
/// the user has made a choice since, even one undone afterwards, such as a filter chosen and
/// chosen back.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ListChoices {
    /// How many times the user has acted on the selection, with the mouse or a key, whether it
    /// changed or not: a click on the row selected already is a choice too.
    pub selection_gestures: u64,
    /// How many times the user has changed the filter, the search or the sort order.
    pub view_changes: u64,
}

/// The torrent to select once a snapshot lists it, after its add was answered: added, a torrent
/// the daemon had already, or an add the daemon has not confirmed in time.
///
/// The window tells it the user's [`ListChoices`] when an add is sent ([`sent`]) and when the
/// daemon answers it ([`answered`]). The torrent is waited for only if the user has made no choice
/// in between, as after Cancel while the add was out; the daemon's answer comes before the
/// snapshot with the torrent, so the wait is by hash. It ends:
/// - when a snapshot lists the torrent: it is selected, once, with the filter and the search
///   cleared if they would hide it ([`Reveal`]);
/// - when the user makes a choice in the list meanwhile (the choices differ at [`follow`]): the
///   user's choice stands;
/// - after the next full list ([`PENDING_SELECTION_LIFE`]): a torrent not listed by then is not
///   coming, and must not jump into the selection later.
///
/// What the window itself does to show the torrent is not a choice of the user's, and the window
/// counts none for it.
///
/// [`sent`]: PendingSelection::sent
/// [`answered`]: PendingSelection::answered
/// [`follow`]: PendingSelection::follow
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PendingSelection {
    /// The adds out, with the user's choices when each was sent.
    sent: HashMap<ActionId, ListChoices>,
    /// The torrent waited for.
    waiting: Option<Waiting>,
}

/// The torrent waited for, until when, and the user's choices when the wait began.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Waiting {
    hash: HashString,
    until: Instant,
    choices: ListChoices,
}

/// What selecting the torrent waited for takes: its hash, and the view to show it in, if the
/// current one hides it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reveal {
    /// The torrent to select.
    pub hash: HashString,
    /// The view with the filter and the search that hid it cleared; `None` when the view shows it.
    pub view: Option<ViewState>,
}

impl PendingSelection {
    /// Notes that the add `id` was sent while the user's choices were `choices`.
    pub fn sent(&mut self, id: ActionId, choices: ListChoices) {
        self.sent.insert(id, choices);
    }

    /// Takes the daemon's answer to the add `id`, which came at `now` with the user's choices at
    /// `choices`: waits for `select`, the torrent the answer names, if the user has made no choice
    /// since the add was sent, in place of any torrent waited for before. Returns whether it waits
    /// for it.
    pub fn answered(
        &mut self,
        id: ActionId,
        select: Option<HashString>,
        choices: ListChoices,
        now: Instant,
    ) -> bool {
        let unchanged = self.sent.remove(&id) == Some(choices);
        match select {
            Some(hash) if unchanged => {
                self.wait_for(hash, choices, now);
                true
            }
            _ => false,
        }
    }

    /// Waits for `hash` from `now` on, with the user's choices at `choices`, in place of any
    /// torrent waited for before: for a torrent the user asked to be shown.
    pub fn wait_for(&mut self, hash: HashString, choices: ListChoices, now: Instant) {
        self.waiting = Some(Waiting {
            hash,
            until: now + PENDING_SELECTION_LIFE,
            choices,
        });
    }

    /// The torrent waited for, if any.
    #[must_use]
    pub fn waiting(&self) -> Option<&HashString> {
        self.waiting.as_ref().map(|waiting| &waiting.hash)
    }

    /// Takes `snapshot`, which came at `now` while the window shows `view` and the user's choices
    /// are `choices`: the torrent to select and the view to show it in, if the snapshot lists the
    /// torrent waited for. The wait then ends, and so it does, with nothing to select, once it has
    /// lasted too long or the user has made a choice.
    pub fn follow(
        &mut self,
        snapshot: &Snapshot,
        view: &ViewState,
        choices: ListChoices,
        now: Instant,
    ) -> Option<Reveal> {
        let waiting = self.waiting.take()?;
        if now >= waiting.until || choices != waiting.choices {
            return None;
        }
        let Some(torrent) = snapshot
            .torrents
            .iter()
            .find(|torrent| torrent.hash == waiting.hash)
        else {
            self.waiting = Some(waiting);
            return None;
        };
        Some(Reveal {
            view: view.revealing(torrent),
            hash: waiting.hash,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EngineConfig;
    use crate::filter::Filter;
    use crate::model::Torrent;
    use crate::snapshot::ConnectionState;
    use crate::state::DisplayState;
    use crate::test_support::torrent;

    fn snapshot(torrents: Vec<Torrent>) -> Snapshot {
        Snapshot {
            torrents,
            listed: true,
            session: None,
            free_space_bytes: None,
            rate_down: 0,
            rate_up: 0,
            connection: ConnectionState::Connected,
            stale: false,
        }
    }

    fn downloading(name: &str) -> Torrent {
        Torrent {
            state: DisplayState::Downloading,
            ..torrent(name)
        }
    }

    const NONE: ListChoices = ListChoices {
        selection_gestures: 0,
        view_changes: 0,
    };

    #[test]
    fn the_torrent_is_selected_once_it_is_listed_and_only_once() {
        let start = Instant::now();
        let new = downloading("New");
        let other = torrent("Other");
        let mut pending = PendingSelection::default();
        pending.sent(ActionId(1), NONE);
        assert!(pending.answered(ActionId(1), Some(new.hash.clone()), NONE, start));
        let view = ViewState::default();

        assert_eq!(
            pending.follow(&snapshot(vec![other.clone()]), &view, NONE, start),
            None
        );
        assert_eq!(pending.waiting(), Some(&new.hash), "it waits on");
        assert_eq!(
            pending.follow(
                &snapshot(vec![other.clone(), new.clone()]),
                &view,
                NONE,
                start
            ),
            Some(Reveal {
                hash: new.hash.clone(),
                view: None,
            })
        );
        assert_eq!(pending.waiting(), None);
        assert_eq!(
            pending.follow(&snapshot(vec![other, new]), &view, NONE, start),
            None,
            "not again"
        );
    }

    #[test]
    fn a_view_that_hides_the_torrent_is_cleared_for_it() {
        let start = Instant::now();
        let new = downloading("New");
        let mut pending = PendingSelection::default();
        pending.wait_for(new.hash.clone(), NONE, start);
        let hiding = ViewState {
            filter: Filter::Paused,
            search: "zzz".to_owned(),
            ..ViewState::default()
        };

        let reveal = pending.follow(&snapshot(vec![new.clone()]), &hiding, NONE, start);

        assert_eq!(
            reveal,
            Some(Reveal {
                hash: new.hash,
                view: Some(ViewState {
                    filter: Filter::All,
                    search: String::new(),
                    ..hiding
                }),
            })
        );
    }

    #[test]
    fn a_choice_made_while_the_add_is_out_means_no_wait() {
        let start = Instant::now();
        let new = downloading("New");
        let view = ViewState::default();
        for since in [
            ListChoices {
                selection_gestures: 1,
                ..NONE
            },
            ListChoices {
                view_changes: 1,
                ..NONE
            },
        ] {
            let mut pending = PendingSelection::default();
            pending.sent(ActionId(1), NONE);

            assert!(!pending.answered(ActionId(1), Some(new.hash.clone()), since, start));

            assert_eq!(pending.waiting(), None);
            assert_eq!(
                pending.follow(&snapshot(vec![new.clone()]), &view, since, start),
                None
            );
        }
    }

    #[test]
    fn a_choice_made_during_the_wait_ends_it() {
        let start = Instant::now();
        let new = downloading("New");
        let view = ViewState::default();
        for later in [
            ListChoices {
                selection_gestures: 1,
                ..NONE
            },
            ListChoices {
                view_changes: 1,
                ..NONE
            },
        ] {
            let mut pending = PendingSelection::default();
            pending.wait_for(new.hash.clone(), NONE, start);

            assert_eq!(
                pending.follow(&snapshot(vec![new.clone()]), &view, later, start),
                None
            );
            assert_eq!(pending.waiting(), None, "over");
            assert_eq!(
                pending.follow(&snapshot(vec![new.clone()]), &view, NONE, start),
                None
            );
        }
    }

    #[test]
    fn an_answer_without_a_torrent_or_an_add_not_sent_waits_for_nothing() {
        let start = Instant::now();
        let new = downloading("New");
        let mut pending = PendingSelection::default();
        pending.sent(ActionId(1), NONE);

        assert!(!pending.answered(ActionId(1), None, NONE, start));
        assert!(
            !pending.answered(ActionId(1), Some(new.hash.clone()), NONE, start),
            "answered already"
        );
        assert!(!pending.answered(ActionId(2), Some(new.hash), NONE, start));
        assert_eq!(pending.waiting(), None);
    }

    #[test]
    fn the_wait_ends_after_the_next_full_list() {
        let start = Instant::now();
        let new = downloading("New");
        let mut pending = PendingSelection::default();
        pending.wait_for(new.hash.clone(), NONE, start);
        let view = ViewState::default();
        let full_list = EngineConfig::default().full_refresh_interval;
        assert!(
            PENDING_SELECTION_LIFE >= full_list * 2,
            "the next full list comes within one interval, and may take a while"
        );

        let late = start + PENDING_SELECTION_LIFE;
        let in_time = start + PENDING_SELECTION_LIFE.saturating_sub(Duration::from_secs(1));
        assert_eq!(
            pending.follow(&snapshot(Vec::new()), &view, NONE, in_time),
            None
        );
        assert_eq!(pending.waiting(), Some(&new.hash), "still in time");
        assert_eq!(
            pending.follow(&snapshot(vec![new]), &view, NONE, late),
            None
        );
        assert_eq!(pending.waiting(), None, "too late, and over");
    }

    #[test]
    fn the_newest_answer_takes_the_place_of_the_wait_before() {
        let start = Instant::now();
        let first = downloading("First");
        let second = downloading("Second");
        let mut pending = PendingSelection::default();
        pending.sent(ActionId(1), NONE);
        pending.sent(ActionId(2), NONE);
        assert!(pending.answered(ActionId(1), Some(first.hash.clone()), NONE, start));

        assert!(pending.answered(ActionId(2), Some(second.hash.clone()), NONE, start));

        assert_eq!(
            pending.follow(
                &snapshot(vec![first, second.clone()]),
                &ViewState::default(),
                NONE,
                start
            ),
            Some(Reveal {
                hash: second.hash,
                view: None,
            })
        );
    }
}
