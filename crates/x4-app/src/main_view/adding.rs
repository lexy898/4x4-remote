//! Adding torrents, as the main view carries it out: the requests of the sources of adds, the
//! queue of what waits, the add dialog for its head, the adds sent and what became of them.
//!
//! Every source sends an [`AddRequest`] into one channel, which [`MainView::listen_for_adds`]
//! pumps on the UI thread: the sources outside the window into the channel the app made at
//! startup, those inside it (the toolbar, the keys, the file dialog, the clipboard and drops, a
//! drop on the dialog among them: the `sources` module) into [`MainView::add_requests`], the
//! sender of that same channel. The items join x4-core's [`AddQueue`], which holds at most
//! [`QUEUE_CAP`](x4_core::QUEUE_CAP); what it has no room for is counted in a notification.
//! Launch arguments 4x4 does not open make one notification, and a second launch raises the
//! window.
//!
//! The head of the queue is read off the UI thread when its turn comes: a `.torrent` file only
//! once `stat` says it is a regular file within the limit (the `adding` module of the crate). A
//! file that cannot be offered is skipped with a notification that names it and the reason, and
//! does not count in the run. The dialog opens for a head that has been read, or after
//! [`READ_GRACE`] for one whose read takes longer, to say so and let the user skip it: a read that
//! never ends never holds the queue, and its thread is abandoned. While another dialog is open,
//! the queue waits until that dialog closes, which tells the view ([`MainView::dialog_closed`]).
//!
//! Nothing is added without the user's Add. Add sends the dialog's request once and keeps it until
//! the service answers ([`Event::AddFinished`](x4_core::Event::AddFinished)); x4-core's
//! [`add_outcome`] says what follows: the folder to remember, the torrent to select, whether the
//! dialog moves on or keeps the form, and the notification. Cancel while an add is out skips the
//! torrent all the same, and its answer still counts; when the service stops, the adds still out
//! say so ([`stopped_outcome`]).
//!
//! The torrent to select waits in x4-core's [`PendingSelection`] until a snapshot lists it; it is
//! then selected, once, with the filter and the search cleared if they would hide it. The view
//! hands it the user's [`ListChoices`] when the add is sent, when it is answered and with every
//! snapshot: any gesture on the selection (even a click on the row selected already) and any
//! change of the filter, the search or the sort order since the add was sent ends the wait, and
//! so does the next full list. What the view does to show the torrent is not counted.

mod sources;

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{App, AppContext as _, Context, Entity, SharedString, Subscription, Task, Window};
use x4_core::{
    Action, ActionId, AddError, AddItem, AddNotice, AddOutcome, AddQueue, Added, Command, FormStep,
    ListChoices, LoadedItem, PendingSelection, SentAdd, add_outcome, stopped_outcome,
};
use x4_rpc::TorrentAdd;

use super::MainView;
use crate::adding::{
    self, AddDialog, AddDialogEvent, AddRequest, AddRequests, FilePicker, LoadError, PromptPicker,
    load,
};
use crate::i18n::{add_notice_text, load_failure_text, queue_full_text, rejected_text};
use crate::search::clear_search;
use crate::torrent_table::select_added;
pub(crate) use sources::AddButton;
pub(super) use sources::on_add_actions;

/// How long the head of the queue may take to read before the dialog opens to say so, with Skip.
const READ_GRACE: Duration = Duration::from_millis(300);
/// How long the window may take to come to the front when asked, before it asks for the user's
/// attention instead: the system may refuse to raise it, or do so a little later.
const RAISE_GRACE: Duration = Duration::from_millis(250);

/// How the head of the queue is read, off the UI thread.
pub(super) type ReadItem = fn(AddItem, &App) -> Task<Result<LoadedItem, LoadError>>;

/// Reads `item` on the background executor. A read that never ends keeps its thread there.
fn read_in_background(item: AddItem, cx: &App) -> Task<Result<LoadedItem, LoadError>> {
    cx.background_executor().spawn(async move { load(item) })
}

