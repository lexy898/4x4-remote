//! Adding torrents, as the main view carries it out: the requests of the sources of adds, the
//! queue of what waits, the add dialog for its head, the adds sent and what became of them.
//!
//! Every source sends an [`AddRequest`] into one channel, which [`MainView::listen_for_adds`]
//! pumps on the UI thread; a drop on the dialog hands its files over directly. The items join
//! x4-core's [`AddQueue`], which holds at most [`QUEUE_CAP`](x4_core::QUEUE_CAP); what it has no
//! room for is counted in a notification. Launch arguments 4x4 does not open make one
//! notification, and a second launch raises the window.
//!
//! The head of the queue is read off the UI thread when its turn comes: a `.torrent` file only
//! once `stat` says it is a regular file within the limit (the `adding` module of the crate). A
//! file that cannot be offered is skipped with a notification that names it and the reason. The
//! dialog opens for a head that has been read, unless another dialog is open: the queue then
//! waits until that dialog closes, which tells the view ([`MainView::dialog_closed`]).
//!
//! Nothing is added without the user's Add. Add sends the dialog's request once and keeps it until
//! the service answers ([`Event::AddFinished`](x4_core::Event::AddFinished)):
//! - added: the folder it was sent to joins the folders used lately (none for the session's own
//!   folder), the torrent is selected once it shows, and the dialog moves on;
//! - a torrent the daemon had already: a notification names it, and it is selected;
//! - not confirmed in time: a notification says so, and the torrent is selected if it shows;
//! - a failure: a notification says why, and the form stays, to be sent again.
//!
//! Cancel while an add is out skips the torrent all the same, and its answer still counts.
//!
//! The torrent to select is kept by its hash until a snapshot lists it. The filter goes back to
//! All and the search is cleared if they would hide it; then it is selected, once, so that a later
//! choice of the user stands.

use std::collections::HashMap;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{AppContext as _, Context, Entity, SharedString, Subscription, Task, Window};
use x4_core::{Action, ActionId, AddError, AddItem, AddQueue, Added, Command, LoadedItem};
use x4_rpc::HashString;

use super::MainView;
use crate::adding::{self, AddDialog, AddDialogEvent, AddRequest, LoadError, Outgoing, load};
use crate::i18n::{
    add_duplicate_text, add_failure_text, add_unconfirmed_text, load_failure_text, queue_full_text,
    rejected_text,
};
use crate::search::clear_search;
use crate::torrent_table::{SelectionChange, change_selection};

/// What the main view keeps of the torrents to add.
#[derive(Default)]
pub(super) struct Adds {
    /// What waits to be added; the head is the dialog's.
    queue: AddQueue,
    /// The open add dialog, and its events.
    dialog: Option<(Entity<AddDialog>, Subscription)>,
    /// The head being read, and the reading; dropping it cancels the reading.
    reading: Option<(AddItem, Task<()>)>,
    /// The adds sent and not answered yet, by their ids.
    out: HashMap<ActionId, Outgoing>,
    /// The torrent to select once a snapshot lists it.
    pending_selection: Option<HashString>,
    /// What the newest notification about an add said.
    last_notice: Option<SharedString>,
    /// The pump of the requests; dropping it stops listening.
    listener: Option<Task<()>>,
}

impl MainView {
    /// Takes the requests that arrive on `adds` from now on, on the UI thread, until the channel
    /// closes or the window does. The sources of adds (the toolbar, the keys, the file picker, the
    /// clipboard, a drop, the launch arguments and a second launch) send into it.
    pub fn listen_for_adds(
        &mut self,
        adds: async_channel::Receiver<AddRequest>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.adds.listener = Some(cx.spawn_in(window, async move |view, cx| {
            while let Ok(request) = adds.recv().await {
                if view
                    .update_in(cx, |view, window, cx| {
                        view.take_add_request(request, window, cx);
                    })
                    .is_err()
                {
                    return;
                }
            }
            tracing::debug!("no more requests to add torrents will come");
        }));
    }

    /// What the newest notification about a torrent to add said: a file that cannot be offered,
    /// what became of an add, a full queue or refused launch arguments.
    #[must_use]
    pub fn last_add_notice(&self) -> Option<&str> {
        self.adds.last_notice.as_deref()
    }

