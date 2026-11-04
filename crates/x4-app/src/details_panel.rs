//! The details panel under the torrent table: a tab bar with General, Files, Peers and Trackers,
//! and under it the open tab for the torrent the selection names, or a line that says why there is none.
//!
//! The panel draws what the main view hands it and decides nothing itself:
//!
//! - the target, x4-core's [`DetailsTarget`]: no torrent ("Select a torrent"), several ("3
//!   torrents selected") or one, with its row of the daemon's list;
//! - the newest details the service sent of the target's torrent. Details of another torrent are
//!   not kept, and those of the old torrent go when the target changes, so the panel never shows
//!   the comment of one torrent under the name of another while the selection moves; the rows that
//!   need them say "—" until they come (the `general` module);
//! - the open tab, which the main view keeps in the state it remembers. A click on a tab only asks
//!   for it ([`DetailsPanelEvent::TabChosen`]), as Alt+1, Alt+2, Alt+3 and Alt+4 do.
//!
//! The Files tab (the `files` module) shows the torrent's files as a tree, with whether each is
//! downloaded, its progress and priority, and the switch of downloading in order. Its changes go
//! to the main view as [`DetailsPanelEvent::Files`], which sends them while the daemon is
//! connected (the panel draws its controls disabled otherwise) and the torrent is still the
//! panel's.
//!
//! The Peers tab (the `peers` module) shows the peers the service reads while it is open, in
//! the shared list of the `grid` module, which the Trackers tab (the `trackers` module) uses too.
//! The trackers are part of the details, which the service sends again every few seconds while
//! the Trackers tab is open.
//!
//! The copy buttons of the General tab ask the main view to copy ([`DetailsPanelEvent::Copy`]),
//! which writes the clipboard at once and gives the keys back to the table.
//!
//! A hidden panel holds nothing it showed: no details, no General tab, no tree or rows of the
//! Files tab, which can be several MiB for a torrent of thousands of files, and no peers or
//! tracker rows. Only the Files tab's open folders and cursor stay. Shown again, the panel is about
//! the selection then, and the read the service makes for it fills it.

mod files;
mod general;
mod grid;
mod peers;
mod trackers;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, v_flex};
use gpui_kit::{
    AnyElement, App, Context, EventEmitter, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, TestSupportExt as _, Window, div,
};
use x4_core::{DetailsTab, DetailsTarget, GeneralModel, Torrent, TorrentDetails, general_model};
use x4_rpc::HashString;

