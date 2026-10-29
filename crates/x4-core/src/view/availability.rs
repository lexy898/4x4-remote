//! Which actions the selected rows allow.

use crate::state::DisplayState;

use super::row::Row;

/// Which actions are enabled for the selected rows, for the toolbar, the context menu and the
/// keys alike. A disabled action does nothing from any of them.
///
/// Everything is `false` without targets. Offline, only `copy_hash` is available: it needs no
/// daemon.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "one flag per action; the toolbar, the menu and the keys each read them by name"
)]
pub struct Availability {
    /// Start: some target is paused or has failed.
    pub start: bool,
    /// Pause: some target is not stopped already.
    pub pause: bool,
    /// Start now, skipping the queue: some target is stopped or waiting in a queue.
    pub start_now: bool,
    /// Verify local data.
    pub verify: bool,
    /// Ask the trackers for peers: some target is downloading or seeding.
    pub reannounce: bool,
    /// Remove from the list, with or without the files.
    pub remove: bool,
    /// Copy the info hashes.
    pub copy_hash: bool,
    /// Copy the magnet links.
    pub copy_magnet: bool,
}

/// What the Space key and a start/pause toggle button do to the targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToggleKind {
    /// Start the targets: all of them are stopped.
    Start,
    /// Pause the targets: some of them run.
    Pause,
}

/// The actions that `targets` allow.
///
/// `connected` is whether the daemon is reachable; the actions are sent to it, so all but
/// `copy_hash` need it.
#[must_use]
pub fn availability(connected: bool, targets: &[&Row]) -> Availability {
    if targets.is_empty() {
        return Availability::default();
    }
    let any = |test: fn(DisplayState) -> bool| targets.iter().any(|row| test(row.state));
    Availability {
        start: connected && any(is_stopped),
        pause: connected && any(|state| !is_stopped(state)),
        start_now: connected
            && any(|state| is_stopped(state) || matches!(state, DisplayState::Queued(_))),
        verify: connected,
        reannounce: connected
            && any(|state| matches!(state, DisplayState::Downloading | DisplayState::Seeding)),
        remove: connected,
        copy_hash: true,
        copy_magnet: connected,
    }
}

/// Whether Space starts or pauses `targets`: it pauses when any of them runs, and starts
/// otherwise. `None` without targets. It does not look at the connection: whether the action is
/// allowed is [`availability()`]'s answer.
#[must_use]
pub fn toggle_kind(targets: &[&Row]) -> Option<ToggleKind> {
    if targets.is_empty() {
        None
    } else if targets.iter().any(|row| !is_stopped(row.state)) {
        Some(ToggleKind::Pause)
    } else {
        Some(ToggleKind::Start)
    }
}

