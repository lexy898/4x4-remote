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
use x4_core::{ActionError, ActionId, Availability, Command, availability};

use super::MainView;
use crate::actions::{
    ActionKind, PauseTorrents, ReannounceTorrents, StartTorrents, StartTorrentsNow, ToggleTorrents,
    TorrentRequest, VerifyTorrents, decide,
};
use crate::key_repeats::KeyRepeats;

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
}

/// The handler of the action `A`, which asks the view of `cx` for `request`, unless the action
/// comes from a repeat of a held key.
fn handler<A: Action>(
    request: TorrentRequest,
    cx: &Context<'_, MainView>,
) -> impl Fn(&A, &mut Window, &mut App) + 'static {
    cx.listener(move |view, _: &A, window, cx| {
        if view.key_repeats.borrow().repeating() {
            tracing::trace!(?request, "a held key repeats; the action is not run again");
            return;
        }
        view.request(request, window, cx);
    })
}
