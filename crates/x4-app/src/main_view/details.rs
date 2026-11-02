//! The details panel under the table, as the main view runs it: what it shows, when the service
//! reads the details of a torrent, the split it shares with the table, and its keys.
//!
//! The panel is about the selected torrents the list shows (x4-core's [`details_target`]): it
//! shows one torrent only when exactly one is selected. The view asks the service to read that
//! torrent's details (`Command::Select`) while the panel is open, and to read none
//! (`Select(None)`) while it is hidden or shows no single torrent. It sends only a change, so a
//! new snapshot, a click on the same row or a redraw of the table sends nothing; it looks again
//! whenever the selection, the rows the list shows or the panel's visibility change. The service
//! paces the reads itself, so the view polls nothing.
//!
//! The panel and the table share a vertical split. The panel keeps the height the user gave it by
//! dragging the handle between them, which is remembered; nothing else changes that height. The
//! window fits the panel to the room it leaves: when the window is too low for the remembered
//! height under the table's header and two rows, the panel is drawn lower, and it grows back as
//! the window does. A hidden panel keeps its height, and is fitted again when it shows.
//!
//! Alt+Enter shows and hides the panel, and Alt+1 and Alt+2 open its tabs, once per press of the
//! key. The open tab and whether the panel is shown are remembered too.

use std::sync::Arc;

use gpui_kit::component::resizable::{resizable_panel, v_resizable};
use gpui_kit::{
    Context, Div, Entity, Focusable as _, InteractiveElement as _, IntoElement, ParentElement as _,
    Pixels, Styled as _, Window, div, px,
};
use x4_core::{
    Command, DetailsTab, DetailsTarget, MAX_DETAILS_HEIGHT, MIN_DETAILS_HEIGHT, TorrentDetails,
    details_target,
};
use x4_rpc::HashString;

use super::copying::write_to_clipboard;
use super::torrent_actions::once_per_press;
use super::{HEADER_HEIGHT, MainView, ROW_HEIGHT};
use crate::details_panel::{DetailsPanel, DetailsPanelEvent};
use crate::keymap::{ShowFilesTab, ShowGeneralTab, ToggleDetails};

/// The element id of the split of the table and the panel.
const DETAILS_SPLIT: &str = "details-split";
/// The lowest the table may be, so that the panel never hides it: its header and two rows.
const TABLE_MIN_HEIGHT: f32 = HEADER_HEIGHT + 2. * ROW_HEIGHT;
/// The difference in height, in pixels, below which the panel is left as it is: the split rounds
/// its sizes.
const FIT_TOLERANCE: f32 = 0.5;

impl MainView {
    /// Follows a change of the selection, of the rows the list shows or of the panel's
    /// visibility: hands the panel what it is about now, and asks the service for the details it
    /// needs, if that changed.
    pub(super) fn follow_details(&mut self, cx: &mut Context<'_, Self>) {
        let target = {
            let table = self.table.read(cx).delegate();
            details_target(table.selection(), table.rows())
        };
        self.ask_for_details(&target);
        let torrent = match &target {
            DetailsTarget::One(hash) => self.last_snapshot.as_ref().and_then(|snapshot| {
                snapshot
                    .torrents
                    .iter()
                    .find(|torrent| &torrent.hash == hash)
            }),
            DetailsTarget::None | DetailsTarget::Many(_) => None,
        };
        self.details_panel
            .update(cx, |panel, cx| panel.show(target, torrent, cx));
    }

    /// Asks the service to read the details of the one torrent of `target` while the panel is
    /// open, and none otherwise; sends nothing when that is what it asked for last.
    fn ask_for_details(&mut self, target: &DetailsTarget) {
        let wanted = match target {
            DetailsTarget::One(hash) if self.ui.details.open => Some(hash.clone()),
            DetailsTarget::One(_) | DetailsTarget::None | DetailsTarget::Many(_) => None,
        };
        if wanted == self.selected_details {
            return;
        }
        tracing::debug!(torrent = ?wanted.as_ref().map(HashString::as_str), "details selected");
        self.selected_details.clone_from(&wanted);
        if self.commands.send(Command::Select(wanted)).is_err() {
            tracing::debug!("the service has stopped; the selection of details was not sent");
        }
    }

    /// Hands the panel the details the service sent; it shows them while they are its torrent's.
    pub(super) fn details_arrived(&self, details: Arc<TorrentDetails>, cx: &mut Context<'_, Self>) {
        self.details_panel
            .update(cx, |panel, cx| panel.take_details(details, cx));
    }

    /// Handles Alt+Enter: hides the panel, keeping its height, or shows it again, fitted to the
    /// window.
    fn toggle_details(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.ui.details.open = !self.ui.details.open;
        tracing::debug!(open = self.ui.details.open, "details panel shown or hidden");
        self.ui_changed(cx);
        if self.ui.details.open {
            self.fit_details(window, cx);
        }
        self.follow_details(cx);
        cx.notify();
    }

