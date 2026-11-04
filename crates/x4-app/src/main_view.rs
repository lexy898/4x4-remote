//! The content of the main window: a toolbar with the search box (the `toolbar` module), the
//! filter sidebar, the torrent table and the status bar, fed by the events of the service.
//!
//! The view pumps the events channel on the UI thread's executor and never waits on the network
//! (the `events` module): awaiting the channel yields until the service sends something, and
//! sending a command never blocks. Each batch of queued events is cut down to its newest snapshot,
//! which `x4-core` turns into the [`TableModel`] and the [`StatusModel`]; the view redraws only
//! when one of them changed, so an idle daemon costs no frames.
//!
//! The torrent table ([`TorrentTable`]) shows the rows; the view owns what they are, as its
//! [`ViewState`]: a click on a filter in the sidebar, an edit in the search box and a click on a
//! header (through [`ViewState::toggle_sort`]) change it, and the view recomputes the rows from the
//! last snapshot and hands them to the table, without asking the daemon. It also keeps what the
//! user does to the columns (shown, moved, resized) in the [`UiState`], and the toolbar's column
//! menu shows and hides them. The selection of several torrents stays in the table, by hash; the
//! view handles the selection keys of [`keymap`](crate::keymap) (the `selection_keys` module:
//! Ctrl+A, Ctrl+Shift+A, Escape, the arrows, Home, End, Page Up and Page Down, with and without
//! Shift) and hands them to it, and passes on what the table does to its own selected row and a
//! right-click on a row.
//!
//! Under the table, the details panel (the `details` module) shows the selected torrent, when
//! exactly one is, and the view asks the service to read its details while the panel is open.
//! Alt+Enter shows and hides the panel; it shares a split with the table, whose handle sets its
//! height. What the panel's Files tab changes goes out as an action too (the `file_actions`
//! module).
//!
//! The selected torrents can be started, paused, started now, verified and reannounced, from their
//! keys, from the toolbar's Start and Pause and from the list's context menu (the
//! `torrent_actions` module). Each action goes out as a command and is kept until the service
//! answers it; a failure is shown as a notification. A held key sends its action once. Delete,
//! Shift+Delete, the toolbar's Remove menu and the context menu open a dialog that asks before the
//! torrents are removed, with or without their files. Their hashes and magnet links can be copied
//! (the `copying` module). The `menu` module carries out the items of the context menu.
//!
//! Torrents to add come as requests from their sources and wait in a queue; the add dialog shows
//! them one at a time, and nothing is added without the user's Add (the `adding` module). The
//! sources inside the window are the toolbar's Add file and Add magnet, Ctrl+O, Ctrl+U, Ctrl+V in
//! the list, the file dialog, the clipboard, and `.torrent` files dropped anywhere on the window,
//! which is highlighted while they hover.
//!
//! The status bar's turtle switches the daemon's alternative speed limits, and its limits button
//! opens a popover that sets the normal ones (the `speed_controls` module); Ctrl+T and Ctrl+L do
//! the same from the keyboard.
//!
//! The view also tells the service whether the user can see the window, so that it polls slowly
//! while nobody looks, and it holds the theme choice of the toolbar's switch.
//!
//! What the user changes in the [`UiState`] (the columns, the sort order, the filter, the theme,
//! the window size and the details panel) is saved a second after the last change, and when the
//! window closes or the app quits (the `persistence` module).

mod adding;
mod copying;
mod details;
mod events;
mod file_actions;
mod menu;
mod persistence;
mod selection_keys;
mod speed_controls;
mod torrent_actions;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use crossbeam_channel::Sender;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::resizable::ResizableState;
use gpui_kit::component::table::{DataTable, TableEvent, TableState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, Size, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, ExternalPaths, Focusable as _, InteractiveElement as _,
    IntoElement, MouseDownEvent, ParentElement as _, Pixels, Render, SharedString,
    StatefulInteractiveElement as _, Styled, Subscription, Task, TestSupportExt as _, Window, div,
    px,
};
use x4_core::service::ServiceHandle;
use x4_core::{
    Availability, ColumnId, Command, DetailsTab, Event, Filter, ServiceStatus, Snapshot, SortState,
    StatusModel, TableModel, UiState, ViewState, Visibility, shown_columns, status_model,
    table_model, visibility,
};
use x4_rpc::HashString;

