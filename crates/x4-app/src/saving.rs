//! When the UI state is saved, and the file it is saved to.
//!
//! The main window remembers its [`UiState`] (the columns, the sort order, the filter, the theme
//! and the window size) between runs. It saves it [`SAVE_DELAY`] after the last change, so that a
//! drag that resizes a column 60 times a second writes once, and at once when the window closes or
//! the app quits. This module holds the decisions as plain values ([`SaveSchedule`]), so that they
//! are tested without a window or a clock, and the file ([`StateFile`]), which never lets an older
//! state land over a newer one. The main view runs the timer and the writes.

use std::mem::{self, Discriminant};
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use x4_core::UiState;

use crate::files::{FilesError, write_ui_state};

/// How long after the last change the state is saved. Each change starts the wait again.
pub(crate) const SAVE_DELAY: Duration = Duration::from_secs(1);

/// What a possible change of the UI state asks of the timer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Change {
    /// Nothing changed, such as when an edit of the search reassigns the same filter and sort
    /// order, or the state is never saved: leave the timer as it is.
    None,
    /// The state differs from the one saved: start the timer again, so that the save comes
    /// [`SAVE_DELAY`] after this change.
    Due,
    /// The state is back to the one saved: drop the timer.
    Undone,
}

/// The kind of a failed write: the variant of its [`FilesError`]. A new kind is worth a warning;
/// the same kind again is not.
pub(crate) type FailureKind = Discriminant<FilesError>;

/// The kind of `error`.
pub(crate) fn failure_kind(error: &FilesError) -> FailureKind {
    mem::discriminant(error)
}

/// What the result of a write means for the status bar's notice that the state cannot be saved,
/// and for the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Saved, and the write before was too: the notice stays hidden.
    Saved,
    /// The first save after a failure: hide the notice.
    SavedAgain,
    /// The first failure since the start or since a save: show the notice and warn.
    Failed,
    /// A failure of another kind than the one before: warn again. The notice is shown already.
    FailedOtherwise,
    /// A failure of the same kind as the one before: the notice stays, and the log needs no
    /// warning, so that a disk that stays full does not fill it.
    StillFailing,
    /// The result came after that of a newer write, which counts instead: nothing changes.
    Late,
}

/// A state to write, numbered in the order the writes are handed out.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Write {
    /// The place of this write in the order, from 1.
    pub(crate) number: u64,
    /// The state as it was when the write was handed out.
    pub(crate) state: UiState,
}

/// When to save the UI state, decided from the state alone.
///
/// The view calls [`changed`](Self::changed) after anything that may change its state,
/// [`due`](Self::due) when the timer runs out, [`closing`](Self::closing) when the window closes
/// and [`finished`](Self::finished) with the result of each write. The states are compared by
/// value, so a call that changed nothing never leads to a write.
#[derive(Debug)]
pub(crate) struct SaveSchedule {
    /// Whether the state may be saved at all. It may not when there is no file to save it to, or
    /// the file must be left alone (written by a newer version, unreadable, corrupt and stuck).
    enabled: bool,
    /// The state at the last call of [`changed`](Self::changed): a call with the same state is
    /// no change.
    seen: UiState,
    /// The state last handed out to be written, or the one the window started from.
    handed: UiState,
    /// How many writes have been handed out; the number of the newest.
    writes: u64,
    /// The newest write handed out and not finished yet.
    in_flight: Option<u64>,
    /// The number of the newest write whose result came; older results come too late to count.
    newest_finished: u64,
    /// The kind of failure of the newest write that finished, if it failed. Then the state handed
    /// out is not on disk, and the next change or the close tries again.
    failing: Option<FailureKind>,
}

impl SaveSchedule {
    /// The schedule of a window that starts from `state`, which is what the file holds (or the
    /// defaults, when there is no file yet). With `enabled` false it never asks for a write.
    pub(crate) fn new(state: &UiState, enabled: bool) -> Self {
        Self {
            enabled,
            seen: state.clone(),
            handed: state.clone(),
            writes: 0,
            in_flight: None,
            newest_finished: 0,
            failing: None,
        }
    }

    /// How many writes have been handed out since the window opened, failed ones included.
    pub(crate) fn writes(&self) -> u64 {
        self.writes
    }

