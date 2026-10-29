//! The content of the main window: a toolbar with the search box, the filter sidebar, the
//! torrent table and the status bar, fed by the events of the service.
//!
//! The view pumps the events channel on the UI thread's executor and never waits on the network:
//! awaiting the channel yields until the service sends something, and sending a command never
//! blocks. Each batch of queued events is cut down to its newest snapshot, which `x4-core` turns
//! into the [`TableModel`] and the [`StatusModel`]; the view redraws only when one of them changed,
//! so an idle daemon costs no frames.
//!
//! The torrent table ([`TorrentTable`]) shows the rows; the view owns what they are, as its
//! [`ViewState`]: a click on a filter in the sidebar, an edit in the search box and a click on a
//! header (through [`ViewState::toggle_sort`]) change it, and the view recomputes the rows from the
//! last snapshot and hands them to the table, without asking the daemon. It also keeps what the
//! user does to the columns (shown, moved, resized) in the [`UiState`], and the toolbar's column
//! menu shows and hides them. The selection stays in the table: this stage reads nothing more for
//! a selected torrent.
//!
//! The view also tells the service whether the user can see the window, so that it polls slowly
//! while nobody looks, and it holds the theme choice of the toolbar's switch.

use std::sync::Arc;

use crossbeam_channel::Sender;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::table::{DataTable, TableEvent, TableState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, Size, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, App, AppContext as _, AsyncApp, Context, Entity, Focusable as _,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, TestSupportExt as _,
    WeakEntity, Window, div, px,
};
use x4_core::service::ServiceHandle;
use x4_core::{
    ColumnId, Command, Event, Filter, ServiceStatus, Snapshot, SortState, StatusModel, TableModel,
    ThemeChoice, UiState, ViewState, Visibility, shown_columns, status_model, table_model,
    visibility,
};

use crate::app::{ClearSearch, ConfigProblem, FocusNext, FocusPrevious, FocusSearch, MAIN_CONTEXT};
use crate::i18n::{column_name_text, connection_text, t, torrent_count_text};
use crate::search::{clear_search, focus_search, new_search_state, search_box};
use crate::sidebar::filter_sidebar;
use crate::status_bar::status_bar;
use crate::theme::{STALE_OPACITY, apply_theme, follow_appearance, next_theme};
use crate::torrent_table::{TorrentTable, new_table_state, show_column, show_rows};

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
    /// Why `config.toml` is not used, until the user dismisses it.
    config_problem: Option<ConfigProblem>,
    /// Where commands for the service go.
    commands: Sender<Command>,
    /// The visibility last sent to the service; `None` before the first.
    last_visibility: Option<Visibility>,
    /// The event pump; dropping the view drops it, which cancels it.
    _events: Task<()>,
    /// Kept so that closing the window stops the service thread.
    _service: Option<ServiceHandle>,
    /// The table's and the search box's events and the window's activation, visibility and
    /// appearance observers.
    _subscriptions: Vec<Subscription>,
}