use crate::actions::PendingActions;
use crate::adding::{AddRequests, drop_highlight_layer, hover_while_dragging};
use crate::app::ConfigProblem;
use crate::clipboard::MagnetCopy;
use crate::details_panel::DetailsPanel;
use crate::i18n::{connection_text, torrent_count_text};
use crate::key_repeats::KeyRepeats;
use crate::keymap::{
    ClearSearch, FocusNext, FocusPrevious, FocusSearch, MAIN_CONTEXT, TORRENT_LIST_CONTEXT,
};
use crate::search::{clear_search, focus_search, new_search_state};
use crate::sidebar::filter_sidebar;
use crate::speed_limits::SpeedLimits;
use crate::status_bar::{LimitsPopover, status_bar};
use crate::theme::{apply_theme, follow_appearance, next_theme};
use crate::toolbar::{ToolbarState, toolbar};
use crate::torrent_table::{
    TorrentTable, follow_table, new_table_state, right_click, show_column, show_rows,
    watch_right_click,
};
pub(crate) use adding::AddButton;
use adding::on_add_actions;
use details::on_details_actions;
use persistence::Saver;
use selection_keys::on_selection_keys;
use speed_controls::on_speed_actions;
use torrent_actions::{follow_key_repeats, on_torrent_actions};

/// The height of a row of the table, for the two lines of the name cell.
const ROW_HEIGHT: f32 = 44.;
/// The height of the table's header.
const HEADER_HEIGHT: f32 = 32.;
/// The id of the layer that highlights the window while files are dragged over it.
const WINDOW_DROP_TARGET: &str = "window-drop-target";

/// The main window's content.
pub struct MainView {
    /// What the status bar shows.
    status: StatusModel,
    /// The filter, the search and the sort order the table follows.
    view: ViewState,
    /// How many times the user has changed `view` ([`MainView::set_view`]); showing an added
    /// torrent changes it without counting.
    view_changes: u64,
    /// The rows of the table and the counts of the sidebar.
    model: TableModel,
    /// The torrent table, whose delegate holds the rows of `model`, the columns and the
    /// selection.
    table: Entity<TableState<TorrentTable>>,
    /// The text of the search box, which `view.search` follows.
    search: Entity<InputState>,
    /// The newest snapshot, so that a change of `view` shows at once, without waiting for the
    /// next one.
    last_snapshot: Option<Arc<Snapshot>>,
    /// The state to remember between runs, as the user has changed it.
    ui: UiState,
    /// Saves `ui` when it changes.
    saver: Saver,
    /// Why `config.toml` is not used, until the user dismisses it.
    config_problem: Option<ConfigProblem>,
    /// Where commands for the service go.
    commands: Sender<Command>,
    /// The visibility last sent to the service; `None` before the first.
    last_visibility: Option<Visibility>,
    /// Which actions the selected torrents allow, as the toolbar shows them; kept up to date with
    /// the table and the connection.
    availability: Availability,
    /// The actions sent and not answered yet, and the ids given out.
    pending: PendingActions,
    /// The read of magnet links that waits for its answer, to copy them.
    magnet: MagnetCopy,
    /// What the newest failure of an action said.
    last_failure: Option<SharedString>,
    /// The key held down now, so that its repeats do not run its action again; shared with the
    /// app's keystroke interceptor, which sees the key-downs before the bindings do.
    key_repeats: Rc<RefCell<KeyRepeats>>,
    /// The form of the speed limits popover.
    speed_limits: Entity<SpeedLimits>,
    /// Whether the speed limits popover is open.
    limits_open: bool,
    /// The details panel under the table.
    details_panel: Entity<DetailsPanel>,
    /// The split of the table and the details panel, with their heights.
    details_split: Entity<ResizableState>,
    /// The torrent whose details the service was last asked to read; `None` for none, which is
    /// also what the service starts with.
    selected_details: Option<HashString>,
    /// The tab of the panel the service was last told is open; the default tab, General, is
    /// also what the service starts with.
    shown_tab: DetailsTab,
    /// The height of the split when the panel was last fitted to it; `None` before the first fit.
    fitted_split: Option<Pixels>,
    /// The torrents to add: the queue, the add dialog and the adds sent.
    adds: adding::Adds,
    /// The event pump; dropping the view drops it, which cancels it.
    _events: Task<()>,
    /// Kept so that closing the window stops the service thread.
    _service: Option<ServiceHandle>,
    /// The table's, the search box's, the speed limits form's and the details panel's events, the
    /// table's and the split's changes, the focus coming back to the table, leaving the speed
    /// limits and getting lost, the window's activation, visibility, appearance and bounds
    /// observers, the save when the app quits, and the keystroke interceptor.
    _subscriptions: Vec<Subscription>,
}