    /// Whether the newest write that finished failed, so the status bar says so.
    pub(crate) fn failing(&self) -> bool {
        self.failing.is_some()
    }

    /// What `state` means after something that may have changed it.
    pub(crate) fn changed(&mut self, state: &UiState) -> Change {
        if !self.enabled || *state == self.seen {
            return Change::None;
        }
        self.seen.clone_from(state);
        if self.is_saved(state) {
            Change::Undone
        } else {
            Change::Due
        }
    }

    /// The timer ran out: the write to start, if `state` is not the one handed out last. A write
    /// still in flight does not hold this one back; [`StateFile`] keeps them in order.
    pub(crate) fn due(&mut self, state: &UiState) -> Option<Write> {
        if !self.enabled || self.is_saved(state) {
            return None;
        }
        Some(self.hand_out(state))
    }

    /// The window closes or the app quits: the write to make at once, if `state` is not known to
    /// be on disk. That includes a write still in flight, which the closing would cancel if it has
    /// not started yet.
    pub(crate) fn closing(&mut self, state: &UiState) -> Option<Write> {
        if !self.enabled || (self.is_saved(state) && self.in_flight.is_none()) {
            return None;
        }
        Some(self.hand_out(state))
    }

    /// Takes the result of write `number`: `None` if it reached the file, or the kind of its
    /// failure. Returns what that means for the notice and the log. A result older than one taken
    /// already is [`Outcome::Late`].
    pub(crate) fn finished(&mut self, number: u64, failure: Option<FailureKind>) -> Outcome {
        if number < self.newest_finished {
            return Outcome::Late;
        }
        self.newest_finished = number;
        if self.in_flight.is_some_and(|in_flight| in_flight <= number) {
            self.in_flight = None;
        }
        let before = mem::replace(&mut self.failing, failure);
        match (before, failure) {
            (None, None) => Outcome::Saved,
            (Some(_), None) => Outcome::SavedAgain,
            (None, Some(_)) => Outcome::Failed,
            (Some(before), Some(now)) if before == now => Outcome::StillFailing,
            (Some(_), Some(_)) => Outcome::FailedOtherwise,
        }
    }

    /// Whether `state` is the one handed out last, and that write has not failed.
    fn is_saved(&self, state: &UiState) -> bool {
        *state == self.handed && self.failing.is_none()
    }

    fn hand_out(&mut self, state: &UiState) -> Write {
        self.writes += 1;
        self.handed.clone_from(state);
        self.in_flight = Some(self.writes);
        Write {
            number: self.writes,
            state: state.clone(),
        }
    }
}

/// The file the UI state is saved to, shared by the writes in the background and the last one
/// when the window closes.
///
/// Writes are made one at a time, and one numbered lower than a write that already reached the
/// file is skipped, so an older state never lands over a newer one, whatever order the writes run
/// in.
#[derive(Debug)]
pub(crate) struct StateFile {
    /// Where the state goes.
    path: PathBuf,
    /// The number of the newest write that reached the file; 0 before the first.
    written: Mutex<u64>,
}