    /// Carries out `request`: queues its items, says which launch arguments were refused, or
    /// raises the window.
    pub(crate) fn take_add_request(
        &mut self,
        request: AddRequest,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match request {
            AddRequest::Items(items) => {
                let count = items.len();
                let dropped = self.adds.queue.push(items);
                tracing::info!(count, dropped, "torrents to add were queued");
                if dropped > 0 {
                    self.add_notice(Notification::warning, queue_full_text(dropped), window, cx);
                }
                self.advance_adds(window, cx);
            }
            AddRequest::Rejected(rejected) => {
                tracing::info!(count = rejected.len(), "launch arguments were refused");
                if let Some(text) = rejected_text(&rejected) {
                    self.add_notice(Notification::warning, text, window, cx);
                }
            }
            AddRequest::Raise => {
                tracing::debug!("the window is raised for another launch");
                window.activate_window();
                window.request_attention();
            }
        }
    }

    /// Handles Cancel and Escape in the add dialog: skips the torrent shown, and shows the next
    /// one or closes the dialog. An add of it that is out still counts when it is answered.
    pub(crate) fn cancel_add(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        tracing::debug!("the torrent shown in the add dialog was skipped");
        self.skip_add(window, cx);
    }

    /// Tells the view that a dialog other than the add dialog closed, so that the torrents that
    /// wait for it can be offered.
    pub(crate) fn dialog_closed(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.advance_adds(window, cx);
    }

    /// Hands the dialog the newest snapshot and the connection, and selects the torrent that waits
    /// to be selected if the snapshot lists it.
    pub(super) fn follow_adds(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if let Some((dialog, _)) = &self.adds.dialog {
            let snapshot = self.last_snapshot.clone();
            let connected = self.status.connected();
            dialog.update(cx, |dialog, cx| {
                dialog.follow(snapshot, connected, window, cx);
            });
        }
        self.apply_pending_selection(window, cx);
    }

    /// Takes the service's answer `result` to the add `id`.
    pub(super) fn add_finished(
        &mut self,
        id: ActionId,
        result: Result<Added, AddError>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(outgoing) = self.adds.out.remove(&id) else {
            tracing::warn!(id = id.0, "an add finished that this window did not start");
            return;
        };
        let shown = self
            .adds
            .dialog
            .as_ref()
            .is_some_and(|(dialog, _)| dialog.read(cx).waiting() == Some(id));
        match result {
            Ok(Added {
                hash,
                name,
                duplicate,
            }) => {
                let name = if name.is_empty() {
                    outgoing.name.clone()
                } else {
                    name
                };
                if duplicate {
                    tracing::info!(torrent = %hash, "the daemon had the torrent already");
                    self.add_notice(Notification::info, add_duplicate_text(&name), window, cx);
                } else {
                    tracing::info!(torrent = %hash, "torrent added");
                    self.remember_folder(&outgoing, cx);
                }
                self.adds.pending_selection = Some(hash);
                if shown {
                    self.skip_add(window, cx);
                }
            }
            Err(AddError::Unconfirmed) => {
                tracing::warn!(torrent = %outgoing.hash, "the daemon has not confirmed the add yet");
                self.add_notice(
                    Notification::info,
                    add_unconfirmed_text(&outgoing.name),
                    window,
                    cx,
                );
                self.adds.pending_selection = Some(outgoing.hash);
                if shown {
                    self.skip_add(window, cx);
                }
            }
            Err(error) => {
                tracing::error!(torrent = %outgoing.hash, "the torrent could not be added");
                // The daemon's text may name a folder on the server.
                tracing::debug!(%error, "why the torrent could not be added");
                self.add_notice(
                    Notification::error,
                    add_failure_text(&outgoing.name, &error),
                    window,
                    cx,
                );
                if shown && let Some((dialog, _)) = &self.adds.dialog {
                    dialog.update(cx, |dialog, cx| dialog.set_waiting(None, cx));
                }
            }
        }
        self.apply_pending_selection(window, cx);
    }

    /// Hands the daemon's `answer` about the free space of `path` to the add dialog.
    pub(super) fn free_space_read(
        &mut self,
        path: x4_rpc::DownloadDir,
        answer: x4_core::FreeSpaceAnswer,
        cx: &mut Context<'_, Self>,
    ) {
        if let Some((dialog, _)) = &self.adds.dialog {
            dialog.update(cx, |dialog, cx| dialog.free_space_read(path, answer, cx));
        } else {
            tracing::debug!("the free space of a folder came after the add dialog closed");
        }
    }