impl MainView {
    /// The view of the service whose channels are `events` and `commands`, starting from the
    /// saved state `ui` (the columns, the filter, the sort order and the theme). The theme itself
    /// is applied by whoever opens the window, before the first frame, and so is the window size.
    /// The table has the focus from the start.
    ///
    /// `service` is the handle of the thread behind the channels: the view keeps it, so that
    /// closing the window stops the thread. Without one (in tests), the channels are all there is.
    ///
    /// The view saves its state to `state_file` when the user changes it, and before the window
    /// closes or the app quits; with `None` it never saves.
    pub fn new(
        events: async_channel::Receiver<Event>,
        commands: Sender<Command>,
        service: Option<ServiceHandle>,
        ui: UiState,
        state_file: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let view_state = ui.view_state();
        let table = Self::new_table(&ui, &view_state, window, cx);
        // The window's own bindings work only while something in it has the focus.
        window.focus(&table.focus_handle(cx), cx);
        let search = cx.new(|cx| new_search_state(window, cx));
        let key_repeats = Rc::new(RefCell::new(KeyRepeats::default()));
        let speed_limits = cx.new(|cx| SpeedLimits::new(Rc::clone(&key_repeats), window, cx));
        let limits_focus = speed_limits.focus_handle(cx);
        let details_panel =
            cx.new(|cx| DetailsPanel::new(ui.details.tab, Rc::clone(&key_repeats), window, cx));
        let details_split = cx.new(|_| ResizableState::default());
        let mut subscriptions = vec![
            cx.subscribe_in(&table, window, Self::on_table_event),
            // The selection and the rows live in the table; the toolbar follows what they allow.
            cx.observe(&table, |view, _, cx| view.follow_table_change(cx)),
            // A key let go while the focus was elsewhere, outside the view or in another window,
            // sent the view no key-up: forget it when the list has the focus again.
            cx.on_focus_in(&table.focus_handle(cx), window, |view, _, _| {
                view.key_repeats.borrow_mut().release();
            }),
            cx.subscribe_in(&search, window, Self::on_search_event),
            cx.subscribe_in(&speed_limits, window, Self::on_limits_applied),
            cx.subscribe_in(&details_panel, window, Self::on_details_event),
            // The window gave the split another height, or the user drags its handle.
            cx.observe_in(&details_split, window, |view, _, window, cx| {
                view.follow_split(window, cx);
            }),
            // What had the focus is gone, as a copy button of the details panel when the panel
            // hides or shows another torrent. The window's keys work only while something in it
            // has the focus, so the table takes it.
            cx.on_focus_lost(window, |view, window, cx| {
                window.focus(&view.table.focus_handle(cx), cx);
            }),
            // Tab, Shift+Tab and Ctrl+F take the focus out of the popover, which then closes. An
            // inactive window has no focus either, so a switch to another app is no reason to
            // close it: what was typed is still there when the user comes back.
            cx.on_focus_out(&limits_focus, window, |view, _, window, cx| {
                if window.is_window_active() {
                    view.limits_lost_focus(cx);
                }
            }),
            cx.observe_window_activation(window, |view, window, cx| {
                view.report_visibility(window, cx);
            }),
            cx.observe_window_visibility(window, |view, _, window, cx| {
                view.report_visibility(window, cx);
            }),
            cx.observe_window_appearance(window, |view, window, cx| {
                follow_appearance(view.ui.theme, window, cx);
            }),
            cx.observe_window_bounds(window, |view, window, cx| {
                view.window_changed(window, cx);
            }),
            // Quitting (Ctrl+Q) closes no window first, so the close below does not see it.
            cx.on_app_quit(|view, cx| {
                view.save_before_close(cx);
                std::future::ready(())
            }),
        ];
        subscriptions.push(follow_key_repeats(&key_repeats, window, cx));
        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |_, cx| {
            if let Err(error) = this.update(cx, Self::save_before_close) {
                tracing::debug!(%error, "the window closes after its view was released");
            }
            true
        });
        let pump = cx.spawn_in(window, async move |view, cx| {
            events::pump(events, view, cx).await;
        });
        let saver = Saver::new(&ui, state_file);
        let mut view = Self {
            status: StatusModel::default(),
            view: view_state,
            view_changes: 0,
            model: TableModel::default(),
            table,
            search,
            last_snapshot: None,
            ui,
            saver,
            config_problem: None,
            commands,
            last_visibility: None,
            availability: Availability::default(),
            pending: PendingActions::default(),
            magnet: MagnetCopy::default(),
            last_failure: None,
            key_repeats,
            speed_limits,
            limits_open: false,
            details_panel,
            details_split,
            selected_details: None,
            shown_tab: DetailsTab::default(),
            fitted_split: None,
            adds: adding::Adds::default(),
            _events: pump,
            _service: service,
            _subscriptions: subscriptions,
        };
        // The service and the details panel start out assuming a visible window on the General
        // tab; tell them how the window opened.
        view.report_visibility(window, cx);
        view.ask_for_tab();
        view.listen_for_adds(AddRequests::new(), window, cx);
        view
    }

    /// The table of the main window, with its rows, columns and selection: the delegate
    /// ([`TorrentTable`]) and the state of gpui-component's table around it.
    #[must_use]
    pub fn table(&self) -> &Entity<TableState<TorrentTable>> {
        &self.table
    }

    /// Makes the Files tab's trees of big torrents on GPUI's background executor from now on,
    /// instead of on a thread of their own each. For tests: the executor of a test makes each tree
    /// in its turn with every other task, so a test sees the tree as soon as it runs the tasks; a
    /// thread of its own ends at a time of its own, and wakes the window from outside the test's
    /// executor.
    pub fn make_trees_on_the_executor(&mut self, cx: &mut Context<'_, Self>) {
        self.details_panel
            .update(cx, |panel, _| panel.make_trees_on_the_executor());
    }

    /// What the status bar shows: the speeds, the free space, the connection and whether the
    /// data is stale.
    #[must_use]
    pub fn status(&self) -> &StatusModel {
        &self.status
    }

    /// The rows the table shows and the counts of the sidebar.
    #[must_use]
    pub fn model(&self) -> &TableModel {
        &self.model
    }

    /// The filter, the search and the sort order the table follows.
    #[must_use]
    pub fn view_state(&self) -> &ViewState {
        &self.view
    }

    /// What the status bar says about the connection; empty while connected, when the bar shows
    /// only the dot.
    #[must_use]
    pub fn connection_text(&self) -> SharedString {
        connection_text(&self.status.indicator())
    }

    /// The state to remember between runs, with the user's changes, such as the theme.
    #[must_use]
    pub fn ui_state(&self) -> &UiState {
        &self.ui
    }

    /// How many times the view has started to save its state since it opened, failed saves
    /// included. Each save writes the whole state once.
    #[must_use]
    pub fn state_saves(&self) -> u64 {
        self.saver.writes()
    }

    /// Shows the torrents `view` asks for (the filter, the search and the sort order), from the
    /// last snapshot and without waiting for the next one, and remembers its filter and sort
    /// order in the [`UiState`]. The search is not remembered, so an edit of it saves nothing.
    /// It is the user's choice: a change counts among the user's choices in the list, which an
    /// added torrent waiting to be selected gives way to.
    pub fn set_view(&mut self, view: ViewState, cx: &mut Context<'_, Self>) {
        if view == self.view {
            return;
        }
        self.view_changes = self.view_changes.wrapping_add(1);
        self.show_view(view, cx);
    }

    /// [`MainView::set_view`] for the window's own reasons, as to show an added torrent: not
    /// counted as the user's choice.
    fn show_view(&mut self, view: ViewState, cx: &mut Context<'_, Self>) {
        if view == self.view {
            return;
        }
        self.ui.filter = view.filter;
        self.ui.sort = SortState {
            key: view.sort_key,
            direction: view.sort_direction,
        };
        self.ui_changed(cx);
        let sort = (view.sort_key, view.sort_direction);
        self.view = view;
        // The header marks the order even when the rows stay as they were.
        self.table.update(cx, |table, cx| {
            table.delegate_mut().set_sort(sort);
            cx.notify();
        });
        self.update_table(cx);
        cx.notify();
    }

    /// Shows the torrents `filter` matches; a click on it in the sidebar calls this.
    fn select_filter(&mut self, filter: Filter, cx: &mut Context<'_, Self>) {
        tracing::debug!(?filter, "filter selected");
        self.set_view(
            ViewState {
                filter,
                ..self.view.clone()
            },
            cx,
        );
    }

    /// Follows the text of the search box into the view, at every edit.
    fn on_search_event(
        &mut self,
        search: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match event {
            InputEvent::Change => {
                let query = search.read(cx).value().to_string();
                self.set_view(
                    ViewState {
                        search: query,
                        ..self.view.clone()
                    },
                    cx,
                );
            }
            InputEvent::PressEnter { .. } | InputEvent::Focus | InputEvent::Blur => {}
        }
    }

    /// Handles [`ClearSearch`]: empties the search box, which shows every torrent the filter
    /// matches again, and hands the focus back to the table.
    fn on_clear_search(
        &mut self,
        _: &ClearSearch,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        clear_search(&self.search, window, cx);
        window.focus(&self.table.focus_handle(cx), cx);
    }

    /// Shows or hides `column` in the table, and remembers it. The name column stays.
    pub(crate) fn toggle_column(&mut self, column: ColumnId, cx: &mut Context<'_, Self>) {
        let changed = self.table.update(cx, |table, cx| {
            let shown = shown_columns(table.delegate().columns()).any(|shown| shown.id == column);
            show_column(table, column, !shown, cx)
        });
        if changed {
            tracing::debug!(?column, "column shown or hidden");
            self.remember_columns(cx);
            cx.notify();
        }
    }

    /// The table of the main window for the saved state `ui`, sorted as `view` says. A click on
    /// a header sorts through the view, and an item of the context menu goes to the view.
    fn new_table(
        ui: &UiState,
        view: &ViewState,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Entity<TableState<TorrentTable>> {
        let this = cx.weak_entity();
        let menu_view = cx.weak_entity();
        let delegate = TorrentTable::new(ui.columns.clone(), (view.sort_key, view.sort_direction))
            .on_sort(move |column, _, cx| {
                if let Err(error) = this.update(cx, |view, cx| view.sort_by(column, cx)) {
                    tracing::debug!(%error, "a header was clicked after the window closed");
                }
            })
            .on_menu(move |choice, targets, window, cx| {
                if let Err(error) = menu_view.update(cx, |view, cx| {
                    view.choose_from_menu(choice, targets, window, cx);
                }) {
                    tracing::debug!(%error, "a menu item was chosen after the window closed");
                }
            });
        cx.new(|cx| new_table_state(delegate, window, cx))
    }

    /// Handles a click on the header of `column`: sorts by it, or flips the direction if the
    /// list is sorted by it already.
    fn sort_by(&mut self, column: ColumnId, cx: &mut Context<'_, Self>) {
        let mut view = self.view.clone();
        view.toggle_sort(column);
        tracing::debug!(
            key = ?view.sort_key,
            direction = ?view.sort_direction,
            "sorted by a column"
        );
        self.set_view(view, cx);
    }

    /// Follows what the user does in the table: what it does to its own selected row (taken over
    /// by the selection, unless it only echoes the selection), a right-click on a row (which
    /// selects it, before its context menu opens), and the columns moved or resized, which are
    /// remembered.
    fn on_table_event(
        &mut self,
        table: &Entity<TableState<TorrentTable>>,
        event: &TableEvent,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match event {
            TableEvent::SelectRow(_) | TableEvent::ClearSelection => {
                table.update(cx, follow_table);
            }
            TableEvent::ColumnWidthsChanged(widths) => {
                if table.update(cx, |table, _| table.delegate_mut().set_widths(widths)) {
                    self.remember_columns(cx);
                }
            }
            // The delegate has moved the column already, before the table emits this.
            TableEvent::MoveColumn(..) => self.remember_columns(cx),
            // Emitted before the context menu is built, which then sees the new selection.
            // It names the row of the frame drawn last, as the table knows it; the delegate knows
            // the torrent drawn there, which is what was right-clicked.
            TableEvent::RightClickedRow(Some(_)) => table.update(cx, right_click),
            // The table emits this whenever its own row moves, the mirror of the lead included,
            // and not when a menu closes; the open menu holds its targets anyway.
            TableEvent::RightClickedRow(None)
            | TableEvent::DoubleClickedRow(_)
            | TableEvent::SelectColumn(_)
            | TableEvent::SelectCell(..)
            | TableEvent::DoubleClickedCell(..)
            | TableEvent::RightClickedCell(..) => {}
        }
    }

    /// Copies the table's columns into the state to remember between runs.
    fn remember_columns(&mut self, cx: &mut Context<'_, Self>) {
        self.ui.columns = self.table.read(cx).delegate().columns().to_vec();
        self.ui_changed(cx);
    }

    /// Whether each column is shown, in the order of [`ColumnId::ALL`], as the column menu lists
    /// them.
    fn column_choices(&self, cx: &App) -> [(ColumnId, bool); ColumnId::ALL.len()] {
        let columns = self.table.read(cx).delegate().columns();
        ColumnId::ALL.map(|id| (id, shown_columns(columns).any(|column| column.id == id)))
    }

    /// Shows that the service could not be started, so no data will come.
    pub(crate) fn service_failed(&mut self, cx: &mut Context<'_, Self>) {
        self.status.service_failed();
        self.show_status(cx);
        self.refresh_availability(cx);
        cx.notify();
    }

    /// Shows that `config.toml` is not used and why, until the user dismisses it.
    pub(crate) fn show_config_problem(
        &mut self,
        problem: ConfigProblem,
        cx: &mut Context<'_, Self>,
    ) {
        self.config_problem = Some(problem);
        cx.notify();
    }

    /// Recomputes the table, the status bar and the actions the selection allows for `snapshot`
    /// and keeps it; returns whether any of them changed.
    fn show(&mut self, snapshot: Arc<Snapshot>, cx: &mut Context<'_, Self>) -> bool {
        let status = status_model(&snapshot);
        self.last_snapshot = Some(snapshot);
        let mut changed = self.update_table(cx);
        if status != self.status {
            self.status = status;
            self.show_status(cx);
            changed = true;
        }
        changed |= self.refresh_availability(cx);
        // The torrent the panel shows may have changed when its row did not.
        self.follow_details(cx);
        changed
    }

    /// Tells the table whether its rows are stale data, which it draws at the stale opacity, and
    /// whether the daemon is connected, which its context menu needs for most of its items; the
    /// details panel whether its Files tab can act and whether its peers are stale; and the speed
    /// limits popover whether it can apply.
    fn show_status(&self, cx: &mut Context<'_, Self>) {
        let stale = self.status.stale;
        let connected = self.status.connected();
        self.table.update(cx, |table, cx| {
            let delegate = table.delegate_mut();
            delegate.set_connected(connected);
            if delegate.set_stale(stale) {
                cx.notify();
            }
        });
        self.details_panel.update(cx, |panel, cx| {
            panel.set_connected(connected, cx);
            panel.set_stale(stale, cx);
        });
        self.show_speed_status(cx);
    }

    /// Recomputes the table from the last snapshot for the current view and hands the rows to
    /// the table; returns whether they changed.
    fn update_table(&mut self, cx: &mut Context<'_, Self>) -> bool {
        let Some(snapshot) = &self.last_snapshot else {
            return false;
        };
        let model = table_model(snapshot, &self.view);
        if model == self.model {
            return false;
        }
        self.model = model;
        let model = &self.model;
        self.table
            .update(cx, |table, cx| show_rows(table, model, cx));
        true
    }

    /// The events channel closed: the service thread stopped or panicked, so the data on screen
    /// will not change again. It stays visible but is marked stale.
    fn service_stopped(&mut self, cx: &mut Context<'_, Self>) {
        // A service that never started has said so already.
        if self.status.service == ServiceStatus::Running {
            tracing::error!("the service stopped; the window shows the last data it sent");
        }
        self.status.service_stopped();
        self.show_status(cx);
        self.refresh_availability(cx);
        cx.notify();
    }

    /// Tells the service and the details panel whether the user can see the window, when that
    /// changed.
    fn report_visibility(&mut self, window: &Window, cx: &mut Context<'_, Self>) {
        let visibility = visibility(window.is_window_active(), window.is_visible());
        if self.last_visibility == Some(visibility) {
            return;
        }
        self.last_visibility = Some(visibility);
        tracing::debug!(?visibility, "window visibility changed");
        self.details_panel.update(cx, |panel, cx| {
            panel.set_window_shown(visibility == Visibility::Visible, cx);
        });
        if self
            .commands
            .send(Command::SetVisibility(visibility))
            .is_err()
        {
            tracing::debug!("the service has stopped; the visibility was not sent");
        }
    }

    /// Moves the theme switch on to the next choice and shows it.
    pub(crate) fn switch_theme(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.ui.theme = next_theme(self.ui.theme);
        tracing::debug!(theme = ?self.ui.theme, "theme switched");
        apply_theme(self.ui.theme, window, cx);
        self.ui_changed(cx);
        cx.notify();
    }

    fn dismiss_config_problem(&mut self, cx: &mut Context<'_, Self>) {
        self.config_problem = None;
        cx.notify();
    }

    /// The torrent table, whose rows are dimmed while the data is stale (the table does that, so
    /// that a row is never dimmed twice). Its accessible name says how many torrents it shows.
    /// Its style is open to the tests, which check that the list itself is never dimmed.
    fn torrent_list(&self) -> impl IntoElement + Styled + use<> {
        let table = self.table.clone();
        div()
            .id("torrent-list")
            .key_context(TORRENT_LIST_CONTEXT)
            // In the capture phase, before the context menu stops the press (`watch_right_click`).
            .capture_any_mouse_down(move |event: &MouseDownEvent, window, cx| {
                watch_right_click(&table, event, window, cx);
            })
            .flex_1()
            .min_w_0()
            .h_full()
            .text_sm()
            .child(
                DataTable::new(&self.table)
                    .with_size(Size::Size(px(ROW_HEIGHT)))
                    .header_row_height(px(HEADER_HEIGHT))
                    .bordered(false),
            )
            .aria_label(torrent_count_text(self.model.rows.len()))
            .test_support()
    }
}