/// Whether the torrent is stopped, by the user or by an error: it does not run until started.
fn is_stopped(state: DisplayState) -> bool {
    matches!(state, DisplayState::Paused | DisplayState::Error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{DisplayState, QueueKind};
    use crate::test_support::torrent;

    fn row_in(state: DisplayState) -> Row {
        Row::from(&crate::model::Torrent {
            state,
            ..torrent("a")
        })
    }

    /// The availability, when connected, of rows in `states`.
    fn online(states: &[DisplayState]) -> Availability {
        let rows: Vec<Row> = states.iter().copied().map(row_in).collect();
        let targets: Vec<&Row> = rows.iter().collect();
        availability(true, &targets)
    }

    const NOTHING: Availability = Availability {
        start: false,
        pause: false,
        start_now: false,
        verify: false,
        reannounce: false,
        remove: false,
        copy_hash: false,
        copy_magnet: false,
    };

    #[test]
    fn availability_is_all_false_without_targets() {
        assert_eq!(availability(true, &[]), NOTHING);
        assert_eq!(availability(false, &[]), NOTHING);
        assert_eq!(Availability::default(), NOTHING);
    }

    #[test]
    fn only_copy_hash_works_offline() {
        let rows = [
            row_in(DisplayState::Paused),
            row_in(DisplayState::Downloading),
        ];
        let targets: Vec<&Row> = rows.iter().collect();

        assert_eq!(
            availability(false, &targets),
            Availability {
                copy_hash: true,
                ..NOTHING
            }
        );

        // Online, the same targets allow everything.
        assert_eq!(
            availability(true, &targets),
            Availability {
                start: true,
                pause: true,
                start_now: true,
                verify: true,
                reannounce: true,
                remove: true,
                copy_hash: true,
                copy_magnet: true,
            }
        );
    }

    #[test]
    fn start_is_available_for_paused_and_error_rows() {
        for state in [DisplayState::Paused, DisplayState::Error] {
            let available = online(&[state]);
            assert!(available.start, "{state:?}");
            assert!(available.start_now, "{state:?}");
            assert!(!available.pause, "{state:?}");
        }
        // One stopped row among running ones is enough.
        assert!(online(&[DisplayState::Seeding, DisplayState::Paused]).start);
        for state in [
            DisplayState::Downloading,
            DisplayState::Seeding,
            DisplayState::Checking,
            DisplayState::Queued(QueueKind::Download),
            DisplayState::Queued(QueueKind::Seed),
        ] {
            assert!(!online(&[state]).start, "{state:?}");
        }
    }

    #[test]
    fn pause_is_available_for_running_rows() {
        for state in [
            DisplayState::Downloading,
            DisplayState::Seeding,
            DisplayState::Queued(QueueKind::Download),
            DisplayState::Queued(QueueKind::Seed),
            DisplayState::Checking,
        ] {
            assert!(online(&[state]).pause, "{state:?}");
        }
        assert!(online(&[DisplayState::Paused, DisplayState::Downloading]).pause);
        assert!(!online(&[DisplayState::Paused, DisplayState::Error]).pause);
    }

    #[test]
    fn start_now_also_skips_the_queue() {
        for state in [
            DisplayState::Queued(QueueKind::Download),
            DisplayState::Queued(QueueKind::Seed),
        ] {
            let available = online(&[state]);
            assert!(available.start_now, "{state:?}");
            assert!(
                !available.start,
                "a queued torrent is already started: {state:?}"
            );
        }
        for state in [
            DisplayState::Downloading,
            DisplayState::Seeding,
            DisplayState::Checking,
        ] {
            assert!(!online(&[state]).start_now, "{state:?}");
        }
    }

    #[test]
    fn reannounce_needs_an_active_row() {
        assert!(online(&[DisplayState::Downloading]).reannounce);
        assert!(online(&[DisplayState::Seeding]).reannounce);
        assert!(online(&[DisplayState::Paused, DisplayState::Seeding]).reannounce);
        for state in [
            DisplayState::Paused,
            DisplayState::Error,
            DisplayState::Checking,
            DisplayState::Queued(QueueKind::Download),
            DisplayState::Queued(QueueKind::Seed),
        ] {
            assert!(!online(&[state]).reannounce, "{state:?}");
        }
    }

    #[test]
    fn verify_remove_and_copy_work_for_any_row_when_online() {
        for state in [
            DisplayState::Downloading,
            DisplayState::Seeding,
            DisplayState::Paused,
            DisplayState::Queued(QueueKind::Seed),
            DisplayState::Checking,
            DisplayState::Error,
        ] {
            let available = online(&[state]);
            assert!(available.verify, "{state:?}");
            assert!(available.remove, "{state:?}");
            assert!(available.copy_hash, "{state:?}");
            assert!(available.copy_magnet, "{state:?}");
        }
    }

    #[test]
    fn toggle_pauses_when_any_target_runs() {
        let kind = |states: &[DisplayState]| {
            let rows: Vec<Row> = states.iter().copied().map(row_in).collect();
            let targets: Vec<&Row> = rows.iter().collect();
            toggle_kind(&targets)
        };

        assert_eq!(kind(&[]), None);
        assert_eq!(kind(&[DisplayState::Seeding]), Some(ToggleKind::Pause));
        assert_eq!(
            kind(&[DisplayState::Paused, DisplayState::Downloading]),
            Some(ToggleKind::Pause)
        );
        assert_eq!(kind(&[DisplayState::Checking]), Some(ToggleKind::Pause));
        assert_eq!(
            kind(&[DisplayState::Queued(QueueKind::Download)]),
            Some(ToggleKind::Pause)
        );
        assert_eq!(kind(&[DisplayState::Paused]), Some(ToggleKind::Start));
        assert_eq!(
            kind(&[DisplayState::Paused, DisplayState::Error]),
            Some(ToggleKind::Start)
        );
    }
}