impl StateFile {
    /// The file at `path`, which is written only when [`write`](Self::write) is called.
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            written: Mutex::new(0),
        }
    }

    /// Writes `write` to the file, atomically ([`write_ui_state`]), unless a newer write got
    /// there first. Blocks while another write runs; call it off the UI thread, except when the
    /// window closes.
    ///
    /// # Errors
    ///
    /// The error of [`write_ui_state`]; the file is left as it was.
    pub(crate) fn write(&self, write: &Write) -> Result<(), FilesError> {
        // A write that panicked left the number as it was before it, which is still right.
        let mut written = self.written.lock().unwrap_or_else(PoisonError::into_inner);
        if *written >= write.number {
            return Ok(());
        }
        write_ui_state(&self.path, &write.state)?;
        *written = write.number;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, io};

    use x4_core::{Filter, SortDirection, SortKey, SortState, ThemeChoice};

    use super::*;
    use crate::files::read_ui_state;

    /// The kind of a failure to create the directory.
    fn no_directory() -> FailureKind {
        failure_kind(&FilesError::CreateDir {
            path: "/x".into(),
            source: io::Error::other("no"),
        })
    }

    /// The kind of a failure to write the file.
    fn no_write() -> FailureKind {
        failure_kind(&FilesError::Write {
            path: "/x/state.toml".into(),
            source: io::Error::other("no"),
        })
    }

    fn sorted_by_size() -> UiState {
        UiState {
            sort: SortState {
                key: SortKey::Size,
                direction: SortDirection::Descending,
            },
            ..UiState::default()
        }
    }

    fn seeding() -> UiState {
        UiState {
            filter: Filter::Seeding,
            ..UiState::default()
        }
    }

    #[test]
    fn a_change_is_due_and_the_same_state_again_is_no_change() {
        let mut schedule = SaveSchedule::new(&UiState::default(), true);

        assert_eq!(schedule.changed(&UiState::default()), Change::None);
        assert_eq!(schedule.changed(&sorted_by_size()), Change::Due);
        // An edit of the search sets the filter and the order to what they were.
        assert_eq!(schedule.changed(&sorted_by_size()), Change::None);
        assert_eq!(schedule.changed(&seeding()), Change::Due);
        assert_eq!(schedule.writes(), 0, "deciding writes nothing");
    }

    #[test]
    fn going_back_to_the_saved_state_drops_the_timer() {
        let mut schedule = SaveSchedule::new(&UiState::default(), true);

        assert_eq!(schedule.changed(&sorted_by_size()), Change::Due);
        assert_eq!(schedule.changed(&UiState::default()), Change::Undone);
        assert_eq!(schedule.due(&UiState::default()), None);
    }

    #[test]
    fn due_hands_out_numbered_states_and_nothing_twice() {
        let mut schedule = SaveSchedule::new(&UiState::default(), true);
        schedule.changed(&sorted_by_size());

        assert_eq!(
            schedule.due(&sorted_by_size()),
            Some(Write {
                number: 1,
                state: sorted_by_size()
            })
        );
        assert_eq!(schedule.due(&sorted_by_size()), None, "handed out already");
        assert_eq!(schedule.changed(&seeding()), Change::Due);
        assert_eq!(schedule.due(&seeding()).map(|write| write.number), Some(2));
        assert_eq!(schedule.writes(), 2);
    }

    #[test]
    fn a_state_that_is_never_saved_asks_for_nothing() {
        let mut schedule = SaveSchedule::new(&UiState::default(), false);

        assert_eq!(schedule.changed(&sorted_by_size()), Change::None);
        assert_eq!(schedule.due(&sorted_by_size()), None);
        assert_eq!(schedule.closing(&sorted_by_size()), None);
        assert_eq!(schedule.writes(), 0);
    }

    #[test]
    fn closing_writes_a_pending_change_or_a_write_still_in_flight() {
        let mut schedule = SaveSchedule::new(&UiState::default(), true);
        assert_eq!(
            schedule.closing(&UiState::default()),
            None,
            "nothing to save"
        );

        // Changed, and the timer has not run out.
        schedule.changed(&sorted_by_size());
        assert_eq!(
            schedule
                .closing(&sorted_by_size())
                .map(|write| write.number),
            Some(1)
        );
        schedule.finished(1, None);
        assert_eq!(schedule.closing(&sorted_by_size()), None, "saved already");

        // Handed out, and the write has not finished: it may never run once the window is gone.
        schedule.changed(&seeding());
        assert_eq!(schedule.due(&seeding()).map(|write| write.number), Some(2));
        assert_eq!(
            schedule.closing(&seeding()).map(|write| write.number),
            Some(3)
        );
    }

    #[test]
    fn a_failure_shows_the_notice_once_and_a_success_clears_it() {
        let mut schedule = SaveSchedule::new(&UiState::default(), true);
        schedule.changed(&sorted_by_size());
        schedule.due(&sorted_by_size());

        assert_eq!(schedule.finished(1, Some(no_write())), Outcome::Failed);
        assert!(schedule.failing());

        schedule.changed(&seeding());
        schedule.due(&seeding());
        assert_eq!(
            schedule.finished(2, Some(no_write())),
            Outcome::StillFailing,
            "once per run"
        );
        assert!(schedule.failing());

        let dark = UiState {
            theme: ThemeChoice::Dark,
            ..seeding()
        };
        schedule.changed(&dark);
        schedule.due(&dark);
        assert_eq!(schedule.finished(3, None), Outcome::SavedAgain);
        assert!(!schedule.failing());
        assert_eq!(schedule.finished(3, None), Outcome::Saved);
    }

    #[test]
    fn a_new_kind_of_failure_is_a_warning_again() {
        let mut schedule = SaveSchedule::new(&UiState::default(), true);

        assert_eq!(schedule.finished(1, Some(no_directory())), Outcome::Failed);
        assert_eq!(
            schedule.finished(2, Some(no_directory())),
            Outcome::StillFailing
        );
        assert_eq!(
            schedule.finished(3, Some(no_write())),
            Outcome::FailedOtherwise
        );
        assert_eq!(
            schedule.finished(4, Some(no_write())),
            Outcome::StillFailing
        );
        assert_eq!(
            schedule.finished(5, Some(no_directory())),
            Outcome::FailedOtherwise
        );
        assert!(schedule.failing());
        assert_eq!(schedule.finished(6, None), Outcome::SavedAgain);
        assert_eq!(schedule.finished(7, Some(no_write())), Outcome::Failed);
    }

    #[test]
    fn a_state_that_failed_is_tried_again_after_the_next_change() {
        let mut schedule = SaveSchedule::new(&UiState::default(), true);
        schedule.changed(&sorted_by_size());
        schedule.due(&sorted_by_size());
        schedule.finished(1, Some(no_write()));

        // Away and back: the state is the one handed out, but it is not on disk.
        assert_eq!(schedule.changed(&seeding()), Change::Due);
        assert_eq!(schedule.changed(&sorted_by_size()), Change::Due);
        assert_eq!(
            schedule.due(&sorted_by_size()).map(|write| write.number),
            Some(2)
        );
        // Nothing changed since: no retry without a change, but one last try at the close.
        schedule.finished(2, Some(no_write()));
        assert_eq!(schedule.changed(&sorted_by_size()), Change::None);
        assert_eq!(
            schedule
                .closing(&sorted_by_size())
                .map(|write| write.number),
            Some(3)
        );
    }

    #[test]
    fn a_result_older_than_one_taken_is_ignored() {
        let mut schedule = SaveSchedule::new(&UiState::default(), true);
        schedule.changed(&sorted_by_size());
        schedule.due(&sorted_by_size());
        schedule.changed(&seeding());
        schedule.due(&seeding());

        assert_eq!(schedule.finished(2, None), Outcome::Saved);
        assert_eq!(
            schedule.finished(1, Some(no_write())),
            Outcome::Late,
            "too late"
        );
        assert!(!schedule.failing());
        assert_eq!(schedule.closing(&seeding()), None, "nothing in flight");
    }

    #[test]
    fn the_state_file_writes_the_state() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("state.toml");
        let file = StateFile::new(path.clone());

        file.write(&Write {
            number: 1,
            state: seeding(),
        })
        .expect("written");

        assert_eq!(read_ui_state(&path).expect("readable").state, seeding());
    }

    #[test]
    fn an_older_write_never_lands_over_a_newer_one() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("state.toml");
        let file = StateFile::new(path.clone());

        file.write(&Write {
            number: 2,
            state: seeding(),
        })
        .expect("written");
        file.write(&Write {
            number: 1,
            state: sorted_by_size(),
        })
        .expect("skipped without an error");

        assert_eq!(read_ui_state(&path).expect("readable").state, seeding());
    }

    #[test]
    fn a_failed_write_can_be_made_again() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        // The directory of the file cannot be created while a file has its name.
        let blocker = dir.path().join("sub");
        fs::write(&blocker, "").expect("written");
        let path = blocker.join("state.toml");
        let file = StateFile::new(path.clone());
        let write = Write {
            number: 1,
            state: seeding(),
        };

        assert!(matches!(
            file.write(&write),
            Err(FilesError::CreateDir { .. })
        ));
        fs::remove_file(&blocker).expect("removed");
        file.write(&write).expect("written this time");

        assert_eq!(read_ui_state(&path).expect("readable").state, seeding());
    }
}
