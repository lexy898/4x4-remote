//! Saving the main view's [`UiState`]: the timer after a change, the writes off the UI thread,
//! the last write when the window closes or the app quits, and the window size.
//!
//! [`SaveSchedule`] decides; this module runs it. The timer is one task, which each change that
//! makes the state differ from the saved one replaces, so the save comes [`SAVE_DELAY`] after the
//! last change. A write gets a copy of the state and runs on the background executor, as
//! `write_ui_state` flushes the file to disk.

use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::{AppContext as _, Context, Task, Window};
use x4_core::UiState;

use super::MainView;
use crate::files::FilesError;
use crate::saving::{Change, Notice, SAVE_DELAY, SaveSchedule, StateFile};
use crate::window_state::remembered_window;

/// What the main view needs to save its state: the decisions, the file and the tasks running.
pub(super) struct Saver {
    /// When to write.
    schedule: SaveSchedule,
    /// Where to write; `None` when the state is never saved.
    file: Option<Arc<StateFile>>,
    /// The wait before the next save; dropping it cancels the save.
    timer: Option<Task<()>>,
    /// The newest write and the handling of its result.
    write: Option<Task<()>>,
}

impl Saver {
    /// Saves the state of a window that starts from `ui` to `state_file`; with `None` it never
    /// saves.
    pub(super) fn new(ui: &UiState, state_file: Option<PathBuf>) -> Self {
        Self {
            schedule: SaveSchedule::new(ui, state_file.is_some()),
            file: state_file.map(|path| Arc::new(StateFile::new(path))),
            timer: None,
            write: None,
        }
    }

    /// Whether the newest save failed, so that the status bar says so.
    pub(super) fn failing(&self) -> bool {
        self.schedule.failing()
    }

    /// How many saves have been started since the window opened.
    pub(super) fn writes(&self) -> u64 {
        self.schedule.writes()
    }
}

impl MainView {
    /// Starts the wait before the next save if the state to remember changed. Call it after
    /// anything that may change [`MainView::ui_state`]; a call that changed nothing costs a
    /// comparison and nothing else.
    pub(super) fn ui_changed(&mut self, cx: &mut Context<'_, Self>) {
        match self.saver.schedule.changed(&self.ui) {
            Change::None => {}
            Change::Undone => self.saver.timer = None,
            Change::Due => {
                // Replacing the timer cancels the one before, so the wait starts again.
                self.saver.timer = Some(cx.spawn(async |view, cx| {
                    cx.background_executor().timer(SAVE_DELAY).await;
                    if let Err(error) = view.update(cx, Self::save_due) {
                        tracing::debug!(%error, "the save came due after the window closed");
                    }
                }));
            }
        }
    }

    /// Remembers the size of the window and whether it is maximized, when the platform reports
    /// new bounds. A move reports them too, but the position is not kept.
    pub(super) fn window_changed(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        self.ui.window = remembered_window(window.window_bounds());
        self.ui_changed(cx);
    }

    /// The timer ran out: writes a copy of the state in the background.
    fn save_due(&mut self, cx: &mut Context<'_, Self>) {
        let Some(file) = self.saver.file.clone() else {
            return;
        };
        let Some(write) = self.saver.schedule.due(&self.ui) else {
            return;
        };
        let number = write.number;
        tracing::debug!(number, "saving the UI state");
        let writing = cx.background_spawn(async move { file.write(&write) });
        // A write before this one may still run: the file keeps them in order. Dropping its task
        // only drops the handling of its result, which this one's replaces.
        self.saver.write = Some(cx.spawn(async move |view, cx| {
            let result = writing.await;
            if let Err(error) = view.update(cx, |view, cx| view.save_finished(number, result, cx)) {
                tracing::debug!(%error, "a save finished after the window closed");
            }
        }));
    }

    /// The window closes or the app quits: writes what is not saved yet, on this thread, as
    /// there will be no later. It is one small file on a local disk.
    pub(super) fn save_before_close(&mut self, cx: &mut Context<'_, Self>) {
        self.saver.timer = None;
        let Some(file) = self.saver.file.clone() else {
            return;
        };
        let Some(write) = self.saver.schedule.closing(&self.ui) else {
            return;
        };
        tracing::debug!(number = write.number, "saving the UI state before closing");
        let result = file.write(&write);
        self.save_finished(write.number, result, cx);
    }

    /// Takes the result of save `number`: logs it, and shows or hides the notice in the status
    /// bar. Only the first failure of a run is a warning, so a disk that stays full does not fill
    /// the log.
    fn save_finished(
        &mut self,
        number: u64,
        result: Result<(), FilesError>,
        cx: &mut Context<'_, Self>,
    ) {
        let notice = self.saver.schedule.finished(number, result.is_ok());
        match result {
            Ok(()) => {
                tracing::debug!(number, "UI state saved");
                if notice == Notice::Clear {
                    tracing::info!("the UI state is saved again");
                    cx.notify();
                }
            }
            Err(error) => {
                if notice == Notice::Show {
                    tracing::warn!(error = %error.report(), "the UI state cannot be saved; the window keeps working and tries again at the next change");
                    cx.notify();
                } else {
                    tracing::debug!(number, error = %error.report(), "the UI state still cannot be saved");
                }
            }
        }
    }
}
