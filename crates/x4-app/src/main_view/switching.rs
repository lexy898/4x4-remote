//! The server the window is set to, and switching it to another one without a restart.
//!
//! A switch replaces the service, and never points the old one elsewhere. The old handle is
//! dropped, which asks its thread to stop (it ends once its request in flight comes back, and the
//! answer goes nowhere), and so is the pump of its events, so that nothing the old service still
//! sends reaches the view. The view then forgets the old daemon: the rows and the selection, the
//! details panel, the status, the actions and the read of magnet links waiting for their answers,
//! the last failure and the notifications on screen, the speed limits form and its popover, the
//! adds sent, the torrent waiting to be selected, and the notice that `config.toml` set no server.
//! What the user arranged stays: the columns, the sort order, the filter, the search, the theme,
//! the panel and its tab, and the torrents waiting to be added.
//!
//! The new service is told what a service is told at the start (`MainView::new`): whether the
//! user can see the window, and which tab of the panel is open. A connector that cannot start the
//! service still drops the old one, as the user asked for the new server; the window then says
//! that the service could not start, and is set to the new server all the same.

use gpui_kit::component::WindowExt as _;
use gpui_kit::{Context, Task, Window};
use x4_core::{ConnectionSettings, DetailsTab, StatusModel, TableModel};

use super::MainView;
use super::events::start_pump;
use crate::clipboard::MagnetCopy;
use crate::connection::{ConnectionSetup, ServiceChannels};
use crate::torrent_table::show_rows;

impl MainView {
    /// Hands the view how it reaches servers: the server it is set to, which the status bar's
    /// tooltip names once connected, how it starts a service for another one, how it checks one
    /// and where it saves the choice. The service it shows now stays.
    pub fn set_connection(&mut self, setup: ConnectionSetup, cx: &mut Context<'_, Self>) {
        tracing::debug!(
            server = setup
                .settings
                .as_ref()
                .map(ConnectionSettings::server_label),
            can_switch = setup.connector.is_some(),
            saves = setup.config_file.is_some(),
            "the window's connection"
        );
        self.connection = setup;
        cx.notify();
    }

    /// The server the window is set to; `None` when no server is set.
    #[must_use]
    pub fn connection_settings(&self) -> Option<&ConnectionSettings> {
        self.connection.settings.as_ref()
    }

    /// Switches the window to the server of `settings`: drops the old service, forgets what the
    /// old daemon showed, and starts a service for the new one with the connector of
    /// [`MainView::set_connection`], which it tells the window's visibility and the open tab. The
    /// UI state and the torrents waiting to be added stay. A window without a connector cannot
    /// switch, and stays as it is.
    pub fn switch_to(
        &mut self,
        settings: ConnectionSettings,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(connector) = self.connection.connector.clone() else {
            tracing::warn!(server = %settings.server_label(), "this window cannot switch servers; it stays as it is");
            return;
        };
        tracing::info!(server = %settings.server_label(), "switching to another server");
        // Before anything else, so that nothing the old service sends reaches the view again.
        self.pump = Task::ready(());
        self.service = None;
        self.forget_the_daemon(window, cx);
        let started = connector(&settings);
        self.connection.settings = Some(settings);
        let channels = started.unwrap_or_else(|error| {
            tracing::error!(%error, "the service could not start; the window has no data to show");
            self.status.service_failed();
            ServiceChannels::disconnected()
        });
        let ServiceChannels {
            events,
            commands,
            handle,
        } = channels;
        self.commands = commands;
        self.service = handle;
        // A disconnected channel is closed at once, which changes nothing of a failed service.
        self.pump = start_pump(events, window, cx);
        self.tell_the_new_service(window, cx);
        self.show_status(cx);
        self.refresh_availability(cx);
        self.follow_details(cx);
        self.follow_adds(window, cx);
        cx.notify();
    }

    /// Forgets everything the old daemon showed or was asked, as the window switches servers.
    fn forget_the_daemon(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.last_snapshot = None;
        self.model = TableModel::default();
        // No rows keep no selection: the anchor and the lead go too.
        let model = &self.model;
        self.table
            .update(cx, |table, cx| show_rows(table, model, cx));
        self.status = StatusModel::default();
        self.pending.forget_all();
        self.magnet = MagnetCopy::default();
        self.last_failure = None;
        window.clear_notifications(cx);
        if self.limits_open {
            self.close_speed_limits(window, cx);
        }
        self.speed_limits
            .update(cx, |form, cx| form.forget(window, cx));
        // The details panel follows the rows: with none it is about no torrent, and lets go of
        // what it showed when the switch hands it that (`follow_details`). A hidden one holds
        // nothing.
        // The notice said that no server is set, which no longer holds.
        self.config_problem = None;
        self.adds_switched(cx);
    }

    /// Tells the new service what a service starts without knowing: whether the user can see the
    /// window, and which tab of the panel is open. It starts with no torrent's details, as the
    /// view does now.
    fn tell_the_new_service(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        self.last_visibility = None;
        self.report_visibility(window, cx);
        self.selected_details = None;
        self.shown_tab = DetailsTab::default();
        self.ask_for_tab();
    }
}

#[cfg(test)]
mod tests;
