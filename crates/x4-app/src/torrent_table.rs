//! The torrent table: the delegate behind gpui-component's [`DataTable`], which holds the rows of
//! the last snapshot, the columns as the user arranged them, the sort order and the selection.
//!
//! The rows come decided from `x4-core` ([`Row`]); the cells word them only when they are drawn,
//! so a snapshot formats no text for the rows off screen.
//!
//! The selection is the delegate's [`Selection`], kept by hash, so it stays on the same torrents
//! when a new snapshot re-sorts or filters the list; the rows that leave the list leave it, and a
//! snapshot never scrolls the view (`show_rows`). The table itself has one selected row only: it
//! mirrors the selection's lead while the lead is selected, and has none otherwise, so that it
//! never paints a row that is not selected. The other selected rows get a mark of the same color
//! from the delegate. Plain, Ctrl and Shift clicks on a row are handled here before the table sees
//! them (`render_tr`), and so are the selection keys, the moving keys of the table included
//! (`change_selection`); a key brings the lead into view. What the table still does to its row by
//! itself, its Escape, is taken over by `follow_table`, which tells it from the table echoing ours
//! by comparing its row with the lead.
//!
//! The table's own sorting and column selection are off. A click on a header goes to the handler
//! set with [`TorrentTable::on_sort`], through which the main view, the owner of the sort order,
//! sorts the rows and hands them back. The columns are the delegate's: the table reports a move
//! or a resize, the delegate applies it to its [`ColumnState`]s, and it gives the stored widths
//! back whenever the table rebuilds its columns, which happens only when one is shown or hidden
//! (`show_column`).
//!
//! [`DataTable`]: gpui_kit::component::table::DataTable

mod cells;

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::table::{Column, TableDelegate, TableState};
use gpui_kit::component::{ActiveTheme as _, Theme, ThemeToken};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ClickEvent, Context, Div, InteractiveElement as _, IntoElement, ParentElement as _,
    Pixels, ScrollStrategy, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _,
    TestSupportExt as _, Window, div, px,
};
use x4_core::{
    ColumnId, ColumnState, EmptyTable, MAX_COLUMN_WIDTH, MIN_COLUMN_WIDTH, Row, Selection,
    SortDirection, SortKey, TableModel, move_shown_column, set_column_shown, set_shown_widths,
    shown_columns,
};

use crate::i18n::{column_name_text, empty_table_text};
use crate::theme::row_opacity;

/// What a click on a column header does, given the column.
type SortHandler = Rc<dyn Fn(ColumnId, &mut Window, &mut App)>;

/// What the user does to the selection, with the mouse or a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectionChange {
    /// A plain click on the row at this index: only that row.
    Click(usize),
    /// A Ctrl+click on the row: it goes in or out of the selection.
    Toggle(usize),
    /// A Shift+click on the row: the range from the anchor to it.
    ExtendTo(usize),
    /// A moving key, `step` rows from the lead (negative is up): only the row it lands on, or with
    /// `extend` (Shift) the range from the anchor to it. Up and Down are one row, Page Up and Page
    /// Down a page, Home and End `isize::MAX` rows.
    Step {
        /// How many rows, and which way.
        step: isize,
        /// Whether the range from the anchor follows the lead.
        extend: bool,
    },
    /// Ctrl+A: every row shown.
    All,
    /// Escape or Ctrl+Shift+A: no row.
    Nothing,
}

/// The delegate of the torrent table: the rows to show, the columns, the sort order the header
/// marks and the selection.
pub struct TorrentTable {
    /// The rows in the order shown, shared with the main view's model.
    rows: Arc<[Row]>,
    /// Every column, shown or hidden, in the order the user arranged them.
    columns: Vec<ColumnState>,
    /// The selected torrents, which stay selected wherever a new snapshot puts them.
    selection: Selection,
    /// The sort order of the rows, which the header of its column marks.
    sort: (SortKey, SortDirection),
    /// What the table says when it has no rows.
    empty: EmptyTable,
    /// Whether the rows are old data, which the daemon has not confirmed lately.
    stale: bool,
    /// Called with the column whose header is clicked.
    on_sort: Option<SortHandler>,
}

