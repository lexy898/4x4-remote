//! The Files tab: the switch of downloading in order, and the files and folders of the torrent as
//! a tree, one line each, with a checkbox for whether it is downloaded, its size, its progress, its
//! priority and, after the name of a file that downloads, how much of it plays from the start.
//!
//! x4-core decides all of it. The tab keeps the [`FileTree`] it draws, made from the details of
//! the panel's torrent while the tab is open (the panel keeps the newest details, makes the tree
//! when the tab opens and lets it go when the tab closes), and a [`FileTreeView`] with the folders
//! open and the cursor, which outlives the tree and is fitted to each new one with
//! [`FileTreeView::retain`]; the rows are what the view lists. A magnet link without
//! metadata has no files yet, and the tab says so instead of the list; before the details come it
//! shows the switch alone.
//!
//! Every change of files is built by the tree that was drawn: a click takes the tree of its frame,
//! a key the tree on screen, and the tree's own `toggle_wanted_action`, `priority_action` and
//! `step_priority_action` take the torrent, the files and the value from that one read. The panel
//! hands the action to the main view in a [`FileRequest`], and the main view sends it only while
//! the daemon is connected and the torrent is still the one the panel is about. Nothing changes on
//! screen until the next read shows what the daemon did.
//!
//! The list is our own `uniform_list`, which builds only the rows in sight, inside the key context
//! `FileList` (the `keys` module). A press anywhere in it gives it the keyboard focus, so that the
//! keys work in it after any click, a click on the switch included; the rows are not tab stops,
//! the list is.

mod keys;
mod rows;

use std::ops::Range;
use std::sync::Arc;

use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, FocusHandle, InteractiveElement as _, IntoElement, MouseDownEvent,
    ParentElement as _, Pixels, Role, ScrollStrategy, StatefulInteractiveElement as _, Styled as _,
    TestSupportExt as _, UniformListScrollHandle, Window, div, uniform_list,
};
use x4_core::{FileRow, FileTree, FileTreeView, NodeKey, Torrent, TorrentDetails};
use x4_rpc::FilePriority;

use super::{DetailsPanel, DetailsPanelEvent};
use crate::actions::FileRequest;
use crate::i18n::t;
use crate::keymap::FILE_LIST_CONTEXT;
use crate::theme::state_color;
use rows::{Drawn, ROW_HEIGHT, file_row};

/// The element id of the file list.
const FILE_LIST_ID: &str = "file-list";

/// What the Files tab shows and what the user has done to it.
pub(super) struct FilesTab {
    /// The tree drawn: of the details of the panel's torrent; `None` until they come, and while
    /// the panel is about no single torrent.
    tree: Option<Arc<FileTree>>,
    /// The details `tree` was made from, so that the same details make no new tree.
    made_from: Option<Arc<TorrentDetails>>,
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
            made_from: None,
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
    /// Returns whether the tab shows something else now.
    pub(super) fn follow(&mut self, details: Option<&Arc<TorrentDetails>>) -> bool {
        let Some(details) = details else {
            let changed = self.tree.is_some();
            self.tree = None;
            self.made_from = None;
            self.rows.clear();
            return changed;
        };
        if self
            .made_from
            .as_ref()
            .is_some_and(|made_from| Arc::ptr_eq(made_from, details))
        {
            return false;
        }
        let tree = Arc::new(FileTree::new(details));
        self.view.retain(&tree);
        self.rows = self.view.rows(&tree);
        self.tree = Some(tree);
        self.made_from = Some(Arc::clone(details));
        true
    }

    /// Lets go of the tree and its rows, as the tab closes: a tree of thousands of files is not
    /// kept for a tab nobody sees. The open folders and the cursor stay, and the next tree of the
    /// same torrent keeps them.
    pub(super) fn leave(&mut self) {
        self.tree = None;
        self.made_from = None;
        self.rows = Vec::new();
    }

    /// Changes the view with `change` on the tree on screen, lists its rows again and scrolls the
    /// cursor into sight. Does nothing before there is a tree, nor for a click on a frame of
    /// another torrent's tree (`drawn`): the same path there is another file. Returns whether the
    /// rows or the cursor changed, so that the list is drawn again only then.
    fn change_view(
        &mut self,
        drawn: Option<&FileTree>,
        change: impl FnOnce(&mut FileTreeView, &FileTree),
    ) -> bool {
        let Some(tree) = self.tree.clone() else {
            return false;
        };
        if drawn.is_some_and(|drawn| drawn.hash() != tree.hash()) {
            return false;
        }
        let cursor_before = self.view.cursor(&tree).cloned();
        change(&mut self.view, &tree);
        let rows = self.view.rows(&tree);
        let cursor = self.view.cursor(&tree).cloned();
        let changed = rows != self.rows || cursor != cursor_before;
        self.rows = rows;
        let at = cursor
            .as_ref()
            .and_then(|cursor| self.rows.iter().position(|row| &row.key == cursor));
        if let Some(index) = at {
            self.scroll.scroll_to_item(index, ScrollStrategy::Nearest);
        }
        changed
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
        let height = self.scroll.0.borrow().base_handle.bounds().size.height;
        rows_in(height).saturating_sub(1).max(1)
    }
}

