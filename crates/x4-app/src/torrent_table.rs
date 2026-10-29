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
//! A click acts on the torrent the button went down on, as the row was drawn then: the press
//! notes its hash, and the click selects that hash. A snapshot can re-sort the rows while the
//! button is down, and the table hands the click over with the index of the row drawn under the
//! pointer when it comes up, which may be another torrent by then. A torrent that has left the
//! list by then is not clicked at all. A right-click notes its torrent the same way.
//!
//! A right-click selects as a file manager does (`right_click`): a row outside the selection
//! becomes the only selected row, and one inside leaves the selection as it is. The context menu
//! then opens for the selected rows (the `context_menu` module): the delegate builds it, enabled as
//! they allow while the daemon is connected or not (`TorrentTable::set_connected`), and hands a
//! choice to the handler set with `TorrentTable::on_menu`, with the rows the menu took when it
//! opened. A right-click that lands on no row opens no visible menu, but gpui-component's empty
//! one takes the keyboard focus; `watch_right_click` gives it back to where it was.
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

use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::table::{Column, TableDelegate, TableState};
use gpui_kit::component::{ActiveTheme as _, Theme, ThemeToken};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ClickEvent, Context, Div, Entity, FocusHandle, Focusable as _, InteractiveElement as _,
    IntoElement, MouseButton, MouseDownEvent, ParentElement as _, Pixels, ScrollStrategy,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, TestSupportExt as _,
    Window, div, px,
};
use x4_core::{
    ColumnId, ColumnState, Edge, EmptyTable, MAX_COLUMN_WIDTH, MIN_COLUMN_WIDTH, Row, Selection,
    SortDirection, SortKey, TableModel, move_shown_column, set_column_shown, set_shown_widths,
    shown_columns,
};
use x4_rpc::HashString;

use crate::context_menu::{MenuChoice, MenuHandler, torrent_menu};
use crate::i18n::{column_name_text, empty_table_text};
use crate::theme::row_opacity;

/// What a click on a column header does, given the column.
type SortHandler = Rc<dyn Fn(ColumnId, &mut Window, &mut App)>;

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

/// How far a right-click on the list has come, from the press to the context menu it opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RightClick {
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
    /// Whether the daemon is connected, which most items of the context menu need.
    connected: bool,
    /// The table's own focus handle, from which the context menu looks up the keys it shows.
    focus: Option<FocusHandle>,
    /// How far the right-click now under way has come, which tells a context menu built for a row
    /// from the empty one that no row gets.
    right_click: RightClick,
    /// The torrent the left button last went down on, as its row was drawn then: what the click
    /// that follows acts on.
    pressed: Option<HashString>,
    /// The torrent the right button last went down on, as its row was drawn then: what the
    /// right-click acts on.
    right_pressed: Option<HashString>,
    /// The row of the plain click just handled, which the table selects by itself after us; the
    /// next `follow_table` takes that as an echo of the click, not as a click of its own.
    click_echo: Option<usize>,
    /// Called with the column whose header is clicked.
    on_sort: Option<SortHandler>,
    /// Called with the item chosen in the context menu.
    on_menu: Option<MenuHandler>,
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
            connected: false,
            focus: None,
            right_click: RightClick::Idle,
            pressed: None,
            right_pressed: None,
            click_echo: None,
            on_sort: None,
            on_menu: None,
        }
    }

    /// Calls `handler` with the column whose header the user clicks. The table does not sort by
    /// itself: the handler sorts the rows and hands them back through a new model.
    #[must_use]
    pub fn on_sort(mut self, handler: impl Fn(ColumnId, &mut Window, &mut App) + 'static) -> Self {
        self.on_sort = Some(Rc::new(handler));
        self
    }

    /// Calls `handler` with the item the user chooses in the context menu, and the rows the menu
    /// took when it opened: the selected rows the list showed then, in the order shown.
    #[must_use]
    pub(crate) fn on_menu(
        mut self,
        handler: impl Fn(MenuChoice, &[Row], &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_menu = Some(Rc::new(handler));
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

    /// Takes whether the daemon is connected, for the context menus opened from now on.
    pub(crate) fn set_connected(&mut self, connected: bool) {
        self.connected = connected;
    }

    /// The index of the torrent `hash` among the rows shown.
    fn index_of(&self, hash: &HashString) -> Option<usize> {
        self.rows.iter().position(|row| &row.hash == hash)
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
    let mut table = TableState::new(delegate, window, cx)
        .col_selectable(false)
        .sortable(false)
        .loop_selection(false)
        .select_on_modifier_click(false);
    // The context menu is built while the table is borrowed, so it cannot ask the table then.
    let focus = table.focus_handle(cx);
    table.delegate_mut().focus = Some(focus);
    table
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
fn click_row(
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
        let row = div()
            .id(("row", row_ix))
            .when_some(self.selection_mark(row_ix, cx.theme()), |row, mark| {
                row.child(mark.test_support())
            });
        // The rows that fill the space below the last torrent take no clicks.
        let Some(hash) = self.rows.get(row_ix).map(|row| row.hash.clone()) else {
            return row;
        };
        let right_hash = hash.clone();
        row
            // The torrent this frame draws here, which a press on the row is meant for.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |table, _: &MouseDownEvent, _, _| {
                    table.delegate_mut().pressed = Some(hash.clone());
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |table, _: &MouseDownEvent, _, _| {
                    table.delegate_mut().right_pressed = Some(right_hash.clone());
                }),
            )
            // Added before the table's own click listener, so it runs first. The table selects
            // its row on a plain click too, which `follow_table` takes as an echo of this.
            .on_click(cx.listener(move |table, event: &ClickEvent, _, cx| {
                click_row(table, row_ix, event, cx);
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

    /// The menu of a right-click, built once as it opens, after `right_click` has selected the
    /// row: it takes the selected rows the list shows now, and acts on them however the list
    /// changes while it is open. A right-click that did not land on a row (below the rows, on the
    /// header) gets an empty menu, which gpui-component does not show; `watch_right_click` gives
    /// the focus back, because this menu is no menu to wait for.
    fn context_menu(
        &mut self,
        _: usize,
        menu: PopupMenu,
        _: &mut Window,
        _: &mut Context<'_, TableState<Self>>,
    ) -> PopupMenu {
        if self.right_click != RightClick::OnRow {
            return menu;
        }
        self.right_click = RightClick::Idle;
        let targets: Rc<[Row]> = self
            .selection
            .targets(&self.rows)
            .into_iter()
            .cloned()
            .collect();
        torrent_menu(
            menu,
            &targets,
            self.connected,
            self.focus.clone(),
            self.on_menu.as_ref(),
        )
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
