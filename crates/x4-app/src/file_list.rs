//! A list of a torrent's files and folders as a tree, one row each, shared by the Files tab of the
//! details panel and the add dialog: the rows (the `rows` module), the keys (the `keys` module),
//! the list element around them, and how the cursor and the open folders change.
//!
//! x4-core decides what the list shows: a [`FileTree`], and a [`FileTreeView`] with the open
//! folders and the cursor, whose rows ([`FileRow`]) are what the list draws. The view that holds
//! the list is its [`FileListHost`]: it keeps the tree, the view, the rows, the focus and the
//! scroll position, and decides what a click on a checkbox or a priority, and Space, `+` and `-`
//! do. The details panel asks the daemon; the add dialog changes its own choice of files.
//!
//! The list is our own `uniform_list`, which builds only the rows in sight, inside the key context
//! `FileList`. A press anywhere in it gives it the keyboard focus, so that the keys work in it
//! after any click; the rows are not tab stops, the list is. Each row draws the columns its host
//! asks for ([`Columns`]): the size, the progress and the priority after the name.

mod keys;
mod rows;

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::{
    AnyElement, App, Context, FocusHandle, InteractiveElement as _, IntoElement, MouseDownEvent,
    ParentElement as _, Pixels, Role, ScrollStrategy, SharedString,
    StatefulInteractiveElement as _, Styled, TestSupportExt as _, UniformListScrollHandle, Window,
    div, px, uniform_list,
};
use x4_core::{FileRow, FileTree, FileTreeView, NodeKey};
use x4_rpc::FilePriority;

use crate::key_repeats::KeyRepeats;
use crate::keymap::FILE_LIST_CONTEXT;
pub(crate) use keys::on_file_keys;
pub(crate) use rows::{Columns, Drawn, ROW_HEIGHT, file_row};

