//! The details panel under the torrent table: a tab bar with General and Files, and under it the
//! open tab for the torrent the selection names, or a line that says why there is none.
//!
//! The panel draws what the main view hands it and decides nothing itself:
//!
//! - the target, x4-core's [`DetailsTarget`]: no torrent ("Select a torrent"), several ("3
//!   torrents selected") or one, with its row of the daemon's list;
//! - the newest details the service sent. They are shown only while their hash is the target's,
//!   so the panel never shows the comment of one torrent under the name of another while the
//!   selection moves; the rows that need them say "—" until they come (the `general` module);
//! - the open tab, which the main view keeps in the state it remembers. A click on a tab only asks
//!   for it ([`DetailsPanelEvent::TabChosen`]), as Alt+1 and Alt+2 do.
//!
//! The Files tab (the `files` module) shows the torrent's files as a tree, with whether each is
//! downloaded, its progress and priority, and the switch of downloading in order. Its changes go
//! to the main view as [`DetailsPanelEvent::Files`], which sends them while the daemon is
//! connected (the panel draws its controls disabled otherwise) and the torrent is still the
//! panel's.
//!
//! The copy buttons of the General tab ask the main view to copy ([`DetailsPanelEvent::Copy`]),
//! which writes the clipboard at once and gives the keys back to the table.

mod files;
mod general;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, v_flex};
use gpui_kit::{
    Context, EventEmitter, InteractiveElement as _, IntoElement, ParentElement as _, Render, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _,
    Window, div,
};
use x4_core::{DetailsTab, DetailsTarget, GeneralModel, Torrent, TorrentDetails, general_model};

use crate::actions::FileRequest;
use crate::i18n::{details_placeholder_text, t};
use crate::key_repeats::KeyRepeats;
use files::FilesTab;

/// The element id of the panel.
const PANEL_ID: &str = "details";
/// The element id of the tab bar; it keys the bar's own state in the window, so it stays.
const TABS_ID: &str = "details-tabs";

/// What the panel asks the main view for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DetailsPanelEvent {
    /// The user clicked the tab.
    TabChosen(DetailsTab),
    /// The user clicked a copy button.
    Copy(CopyRequest),
    /// The user changed the torrent's files, or its downloading in order, in the Files tab.
    Files(FileRequest),
}

/// A text to copy, as a copy button of the panel drew it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CopyRequest {
    /// The text, such as a hash.
    pub(crate) text: String,
    /// What it is, for the log: "hash", "path".
    pub(crate) what: &'static str,
}

/// The details panel.
pub(crate) struct DetailsPanel {
    /// The open tab.
    tab: DetailsTab,
    /// What the panel is about.
    target: DetailsTarget,
    /// The row of the daemon's list for a target of one torrent.
    torrent: Option<Torrent>,
    /// The newest details the service sent, whichever torrent they are of.
    details: Option<Arc<TorrentDetails>>,
    /// What the General tab shows for `torrent` and `details`; `None` without one torrent.
    general: Option<GeneralModel>,
    /// The tree the Files tab shows, made from `details` while they are the torrent's and the tab
    /// is open, and what the user has done to it.
    files: FilesTab,
    /// Whether the daemon is connected, so that the Files tab's controls can act.
    connected: bool,
    /// The key held down now, shared with the main view, so that Space, `+` and `-` in the file
    /// list act once per press.
    key_repeats: Rc<RefCell<KeyRepeats>>,
    /// The file list getting the focus.
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DetailsPanelEvent> for DetailsPanel {}

impl DetailsPanel {
    /// A panel on the tab `tab`, about no torrent yet. `key_repeats` is the main view's: it tells
    /// whether the last key-down repeated a held key.
    pub(crate) fn new(
        tab: DetailsTab,
        key_repeats: Rc<RefCell<KeyRepeats>>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let files = FilesTab::new(cx);
        // A key let go while the focus was elsewhere sent the main view no key-up: forget it when
        // the file list has the focus again, as the table does.
        let subscriptions = vec![cx.on_focus_in(files.focus(), window, |panel, _, _| {
            panel.key_repeats.borrow_mut().release();
        })];
        Self {
            tab,
            target: DetailsTarget::None,
            torrent: None,
            details: None,
            general: None,
            files,
            connected: false,
            key_repeats,
            _subscriptions: subscriptions,
        }
    }

    /// Opens the tab `tab`. The Files tab makes its tree then, from the newest details.
    pub(crate) fn set_tab(&mut self, tab: DetailsTab, cx: &mut Context<'_, Self>) {
        if tab != self.tab {
            self.tab = tab;
            self.follow_files();
            cx.notify();
        }
    }