impl Render for MainView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let toolbar = toolbar(
            &self.search,
            ToolbarState {
                theme: self.ui.theme,
                columns: self.column_choices(cx),
                availability: self.availability,
            },
            cx,
        );
        let table = self.torrent_list();
        let this = cx.weak_entity();
        let theme = cx.theme();
        let counts = self.model.shown_counts();
        let sidebar = filter_sidebar(
            counts.as_ref(),
            self.status.stale,
            self.view.filter,
            theme,
            move |filter, _, cx| {
                if let Err(error) = this.update(cx, |view, cx| view.select_filter(filter, cx)) {
                    tracing::debug!(%error, "a filter was clicked after the window closed");
                }
            },
        );
        let on_dismiss = cx.listener(|view, _, _, cx| view.dismiss_config_problem(cx));
        let status_bar = status_bar(
            &self.status,
            self.config_problem,
            self.saver.failing(),
            &LimitsPopover {
                form: &self.speed_limits,
                open: self.limits_open,
            },
            on_dismiss,
            cx,
        );
        let repeats = &self.key_repeats;
        let root = on_add_actions(
            on_details_actions(
                on_speed_actions(
                    on_selection_keys(on_torrent_actions(v_flex(), repeats, cx), cx),
                    repeats,
                    cx,
                ),
                repeats,
                cx,
            ),
            repeats,
            cx,
        );
        let content = self.list_and_details(table, cx);
        root.key_context(MAIN_CONTEXT)
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .on_action(cx.listener(|_, _: &FocusNext, window, cx| window.focus_next(cx)))
            .on_action(cx.listener(|_, _: &FocusPrevious, window, cx| window.focus_prev(cx)))
            .on_action(cx.listener(|view, _: &FocusSearch, window, cx| {
                focus_search(&view.search, window, cx);
            }))
            .on_action(cx.listener(Self::on_clear_search))
            // Over the table, a row, the search box or the status bar alike; and over the add
            // dialog too, whose own drop needs the drag to count as the mouse just as well.
            .on_drag_move::<ExternalPaths>(hover_while_dragging)
            .on_drop(cx.listener(|view, paths: &ExternalPaths, window, cx| {
                view.files_dropped(paths.paths(), window, cx);
            }))
            .child(toolbar)
            .child(h_flex().flex_1().min_h_0().child(sidebar).child(content))
            .child(status_bar)
            // Last, so that it is painted over the content (`drop_target`).
            .child(drop_highlight_layer(WINDOW_DROP_TARGET, px(0.)))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui_kit::{Entity, TestAppContext};

    use super::*;
    use crate::app;

    #[gpui_kit::test]
    fn dismissing_the_config_problem_redraws(cx: &mut TestAppContext) {
        let (_events, events_rx) = async_channel::unbounded();
        let (commands, _commands_rx) = crossbeam_channel::unbounded();
        let (_, view): (_, Entity<MainView>) = cx
            .update(|cx| {
                app::init(cx);
                app::open_main_window(events_rx, commands, None, &UiState::default(), cx)
            })
            .expect("the window opens");
        cx.update(|cx| {
            view.update(cx, |view, cx| {
                view.show_config_problem(ConfigProblem::Unreadable, cx);
            });
        });
        let redraws = Rc::new(Cell::new(0));
        let counter = Rc::clone(&redraws);
        let _subscription =
            cx.update(|cx| cx.observe(&view, move |_, _| counter.set(counter.get() + 1)));

        cx.update(|cx| view.update(cx, MainView::dismiss_config_problem));

        assert_eq!(cx.update(|cx| view.read(cx).config_problem), None);
        assert_eq!(redraws.get(), 1);
    }

    #[gpui_kit::test]
    fn the_details_panel_knows_whether_the_window_is_shown(cx: &mut TestAppContext) {
        let (_events, events_rx) = async_channel::unbounded();
        let (commands, _commands_rx) = crossbeam_channel::unbounded();
        let (handle, view): (_, Entity<MainView>) = cx
            .update(|cx| {
                app::init(cx);
                app::open_main_window(events_rx, commands, None, &UiState::default(), cx)
            })
            .expect("the window opens");
        let shown = |cx: &mut TestAppContext| {
            cx.update(|cx| view.read(cx).details_panel.read(cx).window_shown())
        };
        // A test window opens without the focus, which the service counts as hidden.
        cx.run_until_parked();
        assert!(!shown(cx), "as it opened");

        cx.update_window(handle, |_, window, _| window.activate_window())
            .expect("the window is open");
        cx.run_until_parked();
        assert!(shown(cx), "active and shown");

        cx.simulate_window_visibility_change(handle, gpui_kit::WindowVisibility::Hidden);
        cx.run_until_parked();
        assert!(!shown(cx), "hidden");
    }

    #[gpui_kit::test]
    fn stale_data_dims_the_rows_of_the_peers_and_the_trackers_tab(cx: &mut TestAppContext) {
        use x4_core::test_support::{details, torrent, tracker};
        use x4_core::{
            ConnectionError, ConnectionState, Peer, PeerList, PeerSources, PeerTraits,
            TorrentDetails,
        };

        use crate::theme::STALE_OPACITY;
        use crate::torrent_table::select_added;

        let (_events, events_rx) = async_channel::unbounded();
        let (commands, _commands_rx) = crossbeam_channel::unbounded();
        let (_, view): (_, Entity<MainView>) = cx
            .update(|cx| {
                app::init(cx);
                app::open_main_window(events_rx, commands, None, &UiState::default(), cx)
            })
            .expect("the window opens");
        let panel = cx.update(|cx| view.read(cx).details_panel.clone());
        let film = torrent("film");
        let snapshot = |connection: ConnectionState, stale: bool| {
            Arc::new(Snapshot {
                torrents: vec![film.clone()],
                listed: true,
                session: None,
                free_space_bytes: None,
                rate_down: 0,
                rate_up: 0,
                connection,
                stale,
            })
        };
        let peer = |address: &str| Peer {
            ip: address.parse().ok(),
            address: address.to_owned(),
            port: 6881,
            client: String::new(),
            progress: 0.0,
            rate_down: 0,
            rate_up: 0,
            traits: PeerTraits::default(),
            flags: String::new(),
            country: None,
        };
        // The panel shows the one torrent selected, on the Peers tab.
        cx.update(|cx| {
            view.update(cx, |view, cx| {
                view.show(snapshot(ConnectionState::Connected, false), cx);
                view.table
                    .update(cx, |table, cx| select_added(table, film.hash.clone(), cx));
            });
        });
        cx.update(|cx| {
            panel.update(cx, |panel, cx| panel.set_tab(DetailsTab::Peers, cx));
            view.update(cx, |view, cx| {
                view.peers_arrived(
                    Arc::new(PeerList {
                        hash: film.hash.clone(),
                        peers: vec![peer("192.0.2.1"), peer("192.0.2.2")],
                        sources: PeerSources::default(),
                    }),
                    cx,
                );
            });
        });
        let peers = |cx: &mut TestAppContext| cx.update(|cx| panel.read(cx).peers_dimming(cx));
        assert_eq!(peers(cx), (None, vec![None, None]), "fresh");

        // The daemon stopped answering: what the panel shows is old.
        let reconnecting = ConnectionState::Reconnecting {
            attempt: 1,
            last_error: ConnectionError::Unreachable,
        };
        cx.update(|cx| view.update(cx, |view, cx| view.show(snapshot(reconnecting, true), cx)));

        assert!(cx.update(|cx| panel.read(cx).is_stale()));
        assert_eq!(
            peers(cx),
            (Some(STALE_OPACITY), vec![Some(STALE_OPACITY); 2]),
            "the summary and the rows of the Peers tab"
        );
        cx.update(|cx| {
            panel.update(cx, |panel, cx| panel.set_tab(DetailsTab::Trackers, cx));
            view.update(cx, |view, cx| {
                view.details_arrived(
                    Arc::new(TorrentDetails {
                        trackers: vec![tracker("a.example.org:80"), tracker("b.example.org:80")],
                        ..details(&film.hash, &[])
                    }),
                    cx,
                );
            });
        });
        let trackers =
            |cx: &mut TestAppContext| cx.update(|cx| panel.read(cx).trackers_dimming(cx));
        assert_eq!(
            trackers(cx),
            [Some(STALE_OPACITY); 2],
            "the rows of the Trackers tab"
        );

        // It answers again.
        cx.update(|cx| {
            view.update(cx, |view, cx| {
                view.show(snapshot(ConnectionState::Connected, false), cx);
            });
        });
        assert!(!cx.update(|cx| panel.read(cx).is_stale()));
        assert_eq!(trackers(cx), [None; 2]);
    }

    /// The table dims its rows while the data is stale; an opacity on the list around it would
    /// multiply with theirs and dim every row twice.
    #[gpui_kit::test]
    fn the_torrent_list_around_the_table_is_never_dimmed(cx: &mut TestAppContext) {
        let (_events, events_rx) = async_channel::unbounded();
        let (commands, _commands_rx) = crossbeam_channel::unbounded();
        let (_, view): (_, Entity<MainView>) = cx
            .update(|cx| {
                app::init(cx);
                app::open_main_window(events_rx, commands, None, &UiState::default(), cx)
            })
            .expect("the window opens");
        let opacity =
            |cx: &mut TestAppContext| cx.update(|cx| view.read(cx).torrent_list().style().opacity);
        assert_eq!(opacity(cx), None, "while the data is fresh");

        cx.update(|cx| view.update(cx, MainView::service_stopped));

        assert!(cx.update(|cx| view.read(cx).status.stale));
        assert_eq!(opacity(cx), None, "while the data is stale");
    }
}
