//! The Connection dialog, as the main view opens it and carries out what it asks for.
//!
//! The dialog opens from the toolbar's Connection button, from Ctrl+, and from a click on the
//! status bar's connection indicator, and by itself at a start with no server set
//! ([`launch`](crate::app::launch)); never over another dialog, as the dialogs keep the keys and
//! the modal overlay covers the rest. It shows the server the window is set to, and tests with the
//! window's check ([`ConnectionSetup`](crate::connection::ConnectionSetup)).
//!
//! Save with the settings the window already has only closes the dialog. Other settings are
//! written to `config.toml` first, when the window has one, and the window then switches to their
//! server ([`MainView::switch_to`]); the notice that the old file could not be used goes, as the
//! file is now one 4x4 wrote. A file that cannot be written changes nothing: the dialog stays
//! open and says why. Without a config file (no home directory, the demo) Save switches without
//! writing. Cancel and Escape close the dialog and change nothing.
//!
//! The torrents to add wait while the dialog is open; once it closes, the head of the queue is
//! offered, by the switch when Save switches, and as after any other dialog otherwise.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::component::WindowExt as _;
use gpui_kit::{
    AppContext as _, Context, Div, Entity, Focusable as _, InteractiveElement as _, Window,
};
use x4_core::ConnectionSettings;

use super::MainView;
use crate::connection_dialog::{self, ConnectionDialog, ConnectionDialogEvent};
use crate::files;
use crate::i18n::connection_save_reason_text;
use crate::key_repeats::{KeyRepeats, once_per_press_of};
use crate::keymap::OpenConnection;

impl MainView {
    /// Opens the Connection dialog, filled with the server the window is set to, unless a dialog
    /// is open already: then it does nothing.
    ///
    /// It opens in any window. In one that cannot switch servers (no connector, as the bare
    /// windows of tests), Save closes it and the window stays as it is.
    ///
    /// The table gets the focus first, so that closing the dialog gives it back to the table,
    /// whether the dialog was opened from a key, the toolbar or the status bar.
    pub fn open_connection(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if window.has_active_dialog(cx) {
            tracing::debug!("a dialog is open already; the Connection dialog does not open");
            return;
        }
        tracing::debug!("the Connection dialog opens");
        window.focus(&self.table.focus_handle(cx), cx);
        let settings = self.connection.settings.clone();
        let check = self.connection.check.clone();
        let repeats = Rc::clone(&self.key_repeats);
        let dialog =
            cx.new(|cx| ConnectionDialog::new(repeats, settings.as_ref(), check, window, cx));
        let events = cx.subscribe_in(&dialog, window, Self::on_connection_dialog_event);
        self.connection_dialog = Some((dialog.clone(), events));
        connection_dialog::open(dialog, window, cx);
    }

    /// Carries out what the Connection dialog asks for. A dialog that is no longer the open one,
    /// as one still closing, is not listened to: closing it again would close another.
    fn on_connection_dialog_event(
        &mut self,
        dialog: &Entity<ConnectionDialog>,
        event: &ConnectionDialogEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self
            .connection_dialog
            .as_ref()
            .is_none_or(|(open, _)| open != dialog)
        {
            tracing::debug!("a Connection dialog that has closed asked for something; ignored");
            return;
        }
        match event {
            ConnectionDialogEvent::Save(settings) => {
                self.save_and_switch(dialog, settings.clone(), window, cx);
            }
            ConnectionDialogEvent::Cancel => {
                tracing::debug!("the Connection dialog was cancelled; nothing changes");
                self.close_connection_dialog(window, cx);
                self.dialog_closed(window, cx);
            }
        }
    }

    /// Handles Save in the Connection dialog with `settings`: closes the dialog if they are the
    /// window's already; otherwise writes them to `config.toml`, if the window has one, and
    /// switches to their server. A write that fails leaves the dialog open, with the reason.
    fn save_and_switch(
        &mut self,
        dialog: &Entity<ConnectionDialog>,
        settings: ConnectionSettings,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        if self.connection.settings.as_ref() == Some(&settings) {
            tracing::debug!("the connection did not change; the dialog closes");
            self.close_connection_dialog(window, cx);
            self.dialog_closed(window, cx);
            return;
        }
        if let Some(path) = &self.connection.config_file {
            // On the UI thread: one small file at the user's Save, whose outcome decides whether
            // the dialog closes, as Cancel could not once the file is written.
            match files::save_connection(path, &settings) {
                Ok(saved) => {
                    tracing::debug!(moved_aside = saved.moved_aside, "the connection is saved");
                    // The file is one 4x4 wrote now.
                    self.dismiss_config_problem(cx);
                }
                Err(error) => {
                    // Both name the file and what the system said, never a setting.
                    tracing::error!(error = %error.report(), "the connection could not be saved; nothing changes");
                    let reason = connection_save_reason_text(&error);
                    dialog.update(cx, |dialog, cx| dialog.show_save_error(reason, cx));
                    return;
                }
            }
        } else {
            tracing::info!("this window has no config file; the server is not saved");
        }
        self.close_connection_dialog(window, cx);
        let switches = self.connection.connector.is_some();
        self.switch_to(settings, window, cx);
        // A switch offers the head of the queue itself.
        if !switches {
            self.dialog_closed(window, cx);
        }
    }

    /// Closes the Connection dialog, if it is open, and lets go of it.
    fn close_connection_dialog(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        if self.connection_dialog.take().is_some() {
            tracing::debug!("the Connection dialog closes");
            window.close_dialog(cx);
        }
    }
}

/// `root`, the main view's root element, with the handler of [`OpenConnection`] (Ctrl+,), which
/// opens the dialog once per press of the key; `repeats` is the view's record of the key held.
pub(super) fn on_connection_actions(
    root: Div,
    repeats: &Rc<RefCell<KeyRepeats>>,
    cx: &Context<'_, MainView>,
) -> Div {
    root.on_action(once_per_press_of::<_, OpenConnection>(
        repeats,
        cx,
        MainView::open_connection,
    ))
}

#[cfg(test)]
mod tests;
