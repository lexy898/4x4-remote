//! What the user does to the selection of the torrent table, and how the table's own selected row
//! follows it: a click, a key or the table's own Escape becomes a [`SelectionChange`], which the
//! delegate's [`Selection`](x4_core::Selection) takes; the table's row then mirrors the lead
//! (`mirror_lead`), and a key brings the lead into view (`reveal_lead`). A click names the torrent
//! its press was on (`click_row`), and `follow_table` takes the table's own selection of a row
//! for what it is: an echo of ours, or a click of its own.

use gpui_kit::component::table::TableState;
use gpui_kit::{ClickEvent, Context, ScrollStrategy};
use x4_core::Edge;
use x4_rpc::HashString;

use super::TorrentTable;

/// What the user does to the selection, with the mouse or a key. A click names its torrent by
/// hash, and changes nothing if the rows no longer show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SelectionChange {
    /// A plain click on the torrent: only that row.
    Click(HashString),
    /// A Ctrl+click on the torrent: it goes in or out of the selection.
    Toggle(HashString),
    /// A Shift+click on the torrent: the range from the anchor to it.
    ExtendTo(HashString),
    /// A moving key, `step` rows from the lead (negative is up): only the row it lands on, or with
    /// `extend` (Shift) the range from the anchor to it. Up and Down are one row, Page Up and Page
    /// Down a page.
    Step {
        /// How many rows, and which way.
        step: isize,
        /// Whether the range from the anchor follows the lead.
        extend: bool,
    },
    /// Home or End: only the first or the last row, or with `extend` (Shift) the range from the
    /// anchor to it. Unlike a step, it reaches its end also when nothing is selected.
    ToEdge {
        /// Which end.
        edge: Edge,
        /// Whether the range from the anchor follows the lead.
        extend: bool,
    },
    /// Ctrl+A: every row shown.
    All,
    /// Escape or Ctrl+Shift+A: no row.
    Nothing,
}

impl TorrentTable {
    /// How many times [`change_selection`](Self::change_selection) has changed the selection; a
    /// new count tells that it has changed since the count was read.
    pub(crate) fn selection_changes(&self) -> u64 {
        self.selection_changes
    }

    /// Applies `change` to the selection; returns whether the selection changed. A click on a
    /// torrent the rows do not show changes nothing.
    pub(crate) fn change_selection(&mut self, change: SelectionChange) -> bool {
        let before = self.selection.clone();
        let Self {
            rows, selection, ..
        } = self;
        let shown = |hash: &HashString| rows.iter().any(|row| &row.hash == hash);
        match change {
            SelectionChange::Click(hash) => {
                if shown(&hash) {
                    selection.click(&hash);
                }
            }
            SelectionChange::Toggle(hash) => {
                if shown(&hash) {
                    selection.toggle(&hash);
                }
            }
            SelectionChange::ExtendTo(hash) => {
                if shown(&hash) {
                    selection.extend_to(&hash, rows);
                }
            }
            SelectionChange::Step { step, extend } => selection.move_lead(step, rows, extend),
            SelectionChange::ToEdge { edge, extend } => selection.move_lead_to(edge, rows, extend),
            SelectionChange::All => selection.select_all(rows),
            SelectionChange::Nothing => selection.clear(),
        }
        let changed = *selection != before;
        if changed {
            self.selection_changes = self.selection_changes.wrapping_add(1);
        }
        changed
    }
}

/// Applies what the user did to the selection of `table`, and moves the table's selected row to
/// the lead. A click and a key bring the lead into view; Ctrl+A leaves the view where it is. A
/// moving key also brings back a lead the user has scrolled away from when it cannot move it, such
/// as Down on the last row. The table is drawn again only when the selection or the view changed.
pub(crate) fn change_selection(
    table: &mut TableState<TorrentTable>,
    change: SelectionChange,
    cx: &mut Context<'_, TableState<TorrentTable>>,
) {
    let scroll = change != SelectionChange::All;
    let reveal = matches!(
        change,
        SelectionChange::Step { .. } | SelectionChange::ToEdge { .. }
    );
    let changed = table.delegate_mut().change_selection(change);
    mirror_lead(table, scroll, cx);
    if reveal {
        reveal_lead(table, cx);
    }
    if changed {
        cx.notify();
    }
}

/// How many rows Page Up and Page Down move the lead by: the rows on screen, less the one that is
/// usually cut off at an edge, and at least one.
pub(crate) fn page_step(table: &TableState<TorrentTable>) -> isize {
    let rows = table.visible_range().rows().len().saturating_sub(1).max(1);
    isize::try_from(rows).unwrap_or(isize::MAX)
}