    /// Remembers the folder `outgoing` was sent to among the folders used lately: the very folder
    /// of the request, and none when it named none (the session's own).
    fn remember_folder(&mut self, outgoing: &Outgoing, cx: &mut Context<'_, Self>) {
        let Some(folder) = &outgoing.add.download_dir else {
            return;
        };
        self.ui.add.remember(folder);
        self.ui_changed(cx);
        if let Some((dialog, _)) = &self.adds.dialog {
            let history = self.ui.add.recent_folders.clone();
            dialog.update(cx, |dialog, cx| dialog.set_history(history, cx));
        }
    }

    /// Moves on from the head of the queue, after it was added or skipped.
    fn skip_add(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.adds.queue.skip();
        self.advance_adds(window, cx);
    }

    /// Offers the head of the queue: the dialog keeps showing it, or reads it; with nothing left,
    /// the dialog closes. While another dialog is open, the queue waits.
    fn advance_adds(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(head) = self.adds.queue.head().cloned() else {
            self.adds.reading = None;
            self.close_add_dialog(window, cx);
            return;
        };
        let position = self.adds.queue.position();
        if let Some((dialog, _)) = &self.adds.dialog {
            if dialog.read(cx).shows(&head) {
                dialog.update(cx, |dialog, cx| dialog.set_position(position, cx));
                return;
            }
            if !self.reads(&head) {
                dialog.update(cx, |dialog, cx| dialog.reading(position, cx));
            }
        } else if window.has_active_dialog(cx) {
            tracing::debug!("another dialog is open; the torrents to add wait");
            return;
        }
        if !self.reads(&head) {
            self.read_head(head, window, cx);
        }
    }

    /// Whether `item` is being read.
    fn reads(&self, item: &AddItem) -> bool {
        self.adds
            .reading
            .as_ref()
            .is_some_and(|(reading, _)| reading == item)
    }

    /// Reads `item`, the head of the queue, off the UI thread, and offers it once it is read.
    fn read_head(&mut self, item: AddItem, window: &mut Window, cx: &mut Context<'_, Self>) {
        let read = item.clone();
        let task = cx.spawn_in(window, async move |view, cx| {
            let given = read.clone();
            let loaded = cx
                .background_executor()
                .spawn(async move { load(given) })
                .await;
            if let Err(error) = view.update_in(cx, |view, window, cx| {
                view.head_read(read, loaded, window, cx);
            }) {
                tracing::debug!(%error, "a torrent to add was read after the window closed");
            }
        });
        self.adds.reading = Some((item, task));
    }

    /// Offers `item`, read as `loaded`, if it is still the head of the queue: in the dialog, which
    /// opens for it unless another dialog is open. A file that cannot be offered is skipped with a
    /// notification.
    fn head_read(
        &mut self,
        item: AddItem,
        loaded: Result<LoadedItem, LoadError>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.adds.reading = None;
        if self.adds.queue.head() != Some(&item) {
            tracing::debug!("a torrent to add was skipped while it was read");
            self.advance_adds(window, cx);
            return;
        }
        let loaded = match loaded {
            Ok(loaded) => loaded,
            Err(error) => {
                tracing::info!(%error, "a torrent file cannot be offered; skipped");
                if let AddItem::File(path) = &item {
                    tracing::debug!(path = %path.display(), "the torrent file skipped");
                    let name = path
                        .file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy();
                    self.add_notice(
                        Notification::error,
                        load_failure_text(&name, &error),
                        window,
                        cx,
                    );
                }
                self.skip_add(window, cx);
                return;
            }
        };
        let position = self.adds.queue.position();
        if let Some((dialog, _)) = &self.adds.dialog {
            dialog.update(cx, |dialog, cx| {
                dialog.show(item, loaded, position, window, cx);
            });
        } else if window.has_active_dialog(cx) {
            tracing::debug!("another dialog opened meanwhile; the torrents to add wait");
        } else {
            self.open_add_dialog(item, loaded, position, window, cx);
        }
    }