impl MainView {
    /// The view of the service whose channels are `events` and `commands`, starting from the
    /// saved state `ui` (the columns, the filter, the sort order and the theme). The theme itself
    /// is applied by whoever opens the window, before the first frame. The table has the focus
    /// from the start.
    ///
    /// `service` is the handle of the thread behind the channels: the view keeps it, so that
    /// closing the window stops the thread. Without one (in tests), the channels are all there is.
    pub fn new(
        events: async_channel::Receiver<Event>,
        commands: Sender<Command>,
        service: Option<ServiceHandle>,
        ui: UiState,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Self {
        let view_state = ui.view_state();
        let table = Self::new_table(&ui, &view_state, window, cx);
        // The window's own bindings work only while something in it has the focus.
        window.focus(&table.focus_handle(cx), cx);
        let search = cx.new(|cx| new_search_state(window, cx));
        let subscriptions = vec![
            cx.subscribe_in(&table, window, Self::on_table_event),
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
        ];
        let pump = cx.spawn(async move |view, cx| pump(events, view, cx).await);
        let mut view = Self {
            status: StatusModel::default(),
            view: view_state,
            model: TableModel::default(),
            table,
            search,
            last_snapshot: None,
            ui,
            config_problem: None,
            commands,
            last_visibility: None,
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

    /// Shows the torrents `view` asks for (the filter, the search and the sort order), from the
    /// last snapshot and without waiting for the next one, and remembers its filter and sort
    /// order in the [`UiState`]. The search is not remembered.
    pub fn set_view(&mut self, view: ViewState, cx: &mut Context<'_, Self>) {
        if view == self.view {
            return;
        }
        self.ui.filter = view.filter;
        self.ui.sort = SortState {
            key: view.sort_key,
            direction: view.sort_direction,
        };
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

    /// Follows what the user does in the table: the selected torrent, and the columns moved or
    /// resized, which are remembered.
    fn on_table_event(
        &mut self,
        table: &Entity<TableState<TorrentTable>>,
        event: &TableEvent,
        _: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        match event {
            TableEvent::SelectRow(row_ix) => {
                let row_ix = *row_ix;
                table.update(cx, |table, _| table.delegate_mut().select_row(row_ix));
            }
            TableEvent::ClearSelection => {
                table.update(cx, |table, _| table.delegate_mut().clear_selected());
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

    /// Copies the table's columns into the state to remember between runs.
    fn remember_columns(&mut self, cx: &App) {
        self.ui.columns = self.table.read(cx).delegate().columns().to_vec();
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

    /// Takes a batch of events, oldest first, and redraws if what the window shows changed.
    fn take_events(&mut self, events: Vec<Event>, cx: &mut Context<'_, Self>) {
        let mut changed = false;
        for event in events {
            match event {
                Event::Snapshot(snapshot) => changed |= self.show(snapshot, cx),
                Event::Details(_) => {
                    tracing::debug!("ignored torrent details: this window shows none yet");
                }
                Event::ActionFinished { id, result } => match result {
                    Ok(()) => tracing::debug!(?id, "an action finished"),
                    Err(error) => tracing::warn!(?id, %error, "an action failed"),
                },
            }
        }
        if changed {
            cx.notify();
        }
    }

    /// Recomputes the table and the status bar for `snapshot` and keeps it; returns whether
    /// either changed.
    fn show(&mut self, snapshot: Arc<Snapshot>, cx: &mut Context<'_, Self>) -> bool {
        let status = status_model(&snapshot);
        self.last_snapshot = Some(snapshot);
        let mut changed = self.update_table(cx);
        if status != self.status {
            self.status = status;
            changed = true;
        }
        changed
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
    fn switch_theme(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        self.ui.theme = next_theme(self.ui.theme);
        tracing::debug!(theme = ?self.ui.theme, "theme switched");
        apply_theme(self.ui.theme, window, cx);
        cx.notify();
    }

    fn dismiss_config_problem(&mut self, cx: &mut Context<'_, Self>) {
        self.config_problem = None;
        cx.notify();
    }

    fn toolbar(&self, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        let theme_label = match self.ui.theme {
            ThemeChoice::System => t!("theme-system"),
            ThemeChoice::Light => t!("theme-light"),
            ThemeChoice::Dark => t!("theme-dark"),
        };
        let border = cx.theme().border;
        h_flex()
            .id("toolbar")
            .flex_none()
            .gap_2()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(border)
            .child(search_box(&self.search, cx.theme()))
            .child(div().flex_1())
            .child(
                Button::new("theme-toggle")
                    .ghost()
                    .small()
                    .label(theme_label)
                    .tooltip(t!("theme-switch-tooltip"))
                    .on_click(cx.listener(|view, _, window, cx| view.switch_theme(window, cx))),
            )
            .child(self.column_menu(cx))
    }

    /// The toolbar button whose menu shows and hides the columns, with a check on each shown
    /// one. The name column's entry cannot be unchecked.
    fn column_menu(&self, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        let choices = self.column_choices(cx);
        let this = cx.weak_entity();
        Button::new("column-menu")
            .ghost()
            .small()
            .label(t!("column-menu"))
            .tooltip(t!("column-menu-tooltip"))
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                choices.into_iter().fold(menu, |menu, (column, shown)| {
                    let this = this.clone();
                    menu.item(
                        PopupMenuItem::new(column_name_text(column))
                            .checked(shown)
                            .disabled(!column.can_hide())
                            .on_click(move |_, _, cx| {
                                if let Err(error) =
                                    this.update(cx, |view, cx| view.toggle_column(column, cx))
                                {
                                    tracing::debug!(%error, "a column was toggled after the window closed");
                                }
                            }),
                    )
                })
            })
    }

    /// The torrent table, dimmed while the data is stale. Its accessible name says how many
    /// torrents it shows.
    fn torrent_list(&self) -> impl IntoElement + use<> {
        div()
            .id("torrent-list")
            .flex_1()
            .min_w_0()
            .h_full()
            .text_sm()
            .when(self.status.stale, |list| list.opacity(STALE_OPACITY))
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
        let toolbar = self.toolbar(cx);
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
        let status_bar = status_bar(&self.status, self.config_problem, theme, on_dismiss);
        v_flex()
            .key_context(MAIN_CONTEXT)
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .on_action(cx.listener(|_, _: &FocusNext, window, cx| window.focus_next(cx)))
            .on_action(cx.listener(|_, _: &FocusPrevious, window, cx| window.focus_prev(cx)))
            .on_action(cx.listener(|view, _: &FocusSearch, window, cx| {
                focus_search(&view.search, window, cx);
            }))
            .on_action(cx.listener(Self::on_clear_search))
            .child(toolbar)
            .child(h_flex().flex_1().min_h_0().child(sidebar).child(table))
            .child(status_bar)
    }
}

/// Feeds the view the events of the service until the view is gone or the channel closes.
///
/// Whatever has queued up while the UI thread was busy is taken in one go and reduced to the
/// newest snapshot, so a slow frame never makes the window replay old states.
async fn pump(
    events: async_channel::Receiver<Event>,
    view: WeakEntity<MainView>,
    cx: &mut AsyncApp,
) {
    while let Ok(first) = events.recv().await {
        let mut batch = vec![first];
        // Only what is queued now, so a busy service cannot keep the loop from yielding.
        for _ in 0..events.len() {
            match events.try_recv() {
                Ok(event) => batch.push(event),
                Err(_) => break,
            }
        }
        let batch = newest_snapshot_only(batch);
        if view
            .update(cx, |view, cx| view.take_events(batch, cx))
            .is_err()
        {
            // The window was closed; dropping `events` tells the service so.
            return;
        }
    }
    if let Err(error) = view.update(cx, MainView::service_stopped) {
        tracing::debug!(%error, "the service stopped after the window closed");
    }
}

/// `events` without the snapshots that a later one in it replaces; the other events keep their
/// order. Each snapshot holds the whole state, so only the newest needs to be shown.
fn newest_snapshot_only(mut events: Vec<Event>) -> Vec<Event> {
    let newest = events
        .iter()
        .rposition(|event| matches!(event, Event::Snapshot(_)));
    let mut index = 0;
    events.retain(|event| {
        let keep = !matches!(event, Event::Snapshot(_)) || Some(index) == newest;
        index += 1;
        keep
    });
    events
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui_kit::{Entity, TestAppContext};
    use x4_core::{ActionError, ActionId, ConnectionState};

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

    fn snapshot(rate_down: u64) -> Event {
        Event::Snapshot(Arc::new(Snapshot {
            torrents: Vec::new(),
            listed: true,
            session: None,
            free_space_bytes: None,
            rate_down,
            rate_up: 0,
            connection: ConnectionState::Connected,
            stale: false,
        }))
    }

    fn finished(id: u64) -> Event {
        Event::ActionFinished {
            id: ActionId(id),
            result: Err(ActionError::NotConnected),
        }
    }

    /// The download rates of the snapshots and the ids of the finished actions, in order.
    fn summary(events: &[Event]) -> Vec<String> {
        events
            .iter()
            .map(|event| match event {
                Event::Snapshot(snapshot) => format!("snapshot {}", snapshot.rate_down),
                Event::ActionFinished { id, .. } => format!("action {}", id.0),
                Event::Details(_) => "details".to_owned(),
            })
            .collect()
    }

    #[test]
    fn only_the_newest_snapshot_is_kept() {
        let events = newest_snapshot_only(vec![snapshot(1), snapshot(2), snapshot(3)]);

        assert_eq!(summary(&events), ["snapshot 3"]);
    }

    #[test]
    fn other_events_keep_their_order_around_the_newest_snapshot() {
        let events = newest_snapshot_only(vec![
            snapshot(1),
            finished(7),
            snapshot(2),
            finished(8),
            snapshot(3),
            finished(9),
        ]);

        assert_eq!(
            summary(&events),
            ["action 7", "action 8", "snapshot 3", "action 9"]
        );
    }

    #[test]
    fn a_batch_without_snapshots_is_left_alone() {
        assert_eq!(
            summary(&newest_snapshot_only(vec![finished(1), finished(2)])),
            ["action 1", "action 2"]
        );
        assert_eq!(
            summary(&newest_snapshot_only(Vec::new())),
            [] as [String; 0]
        );
    }
}
