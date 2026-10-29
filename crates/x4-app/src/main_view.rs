//! The content of the main window: a toolbar, the filter sidebar, the torrent table and the
//! status bar, fed by the events of the service.
//!
//! The view pumps the events channel on the UI thread's executor and never waits on the network:
//! awaiting the channel yields until the service sends something, and sending a command never
//! blocks. Each batch of queued events is cut down to its newest snapshot, which `x4-core` turns
//! into the [`TableModel`] and the [`StatusModel`]; the view redraws only when one of them changed,
//! so an idle daemon costs no frames.
//!
//! The view also tells the service whether the user can see the window, so that it polls slowly
//! while nobody looks, and it holds the theme choice of the toolbar's switch.

use crossbeam_channel::Sender;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AsyncApp, Context, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task,
    TestSupportExt as _, WeakEntity, Window, div, px,
};
use x4_core::service::ServiceHandle;
use x4_core::{
    Command, Event, Snapshot, StatusModel, TableModel, ThemeChoice, UiState, ViewState, Visibility,
    status_model, table_model, visibility,
};

use crate::i18n::{t, torrent_count_text};
use crate::status_bar::{ServiceState, StatusBarParts, connection_label, status_bar};
use crate::theme::{apply_theme, follow_appearance, next_theme};

/// The width of the sidebar.
const SIDEBAR_WIDTH: f32 = 200.;
/// How visible the table stays while its data is old: dimmed, but readable.
const STALE_OPACITY: f32 = 0.5;

/// The main window's content.
pub struct MainView {
    /// What the status bar shows.
    status: StatusModel,
    /// The filter, the search and the sort order the table follows.
    view: ViewState,
    /// The rows of the table and the counts of the sidebar.
    model: TableModel,
    /// The state to remember between runs, as the user has changed it.
    ui: UiState,
    /// Whether the service still runs.
    service: ServiceState,
    /// Why `config.toml` is not used, until the user dismisses it.
    config_problem: Option<SharedString>,
    /// Where commands for the service go.
    commands: Sender<Command>,
    /// The visibility last sent to the service; `None` before the first.
    last_visibility: Option<Visibility>,
    /// The event pump; dropping the view drops it, which cancels it.
    _events: Task<()>,
    /// Kept so that closing the window stops the service thread.
    _service: Option<ServiceHandle>,
    /// The window's activation, visibility and appearance observers.
    _subscriptions: Vec<Subscription>,
}

impl MainView {
    /// The view of the service whose channels are `events` and `commands`, starting from the
    /// saved state `ui` (the filter, the sort order and the theme). The theme itself is applied
    /// by whoever opens the window, before the first frame.
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
        let subscriptions = vec![
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
            view: ui.view_state(),
            model: TableModel::default(),
            ui,
            service: ServiceState::Running,
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

    /// What the status bar says about the connection; empty while connected, when the bar shows
    /// only the dot.
    #[must_use]
    pub fn connection_text(&self) -> SharedString {
        connection_label(&self.status, self.service)
    }

    /// The state to remember between runs, with the user's changes, such as the theme.
    #[must_use]
    pub fn ui_state(&self) -> &UiState {
        &self.ui
    }

    /// Shows that the service could not be started, so no data will come.
    pub(crate) fn service_failed(&mut self, cx: &mut Context<'_, Self>) {
        self.service = ServiceState::Failed;
        self.status.stale = true;
        cx.notify();
    }

    /// Shows that `config.toml` is not used and why, until the user dismisses it.
    pub(crate) fn show_config_problem(
        &mut self,
        problem: SharedString,
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
                Event::Snapshot(snapshot) => changed |= self.show(&snapshot),
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

    /// Recomputes the table and the status bar for `snapshot`; returns whether either changed.
    fn show(&mut self, snapshot: &Snapshot) -> bool {
        let model = table_model(snapshot, &self.view);
        let status = status_model(snapshot);
        let mut changed = false;
        if model != self.model {
            self.model = model;
            changed = true;
        }
        if status != self.status {
            self.status = status;
            changed = true;
        }
        changed
    }

    /// The events channel closed: the service thread stopped or panicked, so the data on screen
    /// will not change again. It stays visible but is marked stale.
    fn service_stopped(&mut self, cx: &mut Context<'_, Self>) {
        // A service that never started has said so already.
        if self.service == ServiceState::Running {
            tracing::error!("the service stopped; the window shows the last data it sent");
            self.service = ServiceState::Stopped;
        }
        self.status.stale = true;
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
            // The search box goes here.
            .child(div().id("search-slot").flex_1())
            .child(
                Button::new("theme-toggle")
                    .ghost()
                    .small()
                    .label(theme_label)
                    .tooltip(t!("theme-switch-tooltip"))
                    .on_click(cx.listener(|view, _, window, cx| view.switch_theme(window, cx))),
            )
            // The column menu goes here.
            .child(div().id("column-menu-slot"))
    }

    fn table(&self, cx: &Context<'_, Self>) -> impl IntoElement + use<> {
        let text = if self.model.rows.is_empty() {
            SharedString::from(t!("torrent-list-empty"))
        } else {
            torrent_count_text(self.model.rows.len())
        };
        div()
            .id("table")
            .flex_1()
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .text_color(cx.theme().muted_foreground)
            .when(self.status.stale, |table| table.opacity(STALE_OPACITY))
            .child(text.clone())
            .aria_label(text)
            .test_support()
    }
}

impl Render for MainView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let toolbar = self.toolbar(cx);
        let table = self.table(cx);
        let theme = cx.theme();
        let sidebar = v_flex()
            .id("sidebar")
            .flex_none()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .border_r_1()
            .border_color(theme.sidebar_border)
            .bg(theme.sidebar);
        let parts = StatusBarParts {
            status: &self.status,
            service: self.service,
            config_problem: self.config_problem.as_ref(),
        };
        let on_dismiss = cx.listener(|view, _, _, cx| view.dismiss_config_problem(cx));
        let status_bar = status_bar(&parts, theme, on_dismiss);
        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
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
    use std::sync::Arc;

    use x4_core::{ActionError, ActionId, ConnectionState};

    use super::*;

    fn snapshot(rate_down: u64) -> Event {
        Event::Snapshot(Arc::new(Snapshot {
            torrents: Vec::new(),
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
