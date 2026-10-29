//! Headless tests for the main window: what it shows from the events of the service, what it
//! tells the service, its theme switch, and the application-wide key bindings.
//!
//! Each test builds the two channels of a service itself and opens the window with no service
//! behind them, so nothing touches the network: the test sends the events and reads the commands.
//!
//! Quitting when the last window closes (`QuitMode`) is not covered: the test platform's `quit`
//! does nothing and `App` has no getter for the quit mode.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::{ActiveTheme as _, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Entity, Subscription, TestAppContext, WindowVisibility,
};
use x4_app::app::{self, Quit};
use x4_app::main_view::MainView;
use x4_core::test_support::torrent;
use x4_core::{
    Command, ConnectionError, ConnectionState, DisplayState, Event, Filter, Snapshot, StatusModel,
    ThemeChoice, Torrent, UiState, ViewState, Visibility, human_size, human_speed,
};

/// An open main window and the far ends of its channels, standing in for the service.
struct Window {
    handle: AnyWindowHandle,
    view: Entity<MainView>,
    events: async_channel::Sender<Event>,
    commands: crossbeam_channel::Receiver<Command>,
}

impl Window {
    /// Initializes the app and opens the main window with `ui` as the saved state.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a window that does not open fails the test"
    )]
    fn open_with(cx: &mut TestAppContext, ui: &UiState) -> Self {
        let (events, events_rx) = async_channel::unbounded();
        let (commands_tx, commands) = crossbeam_channel::unbounded();
        let (handle, view) = cx
            .update(|cx| {
                app::init(cx);
                app::open_main_window(events_rx, commands_tx, None, ui, cx)
            })
            .expect("the main window opens");
        Self {
            handle,
            view,
            events,
            commands,
        }
    }

    fn open(cx: &mut TestAppContext) -> Self {
        Self::open_with(cx, &UiState::default())
    }

    /// Sends `event` as the service would and lets the window take it.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a window that stopped listening fails the test"
    )]
    fn send(&self, event: Event, cx: &mut TestAppContext) {
        self.events
            .send_blocking(event)
            .expect("the window listens");
        cx.run_until_parked();
    }

    fn status(&self, cx: &mut TestAppContext) -> StatusModel {
        cx.update(|cx| self.view.read(cx).status().clone())
    }

    fn rows(&self, cx: &mut TestAppContext) -> usize {
        cx.update(|cx| self.view.read(cx).model().rows.len())
    }

    fn connection_text(&self, cx: &mut TestAppContext) -> String {
        cx.update(|cx| self.view.read(cx).connection_text().to_string())
    }

    /// The commands the window has sent since the last call.
    fn sent(&self) -> Vec<Command> {
        self.commands.try_iter().collect()
    }

    /// Counts the redraws the view asks for from now on; keep the subscription alive.
    fn count_redraws(&self, cx: &mut TestAppContext) -> (Rc<Cell<u32>>, Subscription) {
        let redraws = Rc::new(Cell::new(0));
        let counter = Rc::clone(&redraws);
        let subscription =
            cx.update(|cx| cx.observe(&self.view, move |_, _| counter.set(counter.get() + 1)));
        (redraws, subscription)
    }
}

fn snapshot(torrents: &[&str]) -> Snapshot {
    Snapshot {
        torrents: torrents.iter().map(|name| torrent(name)).collect(),
        session: None,
        free_space_bytes: Some(24_592_841_836),
        rate_down: 2_516_582,
        rate_up: 184_320,
        connection: ConnectionState::Connected,
        stale: false,
    }
}

#[gpui_kit::test]
fn window_opens_without_network(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    cx.run_until_parked();

    assert_eq!(cx.windows().len(), 1);
    assert_eq!(window.status(cx), StatusModel::default());
    assert_eq!(window.status(cx).connection, ConnectionState::Connecting);
    assert_eq!(window.rows(cx), 0);
    assert_eq!(window.connection_text(cx), "Connecting…");
    cx.update_window(window.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("connection").label(), Some("Connecting…"));
    })
    .expect("the window is open");
}