/// The view that holds a file list: what its rows, its keys and its clicks act on.
///
/// The cursor and the open folders change through [`change_file_view`](Self::change_file_view);
/// a click on a checkbox, Space, a choice of priority, `+` and `-` are the host's to carry out.
/// Each click takes the tree of the frame it was drawn in, and each key the tree on screen.
pub(crate) trait FileListHost: Sized + 'static {
    /// The keyboard focus of the list.
    fn file_list_focus(&self) -> &FocusHandle;

    /// The rows of the list at `range`, as drawn now.
    fn file_rows(&mut self, range: Range<usize>, cx: &mut Context<'_, Self>) -> Vec<AnyElement>;

    /// Changes the open folders or the cursor with `change`, and draws the list again if that
    /// changed what it shows or the cursor's row is to be scrolled into sight; for a click, only
    /// when the tree of its frame (`drawn`) is of the same torrent as the tree on screen.
    fn change_file_view(
        &mut self,
        drawn: Option<&FileTree>,
        change: impl FnOnce(&mut FileTreeView, &FileTree),
        cx: &mut Context<'_, Self>,
    );

    /// How many rows Page Up and Page Down move the cursor by.
    fn file_page_step(&self) -> isize;

    /// A click on the checkbox of the row `key` of `tree`, a tree drawn.
    fn click_wanted(&mut self, tree: &FileTree, key: &NodeKey, cx: &mut Context<'_, Self>);

    /// Space: downloads the row under the cursor, or skips it.
    fn toggle_cursor_wanted(&mut self, cx: &mut Context<'_, Self>);

    /// A choice of `priority` from the menu of the row `key` of `tree`, the tree drawn when the
    /// menu opened. Only a list that draws the priorities ([`Columns::priority`]) has the menu;
    /// by default it does nothing.
    fn choose_priority(
        &mut self,
        _tree: &FileTree,
        _key: &NodeKey,
        _priority: FilePriority,
        _cx: &mut Context<'_, Self>,
    ) {
    }

    /// `+` (`up`) or `-`: gives the row under the cursor the next priority up or down. By default
    /// it does nothing, for a list without priorities.
    fn step_cursor_priority(&mut self, _up: bool, _cx: &mut Context<'_, Self>) {}

    /// A click on the row `key` of `tree`, a tree drawn: the cursor goes there.
    fn click_file_row(&mut self, tree: &FileTree, key: &NodeKey, cx: &mut Context<'_, Self>) {
        self.change_file_view(Some(tree), |view, tree| view.set_cursor(tree, key), cx);
    }

    /// A click on the expander of the folder `key` of `tree`: opens or closes it, and puts the
    /// cursor on it.
    fn click_expander(&mut self, tree: &FileTree, key: &NodeKey, cx: &mut Context<'_, Self>) {
        self.change_file_view(
            Some(tree),
            |view, tree| {
                view.toggle_expanded(tree, key);
                view.set_cursor(tree, key);
            },
            cx,
        );
    }

    /// Moves the cursor `step` rows down, or up when `step` is negative.
    fn step_file_cursor(&mut self, step: isize, cx: &mut Context<'_, Self>) {
        self.change_file_view(None, |view, tree| view.move_cursor(tree, step), cx);
    }

    /// Gives the keyboard focus to the list.
    fn focus_file_list(&self, window: &mut Window, cx: &mut App) {
        window.focus(self.file_list_focus(), cx);
    }
}

/// The list `id` of `rows` rows, named `label` for screen readers, with its keys: `focus` is the
/// host's [`file_list_focus`](FileListHost::file_list_focus), `scroll` the scroll position, and
/// `repeats` the main view's record of the key held. The host's
/// [`file_rows`](FileListHost::file_rows) draws the rows in sight. The caller gives the list its
/// size.
pub(crate) fn file_list<H: FileListHost>(
    id: &'static str,
    label: SharedString,
    rows: usize,
    focus: &FocusHandle,
    scroll: &UniformListScrollHandle,
    repeats: &Rc<RefCell<KeyRepeats>>,
    cx: &mut Context<'_, H>,
) -> impl IntoElement + Styled + use<H> {
    let list = div()
        .id(id)
        .test_support()
        .track_focus(focus)
        .key_context(FILE_LIST_CONTEXT)
        .role(Role::Tree)
        .aria_label(label)
        // In the capture phase, before a checkbox or a priority button can keep the focus where
        // it was: after any click in the list, the keys work in it, and a priority menu gives the
        // focus back to it when it closes.
        .capture_any_mouse_down(cx.listener(|host, _: &MouseDownEvent, window, cx| {
            host.focus_file_list(window, cx);
        }));
    on_file_keys(list, repeats, cx).child(
        uniform_list(
            "file-rows",
            rows,
            cx.processor(|host, range: Range<usize>, _window, cx| host.file_rows(range, cx)),
        )
        .track_scroll(scroll)
        .size_full(),
    )
}

/// Changes `view` with `change` on `tree`, the tree on screen, lists its `rows` again and scrolls
/// the cursor into sight in the list scrolled by `scroll`. Does nothing before there is a tree,
/// nor for a click on a frame of another torrent's tree (`drawn`): the same path there is another
/// file.
///
/// Returns whether the list must be drawn again: when the rows or the cursor changed, and when the
/// cursor's row is out of sight, so that a key brings the list back to the cursor after the wheel
/// took it away, even a key that moves the cursor nowhere. A key that changes nothing on screen
/// draws nothing again.
pub(crate) fn change_view(
    tree: Option<&FileTree>,
    view: &mut FileTreeView,
    rows: &mut Vec<FileRow>,
    scroll: &UniformListScrollHandle,
    drawn: Option<&FileTree>,
    change: impl FnOnce(&mut FileTreeView, &FileTree),
) -> bool {
    let Some(tree) = tree else {
        return false;
    };
    if drawn.is_some_and(|drawn| drawn.hash() != tree.hash()) {
        return false;
    }
    let cursor_before = view.cursor(tree).cloned();
    change(view, tree);
    let new_rows = view.rows(tree);
    let cursor = view.cursor(tree).cloned();
    let changed = new_rows != *rows || cursor != cursor_before;
    *rows = new_rows;
    let out_of_sight = cursor
        .as_ref()
        .and_then(|cursor| rows.iter().position(|row| &row.key == cursor))
        .filter(|&index| !in_sight(scroll, index));
    if let Some(index) = out_of_sight {
        scroll.scroll_to_item(index, ScrollStrategy::Nearest);
    }
    changed || out_of_sight.is_some()
}

/// Whether the row at `index` is wholly in sight, in the list scrolled by `scroll` as it was last
/// laid out. Before its first layout the list shows nothing yet to scroll, so every row counts as
/// in sight.
fn in_sight(scroll: &UniformListScrollHandle, index: usize) -> bool {
    let state = scroll.0.borrow();
    let height = state.base_handle.bounds().size.height;
    if height <= Pixels::ZERO {
        return true;
    }
    let top = -state.base_handle.offset().y;
    let row_top = px(ROW_HEIGHT) * index;
    row_top >= top && row_top + px(ROW_HEIGHT) <= top + height
}

/// How many rows Page Up and Page Down move the cursor by in the list scrolled by `scroll`: the
/// rows in sight, less the one that is usually cut off at an edge, and at least one.
pub(crate) fn page_step(scroll: &UniformListScrollHandle) -> isize {
    let height = scroll.0.borrow().base_handle.bounds().size.height;
    rows_in(height).saturating_sub(1).max(1)
}

/// How many whole rows fit in `height`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "a list is a few thousand pixels high at most, far within an isize"
)]
fn rows_in(height: Pixels) -> isize {
    (f32::from(height) / ROW_HEIGHT).floor().max(0.) as isize
}