    /// Shows `target`, with `torrent`, its row of the daemon's list when it is one torrent. Draws
    /// again only when what the panel shows changed.
    pub(crate) fn show(
        &mut self,
        target: DetailsTarget,
        torrent: Option<&Torrent>,
        cx: &mut Context<'_, Self>,
    ) {
        let torrent = match target {
            DetailsTarget::One(_) => torrent,
            DetailsTarget::None | DetailsTarget::Many(_) => None,
        };
        if target == self.target && torrent == self.torrent.as_ref() {
            return;
        }
        self.target = target;
        self.torrent = torrent.cloned();
        self.general = self.general_model();
        self.follow_files();
        cx.notify();
    }

    /// Takes the details the service sent, which are shown while they are the target's. Draws
    /// again only when the General tab or the Files tab changed.
    pub(crate) fn take_details(
        &mut self,
        details: Arc<TorrentDetails>,
        cx: &mut Context<'_, Self>,
    ) {
        self.details = Some(details);
        let general = self.general_model();
        let files_changed = self.follow_files();
        if general != self.general || files_changed {
            self.general = general;
            cx.notify();
        }
    }

    /// Tells the panel whether the daemon is connected; the Files tab's controls act only while
    /// it is.
    pub(crate) fn set_connected(&mut self, connected: bool, cx: &mut Context<'_, Self>) {
        if connected != self.connected {
            self.connected = connected;
            cx.notify();
        }
    }

    /// Fits the Files tab to the newest details while they are the target's, and to none
    /// otherwise; returns whether it shows something else now. Only while the tab is shown: the
    /// progress is read again every few seconds, and a tree of thousands of files is not made
    /// for nothing. Opening the tab fits it then.
    fn follow_files(&mut self) -> bool {
        if self.tab != DetailsTab::Files {
            return false;
        }
        let details = match &self.target {
            DetailsTarget::One(hash) => self
                .details
                .as_ref()
                .filter(|details| &details.hash == hash),
            DetailsTarget::None | DetailsTarget::Many(_) => None,
        };
        self.files.follow(details)
    }

    /// What the General tab shows for the torrent and the newest details: those of another
    /// torrent are left out.
    fn general_model(&self) -> Option<GeneralModel> {
        self.torrent
            .as_ref()
            .map(|torrent| general_model(torrent, self.details.as_deref()))
    }

    /// The tab bar.
    fn tab_bar(&self, cx: &mut Context<'_, Self>) -> TabBar {
        TabBar::new(TABS_ID)
            .underline()
            .small()
            .px_2()
            .selected_index(tab_index(self.tab))
            .on_click(cx.listener(|_, index: &usize, _, cx| choose_tab(*index, cx)))
            .child(Tab::new().label(tab_name(DetailsTab::General)))
            .child(Tab::new().label(tab_name(DetailsTab::Files)))
    }
}

impl Render for DetailsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let tabs = self.tab_bar(cx);
        let body = div()
            .id("details-body")
            .role(Role::TabPanel)
            .aria_label(tab_name(self.tab))
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col();
        let body = match (details_placeholder_text(&self.target), &self.general) {
            (Some(text), _) => body.child(
                div()
                    .id("details-placeholder")
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(muted)
                    .aria_label(text.clone())
                    .child(text)
                    .test_support(),
            ),
            (None, Some(general)) => match (self.tab, &self.torrent) {
                (DetailsTab::General, _) => body
                    .overflow_y_scroll()
                    .child(general::general_tab(general, cx)),
                (DetailsTab::Files, Some(torrent)) => {
                    body.child(self.files_tab(torrent, window, cx))
                }
                (DetailsTab::Files, None) => body,
            },
            (None, None) => body,
        };
        v_flex()
            .id(PANEL_ID)
            .size_full()
            .text_sm()
            .child(tabs)
            .child(body)
            .test_support()
    }
}

/// Handles a click on the tab at `index` of the panel of `cx`: asks the main view for it.
fn choose_tab(index: usize, cx: &mut Context<'_, DetailsPanel>) {
    if let Some(tab) = tab_at(index) {
        tracing::debug!(?tab, "details tab clicked");
        cx.emit(DetailsPanelEvent::TabChosen(tab));
    }
}

/// The position of `tab` in the tab bar.
fn tab_index(tab: DetailsTab) -> usize {
    match tab {
        DetailsTab::General => 0,
        DetailsTab::Files => 1,
    }
}

/// The tab at `index` of the tab bar.
fn tab_at(index: usize) -> Option<DetailsTab> {
    [DetailsTab::General, DetailsTab::Files].get(index).copied()
}

/// The name of `tab`, as its tab says it.
fn tab_name(tab: DetailsTab) -> SharedString {
    match tab {
        DetailsTab::General => t!("details-tab-general"),
        DetailsTab::Files => t!("details-tab-files"),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_tab_has_its_place_in_the_bar() {
        for tab in [DetailsTab::General, DetailsTab::Files] {
            assert_eq!(tab_at(tab_index(tab)), Some(tab));
        }
        assert_eq!(tab_at(2), None);
        assert_eq!(tab_name(DetailsTab::General), "General");
        assert_eq!(tab_name(DetailsTab::Files), "Files");
    }
}