#[gpui_kit::test]
fn snapshot_updates_status_and_rows(cx: &mut TestAppContext) {
    let window = Window::open(cx);

    window.send(
        Event::Snapshot(Arc::new(snapshot(&["alpha", "beta", "gamma"]))),
        cx,
    );

    assert_eq!(window.rows(cx), 3);
    let status = window.status(cx);
    assert_eq!(status.rate_down, human_speed(2_516_582));
    assert_eq!(status.rate_up, human_speed(184_320));
    assert_eq!(status.free_space, Some(human_size(24_592_841_836)));
    assert_eq!(status.connection, ConnectionState::Connected);
    assert!(!status.stale);
    assert_eq!(
        window.connection_text(cx),
        "",
        "connected shows only the dot"
    );
    cx.update_window(window.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("connection").label(), Some("Connected"));
        assert_eq!(window.find("rate-down").label(), Some("↓ 2.4 MiB/s"));
        assert_eq!(window.find("rate-up").label(), Some("↑ 180 KiB/s"));
        assert_eq!(window.find("free-space").label(), Some("22.9 GiB free"));
    })
    .expect("the window is open");
}

#[gpui_kit::test]
fn status_bar_shows_reconnecting_and_stale(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(Event::Snapshot(Arc::new(snapshot(&["alpha", "beta"]))), cx);

    window.send(
        Event::Snapshot(Arc::new(Snapshot {
            connection: ConnectionState::Reconnecting {
                attempt: 2,
                last_error: ConnectionError::Timeout,
            },
            stale: true,
            ..snapshot(&["alpha", "beta"])
        })),
        cx,
    );

    assert!(window.status(cx).stale);
    let text = window.connection_text(cx);
    assert!(text.contains('2'), "{text}");
    assert_eq!(text, "Reconnecting (attempt 2)… timed out");
    assert_eq!(window.rows(cx), 2, "the last data stays visible");
}

#[gpui_kit::test]
fn unsupported_server_is_named_in_the_status_bar(cx: &mut TestAppContext) {
    let window = Window::open(cx);

    window.send(
        Event::Snapshot(Arc::new(Snapshot {
            connection: ConnectionState::Unsupported {
                reason: x4_core::UnsupportedReason::Version("17".into()),
            },
            ..snapshot(&[])
        })),
        cx,
    );

    assert_eq!(
        window.connection_text(cx),
        "Transmission 4.1 or newer required (server RPC 17)"
    );
}

#[gpui_kit::test]
fn queued_snapshots_are_coalesced_into_one_redraw(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    cx.run_until_parked();
    let (redraws, _subscription) = window.count_redraws(cx);

    for torrents in [&["a"][..], &["a", "b"], &["a", "b", "c"]] {
        window
            .events
            .send_blocking(Event::Snapshot(Arc::new(snapshot(torrents))))
            .expect("the window listens");
    }
    cx.run_until_parked();

    assert_eq!(window.rows(cx), 3, "the newest snapshot is shown");
    assert_eq!(redraws.get(), 1);
}

#[gpui_kit::test]
fn an_unchanged_snapshot_does_not_redraw(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(Event::Snapshot(Arc::new(snapshot(&["a", "b"]))), cx);
    let (redraws, _subscription) = window.count_redraws(cx);

    window.send(Event::Snapshot(Arc::new(snapshot(&["a", "b"]))), cx);
    assert_eq!(redraws.get(), 0);

    window.send(
        Event::Snapshot(Arc::new(Snapshot {
            rate_up: 0,
            ..snapshot(&["a", "b"])
        })),
        cx,
    );
    assert_eq!(redraws.get(), 1, "a new speed is a change");
}

