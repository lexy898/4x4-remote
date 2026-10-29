//! The actions on the selected torrents, as the main view carries them out: from their keys (bound
//! in [`keymap`](crate::keymap)) and from the toolbar's buttons alike.
//!
//! A request takes its targets when the user acts: the selected rows the list shows then, in the
//! order shown. It reads which actions they allow at that moment too, and sends nothing when its
//! action is not among them, so a disabled action does nothing from any entry point. The service
//! answers every action; a failure is shown as a notification and kept as [`MainView::last_failure`].
//!
//! One press of an action's key sends the action once: the repeats of a held key run nothing (see
//! the `key_repeats` module), and neither does a request for an action that is already on its way
//! to the same torrents.
//!
//! Delete and Shift+Delete, and the items of the toolbar's Remove menu, only open a dialog that asks
//! first (the `removal` module), for the targets and the summary of that moment; no second dialog
//! opens over it. The dialog's button sends the removal through the same path as the other actions,
//! for the hashes it took when it opened.
//!
//! The view keeps the availability the toolbar shows up to date: it observes the table, which
//! holds the selection and the rows, and draws the toolbar again only when the availability has
//! changed.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{
    Action, App, Context, Div, Focusable as _, InteractiveElement as _, KeyUpEvent, Subscription,
    Window,
};
use x4_core::{ActionError, ActionId, Availability, Command, availability, removal_summary};
use x4_rpc::{HashString, RemoveData};

use super::MainView;
use crate::actions::{
    ActionKind, PauseTorrents, ReannounceTorrents, RemoveTorrents, RemoveTorrentsWithData,
    StartTorrents, StartTorrentsNow, ToggleTorrents, TorrentRequest, VerifyTorrents, decide,
};
use crate::key_repeats::KeyRepeats;
use crate::removal::{self, Removal};

/// Marks the notification of a failed action, together with the kind of action: the next failure
/// of the same kind replaces it, so that a burst of failures leaves one notification per kind, the
/// newest of each.
struct ActionFailed;

impl MainView {
    /// Which actions the selected torrents that the list shows allow, as the toolbar shows them.
    /// Nothing but copying the hashes is allowed unless the service runs and the daemon answers.
    #[must_use]
    pub fn availability(&self) -> Availability {
        self.availability
    }

    /// What the newest failure of an action said, as its notification says it; `None` until an
    /// action fails.
    #[must_use]
    pub fn last_failure(&self) -> Option<&str> {
        self.last_failure.as_deref()
    }

    /// Recomputes which actions the selected torrents allow; returns whether that changed.
    pub(super) fn refresh_availability(&mut self, cx: &App) -> bool {
        let table = self.table.read(cx).delegate();
        let availability = availability(
            self.status.connected(),
            &table.selection().targets(table.rows()),
        );
        let changed = availability != self.availability;
        self.availability = availability;
        changed
    }

    /// Follows a change of the table, which may be a change of the selection or of the rows: draws
    /// the toolbar again if the actions it allows changed.
    pub(super) fn follow_table_change(&mut self, cx: &mut Context<'_, Self>) {
        if self.refresh_availability(cx) {
            cx.notify();
        }
    }

    /// Carries out `request` on the selected torrents that the list shows now, if they allow it;
    /// otherwise sends nothing.
    pub(crate) fn request(
        &mut self,
        request: TorrentRequest,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let table = self.table.read(cx).delegate();
        let targets = table.selection().targets(table.rows());
        let allowed = availability(self.status.connected(), &targets);
        let Some(kind) = decide(request, &targets, allowed) else {
            tracing::debug!(
                ?request,
                "the selection does not allow this action; nothing sent"
            );
            return;
        };
        let hashes: Vec<_> = targets.iter().map(|row| row.hash.clone()).collect();
        self.send(kind, hashes, window, cx);
    }

    /// Sends the action `kind` on the torrents `hashes` and keeps it until it is answered, unless
    /// the action sent last is the same and still waits.
    fn send(
        &mut self,
        kind: ActionKind,
        hashes: Vec<HashString>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let count = hashes.len();
        let Some(id) = self.pending.start(kind, &hashes) else {
            tracing::debug!(
                action = kind.name(),
                count,
                "the same action on the same torrents is on its way; nothing sent"
            );
            return;
        };
        tracing::info!(action = kind.name(), count, "torrent action sent");
        if self
            .commands
            .send(Command::Run(id, kind.action(hashes)))
            .is_err()
        {
            tracing::warn!(
                action = kind.name(),
                "the service has stopped; the action was not sent"
            );
            self.action_finished(id, &Err(ActionError::NotConnected), window, cx);
        }
    }