/// What the main view keeps of the torrents to add.
pub(super) struct Adds {
    /// What waits to be added; the head is the dialog's.
    queue: AddQueue,
    /// The open add dialog, and its events.
    dialog: Option<(Entity<AddDialog>, Subscription)>,
    /// The head being read.
    reading: Option<Reading>,
    /// How the head is read.
    read: ReadItem,
    /// The adds sent and not answered yet, by their ids.
    out: HashMap<ActionId, SentAdd>,
    /// The torrent to select once a snapshot lists it.
    pending: PendingSelection,
    /// What the newest notification about an add said.
    last_notice: Option<SharedString>,
    /// The sender of the channel `listener` pumps, for the sources of adds inside the window.
    requests: async_channel::Sender<AddRequest>,
    /// The pump of the requests; dropping it stops listening.
    listener: Option<Task<()>>,
    /// The look, after the window was asked to come to the front, whether it did.
    raise: Option<Task<()>>,
    /// How the window asks for `.torrent` files.
    picker: Rc<dyn FilePicker>,
    /// The wait for the answer of the file dialog that is open; one is open at a time.
    picking: Option<Task<()>>,
}

impl Default for Adds {
    fn default() -> Self {
        Self {
            queue: AddQueue::default(),
            dialog: None,
            reading: None,
            read: read_in_background,
            out: HashMap::new(),
            pending: PendingSelection::default(),
            last_notice: None,
            // A channel nobody listens to, until the view listens to one of its own.
            requests: AddRequests::new().sender,
            listener: None,
            raise: None,
            picker: Rc::new(PromptPicker),
            picking: None,
        }
    }
}

/// The head of the queue being read: the read, and the wait before the dialog says so.
struct Reading {
    /// The item read.
    item: AddItem,
    /// When the read began, on the executor's clock.
    began: Instant,
    /// The read; dropping it lets go of its answer.
    _read: Task<()>,
    /// The wait before the dialog opens to say that the item is being read.
    slow: Option<Task<()>>,
}

