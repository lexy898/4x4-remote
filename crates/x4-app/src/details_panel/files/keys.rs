//! The keys of the file list, bound in [`keymap`](crate::keymap) in its context `FileList`: Up,
//! Down, Home, End, Page Up and Page Down move the cursor; Left closes the folder under it or goes
//! to the folder around it, and Right opens it or goes into it; Space downloads or skips the row
//! under the cursor, and `+` and `-` raise or lower its priority by a step.
//!
//! The moving keys repeat while held, as in the table. Space, `+` and `-` act once per press of
//! the key, through the main view's record of the key held (the `key_repeats` module): every
//! repeat would flip the row again. They act on the tree on screen, which builds the change.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::{Context, InteractiveElement};
use x4_core::{Edge, FileTreeView};

use super::super::DetailsPanel;
use crate::actions::FileRequest;
use crate::key_repeats::{KeyRepeats, once_per_press_of};
use crate::keymap::{
    CollapseFileFolder, ExpandFileFolder, LowerFilePriority, MoveFileCursorDown,
    MoveFileCursorPageDown, MoveFileCursorPageUp, MoveFileCursorToFirst, MoveFileCursorToLast,
    MoveFileCursorUp, RaiseFilePriority, ToggleFileWanted,
};

/// `list`, the file list, with the handlers of its keys; `repeats` is the main view's record of
/// the key held.
pub(super) fn on_file_keys<E: InteractiveElement>(
    list: E,
    repeats: &Rc<RefCell<KeyRepeats>>,
    cx: &Context<'_, DetailsPanel>,
) -> E {
    list.on_action(cx.listener(|panel, _: &MoveFileCursorUp, _, cx| {
        panel.step_file_cursor(-1, cx);
    }))
    .on_action(cx.listener(|panel, _: &MoveFileCursorDown, _, cx| {
        panel.step_file_cursor(1, cx);
    }))
    .on_action(cx.listener(|panel, _: &MoveFileCursorPageUp, _, cx| {
        let step = panel.files.page_step();
        panel.step_file_cursor(-step, cx);
    }))
    .on_action(cx.listener(|panel, _: &MoveFileCursorPageDown, _, cx| {
        let step = panel.files.page_step();
        panel.step_file_cursor(step, cx);
    }))
    .on_action(cx.listener(|panel, _: &MoveFileCursorToFirst, _, cx| {
        panel.change_file_view(
            None,
            |view, tree| view.cursor_to_edge(tree, Edge::First),
            cx,
        );
    }))
    .on_action(cx.listener(|panel, _: &MoveFileCursorToLast, _, cx| {
        panel.change_file_view(None, |view, tree| view.cursor_to_edge(tree, Edge::Last), cx);
    }))
    .on_action(cx.listener(|panel, _: &CollapseFileFolder, _, cx| {
        panel.change_file_view(None, FileTreeView::collapse_or_parent, cx);
    }))
    .on_action(cx.listener(|panel, _: &ExpandFileFolder, _, cx| {
        panel.change_file_view(None, FileTreeView::expand_or_first_child, cx);
    }))
    .on_action(once_per_press_of::<_, ToggleFileWanted>(
        repeats,
        cx,
        |panel, _, cx| panel.toggle_cursor_wanted(cx),
    ))
    .on_action(once_per_press_of::<_, RaiseFilePriority>(
        repeats,
        cx,
        |panel, _, cx| panel.step_cursor_priority(true, cx),
    ))
    .on_action(once_per_press_of::<_, LowerFilePriority>(
        repeats,
        cx,
        |panel, _, cx| panel.step_cursor_priority(false, cx),
    ))
}

impl DetailsPanel {
    /// Moves the file cursor `step` rows down, or up when `step` is negative.
    fn step_file_cursor(&mut self, step: isize, cx: &mut Context<'_, Self>) {
        self.change_file_view(None, |view, tree| view.move_cursor(tree, step), cx);
    }

    /// Space: downloads the row under the cursor, or skips it, as the tree on screen builds it.
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

    /// `+` (`up`) or `-`: gives the row under the cursor the next priority up or down, as the tree
    /// on screen builds it.
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