#[gpui_kit::test]
fn visibility_changes_send_one_command(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    cx.run_until_parked();
    cx.update_window(window.handle, |_, window, _| window.activate_window())
        .expect("the window is open");
    cx.run_until_parked();
    assert_eq!(
        window.sent().last(),
        Some(&Command::SetVisibility(Visibility::Visible)),
        "an active, shown window is visible"
    );

    cx.simulate_window_visibility_change(window.handle, WindowVisibility::Hidden);
    cx.run_until_parked();
    assert_eq!(
        window.sent(),
        [Command::SetVisibility(Visibility::Hidden)],
        "hiding the window is reported at once"
    );

    cx.simulate_window_visibility_change(window.handle, WindowVisibility::Hidden);
    cx.run_until_parked();
    assert_eq!(window.sent(), [], "hidden again is no change");

    // Losing the focus while hidden changes nothing the service cares about.
    let other = cx.add_window(|_, _| gpui_kit::Empty);
    cx.update_window(other.into(), |_, other, _| other.activate_window())
        .expect("the other window is open");
    cx.run_until_parked();
    assert_eq!(
        window.sent(),
        [],
        "losing the focus while hidden is no change"
    );
}

#[gpui_kit::test]
fn losing_the_focus_hides_the_window_for_the_service(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    cx.update_window(window.handle, |_, window, _| window.activate_window())
        .expect("the window is open");
    cx.run_until_parked();
    window.sent();

    let other = cx.add_window(|_, _| gpui_kit::Empty);
    cx.update_window(other.into(), |_, other, _| other.activate_window())
        .expect("the other window is open");
    cx.run_until_parked();
    assert_eq!(window.sent(), [Command::SetVisibility(Visibility::Hidden)]);

    cx.update_window(window.handle, |_, window, _| window.activate_window())
        .expect("the window is open");
    cx.run_until_parked();
    assert_eq!(window.sent(), [Command::SetVisibility(Visibility::Visible)]);
}

#[gpui_kit::test]
fn the_window_reports_its_visibility_when_it_opens(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    cx.run_until_parked();

    // A test window opens without focus, and a window without focus counts as hidden.
    assert_eq!(window.sent(), [Command::SetVisibility(Visibility::Hidden)]);
}

#[gpui_kit::test]
fn the_window_sends_no_other_command(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(Event::Snapshot(Arc::new(snapshot(&["a", "b"]))), cx);
    cx.update_window(window.handle, |_, window, cx| {
        window.activate_window();
        window.render_frame(cx);
        window.click("theme-toggle", cx);
    })
    .expect("the window is open");
    cx.run_until_parked();
    cx.simulate_window_visibility_change(window.handle, WindowVisibility::Hidden);
    cx.run_until_parked();

    let sent = window.sent();
    assert_ne!(sent, []);
    assert!(
        sent.iter()
            .all(|command| matches!(command, Command::SetVisibility(_))),
        "{sent:?}"
    );
}

#[gpui_kit::test]
fn dropping_the_view_stops_the_pump(cx: &mut TestAppContext) {
    let Window {
        handle,
        view,
        events,
        commands: _commands,
    } = Window::open(cx);
    cx.run_until_parked();
    drop(view);
    assert!(!events.is_closed(), "the window listens while it is open");

    cx.update_window(handle, |_, window, _| window.remove_window())
        .expect("the window is open");
    cx.run_until_parked();

    assert!(events.is_closed(), "the pump let go of the events");
    assert_eq!(events.receiver_count(), 0);
}

#[gpui_kit::test]
fn closed_events_channel_marks_stale(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(Event::Snapshot(Arc::new(snapshot(&["a", "b"]))), cx);
    assert!(!window.status(cx).stale);

    let (redraws, _subscription) = window.count_redraws(cx);

    // The service thread stopped or panicked: its sender is gone.
    window.events.close();
    cx.run_until_parked();

    assert_eq!(redraws.get(), 1, "the window shows it at once");
    assert!(window.status(cx).stale);
    assert_eq!(window.connection_text(cx), "Service stopped");
    assert_eq!(window.rows(cx), 2, "the last data stays visible, dimmed");
    cx.update_window(window.handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("connection").label(), Some("Service stopped"));
    })
    .expect("the window is open");
}

