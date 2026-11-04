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
//! 50 000 files takes tens of milliseconds. So the first tree of more than
//! [`CHEAP_TREE_FILES`](x4_core::CHEAP_TREE_FILES) files is made on a thread of its own
//! ([`on_a_thread`], which says why not on GPUI's pool), not on the window's thread, which stays
//! free to draw; the tab shows no tree until it comes, and the newest details then
//! (`DetailsPanel::tree_built`). A magnet link without
//! metadata has no files yet, and the tab says so instead of the list; so it does for a file list
//! too large to show ([`FileTree::is_too_large`]): more than 50 000 files, or files nested
//! hundreds of folders deep.
//! Before the details come it shows the switch and "—", as the General tab does for what only the
//! details know, and so it does while a tree is made.
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
    AnyElement, App, AppContext as _, Context, FocusHandle, IntoElement, ParentElement as _,
    Styled as _, Task, UniformListScrollHandle, Window,
};
use x4_core::{DetailsTab, FileRow, FileTree, FileTreeView, NodeKey, Torrent, TorrentDetails};
use x4_rpc::{FilePriority, HashString};

use super::{DetailsPanel, DetailsPanelEvent, is_of, note};
use crate::actions::FileRequest;
use crate::file_list::{self, Columns, Drawn, FileListHost, file_row};
use crate::i18n::t;
use crate::own_thread::on_a_thread;
use crate::theme::state_color;

/// The element id of the file list.
const FILE_LIST_ID: &str = "file-list";

/// The name of the thread a tree is made on.
const TREE_THREAD: &str = "file-tree";

/// The stack of that thread. Making a tree recurses nowhere: the paths are walked and the folders
/// sorted in loops; a stack is mapped, not filled, so only what is used takes memory.
const TREE_STACK: usize = 256 * 1024;

/// What makes the tree of some details.
type MakeTree = fn(&Arc<TorrentDetails>) -> FileTree;

/// Where a tree is made off the window's thread, with what makes it; the task gives the tree, or
/// `None` when nothing could make it there.
type MakeOn = fn(Arc<TorrentDetails>, MakeTree, &App) -> Task<Option<FileTree>>;

/// Makes the tree of `details` with `make` on a thread of its own, named `file-tree`.
fn make_on_a_thread(
    details: Arc<TorrentDetails>,
    make: MakeTree,
    cx: &App,
) -> Task<Option<FileTree>> {
    on_a_thread(
        TREE_THREAD,
        TREE_STACK,
        move || Some(make(&details)),
        |_| None,
        cx,
    )
}

/// Makes the tree of `details` with `make` on GPUI's background executor. Under the executor of a
/// test the tree is made in its turn with every other task, where a thread of its own would end at
/// a time of its own: [`DetailsPanel::make_trees_on_the_executor`].
fn make_on_the_executor(
    details: Arc<TorrentDetails>,
    make: MakeTree,
    cx: &App,
) -> Task<Option<FileTree>> {
    cx.background_spawn(async move { Some(make(&details)) })
}

/// What the Files tab shows and what the user has done to it.
pub(super) struct FilesTab {
    /// The tree drawn: of the details of the panel's torrent, which it keeps; `None` until they
    /// come, while the panel is about no single torrent or hidden, while another tab is open, and
    /// while the tree is made.
    tree: Option<Arc<FileTree>>,
    /// The tree being made off the window's thread, while there is no tree to draw.
    building: Option<Building>,
    /// The folders open and the cursor.
    view: FileTreeView,
    /// The rows of `tree` that `view` shows, in order.
    rows: Vec<FileRow>,
    /// The keyboard focus of the list.
    focus: FocusHandle,
    /// The scroll position of the list.
    scroll: UniformListScrollHandle,
    /// Where a tree is made off the window's thread: on a thread of its own, in the app; a test
    /// counts the makings started.
    make_on: MakeOn,
    /// What makes a tree there: [`FileTree::new`]; a test notes the thread.
    make: MakeTree,
}

/// A tree being made off the window's thread.
struct Building {
    /// The torrent whose files it is made of.
    hash: HashString,
    /// The making; dropping it lets the tree go, which then never reaches the panel.
    _task: Task<()>,
}

/// What the Files tab did with the details it was given.
#[derive(Debug, PartialEq, Eq)]
#[must_use]
pub(super) enum Follow {
    /// It shows what it showed.
    Unchanged,
    /// It shows something else now.
    Changed,
    /// The tree of these details is to be made off the window's thread: they list too many files
    /// to make it at once. Meanwhile the tab has no tree.
    Build(Arc<TorrentDetails>),
}