    /// Opens the dialog that asks whether to remove the selected torrents that the list shows now,
    /// from the list or with their files (`data`), if they allow removing; otherwise, or while
    /// another dialog is open, does nothing. The dialog takes the targets and their summary now.
    ///
    /// The table gets the focus first, so that closing the dialog gives it back to the table,
    /// whether the dialog was opened from a key or from the toolbar's menu.
    pub(crate) fn ask_removal(
        &mut self,
        data: RemoveData,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if window.has_active_dialog(cx) {
            tracing::debug!(?data, "a dialog is open already; no second one opens");
            return;
        }
        let hashes: Vec<HashString> = {
            let table = self.table.read(cx).delegate();
            let targets = table.selection().targets(table.rows());
            if !availability(self.status.connected(), &targets).remove {
                tracing::debug!(?data, "the selection cannot be removed now; nothing asked");
                return;
            }
            targets.iter().map(|row| row.hash.clone()).collect()
        };
        let Some(snapshot) = &self.last_snapshot else {
            tracing::debug!(?data, "no list has come yet; nothing asked");
            return;
        };
        let summary = removal_summary(snapshot, &hashes);
        tracing::debug!(?data, count = hashes.len(), "asking before a removal");
        window.focus(&self.table.focus_handle(cx), cx);
        removal::open(
            Removal::new(data, hashes, &summary),
            cx.weak_entity(),
            window,
            cx,
        );
    }

    /// Handles the confirm button of a removal dialog: removes the torrents `hashes` the dialog
    /// took when it opened, keeping or deleting their files as `data` says, whatever the list shows
    /// now. Returns `false`, and sends nothing, while the daemon is not connected; the dialog then
    /// stays open.
    pub(crate) fn confirm_removal(
        &mut self,
        data: RemoveData,
        hashes: Vec<HashString>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if !self.status.connected() {
            tracing::debug!(
                ?data,
                count = hashes.len(),
                "the daemon is not connected; the removal is not sent"
            );
            return false;
        }
        self.send(ActionKind::Remove(data), hashes, window, cx);
        true
    }

    /// Handles a press of a toolbar button: carries out `request`, then hands the keyboard focus
    /// back to the table. A click leaves the focus where it is, but a button pressed with the
    /// keyboard holds it, and the list's keys would no longer reach the list.
    pub(crate) fn press_button(
        &mut self,
        request: TorrentRequest,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        self.request(request, window, cx);
        window.focus(&self.table.focus_handle(cx), cx);
    }

    /// Takes the service's answer to the action `id`. A failure is shown as a notification, which
    /// stays until the user dismisses it with a click or a newer failure of the same kind of
    /// action takes its place, and is kept as the last failure; a success shows nothing, as the
    /// next snapshot shows what changed.
    pub(super) fn action_finished(
        &mut self,
        id: ActionId,
        result: &Result<(), ActionError>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(failure) = self.pending.finish(id, result) else {
            return;
        };
        let kind = failure.kind.map_or("other", ActionKind::name);
        window.push_notification(
            Notification::error(failure.text.clone())
                .id1::<ActionFailed>(kind)
                .autohide(false)
                // A click anywhere on it dismisses it, not only on its close button.
                .on_click(|_, _, _| {}),
            cx,
        );
        self.last_failure = Some(failure.text);
    }
}

/// Follows the key-downs in `window` into `repeats`, each before its binding runs, so that the
/// binding's action knows whether the key repeats. The key-ups come from [`on_torrent_actions`].
pub(super) fn follow_key_repeats(
    repeats: &Rc<RefCell<KeyRepeats>>,
    window: &Window,
    cx: &mut App,
) -> Subscription {
    let ours = window.window_handle();
    let repeats = Rc::clone(repeats);
    cx.intercept_keystrokes(move |event, window, _| {
        if window.window_handle() == ours {
            repeats.borrow_mut().key_down(&event.keystroke);
        }
    })
}

