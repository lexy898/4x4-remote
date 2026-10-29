//! The content of the main window: a toolbar with the search box (the `toolbar` module), the
//! filter sidebar, the torrent table and the status bar, fed by the events of the service.
//!
//! The view pumps the events channel on the UI thread's executor and never waits on the network
//! (the `events` module): awaiting the channel yields until the service sends something, and
//! sending a command never blocks. Each batch of queued events is cut down to its newest snapshot,
//! which `x4-core` turns into the [`TableModel`] and the [`StatusModel`]; the view redraws only when
//! one of them changed, so an idle daemon costs no frames.
//!
//! The torrent table ([`TorrentTable`]) shows the rows; the view owns what they are, as its
//! [`ViewState`]: a click on a filter in the sidebar, an edit in the search box and a click on a
//! header (through [`ViewState::toggle_sort`]) change it, and the view recomputes the rows from the
//! last snapshot and hands them to the table, without asking the daemon. It also keeps what the
//! user does to the columns (shown, moved, resized) in the [`UiState`], and the toolbar's column
//! menu shows and hides them. The selection of several torrents stays in the table, by hash; the
//! view handles the selection keys of [`keymap`](crate::keymap) (Ctrl+A, Ctrl+Shift+A, Escape,
//! the arrows, Home, End, Page Up and Page Down, with and without Shift) and hands them to it, and
//! passes on what the table does to its own selected row. This stage reads nothing more for a
//! selected torrent.
//!
//! The selected torrents can be started, paused, started now, verified and reannounced, from their
//! keys and from the toolbar's Start and Pause (the `torrent_actions` module). Each action goes
//! out as a command and is kept until the service answers it; a failure is shown as a notification.
//! A held key sends its action once.
//!
//! The view also tells the service whether the user can see the window, so that it polls slowly
//! while nobody looks, and it holds the theme choice of the toolbar's switch.
//!
//! What the user changes in the [`UiState`] (the columns, the sort order, the filter, the theme
//! and the window size) is saved a second after the last change, and when the window closes or
//! the app quits (the `persistence` module).

mod events;
mod persistence;
mod torrent_actions;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use crossbeam_channel::Sender;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::table::{DataTable, TableEvent, TableState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, Size, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Task, TestSupportExt as _, Window, div, px,
};
use x4_core::service::ServiceHandle;
use x4_core::{
    Availability, ColumnId, Command, Edge, Event, Filter, ServiceStatus, Snapshot, SortState,
    StatusModel, TableModel, UiState, ViewState, Visibility, shown_columns, status_model,
    table_model, visibility,
};

use crate::actions::PendingActions;
use crate::app::ConfigProblem;
use crate::i18n::{connection_text, torrent_count_text};
use crate::key_repeats::KeyRepeats;
use crate::keymap::{
    ClearSearch, DeselectTorrents, ExtendSelectionDown, ExtendSelectionToFirst,
    ExtendSelectionToLast, ExtendSelectionUp, FocusNext, FocusPrevious, FocusSearch, MAIN_CONTEXT,
    MoveSelectionDown, MoveSelectionPageDown, MoveSelectionPageUp, MoveSelectionToFirst,
    MoveSelectionToLast, MoveSelectionUp, SelectAllTorrents, TORRENT_LIST_CONTEXT,
};
use crate::search::{clear_search, focus_search, new_search_state};
use crate::sidebar::filter_sidebar;
use crate::status_bar::status_bar;
use crate::theme::{apply_theme, follow_appearance, next_theme};
use crate::toolbar::{ToolbarState, toolbar};
use crate::torrent_table::{
    SelectionChange, TorrentTable, change_selection, follow_table, new_table_state, page_step,
    show_column, show_rows,
};
use persistence::Saver;
use torrent_actions::{follow_key_repeats, on_torrent_actions};

/// The height of a row of the table, for the two lines of the name cell.
const ROW_HEIGHT: f32 = 44.;
/// The height of the table's header.
const HEADER_HEIGHT: f32 = 32.;

/// The main window's content.
pub struct MainView {
    /// What the status bar shows.
    status: StatusModel,
    /// The filter, the search and the sort order the table follows.
    view: ViewState,
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
    /// The actions sent and not answered yet.
    pending: PendingActions,
    /// What the newest failure of an action said.
    last_failure: Option<SharedString>,
    /// The key held down now, so that its repeats do not run its action again; shared with the
    /// app's keystroke interceptor, which sees the key-downs before the bindings do.
    key_repeats: Rc<RefCell<KeyRepeats>>,
    /// The event pump; dropping the view drops it, which cancels it.
    _events: Task<()>,
    /// Kept so that closing the window stops the service thread.
    _service: Option<ServiceHandle>,
    /// The table's and the search box's events, the table's changes, the focus coming back to the
    /// table, the window's activation, visibility, appearance and bounds observers, the save when
    /// the app quits, and the keystroke interceptor.
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
            cx.observe_window_activation(window, |view, window, _| {
                view.report_visibility(window);
            }),
            cx.observe_window_visibility(window, |view, _, window, _| {
                view.report_visibility(window);
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
            last_failure: None,
            key_repeats,
            _events: pump,
            _service: service,
            _subscriptions: subscriptions,
        };
        // The service starts out assuming a visible window; tell it how the window opened.
        view.report_visibility(window);
        view
    }