impl FilesTab {
    /// A tab with no tree yet.
    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            tree: None,
            building: None,
            view: FileTreeView::default(),
            rows: Vec::new(),
            focus: cx.focus_handle().tab_stop(true),
            scroll: UniformListScrollHandle::new(),
            make_on: make_on_a_thread,
            make: FileTree::new,
        }
    }

    /// The keyboard focus of the list.
    pub(super) fn focus(&self) -> &FocusHandle {
        &self.focus
    }

    /// Takes `details`, those of the panel's torrent, or `None` while it has none: makes their
    /// tree, unless it was made from them already, and fits the open folders and the cursor to it.
    /// The tree of details that list the files of the tree drawn keeps its folders and files.
    ///
    /// The tree of other files is made at once only when that is cheap
    /// ([`FileTree::is_cheap`]); otherwise the tab lets go of the tree it draws, whose rows and
    /// actions belong to another list of files, and asks for the tree to be made off the window's
    /// thread ([`Follow::Build`]).
    pub(super) fn follow(&mut self, details: Option<&Arc<TorrentDetails>>) -> Follow {
        let Some(details) = details else {
            let changed = self.tree.is_some();
            self.leave();
            return if changed {
                Follow::Changed
            } else {
                Follow::Unchanged
            };
        };
        if self
            .tree
            .as_ref()
            .is_some_and(|tree| tree.is_made_from(details))
        {
            return Follow::Unchanged;
        }
        let tree = match self.tree.as_ref().and_then(|tree| tree.refreshed(details)) {
            Some(tree) => tree,
            None if FileTree::is_cheap(details) => FileTree::new(details),
            None => {
                self.tree = None;
                drop(std::mem::take(&mut self.rows));
                return Follow::Build(Arc::clone(details));
            }
        };
        self.show_tree(tree);
        Follow::Changed
    }

    /// Takes `built`, the tree made off the window's thread, with `details`, the newest the panel
    /// has of its torrent, which may have come while it was made: the tree shows them, keeping its
    /// folders and files when they list the same files. When they list other files, their tree
    /// is made as [`follow`](Self::follow) makes it.
    pub(super) fn take_built(
        &mut self,
        built: FileTree,
        details: Option<&Arc<TorrentDetails>>,
    ) -> Follow {
        self.building = None;
        let tree = details.and_then(|details| {
            if built.is_made_from(details) {
                Some(built)
            } else {
                built.refreshed(details)
            }
        });
        match tree {
            Some(tree) => {
                self.show_tree(tree);
                Follow::Changed
            }
            None => self.follow(details),
        }
    }

    /// Whether the tab has a tree to draw.
    pub(super) fn has_tree(&self) -> bool {
        self.tree.is_some()
    }

    /// Whether a tree of the torrent `hash` is being made.
    pub(super) fn is_building(&self, hash: &HashString) -> bool {
        self.building
            .as_ref()
            .is_some_and(|building| building.hash == *hash)
    }

    /// Draws `tree`: fits the open folders and the cursor to it and lists its rows. A tree still
    /// being made is of no use any more.
    fn show_tree(&mut self, tree: FileTree) {
        let tree = Arc::new(tree);
        self.view.retain(&tree);
        self.rows = self.view.rows(&tree);
        self.tree = Some(tree);
        self.building = None;
    }

    /// Lets go of the tree, the details it was made from and its rows, with the room they took,
    /// and of a tree being made, as the tab closes, the panel hides or its torrent has no details:
    /// a tree of thousands of files is not kept for a tab nobody sees. The open folders and the
    /// cursor stay, and the next tree of the same torrent keeps them.
    pub(super) fn leave(&mut self) {
        self.tree = None;
        self.building = None;
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
    /// Has the tree of `details` made on a thread of its own, unless a tree of their torrent is
    /// being made already: that one is fitted to the newest details when it comes. The tree goes
    /// to [`tree_built`](Self::tree_built).
    pub(super) fn build_tree(&mut self, details: Arc<TorrentDetails>, cx: &mut Context<'_, Self>) {
        if self.files.is_building(&details.hash) {
            return;
        }
        tracing::debug!(
            files = details.files.len(),
            "making a file tree off the window's thread"
        );
        let hash = details.hash.clone();
        let making = (self.files.make_on)(details, self.files.make, cx);
        let task = cx.spawn({
            let hash = hash.clone();
            async move |panel, cx| {
                let tree = making.await;
                let taken = panel.update(cx, |panel, cx| match tree {
                    Some(tree) => panel.tree_built(tree, cx),
                    None => panel.make_tree_here(&hash, cx),
                });
                if let Err(error) = taken {
                    tracing::debug!(%error, "a file tree was made after the panel closed");
                }
            }
        });
        self.files.building = Some(Building { hash, _task: task });
    }

    /// Makes the tree of the torrent `hash` on the window's thread, from the newest details, when
    /// no other thread could: late, rather than never.
    fn make_tree_here(&mut self, hash: &HashString, cx: &mut Context<'_, Self>) {
        if !self.files.is_building(hash) {
            return;
        }
        let Some(details) = self
            .details
            .as_ref()
            .filter(|details| details.hash == *hash)
        else {
            return;
        };
        tracing::warn!("the file tree is made on the window's thread instead");
        let tree = FileTree::new(details);
        self.tree_built(tree, cx);
    }

    /// Makes the Files tab's trees on GPUI's background executor from now on; for tests
    /// ([`make_on_the_executor`]).
    pub(crate) fn make_trees_on_the_executor(&mut self) {
        self.files.make_on = make_on_the_executor;
    }

    /// Takes `tree`, made off the window's thread, if the Files tab still waits for it: the tab is
    /// open, on the torrent the tree is of. Dropping the making when the tab closes or the torrent
    /// changes keeps any other tree from coming; this checks a tree that was on its way then.
    fn tree_built(&mut self, tree: FileTree, cx: &mut Context<'_, Self>) {
        let wanted = self.tab == DetailsTab::Files
            && is_of(&self.target, tree.hash())
            && self.files.is_building(tree.hash());
        if !wanted {
            tracing::debug!("a file tree came for what the Files tab no longer shows; dropped");
            return;
        }
        match self.files.take_built(tree, self.details.as_ref()) {
            Follow::Unchanged => {}
            Follow::Changed => cx.notify(),
            Follow::Build(details) => self.build_tree(details, cx),
        }
    }

    /// The Files tab, for the panel's torrent `torrent`.
    pub(super) fn files_tab(
        &self,
        torrent: &Torrent,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> AnyElement {
        let body = match &self.files.tree {
            // The details have not come yet; they come within a moment.
            None => note("files-pending", t!("details-pending"), cx),
            Some(tree) if tree.is_too_large() => note("files-too-large", t!("files-too-large"), cx),
            Some(tree) if tree.is_empty() => note("files-no-metadata", t!("files-no-metadata"), cx),
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