#[gpui_kit::test]
fn changing_the_view_recomputes_the_rows_without_a_new_snapshot(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(
        Event::Snapshot(Arc::new(Snapshot {
            torrents: vec![
                torrent("seeding"),
                Torrent {
                    state: DisplayState::Paused,
                    ..torrent("paused")
                },
            ],
            ..snapshot(&[])
        })),
        cx,
    );
    assert_eq!(window.rows(cx), 2);
    let (redraws, _subscription) = window.count_redraws(cx);

    cx.update(|cx| {
        window.view.update(cx, |view, cx| {
            view.set_view(
                ViewState {
                    filter: Filter::Seeding,
                    ..ViewState::default()
                },
                cx,
            );
        });
    });
    cx.run_until_parked();

    let names: Vec<String> = cx.update(|cx| {
        window
            .view
            .read(cx)
            .model()
            .rows
            .iter()
            .map(|row| row.name.clone())
            .collect()
    });
    assert_eq!(names, ["seeding"]);
    assert_eq!(redraws.get(), 1);
    let ui = cx.update(|cx| window.view.read(cx).ui_state().clone());
    assert_eq!(ui.filter, Filter::Seeding, "the filter is remembered");

    cx.update(|cx| {
        window.view.update(cx, |view, cx| {
            view.set_view(
                ViewState {
                    filter: Filter::Seeding,
                    ..ViewState::default()
                },
                cx,
            );
        });
    });
    assert_eq!(redraws.get(), 1, "the same view again changes nothing");
    assert_eq!(window.sent().len(), 1, "only the visibility at the start");
}

#[gpui_kit::test]
fn theme_toggle_cycles_system_light_dark(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let theme = |cx: &mut TestAppContext| {
        cx.update(|cx| (window.view.read(cx).ui_state().theme, cx.theme().mode))
    };
    // The test platform's appearance is light.
    assert_eq!(theme(cx), (ThemeChoice::System, ThemeMode::Light));

    let click = |cx: &mut TestAppContext| {
        cx.update_window(window.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("theme-toggle", cx);
        })
        .expect("the window is open");
    };
    click(cx);
    assert_eq!(theme(cx), (ThemeChoice::Light, ThemeMode::Light));
    click(cx);
    assert_eq!(theme(cx), (ThemeChoice::Dark, ThemeMode::Dark));
    click(cx);
    assert_eq!(theme(cx), (ThemeChoice::System, ThemeMode::Light));
}

#[gpui_kit::test]
fn the_saved_theme_applies_when_the_window_opens(cx: &mut TestAppContext) {
    let ui = UiState {
        theme: ThemeChoice::Dark,
        ..UiState::default()
    };

    let window = Window::open_with(cx, &ui);

    let (choice, mode) = cx.update(|cx| (window.view.read(cx).ui_state().theme, cx.theme().mode));
    assert_eq!(choice, ThemeChoice::Dark);
    assert_eq!(mode, ThemeMode::Dark);
}

#[gpui_kit::test]
fn init_installs_a_global_quit_handler(cx: &mut TestAppContext) {
    // With no window open, only a global listener can make the action available.
    let available = cx.update(|cx| {
        app::init(cx);
        cx.is_action_available(&Quit)
    });

    assert!(available);
}

#[gpui_kit::test]
fn ctrl_q_dispatches_quit(cx: &mut TestAppContext) {
    let quit_requested = Rc::new(Cell::new(false));
    let window = Window::open(cx);
    cx.update(|cx| {
        let quit_requested = Rc::clone(&quit_requested);
        // The test platform ignores `quit`, so a second listener records the dispatch. Global
        // listeners run newest first and stop the dispatch unless they propagate it, so this
        // one propagates to keep `init`'s handler in the chain.
        cx.on_action(move |_: &Quit, cx| {
            quit_requested.set(true);
            cx.propagate();
        });
    });

    cx.simulate_keystrokes(window.handle, "ctrl-q");

    assert!(quit_requested.get());
}