use crate::actions::FileRequest;
use crate::i18n::{details_placeholder_text, t};
use crate::key_repeats::KeyRepeats;
use files::FilesTab;
use peers::PeersTab;
use trackers::TrackersTab;

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
    /// The newest details the service sent of the target's torrent; `None` until they come, and
    /// while the target is no single torrent.
    details: Option<Arc<TorrentDetails>>,
    /// What the General tab shows for `torrent` and `details`; `None` without one torrent.
    general: Option<GeneralModel>,
    /// The tree the Files tab shows, made from `details` while they are the torrent's and the tab
    /// is open, and what the user has done to it.
    files: FilesTab,
    /// The peers the Peers tab shows and the order chosen for them.
    peers: PeersTab,
    /// The rows the Trackers tab shows, made from `details`.
    trackers: TrackersTab,
    /// Whether the daemon is connected, so that the Files tab's controls can act.
    connected: bool,
    /// Whether what the panel shows is stale, as while the daemon does not answer: the rows of
    /// the Peers and the Trackers tab are dimmed then.
    stale: bool,
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
            peers: PeersTab::new(),
            trackers: TrackersTab::new(),
            connected: false,
            stale: false,
            key_repeats,
            _subscriptions: subscriptions,
        }
    }

    /// Opens the tab `tab`. The Files tab makes its tree then, from the newest details, and lets
    /// it go when another tab opens; its open folders and cursor stay for its next tree. The
    /// Peers tab waits for the peers the service reads now that it is open, and lets them and
    /// their order go when another tab opens. The Trackers tab shows the trackers of the newest
    /// details at once, and lets its rows go when another tab opens.
    pub(crate) fn set_tab(&mut self, tab: DetailsTab, cx: &mut Context<'_, Self>) {
        if tab != self.tab {
            self.tab = tab;
            match tab {
                DetailsTab::Files => {
                    self.peers.leave();
                    self.follow_files();
                }
                DetailsTab::Peers => self.files.leave(),
                DetailsTab::General | DetailsTab::Trackers => {
                    self.files.leave();
                    self.peers.leave();
                }
            }
            self.follow_trackers(cx);
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
        // The details of a torrent the panel no longer shows are of no more use.
        self.details = self
            .details
            .take()
            .filter(|details| is_of(&target, &details.hash));
        self.peers.follow(&target);
        self.target = target;
        self.torrent = torrent.cloned();
        self.general = self.general_model();
        self.follow_files();
        self.follow_trackers(cx);
        cx.notify();
    }

    /// Lets go of everything the panel shows, as it hides: the target, the details, the General
    /// tab, the Files tab's tree and rows, the peers with their order, and the tracker rows. The
    /// Files tab's open folders and cursor stay for the next tree of the same torrent. A hidden
    /// panel is not drawn, so nothing is drawn again.
    pub(crate) fn hide(&mut self) {
        self.target = DetailsTarget::None;
        self.torrent = None;
        self.details = None;
        self.general = None;
        self.files.leave();
        self.peers.leave();
        self.trackers.leave();
    }

    /// Takes the details the service sent, if they are of the target's torrent: a read that was
    /// on its way when the target changed or the panel hid is dropped. Draws again only when the
    /// General tab, the Files tab or the Trackers tab changed.
    pub(crate) fn take_details(
        &mut self,
        details: Arc<TorrentDetails>,
        cx: &mut Context<'_, Self>,
    ) {
        if !is_of(&self.target, &details.hash) {
            tracing::debug!("details of a torrent the panel does not show; dropped");
            return;
        }
        self.details = Some(details);
        let general = self.general_model();
        let files_changed = self.follow_files();
        let trackers_changed = self.follow_trackers(cx);
        if general != self.general || files_changed || trackers_changed {
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

    /// Tells the panel whether what it shows is stale, as while the daemon does not answer; the
    /// Peers and the Trackers tab dim their rows then.
    pub(crate) fn set_stale(&mut self, stale: bool, cx: &mut Context<'_, Self>) {
        if stale != self.stale {
            self.stale = stale;
            cx.notify();
        }
    }

    /// Fits the Files tab to the details, and to none without them; returns whether it shows
    /// something else now. Only while the tab is shown: the progress is read again every few
    /// seconds, and a tree of thousands of files is not made for nothing. Opening the tab fits it
    /// then.
    fn follow_files(&mut self) -> bool {
        if self.tab != DetailsTab::Files {
            return false;
        }
        self.files.follow(self.details.as_ref())
    }

    /// What the General tab shows for the torrent and its details.
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
            .child(Tab::new().label(tab_name(DetailsTab::Peers)))
            .child(Tab::new().label(tab_name(DetailsTab::Trackers)))
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
                (DetailsTab::Peers, Some(torrent)) => body.child(self.peers_tab(torrent, cx)),
                (DetailsTab::Trackers, Some(_)) => body.child(self.trackers_tab(cx)),
                (DetailsTab::Files | DetailsTab::Peers | DetailsTab::Trackers, None) => body,
            },
            (None, None) => body,
        };
        v_flex()
            .id(PANEL_ID)
            .size_full()
            .text_sm()
            .child(tabs)
            .child(body.test_support())
            .test_support()
    }
}

/// Whether `target` is the one torrent `hash`.
fn is_of(target: &DetailsTarget, hash: &HashString) -> bool {
    matches!(target, DetailsTarget::One(one) if one == hash)
}

/// A line of muted text `id` in place of what a tab lists, such as "—" before the first read or
/// why there is nothing to list.
fn note(id: &'static str, text: impl Into<SharedString>, cx: &App) -> AnyElement {
    let text = text.into();
    div()
        .id(id)
        .flex_1()
        .px_3()
        .py_2()
        .text_color(cx.theme().muted_foreground)
        .aria_label(text.clone())
        .child(text)
        .test_support()
        .into_any_element()
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
        DetailsTab::Peers => 2,
        DetailsTab::Trackers => 3,
    }
}

/// The tab at `index` of the tab bar.
fn tab_at(index: usize) -> Option<DetailsTab> {
    [
        DetailsTab::General,
        DetailsTab::Files,
        DetailsTab::Peers,
        DetailsTab::Trackers,
    ]
    .get(index)
    .copied()
}

/// The name of `tab`, as its tab says it.
fn tab_name(tab: DetailsTab) -> SharedString {
    match tab {
        DetailsTab::General => t!("details-tab-general"),
        DetailsTab::Files => t!("details-tab-files"),
        DetailsTab::Peers => t!("details-tab-peers"),
        DetailsTab::Trackers => t!("details-tab-trackers"),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_tab_has_its_place_in_the_bar() {
        let tabs = [
            DetailsTab::General,
            DetailsTab::Files,
            DetailsTab::Peers,
            DetailsTab::Trackers,
        ];
        for (index, tab) in tabs.into_iter().enumerate() {
            assert_eq!(tab_index(tab), index, "{tab:?}");
            assert_eq!(tab_at(index), Some(tab));
        }
        assert_eq!(tab_at(4), None);
        assert_eq!(
            tabs.map(tab_name),
            ["General", "Files", "Peers", "Trackers"]
        );
    }
}
