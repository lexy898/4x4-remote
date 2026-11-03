//! The torrent to select once it shows in the list, after an add: its whole life, from the add's
//! answer to its selection, the user's own choice or the next full list.

use std::time::{Duration, Instant};

use x4_rpc::HashString;

use crate::snapshot::Snapshot;
use crate::view::ViewState;

/// How long a [`PendingSelection`] waits for its torrent: two full lists of the engine's default
/// schedule (one a minute), so that the next full list read after the add has come and gone,
/// however slow it was. The engine sends no snapshot for a full list that changed nothing, so the
/// list itself cannot be watched; its schedule can.
pub const PENDING_SELECTION_LIFE: Duration = Duration::from_secs(120);

/// The torrent to select once a snapshot lists it, after its add was answered: added, a torrent
/// the daemon had already, or an add the daemon has not confirmed in time.
///
/// The daemon's answer comes before the snapshot with the torrent, so the window waits for it by
/// hash. The wait ends:
/// - when a snapshot lists the torrent: it is selected, once, with the filter and the search
///   cleared if they would hide it ([`Reveal`]);
/// - when the user changes the selection, the filter or the search meanwhile ([`cancel`]): the
///   user's choice stands;
/// - after the next full list ([`PENDING_SELECTION_LIFE`]): a torrent not listed by then is not
///   coming, and must not jump into the selection later.
///
/// [`cancel`]: PendingSelection::cancel
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PendingSelection {
    waiting: Option<Waiting>,
}

/// The torrent waited for, and until when.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Waiting {
    hash: HashString,
    until: Instant,
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
    /// Waits for `hash` from `now` on, in place of any torrent waited for before.
    pub fn wait_for(&mut self, hash: HashString, now: Instant) {
        self.waiting = Some(Waiting {
            until: now + PENDING_SELECTION_LIFE,
            hash,
        });
    }

    /// Ends the wait, as when the user has changed the selection, the filter or the search.
    pub fn cancel(&mut self) {
        self.waiting = None;
    }

    /// The torrent waited for, if any.
    #[must_use]
    pub fn waiting(&self) -> Option<&HashString> {
        self.waiting.as_ref().map(|waiting| &waiting.hash)
    }

    /// Takes `snapshot`, which came at `now`, while the window shows `view`: the torrent to select
    /// and the view to show it in, if the snapshot lists the torrent waited for. The wait then
    /// ends, and so it does, with nothing to select, once it has lasted too long.
    pub fn follow(
        &mut self,
        snapshot: &Snapshot,
        view: &ViewState,
        now: Instant,
    ) -> Option<Reveal> {
        let waiting = self.waiting.take()?;
        if now >= waiting.until {
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

    #[test]
    fn the_torrent_is_selected_once_it_is_listed_and_only_once() {
        let start = Instant::now();
        let new = downloading("New");
        let other = torrent("Other");
        let mut pending = PendingSelection::default();
        pending.wait_for(new.hash.clone(), start);
        let view = ViewState::default();

        assert_eq!(
            pending.follow(&snapshot(vec![other.clone()]), &view, start),
            None
        );
        assert_eq!(pending.waiting(), Some(&new.hash), "it waits on");
        assert_eq!(
            pending.follow(&snapshot(vec![other.clone(), new.clone()]), &view, start),
            Some(Reveal {
                hash: new.hash.clone(),
                view: None,
            })
        );
        assert_eq!(pending.waiting(), None);
        assert_eq!(
            pending.follow(&snapshot(vec![other, new]), &view, start),
            None,
            "not again"
        );
    }

    #[test]
    fn a_view_that_hides_the_torrent_is_cleared_for_it() {
        let start = Instant::now();
        let new = downloading("New");
        let mut pending = PendingSelection::default();
        pending.wait_for(new.hash.clone(), start);
        let hiding = ViewState {
            filter: Filter::Paused,
            search: "zzz".to_owned(),
            ..ViewState::default()
        };

        let reveal = pending.follow(&snapshot(vec![new.clone()]), &hiding, start);

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
    fn the_users_own_choice_ends_the_wait() {
        let start = Instant::now();
        let new = downloading("New");
        let mut pending = PendingSelection::default();
        pending.wait_for(new.hash.clone(), start);

        pending.cancel();

        assert_eq!(pending.waiting(), None);
        assert_eq!(
            pending.follow(&snapshot(vec![new]), &ViewState::default(), start),
            None
        );
    }

    #[test]
    fn the_wait_ends_after_the_next_full_list() {
        let start = Instant::now();
        let new = downloading("New");
        let mut pending = PendingSelection::default();
        pending.wait_for(new.hash.clone(), start);
        let view = ViewState::default();
        let full_list = EngineConfig::default().full_refresh_interval;
        assert!(
            PENDING_SELECTION_LIFE >= full_list * 2,
            "the next full list comes within one interval, and may take a while"
        );

        let late = start + PENDING_SELECTION_LIFE;
        let in_time = start + PENDING_SELECTION_LIFE.saturating_sub(Duration::from_secs(1));
        assert_eq!(pending.follow(&snapshot(Vec::new()), &view, in_time), None);
        assert_eq!(pending.waiting(), Some(&new.hash), "still in time");
        assert_eq!(pending.follow(&snapshot(vec![new]), &view, late), None);
        assert_eq!(pending.waiting(), None, "too late, and over");
    }

    #[test]
    fn a_new_wait_takes_the_place_of_the_old_one() {
        let start = Instant::now();
        let first = downloading("First");
        let second = downloading("Second");
        let mut pending = PendingSelection::default();
        pending.wait_for(first.hash.clone(), start);

        pending.wait_for(second.hash.clone(), start);

        assert_eq!(
            pending.follow(
                &snapshot(vec![first, second.clone()]),
                &ViewState::default(),
                start
            ),
            Some(Reveal {
                hash: second.hash,
                view: None,
            })
        );
    }
}