    /// The table of the main window, with its rows, columns and selection: the delegate
    /// ([`TorrentTable`]) and the state of gpui-component's table around it.
    #[must_use]
    pub fn table(&self) -> &Entity<TableState<TorrentTable>> {
        &self.table
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
    pub fn set_view(&mut self, view: ViewState, cx: &mut Context<'_, Self>) {
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
    /// a header sorts through the view.
    fn new_table(
        ui: &UiState,
        view: &ViewState,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Entity<TableState<TorrentTable>> {
        let this = cx.weak_entity();
        let delegate = TorrentTable::new(ui.columns.clone(), (view.sort_key, view.sort_direction))
            .on_sort(move |column, _, cx| {
                if let Err(error) = this.update(cx, |view, cx| view.sort_by(column, cx)) {
                    tracing::debug!(%error, "a header was clicked after the window closed");
                }
            });
        cx.new(|cx| new_table_state(delegate, window, cx))
    }

    /// Handles a click on the header of `column`: sorts by it, or flips the direction if the
    /// list is sorted by it already.
    fn sort_by(&mut self, column: ColumnId, cx: &mut Context<'_, Self>) {
        let mut view = self.view.clone();
        view.toggle_sort(column);
        tracing::debug!(key = ?view.sort_key, direction = ?view.sort_direction, "sorted by a column");
        self.set_view(view, cx);
    }

    /// Follows what the user does in the table: what it does to its own selected row (taken over
    /// by the selection, unless it only echoes the selection), and the columns moved or resized,
    /// which are remembered.
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
            TableEvent::DoubleClickedRow(_)
            | TableEvent::SelectColumn(_)
            | TableEvent::SelectCell(..)
            | TableEvent::DoubleClickedCell(..)
            | TableEvent::RightClickedRow(_)
            | TableEvent::RightClickedCell(..) => {}
        }
    }

    /// Handles a selection key of the torrent list.
    fn change_selection(&self, change: SelectionChange, cx: &mut Context<'_, Self>) {
        self.table
            .update(cx, |table, cx| change_selection(table, change, cx));
    }

    /// Handles Page Up (`pages` −1) or Page Down (1): selects only the row a page from the lead.
    fn change_selection_by_page(&self, pages: isize, cx: &mut Context<'_, Self>) {
        self.table.update(cx, |table, cx| {
            let step = page_step(table).saturating_mul(pages);
            change_selection(
                table,
                SelectionChange::Step {
                    step,
                    extend: false,
                },
                cx,
            );
        });
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
        self.show_stale(cx);
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
            self.show_stale(cx);
            changed = true;
        }
        changed |= self.refresh_availability(cx);
        changed
    }

    /// Tells the table whether its rows are stale data, which it draws at the stale opacity.
    fn show_stale(&self, cx: &mut Context<'_, Self>) {
        let stale = self.status.stale;
        self.table.update(cx, |table, cx| {
            if table.delegate_mut().set_stale(stale) {
                cx.notify();
            }
        });
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
        self.show_stale(cx);
        self.refresh_availability(cx);
        cx.notify();
    }

    /// Tells the service whether the user can see the window, when that changed.
    fn report_visibility(&mut self, window: &Window) {
        let visibility = visibility(window.is_window_active(), window.is_visible());
        if self.last_visibility == Some(visibility) {
            return;
        }
        self.last_visibility = Some(visibility);
        tracing::debug!(?visibility, "window visibility changed");
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
    fn torrent_list(&self) -> impl IntoElement + use<> {
        div()
            .id("torrent-list")
            .key_context(TORRENT_LIST_CONTEXT)
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
            theme,
            on_dismiss,
        );
        let root = on_torrent_actions(v_flex(), cx);
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
            .on_action(cx.listener(|view, _: &SelectAllTorrents, _, cx| {
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
            .child(toolbar)
            .child(h_flex().flex_1().min_h_0().child(sidebar).child(table))
            .child(status_bar)
    }
}

/// A move of the lead by `step` rows (negative is up), extending the range or not.
const fn step(step: isize, extend: bool) -> SelectionChange {
    SelectionChange::Step { step, extend }
}

/// A move of the lead to an end of the list (Home, End), extending the range or not.
const fn to_edge(edge: Edge, extend: bool) -> SelectionChange {
    SelectionChange::ToEdge { edge, extend }
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
}