impl TorrentTable {
    /// A table with no rows yet, showing `columns` (in their order, the hidden ones left out), and
    /// marking `sort` in the header.
    #[must_use]
    pub fn new(columns: Vec<ColumnState>, sort: (SortKey, SortDirection)) -> Self {
        Self {
            rows: Arc::new([]),
            columns,
            selection: Selection::default(),
            sort,
            empty: EmptyTable::Waiting,
            stale: false,
            on_sort: None,
        }
    }

    /// Calls `handler` with the column whose header the user clicks. The table does not sort by
    /// itself: the handler sorts the rows and hands them back through a new model.
    #[must_use]
    pub fn on_sort(mut self, handler: impl Fn(ColumnId, &mut Window, &mut App) + 'static) -> Self {
        self.on_sort = Some(Rc::new(handler));
        self
    }

    /// Takes the rows of a new snapshot, or of a new filter or search. The selection keeps the
    /// torrents that are still among them, wherever they are now, and drops the others, so that an
    /// action never reaches a torrent the user cannot see. Returns whether the rows or the
    /// selection changed, that is whether the table must be drawn again.
    pub(crate) fn set_rows(&mut self, rows: Arc<[Row]>) -> bool {
        let rows_changed = !Arc::ptr_eq(&self.rows, &rows) && *self.rows != *rows;
        self.rows = rows;
        let selection_changed = self.selection.retain_visible(&self.rows);
        rows_changed || selection_changed
    }

    /// The rows in the order shown.
    #[must_use]
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Every column, shown or hidden, in the order the user arranged them, with their widths.
    #[must_use]
    pub fn columns(&self) -> &[ColumnState] {
        &self.columns
    }

    /// The selected torrents. What an action reaches is `selection().targets(rows())`: the
    /// selected rows that are shown, in the order shown.
    #[must_use]
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// The row of the selection's lead (the row the keys move from), while the lead is selected:
    /// the row the table marks as its own selected row. `None` when nothing is selected, and when
    /// a Ctrl+click took the lead out of the selection.
    #[must_use]
    pub fn lead_index(&self) -> Option<usize> {
        let lead = self
            .selection
            .lead()
            .filter(|lead| self.selection.contains(lead))?;
        self.rows.iter().position(|row| &row.hash == lead)
    }

    /// The sort order the header marks.
    #[must_use]
    pub fn sort(&self) -> (SortKey, SortDirection) {
        self.sort
    }

    /// Marks `sort` in the header.
    pub(crate) fn set_sort(&mut self, sort: (SortKey, SortDirection)) {
        self.sort = sort;
    }

    /// Applies `change` to the selection; returns whether the selection changed. A click on an
    /// index past the rows changes nothing.
    pub(crate) fn change_selection(&mut self, change: SelectionChange) -> bool {
        let before = self.selection.clone();
        let Self {
            rows, selection, ..
        } = self;
        let hash_at = |row_ix: usize| rows.get(row_ix).map(|row| &row.hash);
        match change {
            SelectionChange::Click(row_ix) => {
                if let Some(hash) = hash_at(row_ix) {
                    selection.click(hash);
                }
            }
            SelectionChange::Toggle(row_ix) => {
                if let Some(hash) = hash_at(row_ix) {
                    selection.toggle(hash);
                }
            }
            SelectionChange::ExtendTo(row_ix) => {
                if let Some(hash) = hash_at(row_ix) {
                    selection.extend_to(hash, rows);
                }
            }
            SelectionChange::Step { step, extend } => selection.move_lead(step, rows, extend),
            SelectionChange::All => selection.select_all(rows),
            SelectionChange::Nothing => selection.clear(),
        }
        *selection != before
    }

    /// Whether the rows are drawn as stale data.
    #[must_use]
    pub fn stale(&self) -> bool {
        self.stale
    }

    /// Draws the rows as stale data, or as fresh again; returns whether that changed.
    pub(crate) fn set_stale(&mut self, stale: bool) -> bool {
        let changed = self.stale != stale;
        self.stale = stale;
        changed
    }

