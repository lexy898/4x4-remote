//! The Files tab: the switch of downloading in order, and the files and folders of the torrent as
//! a tree, one line each, with a checkbox for whether it is downloaded, its size, its progress, its
//! priority and, after the name of a file that downloads, how much of it plays from the start.
//!
//! x4-core decides all of it. The tab keeps the [`FileTree`] it draws, made from the details of
//! the panel's torrent while the tab is open (the panel keeps the newest details, makes the tree
//! when the tab opens and lets it go when the tab closes), and a [`FileTreeView`] with the folders
//! open and the cursor, which outlives the tree and is fitted to each new one with
//! [`FileTreeView::retain`]; the rows are what the view lists. A read that lists the same files,
//! as the reads of a torrent's progress do every few seconds, keeps the tree's folders and files
//! ([`FileTree::refreshed`]): only the first read after the tab opens sorts the names, which for
//! 50 000 files takes tens of milliseconds of the window's thread. A magnet link without
//! metadata has no files yet, and the tab says so instead of the list; so it does for a file list
//! too large to show ([`FileTree::is_too_large`]): more than 50 000 files, or files nested
//! hundreds of folders deep.
//! Before the details come it shows the switch and "—", as the General tab does for what only the
//! details know.
//!
//! Every change of files is built by the tree that was drawn: a click takes the tree of its frame,
//! a key the tree on screen, and the tree's own `toggle_wanted_action`, `priority_action` and
//! `step_priority_action` take the torrent, the files and the value from that one read. The panel
//! hands the action to the main view in a [`FileRequest`], and the main view sends it only while
//! the daemon is connected and the torrent is still the one the panel is about. Nothing changes on
//! screen until the next read shows what the daemon did.
//!
//! The list is the shared file list ([`file_list`](crate::file_list)), with every column: the size,
//! the progress and the priority. A press anywhere in it gives it the keyboard focus, so that the
//! keys work in it after any click, a click on the switch included; the rows are not tab stops,
//! the list is.

use std::ops::Range;
use std::sync::Arc;

use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, FocusHandle, InteractiveElement as _, IntoElement,
    ParentElement as _, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _,
    UniformListScrollHandle, Window, div,
};
use x4_core::{FileRow, FileTree, FileTreeView, NodeKey, Torrent, TorrentDetails};
use x4_rpc::FilePriority;

use super::{DetailsPanel, DetailsPanelEvent};
use crate::actions::FileRequest;
use crate::file_list::{self, Columns, Drawn, FileListHost, file_row};
use crate::i18n::t;
use crate::theme::state_color;

/// The element id of the file list.
const FILE_LIST_ID: &str = "file-list";

/// What the Files tab shows and what the user has done to it.
pub(super) struct FilesTab {
    /// The tree drawn: of the details of the panel's torrent, which it keeps; `None` until they
    /// come, while the panel is about no single torrent or hidden, and while another tab is open.
    tree: Option<Arc<FileTree>>,
    /// The folders open and the cursor.
    view: FileTreeView,
    /// The rows of `tree` that `view` shows, in order.
    rows: Vec<FileRow>,
    /// The keyboard focus of the list.
    focus: FocusHandle,
    /// The scroll position of the list.
    scroll: UniformListScrollHandle,
}