    /// Opens the tab `tab` of the panel, from its key or a click on it, and remembers it.
    fn show_details_tab(&mut self, tab: DetailsTab, cx: &mut Context<'_, Self>) {
        if self.ui.details.tab == tab {
            return;
        }
        tracing::debug!(?tab, "details tab shown");
        self.ui.details.tab = tab;
        self.ui_changed(cx);
        self.details_panel
            .update(cx, |panel, cx| panel.set_tab(tab, cx));
    }

    /// Carries out what the panel asks for: a tab; a copy, which writes the clipboard at once
    /// and gives the keys back to the table, as a toolbar button does; or a change in the Files
    /// tab, which goes if it can go now.
    pub(super) fn on_details_event(
        &mut self,
        _: &Entity<DetailsPanel>,
        event: &DetailsPanelEvent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match event {
            DetailsPanelEvent::TabChosen(tab) => self.show_details_tab(*tab, cx),
            DetailsPanelEvent::Copy(copy) => {
                write_to_clipboard(copy.text.clone(), 1, copy.what, cx);
                window.focus(&self.table.focus_handle(cx), cx);
            }
            DetailsPanelEvent::Files(request) => self.request_files(request, window, cx),
        }
    }

    /// The user let go of the handle between the table and the panel: remembers the height the
    /// drag left the panel at. The split reports nothing else here, not even its own fits.
    fn details_dragged(&mut self, cx: &mut Context<'_, Self>) {
        let Some(&height) = self.details_split.read(cx).sizes().get(1) else {
            return;
        };
        let height = f32::from(height).clamp(MIN_DETAILS_HEIGHT, MAX_DETAILS_HEIGHT);
        tracing::debug!(height, "details panel resized");
        self.ui.details.height = height;
        self.ui_changed(cx);
    }

    /// Follows a change of the split: when the window gave it another height, fits the panel to
    /// it. A drag changes the split too, but not its height, so the fit never undoes a drag.
    pub(super) fn follow_split(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let container = self.details_split.read(cx).container_size();
        if !self.ui.details.open || self.fitted_split == Some(container) {
            return;
        }
        self.fit_details(window, cx);
    }

    /// Draws the panel at its remembered height, or lower if the split has less room for it
    /// under the table's minimum. Before the split's first layout there is nothing to fit.
    fn fit_details(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let (container, drawn) = {
            let split = self.details_split.read(cx);
            (split.container_size(), split.sizes().get(1).copied())
        };
        let Some(drawn) = drawn.filter(|_| container > Pixels::ZERO) else {
            return;
        };
        self.fitted_split = Some(container);
        let height = self
            .ui
            .details
            .fitted_height(f32::from(container) - TABLE_MIN_HEIGHT);
        if (f32::from(drawn) - height).abs() <= FIT_TOLERANCE {
            return;
        }
        tracing::debug!(height, "details panel fitted to the window");
        self.details_split.update(cx, |split, cx| {
            split.resize_panel(1, px(height), window, cx);
        });
    }

    /// The torrent list `list` above the panel, with the handle between them. A drag of the
    /// handle is remembered when it ends.
    pub(super) fn list_and_details<List: IntoElement>(
        &self,
        list: List,
        cx: &Context<'_, Self>,
    ) -> impl IntoElement + use<List> {
        let this = cx.weak_entity();
        // The split overflows a window lower than both minimums; it is cut off at the status bar.
        div().flex_1().min_w_0().h_full().overflow_hidden().child(
            v_resizable(DETAILS_SPLIT)
                .with_state(&self.details_split)
                .on_resize(move |_, _, cx| {
                    if let Err(error) = this.update(cx, Self::details_dragged) {
                        tracing::debug!(%error, "the panel was resized after the window closed");
                    }
                })
                .child(
                    resizable_panel()
                        .size_range(px(TABLE_MIN_HEIGHT)..Pixels::MAX)
                        .child(list),
                )
                .child(
                    resizable_panel()
                        .visible(self.ui.details.open)
                        // Only the first layout takes it; after that the split keeps the height.
                        .size(px(self.ui.details.height))
                        .size_range(px(MIN_DETAILS_HEIGHT)..px(MAX_DETAILS_HEIGHT))
                        .fixed()
                        .child(self.details_panel.clone()),
                ),
        )
    }
}

/// `root`, the main view's root element, with the handlers of Alt+Enter, Alt+1 and Alt+2, which
/// run once per press of their key.
pub(super) fn on_details_actions(root: Div, cx: &Context<'_, MainView>) -> Div {
    root.on_action(once_per_press::<ToggleDetails>(
        cx,
        MainView::toggle_details,
    ))
    .on_action(once_per_press::<ShowGeneralTab>(cx, |view, _, cx| {
        view.show_details_tab(DetailsTab::General, cx);
    }))
    .on_action(once_per_press::<ShowFilesTab>(cx, |view, _, cx| {
        view.show_details_tab(DetailsTab::Files, cx);
    }))
}