    /// The content of the cell at `row_ix` and `col_ix`, at the row's opacity. The opacity is on
    /// the content, not on the row, so that the background of a selected row keeps its full
    /// color.
    fn cell_element(&self, row_ix: usize, col_ix: usize, theme: &Theme) -> Div {
        let content = div().size_full();
        match (self.rows.get(row_ix), self.shown_id(col_ix)) {
            (Some(row), Some(column)) => content
                .when_some(row_opacity(self.stale, row.dimmed), Div::opacity)
                .child(cells::cell(row, column, theme)),
            _ => content,
        }
    }

    /// The mark of a selected row that the table does not paint itself: every selected row but
    /// the lead, which is the table's own selected row. It lies under the cells, over the whole
    /// row, in the color the table paints its own row with, so that every selected row looks the
    /// same. It is a child rather than the row's background, because the table's hover color
    /// replaces that background under the pointer, and the row would look unselected; the mark
    /// stays over the hover color. An idle row's dimmed cells leave it at full color.
    fn selection_mark(&self, row_ix: usize, theme: &Theme) -> Option<Stateful<Div>> {
        let hash = &self.rows.get(row_ix)?.hash;
        // A selected lead is the table's row (`lead_index`).
        let marked = self.selection.contains(hash) && self.selection.lead() != Some(hash);
        marked.then(|| {
            div()
                .id(("row-selection", row_ix))
                .absolute()
                .inset_0()
                .bg(selected_background(theme))
        })
    }

    /// Takes the widths of the shown columns, in order, as the table reports them; returns
    /// whether any changed.
    pub(crate) fn set_widths(&mut self, widths: &[Pixels]) -> bool {
        let widths: Vec<f32> = widths.iter().copied().map(f32::from).collect();
        set_shown_widths(&mut self.columns, &widths)
    }

    /// The shown column at `col_ix`, counted as the table counts them.
    fn shown(&self, col_ix: usize) -> Option<&ColumnState> {
        shown_columns(&self.columns).nth(col_ix)
    }

    fn shown_id(&self, col_ix: usize) -> Option<ColumnId> {
        self.shown(col_ix).map(|column| column.id)
    }
}

/// The table's state around `delegate`: its selected row does not wrap around at either end, a
/// Ctrl or Shift click leaves it alone (the delegate handles those), and none of the table's own
/// sorting or column selection.
pub(crate) fn new_table_state(
    delegate: TorrentTable,
    window: &mut Window,
    cx: &mut Context<'_, TableState<TorrentTable>>,
) -> TableState<TorrentTable> {
    TableState::new(delegate, window, cx)
        .col_selectable(false)
        .sortable(false)
        .loop_selection(false)
        .select_on_modifier_click(false)
}

/// The background of a selected row: the color gpui-component's table paints its own selected
/// row with (`TableState`'s row rendering), so that our marks match it.
fn selected_background(theme: &Theme) -> ThemeToken {
    if theme.list.active_highlight {
        theme.tokens.table_active
    } else {
        theme.tokens.accent
    }
}

