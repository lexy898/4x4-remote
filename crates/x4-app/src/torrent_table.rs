//! The torrent table: the delegate behind gpui-component's [`DataTable`], which holds the rows of
//! the last snapshot, the columns as the user arranged them, the sort order and the selection.
//!
//! The rows come decided from `x4-core` ([`Row`]); the cells word them only when they are drawn,
//! so a snapshot formats no text for the rows off screen. The selection is kept by hash, so it
//! stays on the same torrent when a new snapshot re-sorts or filters the list, and a snapshot
//! never scrolls the view (`show_rows`).
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

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::table::{Column, TableDelegate, TableState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Context, Div, InteractiveElement as _, IntoElement, ParentElement as _, Pixels,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _,
    Window, div, px,
};
use x4_core::{
    ColumnId, ColumnState, EmptyTable, MAX_COLUMN_WIDTH, MIN_COLUMN_WIDTH, Row, SortDirection,
    SortKey, TableModel, move_shown_column, set_column_shown, set_shown_widths, shown_columns,
};
use x4_rpc::HashString;

use crate::i18n::{column_name_text, empty_table_text, sort_mark_text};
use crate::theme::DIMMED_OPACITY;

/// What a click on a column header does, given the column.
type SortHandler = Rc<dyn Fn(ColumnId, &mut Window, &mut App)>;

/// The delegate of the torrent table: the rows to show, the columns, the sort order the header
/// marks and the selected torrent.
pub struct TorrentTable {
    /// The rows in the order shown, shared with the main view's model.
    rows: Arc<[Row]>,
    /// Every column, shown or hidden, in the order the user arranged them.
    columns: Vec<ColumnState>,
    /// The selected torrent, which stays selected wherever a new snapshot puts it.
    selected: Option<HashString>,
    /// The sort order of the rows, which the header of its column marks.
    sort: (SortKey, SortDirection),
    /// What the table says when it has no rows.
    empty: EmptyTable,
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
            selected: None,
            sort,
            empty: EmptyTable::NoTorrents,
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

    /// Takes the rows of a new snapshot. The selection stays on the same torrent if it is still
    /// among them, wherever it is now, and is dropped otherwise.
    pub fn set_rows(&mut self, rows: Arc<[Row]>) {
        if let Some(selected) = &self.selected
            && !rows.iter().any(|row| &row.hash == selected)
        {
            self.selected = None;
        }
        self.rows = rows;
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

    /// The selected torrent, if any.
    #[must_use]
    pub fn selected(&self) -> Option<&HashString> {
        self.selected.as_ref()
    }

    /// The row the selected torrent is in now, if any.
    #[must_use]
    pub fn selected_index(&self) -> Option<usize> {
        let selected = self.selected.as_ref()?;
        self.rows.iter().position(|row| &row.hash == selected)
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

    /// Selects the torrent in the row at `row_ix`; an index past the rows selects nothing.
    pub(crate) fn select_row(&mut self, row_ix: usize) {
        self.selected = self.rows.get(row_ix).map(|row| row.hash.clone());
    }

    /// Selects nothing.
    pub(crate) fn clear_selected(&mut self) {
        self.selected = None;
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

/// The table's state around `delegate`: one selected row at a time, which does not wrap around
/// at either end, and none of the table's own sorting or column selection.
pub(crate) fn new_table_state(
    delegate: TorrentTable,
    window: &mut Window,
    cx: &mut Context<'_, TableState<TorrentTable>>,
) -> TableState<TorrentTable> {
    TableState::new(delegate, window, cx)
        .col_selectable(false)
        .sortable(false)
        .loop_selection(false)
}

/// Shows the rows of `model` in `table`. The table's selected row follows the selected torrent
/// to its new place, or is cleared when the torrent is gone, and the view stays where it is,
/// even when the torrent moved out of it.
pub(crate) fn show_rows(
    table: &mut TableState<TorrentTable>,
    model: &TableModel,
    cx: &mut Context<'_, TableState<TorrentTable>>,
) {
    let delegate = table.delegate_mut();
    delegate.set_rows(Arc::clone(&model.rows));
    delegate.empty = model.empty().unwrap_or(EmptyTable::NoTorrents);
    let selected = table.delegate().selected_index();
    if selected != table.selected_row() {
        match selected {
            Some(row_ix) => {
                // Selecting a row scrolls to it. A snapshot must not move the view, so that
                // scroll is taken back; one the user asked for earlier in this frame stays.
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
    cx.notify();
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
        let mark = (column.sort_key() == key).then(|| sort_mark_text(direction));
        let on_sort = self.on_sort.clone();
        cells::header(column, mark, move |window, cx| {
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
        _: &mut Context<'_, TableState<Self>>,
    ) -> Stateful<Div> {
        let dimmed = self.rows.get(row_ix).is_some_and(|row| row.dimmed);
        div()
            .id(("row", row_ix))
            .when(dimmed, |row| row.opacity(DIMMED_OPACITY))
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<'_, TableState<Self>>,
    ) -> impl IntoElement {
        match (self.rows.get(row_ix), self.shown_id(col_ix)) {
            (Some(row), Some(column)) => cells::cell(row, column, cx.theme()),
            _ => div().into_any_element(),
        }
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
            tracing::debug!(
                col_ix,
                to_ix,
                "the table moved a column the delegate does not have"
            );
        }
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<'_, TableState<Self>>,
    ) -> impl IntoElement {
        let text = empty_table_text(self.empty);
        div()
            .id("table-empty")
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(cx.theme().muted_foreground)
            .child(text.clone())
            .aria_label(text)
            .test_support()
    }
}

#[cfg(test)]
mod tests;