/// `root`, the main view's root element, with the handlers of the actions on torrents, and the
/// listener of key-ups that lets a held key go (before any element inside can stop the key-up).
pub(super) fn on_torrent_actions(root: Div, cx: &Context<'_, MainView>) -> Div {
    root.capture_key_up(cx.listener(|view, _: &KeyUpEvent, _, _| {
        view.key_repeats.borrow_mut().release();
    }))
    .on_action(handler::<StartTorrents>(TorrentRequest::Start, cx))
    .on_action(handler::<PauseTorrents>(TorrentRequest::Pause, cx))
    .on_action(handler::<StartTorrentsNow>(TorrentRequest::StartNow, cx))
    .on_action(handler::<ToggleTorrents>(TorrentRequest::Toggle, cx))
    .on_action(handler::<VerifyTorrents>(TorrentRequest::Verify, cx))
    .on_action(handler::<ReannounceTorrents>(
        TorrentRequest::Reannounce,
        cx,
    ))
    .on_action(once_per_press::<RemoveTorrents>(cx, |view, window, cx| {
        view.ask_removal(RemoveData::Keep, window, cx);
    }))
    .on_action(once_per_press::<RemoveTorrentsWithData>(
        cx,
        |view, window, cx| view.ask_removal(RemoveData::Delete, window, cx),
    ))
}

/// The handler of the action `A`, which asks the view of `cx` for `request`, unless the action
/// comes from a repeat of a held key.
fn handler<A: Action>(
    request: TorrentRequest,
    cx: &Context<'_, MainView>,
) -> impl Fn(&A, &mut Window, &mut App) + 'static {
    once_per_press::<A>(cx, move |view, window, cx| {
        view.request(request, window, cx);
    })
}

/// The handler of the action `A`, which runs `run` on the view of `cx`, unless the action comes
/// from a repeat of a held key.
fn once_per_press<A: Action>(
    cx: &Context<'_, MainView>,
    run: impl Fn(&mut MainView, &mut Window, &mut Context<'_, MainView>) + 'static,
) -> impl Fn(&A, &mut Window, &mut App) + 'static {
    cx.listener(move |view, action: &A, window, cx| {
        if view.key_repeats.borrow().repeating() {
            tracing::trace!(
                action = action.name(),
                "a held key repeats; the action is not run again"
            );
            return;
        }
        run(view, window, cx);
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gpui_kit::{AppContext as _, Entity, TestAppContext};
    use x4_core::test_support::torrent;
    use x4_core::{Action, ConnectionState, Snapshot, UiState, status_model};

    use super::*;
    use crate::app;

    #[gpui_kit::test]
    fn a_removal_confirmed_while_disconnected_is_not_sent(cx: &mut TestAppContext) {
        let (_events, events_rx) = async_channel::unbounded();
        let (commands, commands_rx) = crossbeam_channel::unbounded();
        let (handle, view): (_, Entity<MainView>) = cx
            .update(|cx| {
                app::init(cx);
                app::open_main_window(events_rx, commands, None, &UiState::default(), cx)
            })
            .expect("the window opens");
        let hashes = vec![torrent("a").hash];
        let confirm = |cx: &mut TestAppContext| {
            cx.update_window(handle, |_, window, cx| {
                view.update(cx, |view, cx| {
                    view.confirm_removal(RemoveData::Delete, hashes.clone(), window, cx)
                })
            })
            .expect("the window is open")
        };
        let sent = || -> Vec<Action> {
            commands_rx
                .try_iter()
                .filter_map(|command| match command {
                    Command::Run(_, action) => Some(action),
                    _ => None,
                })
                .collect()
        };

        // Not connected: a click that came before its button was drawn disabled sends nothing,
        // and the dialog stays open.
        assert!(!confirm(cx));
        assert_eq!(sent(), []);

        let connected = Snapshot {
            torrents: vec![torrent("a")],
            listed: true,
            session: None,
            free_space_bytes: None,
            rate_down: 0,
            rate_up: 0,
            connection: ConnectionState::Connected,
            stale: false,
        };
        cx.update(|cx| {
            view.update(cx, |view, _| {
                view.status = status_model(&Arc::new(connected));
            });
        });
        assert!(confirm(cx));
        assert_eq!(
            sent(),
            [Action::Remove {
                hashes: hashes.clone(),
                data: RemoveData::Delete
            }]
        );
    }
}
