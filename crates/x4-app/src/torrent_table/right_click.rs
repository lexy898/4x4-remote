//! A right-click on the torrent list, from the press to the context menu it opens: it selects as a
//! file manager does (`right_click`), and a right-click that lands on no row gets the keyboard
//! focus back from gpui-component's empty menu (`watch_right_click`).

use gpui_kit::component::table::TableState;
use gpui_kit::{App, Context, Entity, Focusable as _, MouseButton, MouseDownEvent, Window};

use super::{SelectionChange, TorrentTable, change_selection};

/// How far a right-click on the list has come, from the press to the context menu it opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RightClick {
    /// No right-click is under way, or its menu has been built.
    Idle,
    /// The press has come (`watch_right_click`), and it has not landed on a row. The table goes on
    /// marking the row it last right-clicked, so the context menu may still be asked for that old
    /// row; it must not open that row's menu. If no menu is built for a row, gpui-component's
    /// empty menu, which it never draws, has taken the keyboard focus, and `watch_right_click`
    /// gives it back.
    Pressed,
    /// The press has landed on a row (`right_click`), and the menu built next is that row's.
    OnRow,
}

/// Takes a right-click on a row, before its context menu is built, as a file manager does. It acts
/// on the torrent the button went down on, as its row was drawn then
/// (`TorrentTable::right_pressed`), wherever a snapshot has put it since. A torrent outside the
/// selection becomes the only selected one (and the lead), as a click would make it; one inside
/// leaves the selection as it is, so that the menu acts on all of it. The table marks its row as
/// the right-clicked one, which its context menu opens for: moving its own row to the lead unmarks
/// it. The menu that the right-click opens next is the torrent's. A torrent that has left the list
/// by then is not right-clicked: no row is marked, and no menu opens.
pub(crate) fn right_click(
    table: &mut TableState<TorrentTable>,
    cx: &mut Context<'_, TableState<TorrentTable>>,
) {
    let delegate = table.delegate_mut();
    let Some(hash) = delegate.right_pressed.take() else {
        return;
    };
    let Some(row_ix) = delegate.index_of(&hash) else {
        table.set_right_clicked_row(None, cx);
        return;
    };
    delegate.right_click = RightClick::OnRow;
    if !delegate.selection.contains(&hash) {
        change_selection(table, SelectionChange::Click(hash), cx);
    }
    if table.right_clicked_row() != Some(row_ix) {
        table.set_right_clicked_row(Some(row_ix), cx);
    }
}

/// Watches a mouse press on the list, which must be handed to it in the capture phase: the context
/// menu stops the press in the bubble phase. A right-click that lands on no row (below the rows, on
/// the header) opens an empty menu that gpui-component does not draw but gives the keyboard focus,
/// and the list's keys would be dead until the user clicked again. So the focus is noted before the
/// press does anything, and given back after the menu has had its turn, unless a menu was built.
///
/// The focus goes back from two deferred calls in a row: the context menu builds itself in a
/// deferred call of its own, queued after ours, and deferred calls run in the order they were
/// queued, so a call queued by ours runs after it.
pub(crate) fn watch_right_click(
    table: &Entity<TableState<TorrentTable>>,
    event: &MouseDownEvent,
    window: &mut Window,
    cx: &mut App,
) {
    if event.button != MouseButton::Right {
        return;
    }
    let before = window.focused(cx);
    table.update(cx, |table, _| {
        table.delegate_mut().right_click = RightClick::Pressed;
    });
    let table = table.clone();
    window.defer(cx, move |window, cx| {
        window.defer(cx, move |window, cx| {
            let unanswered = table.update(cx, |table, _| {
                let delegate = table.delegate_mut();
                let unanswered = delegate.right_click == RightClick::Pressed;
                if unanswered {
                    delegate.right_click = RightClick::Idle;
                }
                unanswered
            });
            if unanswered {
                let focus = before.unwrap_or_else(|| table.focus_handle(cx));
                window.focus(&focus, cx);
            }
        });
    });
}