/// How many whole rows fit in `height`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "a list is a few thousand pixels high at most, far within an isize"
)]
fn rows_in(height: Pixels) -> isize {
    (f32::from(height) / ROW_HEIGHT).floor().max(0.) as isize
}

impl DetailsPanel {
    /// The Files tab, for the panel's torrent `torrent`.
    pub(super) fn files_tab(
        &self,
        torrent: &Torrent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let body = match &self.files.tree {
            // The details have not come yet; they come within a moment.
            None => div().flex_1().into_any_element(),
            Some(tree) if tree.is_empty() => {
                let text = t!("files-no-metadata");
                div()
                    .id("files-no-metadata")
                    .flex_1()
                    .px_3()
                    .py_2()
                    .text_color(cx.theme().muted_foreground)
                    .aria_label(text.clone())
                    .child(text)
                    .test_support()
                    .into_any_element()
            }
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
        let list = div()
            .id(FILE_LIST_ID)
            .test_support()
            .track_focus(&self.files.focus)
            .key_context(FILE_LIST_CONTEXT)
            .role(Role::Tree)
            .aria_label(t!("files-list"))
            .flex_1()
            .min_h_0()
            .w_full()
            // In the capture phase, before a checkbox or a priority button can keep the focus
            // where it was: after any click in the list, the keys work in it, and a priority
            // menu gives the focus back to it when it closes.
            .capture_any_mouse_down(cx.listener(|panel, _: &MouseDownEvent, window, cx| {
                panel.focus_file_list(window, cx);
            }));
        keys::on_file_keys(list, &self.key_repeats, cx).child(
            uniform_list(
                "file-rows",
                self.files.rows.len(),
                cx.processor(|panel, range: Range<usize>, _window, cx| panel.file_rows(range, cx)),
            )
            .track_scroll(&self.files.scroll)
            .size_full(),
        )
    }

    /// The rows of the list at `range`, as drawn now; each takes the tree of this frame.
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
        };
        self.files
            .rows
            .get(range)
            .unwrap_or_default()
            .iter()
            .map(|row| file_row(row, &drawn, cx))
            .collect()
    }

    /// Gives the keyboard focus to the file list.
    fn focus_file_list(&self, window: &mut Window, cx: &mut App) {
        window.focus(&self.files.focus, cx);
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

    /// Changes the open folders or the cursor with `change`, and draws the list again if that
    /// changed what it shows; for a click, only when the tree of its frame (`drawn`) is of the
    /// same torrent.
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

    /// A click on the row `key` of `tree`, a tree drawn: the cursor goes there.
    pub(super) fn click_file_row(
        &mut self,
        tree: &FileTree,
        key: &NodeKey,
        cx: &mut Context<'_, Self>,
    ) {
        self.change_file_view(Some(tree), |view, tree| view.set_cursor(tree, key), cx);
    }

    /// A click on the expander of the folder `key` of `tree`: opens or closes it, and puts the
    /// cursor on it.
    pub(super) fn click_expander(
        &mut self,
        tree: &FileTree,
        key: &NodeKey,
        cx: &mut Context<'_, Self>,
    ) {
        self.change_file_view(
            Some(tree),
            |view, tree| {
                view.toggle_expanded(tree, key);
                view.set_cursor(tree, key);
            },
            cx,
        );
    }

    /// A click on the checkbox of the row `key` of `tree`, a tree drawn: the cursor goes there,
    /// and the tree builds the change.
    pub(super) fn click_wanted(
        &mut self,
        tree: &FileTree,
        key: &NodeKey,
        cx: &mut Context<'_, Self>,
    ) {
        self.click_file_row(tree, key, cx);
        Self::ask(
            tree.toggle_wanted_action(key)
                .and_then(FileRequest::of_files),
            cx,
        );
    }

    /// A choice of `priority` from the menu of the row `key` of `tree`, the tree drawn when the
    /// menu opened.
    pub(super) fn choose_priority(
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
}

#[cfg(test)]
mod tests;