/// Shows the rows of `model` in `table`. The selection keeps the torrents still shown, the table's
/// selected row follows the lead to its new place, or is cleared when the lead is gone, and the
/// view stays where it is, even when the lead moved out of it. The table is drawn again only when
/// the rows, the selection or the empty text changed.
pub(crate) fn show_rows(
    table: &mut TableState<TorrentTable>,
    model: &TableModel,
    cx: &mut Context<'_, TableState<TorrentTable>>,
) {
    let delegate = table.delegate_mut();
    let mut changed = delegate.set_rows(Arc::clone(&model.rows));
    if let Some(empty) = model.empty() {
        changed |= delegate.empty != empty;
        delegate.empty = empty;
    }
    mirror_lead(table, false, cx);
    if changed {
        cx.notify();
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
    let changed = table.delegate_mut().change_selection(change);
    mirror_lead(table, change != SelectionChange::All, cx);
    if matches!(change, SelectionChange::Step { .. }) {
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
/// selecting the row of a plain click after `render_tr` has. A `SelectRow` that is no echo (a
/// caller that sets the table's row itself, such as the demo's `--select`) is taken as a click.
/// Called for each of the table's `SelectRow` and `ClearSelection` events. The table's row is read
/// when the event arrives, not taken from the event, so an event that a later change has
/// overtaken changes nothing.
pub(crate) fn follow_table(
    table: &mut TableState<TorrentTable>,
    cx: &mut Context<'_, TableState<TorrentTable>>,
) {
    let row = table.selected_row();
    if row == table.delegate().lead_index() {
        return;
    }
    let change = row.map_or(SelectionChange::Nothing, SelectionChange::Click);
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
fn mirror_lead(
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

/// Shows or hides the column `id` and rebuilds the table's columns, keeping the widths the user
/// gave them; returns whether anything changed. The name column cannot be hidden.
pub(crate) fn show_column(
    table: &mut TableState<TorrentTable>,
    id: ColumnId,
    shown: bool,
    cx: &mut Context<'_, TableState<TorrentTable>>,
) -> bool {
    // The table's widths follow a resize as it happens, and rebuilding its columns reads them
    // from the delegate, so take them first.
    let widths: Vec<Pixels> = (0..).map_while(|col_ix| table.col_width(col_ix)).collect();
    let delegate = table.delegate_mut();
    delegate.set_widths(&widths);
    if !set_column_shown(&mut delegate.columns, id, shown) {
        return false;
    }
    table.refresh(cx);
    cx.notify();
    true
}

impl TableDelegate for TorrentTable {
    fn columns_count(&self, _: &App) -> usize {
        shown_columns(&self.columns).count()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> Column {
        let Some(column) = self.shown(col_ix) else {
            return Column::default();
        };
        Column::new(column.id.key(), column_name_text(column.id))
            .width(px(column.width))
            .min_width(px(MIN_COLUMN_WIDTH))
            .max_width(px(MAX_COLUMN_WIDTH))
            .selectable(false)
            .when(cells::is_numeric(column.id), Column::text_right)
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<'_, TableState<Self>>,
    ) -> impl IntoElement {
        let Some(column) = self.shown_id(col_ix) else {
            return div().into_any_element();
        };
        let (key, direction) = self.sort;
        let sorted = (column.sort_key() == key).then_some(direction);
        let on_sort = self.on_sort.clone();
        cells::header(column, sorted, move |window, cx| {
            if let Some(on_sort) = &on_sort {
                on_sort(column, window, cx);
            }
        })
        .into_any_element()
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        cx: &mut Context<'_, TableState<Self>>,
    ) -> Stateful<Div> {
        // No opacity here: it would also fade the selected row's background (`cell_element`).
        div()
            .id(("row", row_ix))
            .when_some(self.selection_mark(row_ix, cx.theme()), |row, mark| {
                row.child(mark.test_support())
            })
            // Added before the table's own click listener, so it runs first. The table selects
            // its row on a plain click too, which then echoes this.
            .on_click(cx.listener(move |table, event: &ClickEvent, _, cx| {
                let modifiers = event.modifiers();
                let change = if modifiers.shift {
                    SelectionChange::ExtendTo(row_ix)
                } else if modifiers.secondary(cx) {
                    SelectionChange::Toggle(row_ix)
                } else {
                    SelectionChange::Click(row_ix)
                };
                change_selection(table, change, cx);
            }))
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<'_, TableState<Self>>,
    ) -> impl IntoElement {
        self.cell_element(row_ix, col_ix, cx.theme())
    }

    fn cell_label(&self, row_ix: usize, col_ix: usize, _: &App) -> Option<SharedString> {
        Some(cells::cell_text(
            self.rows.get(row_ix)?,
            self.shown_id(col_ix)?,
        ))
    }

    fn move_column(
        &mut self,
        col_ix: usize,
        to_ix: usize,
        _: &mut Window,
        _: &mut Context<'_, TableState<Self>>,
    ) {
        if !move_shown_column(&mut self.columns, col_ix, to_ix) {
            tracing::warn!(
                col_ix,
                to_ix,
                "the table moved a column the delegate does not have; their columns differ"
            );
        }
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<'_, TableState<Self>>,
    ) -> impl IntoElement {
        div()
            .id("table-empty")
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(cx.theme().muted_foreground)
            .when_some(empty_table_text(self.empty), |empty, text| {
                empty.child(text.clone()).aria_label(text)
            })
            .test_support()
    }
}

#[cfg(test)]
mod tests;