impl MainView {
    /// Takes the requests that arrive on the channel `adds` from now on, on the UI thread, until
    /// the window closes, and keeps its sender for the sources inside the window
    /// ([`MainView::add_requests`]). The sources of adds (the toolbar, the keys, the file picker,
    /// the clipboard, a drop, the launch arguments and a second launch) send into it. The view
    /// listens to a channel of its own from the start; the app hands it the one of
    /// [`Startup::adds`](crate::app::Startup::adds) when it launches.
    ///
    /// The view has one channel: a later call replaces the one before, whose receiver is no
    /// longer listened to.
    pub fn listen_for_adds(
        &mut self,
        adds: AddRequests,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let AddRequests { sender, receiver } = adds;
        self.adds.requests = sender;
        self.adds.listener = Some(cx.spawn_in(window, async move |view, cx| {
            while let Ok(request) = receiver.recv().await {
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

    /// The sender of the channel the window takes its requests to add torrents from: the sources
    /// of adds inside the window (the toolbar, the keys, the clipboard, a drop) send into it, so
    /// that every request goes the one way. It is the channel of the last
    /// [`MainView::listen_for_adds`].
    #[must_use]
    pub fn add_requests(&self) -> &async_channel::Sender<AddRequest> {
        &self.adds.requests
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
            AddRequest::Raise => self.raise(window, cx),
        }
    }

    /// Handles Cancel, Skip and Escape in the add dialog: skips the torrent shown, or the file
    /// being read, and shows the next one or closes the dialog. An add of it that is out still
    /// counts when it is answered.
    pub(crate) fn cancel_add(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        tracing::debug!("the torrent of the add dialog was skipped");
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

    /// The service has stopped, so the adds still out will never be answered: each says so, in
    /// the order they were sent, and the dialog does not wait for them for good.
    pub(super) fn adds_service_stopped(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let mut out: Vec<(ActionId, SentAdd)> = self.adds.out.drain().collect();
        out.sort_by_key(|(id, _)| id.0);
        for (id, sent) in out {
            tracing::warn!(torrent = %sent.hash, "the service stopped before the add was answered");
            self.follow_outcome(id, stopped_outcome(&sent), window, cx);
        }
    }

    /// Takes the service's answer `result` to the add `id`, and does what x4-core's
    /// [`add_outcome`] says follows.
    pub(super) fn add_finished(
        &mut self,
        id: ActionId,
        result: Result<Added, AddError>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(sent) = self.adds.out.remove(&id) else {
            tracing::warn!(id = id.0, "an add finished that this window did not start");
            return;
        };
        log_answer(&sent, &result);
        self.follow_outcome(id, add_outcome(&sent, result), window, cx);
    }

    /// Does what `outcome` says follows the add `id`: remembers the folder, says what became of
    /// the add, waits for the torrent to select unless the user has made a choice in the list
    /// since the add was sent, and moves the dialog on or keeps its form.
    fn follow_outcome(
        &mut self,
        id: ActionId,
        outcome: AddOutcome,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let shown = self
            .adds
            .dialog
            .as_ref()
            .is_some_and(|(dialog, _)| dialog.read(cx).waiting() == Some(id));
        if let Some(folder) = &outcome.remember {
            self.ui.add.remember(folder);
            self.ui_changed(cx);
            if let Some((dialog, _)) = &self.adds.dialog {
                let history = self.ui.add.recent_folders.clone();
                dialog.update(cx, |dialog, cx| dialog.set_history(history, cx));
            }
        }
        if let Some(notice) = &outcome.notice {
            let kind = match notice {
                AddNotice::Failed { .. } => Notification::error,
                AddNotice::ServiceStopped { .. } => Notification::warning,
                AddNotice::Duplicate { .. } | AddNotice::Unconfirmed { .. } => Notification::info,
            };
            self.add_notice(kind, add_notice_text(notice), window, cx);
        }
        let selects = outcome.select.is_some();
        let choices = self.list_choices(cx);
        let now = cx.background_executor().now();
        if !self.adds.pending.answered(id, outcome.select, choices, now) && selects {
            tracing::debug!("the user chose in the list meanwhile; the torrent is not selected");
        }
        if shown {
            match outcome.form {
                FormStep::Advance => self.skip_add(window, cx),
                FormStep::Keep => {
                    if let Some((dialog, _)) = &self.adds.dialog {
                        dialog.update(cx, |dialog, cx| dialog.set_waiting(None, cx));
                    }
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

    /// Brings the window to the front, and asks for the user's attention if it has not come by
    /// [`RAISE_GRACE`].
    fn raise(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        tracing::debug!("the window is raised for another launch");
        window.activate_window();
        self.adds.raise = Some(cx.spawn_in(window, async move |_, cx| {
            cx.background_executor().timer(RAISE_GRACE).await;
            let looked = cx.update(|window, _| {
                if !window.is_window_active() {
                    tracing::debug!("the window did not come to the front; asking for attention");
                    window.request_attention();
                }
            });
            if let Err(error) = looked {
                tracing::debug!(%error, "the window closed before it came to the front");
            }
        }));
    }

    /// What the user has chosen in the list so far: the gestures on the selection, and the changes
    /// of the filter, the search and the sort order.
    fn list_choices(&self, cx: &App) -> ListChoices {
        ListChoices {
            selection_gestures: self.table.read(cx).delegate().selection_gestures(),
            view_changes: self.view_changes,
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
        let reads = self.reads(&head);
        if let Some((dialog, _)) = &self.adds.dialog {
            let shows = dialog.read(cx).shows(&head);
            dialog.update(cx, |dialog, cx| {
                if shows || reads {
                    dialog.set_position(position, cx);
                } else {
                    dialog.reading(position, cx);
                }
            });
            if shows {
                return;
            }
        } else if window.has_active_dialog(cx) {
            tracing::debug!("another dialog is open; the torrents to add wait");
            return;
        }
        if reads {
            self.watch_slow_read(window, cx);
        } else {
            self.read_head(head, window, cx);
        }
    }

    /// Whether `item` is being read.
    fn reads(&self, item: &AddItem) -> bool {
        self.adds
            .reading
            .as_ref()
            .is_some_and(|reading| &reading.item == item)
    }

    /// Reads `item`, the head of the queue, off the UI thread, and offers it once it is read.
    /// Without a dialog open, the dialog opens after [`READ_GRACE`] to say that it is being read.
    fn read_head(&mut self, item: AddItem, window: &mut Window, cx: &mut Context<'_, Self>) {
        let reading = (self.adds.read)(item.clone(), cx);
        let read = item.clone();
        let task = cx.spawn_in(window, async move |view, cx| {
            let loaded = reading.await;
            if let Err(error) = view.update_in(cx, |view, window, cx| {
                view.head_read(read, loaded, window, cx);
            }) {
                tracing::debug!(%error, "a torrent to add was read after the window closed");
            }
        });
        self.adds.reading = Some(Reading {
            item,
            began: cx.background_executor().now(),
            _read: task,
            slow: None,
        });
        if self.adds.dialog.is_none() {
            self.watch_slow_read(window, cx);
        }
    }

    /// Opens the add dialog to say that the head is being read, with Skip, once it has been read
    /// for [`READ_GRACE`]: now, if that time has passed, as behind another dialog that has just
    /// closed, or when it comes.
    fn watch_slow_read(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let now = cx.background_executor().now();
        let Some(reading) = &mut self.adds.reading else {
            return;
        };
        let item = reading.item.clone();
        let left = READ_GRACE.saturating_sub(now.saturating_duration_since(reading.began));
        if left.is_zero() {
            self.read_is_slow(&item, window, cx);
            return;
        }
        reading.slow = Some(cx.spawn_in(window, async move |view, cx| {
            cx.background_executor().timer(left).await;
            if let Err(error) = view.update_in(cx, |view, window, cx| {
                view.read_is_slow(&item, window, cx);
            }) {
                tracing::debug!(%error, "a slow read outlived the window");
            }
        }));
    }

    /// Opens the dialog to say that `item` is being read, with Skip, if it still is and nothing
    /// else is shown.
    fn read_is_slow(&mut self, item: &AddItem, window: &mut Window, cx: &mut Context<'_, Self>) {
        if !self.reads(item) || self.adds.dialog.is_some() || window.has_active_dialog(cx) {
            return;
        }
        tracing::info!("a torrent file takes long to read; the dialog offers to skip it");
        self.open_add_dialog(None, window, cx);
    }

    /// Offers `item`, read as `loaded`, if it is still the head of the queue: in the dialog, which
    /// opens for it unless another dialog is open. A file that cannot be offered is dropped from
    /// the run with a notification.
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
                self.adds.queue.drop_unreadable();
                self.advance_adds(window, cx);
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
            self.open_add_dialog(Some((item, loaded)), window, cx);
        }
    }

    /// Opens the add dialog for the head of the queue: `read`, the item and what was read of it,
    /// or `None` to say that it is being read.
    fn open_add_dialog(
        &mut self,
        read: Option<(AddItem, LoadedItem)>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let history = self.ui.add.recent_folders.clone();
        let snapshot = self.last_snapshot.clone();
        let connected = self.status.connected();
        let repeats = Rc::clone(&self.key_repeats);
        let dialog = cx.new(|cx| AddDialog::new(repeats, history, snapshot, connected, window, cx));
        let events = cx.subscribe_in(&dialog, window, Self::on_add_dialog_event);
        self.adds.dialog = Some((dialog.clone(), events));
        let position = self.adds.queue.position();
        dialog.update(cx, |dialog, cx| match read {
            Some((item, loaded)) => dialog.show(item, loaded, position, window, cx),
            None => dialog.reading(position, cx),
        });
        tracing::debug!("the add dialog opens");
        adding::open(dialog, window, cx);
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
            AddDialogEvent::Add { add, sent } => {
                self.send_add(dialog, add.as_ref().clone(), sent.clone(), window, cx);
            }
            AddDialogEvent::Show(hash) => {
                tracing::debug!(torrent = %hash, "the torrent in the list is shown");
                let choices = self.list_choices(cx);
                let now = cx.background_executor().now();
                self.adds.pending.wait_for(hash.clone(), choices, now);
                self.apply_pending_selection(window, cx);
                self.skip_add(window, cx);
            }
            AddDialogEvent::Skip => self.cancel_add(window, cx),
            AddDialogEvent::Dropped(paths) => self.files_dropped(paths, window, cx),
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

    /// Sends `add`, the request of the torrent `dialog` shows, and keeps `sent` until it is
    /// answered. If the service has stopped, it is answered at once as not connected.
    fn send_add(
        &mut self,
        dialog: &Entity<AddDialog>,
        add: TorrentAdd,
        sent: SentAdd,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let id = self.pending.next_id();
        tracing::info!(
            torrent = %sent.hash,
            magnet = matches!(add.source, x4_rpc::AddSource::Magnet(_)),
            paused = add.paused,
            unwanted = add.files_unwanted.len(),
            "torrent add sent"
        );
        dialog.update(cx, |dialog, cx| dialog.set_waiting(Some(id), cx));
        self.adds.out.insert(id, sent);
        let choices = self.list_choices(cx);
        self.adds.pending.sent(id, choices);
        if self
            .commands
            .send(Command::Run(id, Action::Add(add)))
            .is_err()
        {
            tracing::warn!("the service has stopped; the add was not sent");
            self.add_finished(id, Err(AddError::NotConnected), window, cx);
        }
    }

    /// Selects the torrent that waits to be selected, once a snapshot lists it, after the filter
    /// and the search that would hide it are cleared.
    fn apply_pending_selection(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.adds.pending.waiting().is_none() {
            return;
        }
        let Some(snapshot) = self.last_snapshot.clone() else {
            return;
        };
        let now = cx.background_executor().now();
        let choices = self.list_choices(cx);
        let Some(reveal) = self
            .adds
            .pending
            .follow(&snapshot, &self.view, choices, now)
        else {
            if self.adds.pending.waiting().is_none() {
                tracing::debug!(
                    "the user chose in the list, or the added torrent did not show in time; \
                     it is not selected"
                );
            }
            return;
        };
        if let Some(view) = reveal.view {
            tracing::debug!("the filter and the search are cleared to show the added torrent");
            let search_cleared = view.search != self.view.search;
            // The view first, so that the search box's own report of the cleared search changes
            // nothing more and counts as no choice of the user's.
            self.show_view(view, cx);
            if search_cleared {
                clear_search(&self.search, window, cx);
            }
        }
        tracing::debug!(torrent = %reveal.hash, "the added torrent is selected");
        self.table
            .update(cx, |table, cx| select_added(table, reveal.hash, cx));
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

/// Logs the daemon's answer `result` to `sent`. The daemon's text of a refusal may name a folder
/// on the server, so it goes to the debug log only.
fn log_answer(sent: &SentAdd, result: &Result<Added, AddError>) {
    match result {
        Ok(added) if added.duplicate => {
            tracing::info!(torrent = %added.hash, "the daemon had the torrent already");
        }
        Ok(added) => tracing::info!(torrent = %added.hash, "torrent added"),
        Err(AddError::Unconfirmed) => {
            tracing::warn!(torrent = %sent.hash, "the daemon has not confirmed the add");
        }
        Err(error) => {
            tracing::error!(torrent = %sent.hash, "the torrent could not be added");
            tracing::debug!(%error, "why the torrent could not be added");
        }
    }
}

#[cfg(test)]
mod tests;
