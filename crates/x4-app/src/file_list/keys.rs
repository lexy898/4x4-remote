//! The keys of the file list, bound in [`keymap`](crate::keymap) in its context `FileList`: Up,
//! Down, Home, End, Page Up and Page Down move the cursor; Left closes the folder under it or goes
//! to the folder around it, and Right opens it or goes into it; Space downloads or skips the row
//! under the cursor, and `+` and `-` raise or lower its priority by a step, in a list that has
//! priorities.
//!
//! The moving keys repeat while held, as in the table. Space, `+` and `-` act once per press of
//! the key, through the main view's record of the key held (the `key_repeats` module): every
//! repeat would flip the row again. They act on the tree on screen, through the list's
//! [`FileListHost`].

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::{Context, InteractiveElement};
use x4_core::{Edge, FileTreeView};

use super::FileListHost;
use crate::key_repeats::{KeyRepeats, once_per_press_of};
use crate::keymap::{
    CollapseFileFolder, ExpandFileFolder, LowerFilePriority, MoveFileCursorDown,
    MoveFileCursorPageDown, MoveFileCursorPageUp, MoveFileCursorToFirst, MoveFileCursorToLast,
    MoveFileCursorUp, RaiseFilePriority, ToggleFileWanted,
};

/// `list`, a file list, with the handlers of its keys, which go to the host of `cx`; `repeats` is
/// the main view's record of the key held.
pub(crate) fn on_file_keys<E: InteractiveElement, H: FileListHost>(
    list: E,
    repeats: &Rc<RefCell<KeyRepeats>>,
    cx: &Context<'_, H>,
) -> E {
    list.on_action(cx.listener(|host: &mut H, _: &MoveFileCursorUp, _, cx| {
        host.step_file_cursor(-1, cx);
    }))
    .on_action(cx.listener(|host: &mut H, _: &MoveFileCursorDown, _, cx| {
        host.step_file_cursor(1, cx);
    }))
    .on_action(
        cx.listener(|host: &mut H, _: &MoveFileCursorPageUp, _, cx| {
            let step = host.file_page_step();
            host.step_file_cursor(-step, cx);
        }),
    )
    .on_action(
        cx.listener(|host: &mut H, _: &MoveFileCursorPageDown, _, cx| {
            let step = host.file_page_step();
            host.step_file_cursor(step, cx);
        }),
    )
    .on_action(
        cx.listener(|host: &mut H, _: &MoveFileCursorToFirst, _, cx| {
            host.change_file_view(
                None,
                |view, tree| view.cursor_to_edge(tree, Edge::First),
                cx,
            );
        }),
    )
    .on_action(
        cx.listener(|host: &mut H, _: &MoveFileCursorToLast, _, cx| {
            host.change_file_view(None, |view, tree| view.cursor_to_edge(tree, Edge::Last), cx);
        }),
    )
    .on_action(cx.listener(|host: &mut H, _: &CollapseFileFolder, _, cx| {
        host.change_file_view(None, FileTreeView::collapse_or_parent, cx);
    }))
    .on_action(cx.listener(|host: &mut H, _: &ExpandFileFolder, _, cx| {
        host.change_file_view(None, FileTreeView::expand_or_first_child, cx);
    }))
    .on_action(once_per_press_of::<_, ToggleFileWanted>(
        repeats,
        cx,
        |host: &mut H, _, cx| host.toggle_cursor_wanted(cx),
    ))
    .on_action(once_per_press_of::<_, RaiseFilePriority>(
        repeats,
        cx,
        |host: &mut H, _, cx| host.step_cursor_priority(true, cx),
    ))
    .on_action(once_per_press_of::<_, LowerFilePriority>(
        repeats,
        cx,
        |host: &mut H, _, cx| host.step_cursor_priority(false, cx),
    ))
}
