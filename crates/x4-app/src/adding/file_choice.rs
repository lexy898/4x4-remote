//! The choice of files of a `.torrent` in the add dialog: the shared file list
//! ([`file_list`](crate::file_list)) with the checkboxes and the sizes, and no progress or
//! priority, which mean nothing before the torrent is added.
//!
//! x4-core's [`AddForm`](x4_core::AddForm) holds the choice, and each click changes the wanted
//! states of its tree; the list draws that tree with the folders the user opened and the cursor
//! kept, as the Files tab does. A click acts only on the tree of its own torrent: the dialog
//! shows one torrent after another, and the same path in the next one is another file. The keys
//! are the Files tab's, but `+` and `-` do nothing: no priorities are set when a torrent is
//! added.
//!
//! The list is given a height of its own, from the window's: inside the dialog's scrolling body,
//! a list that only fills what is left would get almost nothing.

use std::ops::Range;
use std::sync::Arc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    AnyElement, App, Context, FocusHandle, IntoElement, Pixels, Styled as _,
    UniformListScrollHandle, Window, px,
};
use x4_core::{FileRow, FileTree, FileTreeView, NodeKey};

use super::dialog::AddDialog;
use crate::file_list::{self, Columns, Drawn, FileListHost, ROW_HEIGHT, file_row};
use crate::i18n::t;

/// The fewest rows the list shows room for, however small the window.
const MIN_ROWS: f32 = 3.;
/// The most rows it shows at once, however tall the window.
const MAX_ROWS: f32 = 12.;
/// The room the rest of the dialog takes, with the margins around it, which the list leaves free.
const ROOM_AROUND: f32 = 420.;

/// The file list of the add dialog: the tree on screen, the folders open and the cursor.
pub(super) struct FileChoiceList {
    /// The tree drawn: the form's, which shows the choice; `None` for a torrent without a choice.
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

impl FileChoiceList {
    /// A list with no tree yet.
    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            tree: None,
            view: FileTreeView::default(),
            rows: Vec::new(),
            focus: cx.focus_handle().tab_stop(true),
            scroll: UniformListScrollHandle::new(),
        }
    }

    /// Shows `tree`, the form's tree, with the open folders and the cursor fitted to it; a tree of
    /// another torrent starts the view afresh. `None` shows no list.
    pub(super) fn show(&mut self, tree: Option<&Arc<FileTree>>) {
        if let Some(tree) = tree {
            self.view.retain(tree);
            self.rows = self.view.rows(tree);
            self.tree = Some(Arc::clone(tree));
        } else {
            self.tree = None;
            drop(std::mem::take(&mut self.rows));
        }
    }

    /// The tree on screen and the row of it the cursor is on, if any.
    fn at_cursor(&self) -> Option<(Arc<FileTree>, NodeKey)> {
        let tree = self.tree.clone()?;
        let key = self.view.cursor(&tree)?.clone();
        Some((tree, key))
    }
}

/// The height of a list of `rows` rows in a window `viewport_height` high: all of them when they
/// fit, between [`MIN_ROWS`] and [`MAX_ROWS`] otherwise, within what the rest of the dialog leaves.
fn list_height(rows: usize, viewport_height: Pixels) -> Pixels {
    let room = (f32::from(viewport_height) - ROOM_AROUND)
        .clamp(MIN_ROWS * ROW_HEIGHT, MAX_ROWS * ROW_HEIGHT);
    let wanted = f32::from(u16::try_from(rows).unwrap_or(u16::MAX)) * ROW_HEIGHT;
    px(wanted.min(room).max(ROW_HEIGHT))
}

impl AddDialog {
    /// The list of the files of the form's torrent, at a height of its own.
    pub(super) fn file_choice(
        &self,
        window: &Window,
        cx: &mut Context<'_, Self>,
    ) -> impl IntoElement + use<> {
        let height = list_height(self.files.rows.len(), window.viewport_size().height);
        file_list::file_list(
            "add-file-list",
            t!("add-files-list").into(),
            self.files.rows.len(),
            &self.files.focus,
            &self.files.scroll,
            &self.key_repeats,
            cx,
        )
        .w_full()
        .h(height)
        .border_1()
        .border_color(cx.theme().border)
        .rounded(cx.theme().radius)
    }

    /// Applies a click on the checkbox of `key`, or Space on it, to the form of the torrent of
    /// `tree`: the form changes the wanted states of its tree in place, and the list shows it
    /// again.
    fn toggle_wanted(&mut self, tree: &FileTree, key: &NodeKey, cx: &mut Context<'_, Self>) {
        let Some(shown) = &mut self.shown else {
            return;
        };
        if shown
            .form
            .tree()
            .is_none_or(|current| current.hash() != tree.hash())
        {
            tracing::debug!("a checkbox of another torrent's files; nothing changed");
            return;
        }
        shown.form.toggle_wanted(key);
        let tree = shown.form.tree().cloned();
        self.files.show(tree.as_ref());
        self.recheck();
        cx.notify();
    }
}

/// The add dialog's list: its checkboxes change the form's choice of files, and nothing is sent.
impl FileListHost for AddDialog {
    fn file_list_focus(&self) -> &FocusHandle {
        &self.files.focus
    }

    /// The rows of the list at `range`, as drawn now; each takes the tree of this frame. Their
    /// checkboxes always act, the daemon connected or not, and only the sizes are drawn.
    fn file_rows(&mut self, range: Range<usize>, cx: &mut Context<'_, Self>) -> Vec<AnyElement> {
        let Some(tree) = self.files.tree.clone() else {
            return Vec::new();
        };
        let drawn = Drawn {
            cursor: self.files.view.cursor(&tree).cloned(),
            tree,
            enabled: true,
            color: cx.theme().primary,
            columns: Columns {
                size: true,
                progress: false,
                priority: false,
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
        let files = &mut self.files;
        if file_list::change_view(
            files.tree.as_deref(),
            &mut files.view,
            &mut files.rows,
            &files.scroll,
            drawn,
            change,
        ) {
            cx.notify();
        }
    }

    fn file_page_step(&self) -> isize {
        file_list::page_step(&self.files.scroll)
    }

    /// The cursor goes to the row, and the form's choice changes.
    fn click_wanted(&mut self, tree: &FileTree, key: &NodeKey, cx: &mut Context<'_, Self>) {
        self.click_file_row(tree, key, cx);
        self.toggle_wanted(tree, key, cx);
    }

    fn toggle_cursor_wanted(&mut self, cx: &mut Context<'_, Self>) {
        if let Some((tree, key)) = self.files.at_cursor() {
            self.toggle_wanted(&tree, &key, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_as_high_as_its_rows_within_what_the_window_leaves() {
        let row = px(ROW_HEIGHT);
        assert_eq!(list_height(1, px(800.)), row);
        assert_eq!(list_height(4, px(800.)), row * 4.);
        assert_eq!(list_height(0, px(800.)), row, "never nothing");
        assert_eq!(list_height(2000, px(600.)), px(600. - ROOM_AROUND));
        assert_eq!(list_height(2000, px(3000.)), row * MAX_ROWS);
        assert_eq!(list_height(2000, px(300.)), row * MIN_ROWS);
    }
}
