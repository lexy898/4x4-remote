//! The selection keys of the torrent list, as the main view handles them: Ctrl+A, Ctrl+Shift+A and
//! Escape, the arrows, Home, End, Page Up and Page Down, with and without Shift (bound in
//! [`keymap`](crate::keymap)). Each hands its change to the table, which holds the selection.

use gpui_kit::{Context, Div, InteractiveElement as _};
use x4_core::Edge;

use super::MainView;
use crate::keymap::{
    DeselectTorrents, ExtendSelectionDown, ExtendSelectionToFirst, ExtendSelectionToLast,
    ExtendSelectionUp, MoveSelectionDown, MoveSelectionPageDown, MoveSelectionPageUp,
    MoveSelectionToFirst, MoveSelectionToLast, MoveSelectionUp, SelectAllTorrents,
};
use crate::torrent_table::{SelectionChange, change_selection, page_step};

impl MainView {
    /// Handles a selection key of the torrent list.
    fn change_selection(&self, change: SelectionChange, cx: &mut Context<'_, Self>) {
        self.table
            .update(cx, |table, cx| change_selection(table, change, cx));
    }

    /// Handles Page Up (`pages` −1) or Page Down (1): selects only the row a page from the lead.
    fn change_selection_by_page(&self, pages: isize, cx: &mut Context<'_, Self>) {
        self.table.update(cx, |table, cx| {
            let step = page_step(table).saturating_mul(pages);
            change_selection(table, self::step(step, false), cx);
        });
    }
}

/// `root`, the main view's root element, with the handlers of the selection keys.
pub(super) fn on_selection_keys(root: Div, cx: &Context<'_, MainView>) -> Div {
    root.on_action(cx.listener(|view, _: &SelectAllTorrents, _, cx| {
        view.change_selection(SelectionChange::All, cx);
    }))
    .on_action(cx.listener(|view, _: &DeselectTorrents, _, cx| {
        view.change_selection(SelectionChange::Nothing, cx);
    }))
    .on_action(cx.listener(|view, _: &MoveSelectionUp, _, cx| {
        view.change_selection(step(-1, false), cx);
    }))
    .on_action(cx.listener(|view, _: &MoveSelectionDown, _, cx| {
        view.change_selection(step(1, false), cx);
    }))
    .on_action(cx.listener(|view, _: &ExtendSelectionUp, _, cx| {
        view.change_selection(step(-1, true), cx);
    }))
    .on_action(cx.listener(|view, _: &ExtendSelectionDown, _, cx| {
        view.change_selection(step(1, true), cx);
    }))
    .on_action(cx.listener(|view, _: &MoveSelectionToFirst, _, cx| {
        view.change_selection(to_edge(Edge::First, false), cx);
    }))
    .on_action(cx.listener(|view, _: &MoveSelectionToLast, _, cx| {
        view.change_selection(to_edge(Edge::Last, false), cx);
    }))
    .on_action(cx.listener(|view, _: &ExtendSelectionToFirst, _, cx| {
        view.change_selection(to_edge(Edge::First, true), cx);
    }))
    .on_action(cx.listener(|view, _: &ExtendSelectionToLast, _, cx| {
        view.change_selection(to_edge(Edge::Last, true), cx);
    }))
    .on_action(cx.listener(|view, _: &MoveSelectionPageUp, _, cx| {
        view.change_selection_by_page(-1, cx);
    }))
    .on_action(cx.listener(|view, _: &MoveSelectionPageDown, _, cx| {
        view.change_selection_by_page(1, cx);
    }))
}

/// A move of the lead by `step` rows (negative is up), extending the range or not.
const fn step(step: isize, extend: bool) -> SelectionChange {
    SelectionChange::Step { step, extend }
}

/// A move of the lead to an end of the list (Home, End), extending the range or not.
const fn to_edge(edge: Edge, extend: bool) -> SelectionChange {
    SelectionChange::ToEdge { edge, extend }
}