    /// Opens the add dialog for `item`, read as `loaded`, at `position` in the queue.
    fn open_add_dialog(
        &mut self,
        item: AddItem,
        loaded: LoadedItem,
        position: Option<(usize, usize)>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let history = self.ui.add.recent_folders.clone();
        let snapshot = self.last_snapshot.clone();
        let connected = self.status.connected();
        let repeats = std::rc::Rc::clone(&self.key_repeats);
        let dialog = cx.new(|cx| AddDialog::new(repeats, history, snapshot, connected, window, cx));
        let events = cx.subscribe_in(&dialog, window, Self::on_add_dialog_event);
        self.adds.dialog = Some((dialog.clone(), events));
        dialog.update(cx, |dialog, cx| {
            dialog.show(item, loaded, position, window, cx);
        });
        tracing::debug!("the add dialog opens");
        adding::open(dialog, cx.weak_entity(), window, cx);
    }

    /// Closes the add dialog, if it is open.
    fn close_add_dialog(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.adds.dialog.take().is_some() {
            tracing::debug!("the add dialog closes");
            window.close_dialog(cx);
        }
    }

    /// Carries out what the add dialog asks for.
    fn on_add_dialog_event(
        &mut self,
        dialog: &Entity<AddDialog>,
        event: &AddDialogEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match event {
            AddDialogEvent::Add(outgoing) => self.send_add(dialog, outgoing.clone(), window, cx),
            AddDialogEvent::Show(hash) => {
                tracing::debug!(torrent = %hash, "the torrent in the list is shown");
                self.adds.pending_selection = Some(hash.clone());
                self.apply_pending_selection(window, cx);
                self.skip_add(window, cx);
            }
            AddDialogEvent::AskFreeSpace(folder) => {
                tracing::debug!(
                    folder = folder.as_str(),
                    "asking for the free space of a folder"
                );
                if self
                    .commands
                    .send(Command::FreeSpace(folder.clone()))
                    .is_err()
                {
                    tracing::debug!("the service has stopped; the free space was not asked for");
                }
            }
        }
    }

    /// Sends `outgoing`, the add of the torrent `dialog` shows, and keeps it until it is answered.
    /// If the service has stopped, it is answered at once as not connected.
    fn send_add(
        &mut self,
        dialog: &Entity<AddDialog>,
        outgoing: Outgoing,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let id = self.pending.next_id();
        tracing::info!(
            torrent = %outgoing.hash,
            magnet = adding::is_magnet(&outgoing.add),
            paused = outgoing.add.paused,
            unwanted = outgoing.add.files_unwanted.len(),
            "torrent add sent"
        );
        dialog.update(cx, |dialog, cx| dialog.set_waiting(Some(id), cx));
        let action = Action::Add(outgoing.add.clone());
        self.adds.out.insert(id, outgoing);
        if self.commands.send(Command::Run(id, action)).is_err() {
            tracing::warn!("the service has stopped; the add was not sent");
            self.add_finished(id, Err(AddError::NotConnected), window, cx);
        }
    }

    /// Selects the torrent that waits to be selected, once a snapshot lists it, after the filter
    /// and the search that would hide it are cleared.
    fn apply_pending_selection(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(hash) = self.adds.pending_selection.clone() else {
            return;
        };
        let revealing = {
            let Some(torrent) = self.last_snapshot.as_ref().and_then(|snapshot| {
                snapshot
                    .torrents
                    .iter()
                    .find(|torrent| torrent.hash == hash)
            }) else {
                return;
            };
            self.view.revealing(torrent)
        };
        self.adds.pending_selection = None;
        if let Some(view) = revealing {
            tracing::debug!("the filter and the search are cleared to show the added torrent");
            if view.search != self.view.search {
                clear_search(&self.search, window, cx);
            }
            self.set_view(view, cx);
        }
        tracing::debug!(torrent = %hash, "the added torrent is selected");
        self.table.update(cx, |table, cx| {
            change_selection(table, SelectionChange::Click(hash), cx);
        });
    }

    /// Shows `text` as a notification made by `kind` (info, warning or error), which a click
    /// dismisses, and keeps it as the last notice about an add.
    fn add_notice(
        &mut self,
        kind: fn(SharedString) -> Notification,
        text: SharedString,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        window.push_notification(kind(text.clone()).on_click(|_, _, _| {}), cx);
        self.adds.last_notice = Some(text);
    }
}