/// Takes over what the table did to its own selected row, when that is not the lead. Every key
/// that moves the table's row is ours now, so what is left is the table's Escape, which cleared
/// its row and so clears the selection, and the echoes: of [`mirror_lead`], and of the table
/// selecting the row of a plain click after `render_tr` has. That row is the one under the
/// pointer when the button came up, which need not be the torrent clicked (`click_row`), so the
/// table's row goes back to the lead. A `SelectRow` that is no echo (a caller that sets the
/// table's row itself, such as the demo's `--select`) is taken as a click. Called for each of the
/// table's `SelectRow` and `ClearSelection` events. The table's row is read when the event
/// arrives, not taken from the event, so an event that a later change has overtaken changes
/// nothing.
pub(crate) fn follow_table(
    table: &mut TableState<TorrentTable>,
    cx: &mut Context<'_, TableState<TorrentTable>>,
) {
    // The first event after a click is the click's own, so the echo it expects goes with it.
    let echo = table.delegate_mut().click_echo.take();
    let row = table.selected_row();
    if row == table.delegate().lead_index() {
        return;
    }
    let change = match row {
        None => SelectionChange::Nothing,
        Some(row_ix) if Some(row_ix) == echo => {
            mirror_lead(table, true, cx);
            return;
        }
        Some(row_ix) => match table.delegate().rows.get(row_ix) {
            Some(row) => SelectionChange::Click(row.hash.clone()),
            None => return,
        },
    };
    change_selection(table, change, cx);
}

/// Takes a click on the row drawn at `row_ix`, which acts on the torrent the button went down on
/// (`TorrentTable::pressed`), wherever a snapshot has put it since, and on nothing if it has left
/// the list. A plain click selects it alone, Ctrl+click toggles it and Shift+click selects the
/// range to it. The table then selects its row at `row_ix` by itself, which `follow_table` takes
/// back.
pub(super) fn click_row(
    table: &mut TableState<TorrentTable>,
    row_ix: usize,
    event: &ClickEvent,
    cx: &mut Context<'_, TableState<TorrentTable>>,
) {
    let modifiers = event.modifiers();
    let delegate = table.delegate_mut();
    let pressed = delegate.pressed.take();
    // What the table's own click listener does with it (`select_on_modifier_click(false)`).
    let plain = !(modifiers.shift || modifiers.secondary(cx));
    if plain {
        delegate.click_echo = Some(row_ix);
    }
    let Some(hash) = pressed else {
        return;
    };
    let change = if modifiers.shift {
        SelectionChange::ExtendTo(hash)
    } else if modifiers.secondary(cx) {
        SelectionChange::Toggle(hash)
    } else {
        SelectionChange::Click(hash)
    };
    change_selection(table, change, cx);
}

/// Scrolls the lead into view when it is off screen. A lead on screen, even if partly, leaves the
/// view as it is and draws nothing, so that a key held on the last row costs no frames.
fn reveal_lead(
    table: &mut TableState<TorrentTable>,
    cx: &mut Context<'_, TableState<TorrentTable>>,
) {
    let Some(row_ix) = table.delegate().lead_index() else {
        return;
    };
    if table.visible_range().rows().contains(&row_ix) {
        return;
    }
    table
        .vertical_scroll_handle
        .scroll_to_item(row_ix, ScrollStrategy::Nearest);
    cx.notify();
}

/// Makes the table's own selected row the lead while the lead is selected, and clears it
/// otherwise, so that the table never paints a row the selection does not hold. With `scroll`,
/// the table scrolls the lead into view as it selects it; without, a scroll it would start is
/// taken back, and one the user asked for earlier in this frame stays.
pub(super) fn mirror_lead(
    table: &mut TableState<TorrentTable>,
    scroll: bool,
    cx: &mut Context<'_, TableState<TorrentTable>>,
) {
    let lead = table.delegate().lead_index();
    if lead == table.selected_row() {
        return;
    }
    match lead {
        Some(row_ix) if scroll => table.set_selected_row(row_ix, cx),
        Some(row_ix) => {
            let pending = table
                .vertical_scroll_handle
                .0
                .borrow()
                .deferred_scroll_to_item;
            table.set_selected_row(row_ix, cx);
            table
                .vertical_scroll_handle
                .0
                .borrow_mut()
                .deferred_scroll_to_item = pending;
        }
        None => table.clear_selection(cx),
    }
}