impl FilesTab {
    /// A tab with no tree yet.
    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            tree: None,
            view: FileTreeView::default(),
            rows: Vec::new(),
            focus: cx.focus_handle().tab_stop(true),
            scroll: UniformListScrollHandle::new(),
        }
    }

    /// The keyboard focus of the list.
    pub(super) fn focus(&self) -> &FocusHandle {
        &self.focus
    }

    /// Takes `details`, those of the panel's torrent, or `None` while it has none: makes their
    /// tree, unless it was made from them already, and fits the open folders and the cursor to it.
    /// The tree of details that list the files of the tree drawn keeps its folders and files.
    /// Returns whether the tab shows something else now.
    pub(super) fn follow(&mut self, details: Option<&Arc<TorrentDetails>>) -> bool {
        let Some(details) = details else {
            let changed = self.tree.is_some();
            self.leave();
            return changed;
        };
        if self
            .tree
            .as_ref()
            .is_some_and(|tree| tree.is_made_from(details))
        {
            return false;
        }
        let tree = self
            .tree
            .as_ref()
            .and_then(|tree| tree.refreshed(details))
            .unwrap_or_else(|| FileTree::new(details));
        let tree = Arc::new(tree);
        self.view.retain(&tree);
        self.rows = self.view.rows(&tree);
        self.tree = Some(tree);
        true
    }

    /// Lets go of the tree, the details it was made from and its rows, with the room they took,
    /// as the tab closes, the panel hides or its torrent has no details: a tree of thousands of
    /// files is not kept for a tab nobody sees. The open folders and the cursor stay, and the next
    /// tree of the same torrent keeps them.
    pub(super) fn leave(&mut self) {
        self.tree = None;
        drop(std::mem::take(&mut self.rows));
    }

    /// Changes the view with `change` on the tree on screen, lists its rows again and scrolls the
    /// cursor into sight. Does nothing before there is a tree, nor for a click on a frame of
    /// another torrent's tree (`drawn`): the same path there is another file.
    ///
    /// Returns whether the list must be drawn again: when the rows or the cursor changed, and
    /// when the cursor's row is out of sight, so that a key brings the list back to the cursor
    /// after the wheel took it away, even a key that moves the cursor nowhere. A key that
    /// changes nothing on screen draws nothing again.
    fn change_view(
        &mut self,
        drawn: Option<&FileTree>,
        change: impl FnOnce(&mut FileTreeView, &FileTree),
    ) -> bool {
        file_list::change_view(
            self.tree.as_deref(),
            &mut self.view,
            &mut self.rows,
            &self.scroll,
            drawn,
            change,
        )
    }

    /// The tree on screen and the row of it the cursor is on, if any.
    fn at_cursor(&self) -> Option<(Arc<FileTree>, NodeKey)> {
        let tree = self.tree.clone()?;
        let key = self.view.cursor(&tree)?.clone();
        Some((tree, key))
    }

    /// How many rows Page Up and Page Down move the cursor by: the rows in sight, less the one
    /// that is usually cut off at an edge, and at least one.
    fn page_step(&self) -> isize {
        file_list::page_step(&self.scroll)
    }
}

impl DetailsPanel {
    /// The Files tab, for the panel's torrent `torrent`.
    pub(super) fn files_tab(
        &self,
        torrent: &Torrent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let note = |id: &'static str, text: String| {
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
        };
        let body = match &self.files.tree {
            // The details have not come yet; they come within a moment.
            None => note("files-pending", t!("details-pending")),
            Some(tree) if tree.is_too_large() => note("files-too-large", t!("files-too-large")),
            Some(tree) if tree.is_empty() => note("files-no-metadata", t!("files-no-metadata")),
            Some(_) => self.file_list(window, cx).into_any_element(),
        };
        v_flex()
            .size_full()
            .child(self.sequential_switch(torrent, cx))
            .child(body)
            .into_any_element()
    }

    /// The switch of downloading `torrent`'s pieces in order, as the daemon's list has it.
    fn sequential_switch(&self, torrent: &Torrent, cx: &Context<'_, Self>) -> impl IntoElement {
        let panel = cx.weak_entity();
        let hash = torrent.hash.clone();
        h_flex().flex_none().px_3().py_1().child(
            Switch::new("file-sequential")
                .small()
                .checked(torrent.sequential)
                .label(t!("files-sequential"))
                .disabled(!self.connected)
                .on_change(move |enabled, window, cx| {
                    let request = FileRequest::sequential(hash.clone(), *enabled);
                    if let Err(error) = panel.update(cx, |panel, cx| {
                        Self::ask(Some(request), cx);
                        panel.focus_file_list(window, cx);
                    }) {
                        tracing::debug!(%error, "the switch was used after the panel closed");
                    }
                }),
        )
    }

    /// The list of files and folders, with its keys.
    fn file_list(&self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        file_list::file_list(
            FILE_LIST_ID,
            t!("files-list").into(),
            self.files.rows.len(),
            &self.files.focus,
            &self.files.scroll,
            &self.key_repeats,
            cx,
        )
        .flex_1()
        .min_h_0()
        .w_full()
    }

    /// Hands `request` to the main view, which sends it if it can go; nothing for `None`, which
    /// is what a tree builds for a row without files.
    fn ask(request: Option<FileRequest>, cx: &mut Context<'_, Self>) {
        if let Some(request) = request {
            cx.emit(DetailsPanelEvent::Files(request));
        } else {
            tracing::debug!("the row reaches no files; nothing asked");
        }
    }
}

/// The Files tab's list: its checkboxes and priorities ask the main view to change the torrent's
/// files, as the tree that was drawn builds the change.
impl FileListHost for DetailsPanel {
    fn file_list_focus(&self) -> &FocusHandle {
        &self.files.focus
    }

    /// The rows of the list at `range`, as drawn now; each takes the tree of this frame. Their
    /// controls act only while the daemon is connected, and the bars take the torrent's color.
    fn file_rows(&mut self, range: Range<usize>, cx: &mut Context<'_, Self>) -> Vec<AnyElement> {
        let Some(tree) = self.files.tree.clone() else {
            return Vec::new();
        };
        let theme = cx.theme();
        let drawn = Drawn {
            cursor: self.files.view.cursor(&tree).cloned(),
            tree,
            enabled: self.connected,
            color: self
                .torrent
                .as_ref()
                .map_or(theme.primary, |torrent| state_color(torrent.state, theme)),
            columns: Columns {
                size: true,
                progress: true,
                priority: true,
            },
        };
        self.files
            .rows
            .get(range)
            .unwrap_or_default()
            .iter()
            .map(|row| file_row(row, &drawn, cx))
            .collect()
    }

    fn change_file_view(
        &mut self,
        drawn: Option<&FileTree>,
        change: impl FnOnce(&mut FileTreeView, &FileTree),
        cx: &mut Context<'_, Self>,
    ) {
        if self.files.change_view(drawn, change) {
            cx.notify();
        }
    }

    fn file_page_step(&self) -> isize {
        self.files.page_step()
    }

    /// The cursor goes to the row, and the tree builds the change.
    fn click_wanted(&mut self, tree: &FileTree, key: &NodeKey, cx: &mut Context<'_, Self>) {
        self.click_file_row(tree, key, cx);
        Self::ask(
            tree.toggle_wanted_action(key)
                .and_then(FileRequest::of_files),
            cx,
        );
    }

    /// As the tree on screen builds it.
    fn toggle_cursor_wanted(&mut self, cx: &mut Context<'_, Self>) {
        let Some((tree, key)) = self.files.at_cursor() else {
            return;
        };
        Self::ask(
            tree.toggle_wanted_action(&key)
                .and_then(FileRequest::of_files),
            cx,
        );
    }

    /// The cursor goes to the row, and the tree drawn when the menu opened builds the change.
    fn choose_priority(
        &mut self,
        tree: &FileTree,
        key: &NodeKey,
        priority: FilePriority,
        cx: &mut Context<'_, Self>,
    ) {
        self.click_file_row(tree, key, cx);
        Self::ask(
            tree.priority_action(key, priority)
                .and_then(FileRequest::of_files),
            cx,
        );
    }

    /// As the tree on screen builds it.
    fn step_cursor_priority(&mut self, up: bool, cx: &mut Context<'_, Self>) {
        let Some((tree, key)) = self.files.at_cursor() else {
            return;
        };
        Self::ask(
            tree.step_priority_action(&key, up)
                .and_then(FileRequest::of_files),
            cx,
        );
    }
}

#[cfg(test)]
mod tests;
