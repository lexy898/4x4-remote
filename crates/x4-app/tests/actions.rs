//! Headless tests for acting on the selected torrents: Start, Pause, Start now, the Space toggle,
//! Verify and Reannounce from their keys and from the toolbar, what the window does while the
//! daemon is not connected, the keys in the search box, and how a failed action is shown.
//!
//! Each test builds the two channels of a service itself and opens the window with no service
//! behind them, so nothing touches the network: the test reads the commands the window sends and
//! answers them with the events a service would send.

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Entity, KeyContext, Modifiers, OperatingSystem,
    TestAppContext,
};
use x4_app::actions::{PauseTorrents, StartTorrents};
use x4_app::app;
use x4_app::main_view::MainView;
use x4_core::test_support::torrent;
use x4_core::{
    Action, ActionError, ActionId, Availability, Command, ConnectionError, ConnectionState,
    DisplayState, Event, QueueKind, Snapshot, Torrent, UiState,
};
use x4_rpc::HashString;

/// The id of the search box.
const SEARCH: &str = "search";
/// The ids of the toolbar's Start and Pause buttons.
const START: &str = "start-torrents";
const PAUSE: &str = "pause-torrents";
/// The id of a notification.
const NOTIFICATION: &str = "notification";

/// An open main window and the far ends of its channels, standing in for the service.
struct Window {
    handle: AnyWindowHandle,
    view: Entity<MainView>,
    events: async_channel::Sender<Event>,
    commands: crossbeam_channel::Receiver<Command>,
}

impl Window {
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a window that does not open fails the test"
    )]
    fn open(cx: &mut TestAppContext) -> Self {
        let (events, events_rx) = async_channel::unbounded();
        let (commands_tx, commands) = crossbeam_channel::unbounded();
        let (handle, view) = cx
            .update(|cx| {
                app::init(cx);
                app::open_main_window(events_rx, commands_tx, None, &UiState::default(), cx)
            })
            .expect("the main window opens");
        cx.run_until_parked();
        Self {
            handle,
            view,
            events,
            commands,
        }
    }

    /// Sends `event` as the service would and lets the window take it.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a window that stopped listening fails the test"
    )]
    fn send_event(&self, event: Event, cx: &mut TestAppContext) {
        self.events
            .send_blocking(event)
            .expect("the window listens");
        cx.run_until_parked();
    }

    /// Sends a snapshot of `torrents` with the daemon connected.
    fn send(&self, torrents: Vec<Torrent>, cx: &mut TestAppContext) {
        self.send_with(torrents, ConnectionState::Connected, cx);
    }

    /// Sends a snapshot of `torrents` with the connection in `connection`.
    fn send_with(
        &self,
        torrents: Vec<Torrent>,
        connection: ConnectionState,
        cx: &mut TestAppContext,
    ) {
        let stale = connection != ConnectionState::Connected;
        self.send_event(
            Event::Snapshot(Arc::new(Snapshot {
                torrents,
                listed: true,
                session: None,
                free_space_bytes: None,
                rate_down: 0,
                rate_up: 0,
                connection,
                stale,
            })),
            cx,
        );
    }

    /// Answers the action `id` with `result`, as the service does once the daemon has answered.
    fn finish(&self, id: ActionId, result: Result<(), ActionError>, cx: &mut TestAppContext) {
        self.send_event(Event::ActionFinished { id, result }, cx);
    }

    /// Runs `act` on the window after drawing a frame, then lets what it started finish.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a closed window fails the test"
    )]
    fn act<R>(
        &self,
        cx: &mut TestAppContext,
        act: impl FnOnce(&mut gpui_kit::Window, &mut App) -> R,
    ) -> R {
        let result = cx
            .update_window(self.handle, |_, window, cx| {
                window.render_frame(cx);
                act(window, cx)
            })
            .expect("the window is open");
        cx.run_until_parked();
        result
    }

    fn click(&self, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) {
        let id = id.into();
        self.act(cx, |window, cx| window.click(id, cx));
    }

    fn click_row(&self, row: usize, cx: &mut TestAppContext) {
        self.click(("row", row), cx);
    }

    fn ctrl_click_row(&self, row: usize, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| {
            window.click_with_modifiers(("row", row), Modifiers::control(), cx);
        });
    }

    fn press(&self, key: &str, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| window.press(key, cx));
    }

    /// Whether the toolbar button `id` is drawn disabled.
    fn disabled(&self, id: &'static str, cx: &mut TestAppContext) -> bool {
        self.act(cx, |window, _| window.find(id).disabled() == Some(true))
    }

    fn availability(&self, cx: &mut TestAppContext) -> Availability {
        cx.update(|cx| self.view.read(cx).availability())
    }

    fn last_failure(&self, cx: &mut TestAppContext) -> Option<String> {
        cx.update(|cx| self.view.read(cx).last_failure().map(str::to_owned))
    }

    /// Whether a notification is on screen.
    fn notified(&self, cx: &mut TestAppContext) -> bool {
        self.act(cx, |window, _| window.try_find(NOTIFICATION).is_some())
    }

    /// The commands the window has sent so far, other than the window's visibility.
    fn sent(&self) -> Vec<Command> {
        self.commands
            .try_iter()
            .filter(|command| !matches!(command, Command::SetVisibility(_)))
            .collect()
    }

    /// The one action the window has sent since the last look, with its id.
    #[track_caller]
    #[expect(
        clippy::panic,
        reason = "a test helper: anything but one action fails the test"
    )]
    fn sent_action(&self) -> (ActionId, Action) {
        match self.sent().as_slice() {
            [Command::Run(id, action)] => (*id, action.clone()),
            other => panic!("expected one action, got {other:?}"),
        }
    }
}

fn in_state(name: &str, state: DisplayState) -> Torrent {
    Torrent {
        state,
        ..torrent(name)
    }
}

fn paused(name: &str) -> Torrent {
    in_state(name, DisplayState::Paused)
}

fn downloading(name: &str) -> Torrent {
    in_state(name, DisplayState::Downloading)
}

/// The hashes of the torrents called `names`, in that order.
fn hashes(names: &[&str]) -> Vec<HashString> {
    names.iter().map(|name| torrent(name).hash).collect()
}

fn reconnecting() -> ConnectionState {
    ConnectionState::Reconnecting {
        attempt: 2,
        last_error: ConnectionError::Timeout,
    }
}

#[gpui_kit::test]
fn ctrl_s_starts_the_selected_paused_torrents(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(
        vec![paused("a"), torrent("b"), paused("c"), paused("d")],
        cx,
    );
    // Selected from the bottom up: the command still lists them as the list shows them.
    window.click_row(3, cx);
    window.ctrl_click_row(0, cx);
    window.ctrl_click_row(2, cx);

    window.press("ctrl-s", cx);

    let (_, action) = window.sent_action();
    assert_eq!(action, Action::Start(hashes(&["a", "c", "d"])));
}

#[gpui_kit::test]
fn space_pauses_when_any_target_runs(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![paused("a"), downloading("b"), paused("c")], cx);

    window.press("ctrl-a", cx);
    window.press("space", cx);
    assert_eq!(
        window.sent_action().1,
        Action::Stop(hashes(&["a", "b", "c"]))
    );

    // Only stopped torrents: Space starts them.
    window.click_row(0, cx);
    window.ctrl_click_row(2, cx);
    window.press("space", cx);
    assert_eq!(window.sent_action().1, Action::Start(hashes(&["a", "c"])));

    // Nothing selected: nothing to toggle.
    window.press("escape", cx);
    window.press("space", cx);
    assert_eq!(window.sent(), []);
}

#[gpui_kit::test]
fn start_now_and_verify_and_reannounce_send_their_actions(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(
        vec![
            paused("a"),
            downloading("b"),
            in_state("c", DisplayState::Queued(QueueKind::Download)),
        ],
        cx,
    );
    window.press("ctrl-a", cx);

    window.press("ctrl-shift-s", cx);
    let (start_now, action) = window.sent_action();
    assert_eq!(action, Action::StartNow(hashes(&["a", "b", "c"])));
    window.press("ctrl-shift-r", cx);
    let (verify, action) = window.sent_action();
    assert_eq!(action, Action::Verify(hashes(&["a", "b", "c"])));
    window.press("ctrl-r", cx);
    let (reannounce, action) = window.sent_action();
    assert_eq!(action, Action::Reannounce(hashes(&["a", "b", "c"])));
    window.press("ctrl-p", cx);
    let (pause, action) = window.sent_action();
    assert_eq!(action, Action::Stop(hashes(&["a", "b", "c"])));

    let ids: HashSet<ActionId> = [start_now, verify, reannounce, pause].into();
    assert_eq!(ids.len(), 4, "every action has its own id: {ids:?}");

    // Reannouncing reaches only torrents that run, and starting now only those that do not.
    window.click_row(0, cx);
    window.press("ctrl-r", cx);
    assert_eq!(window.sent(), [], "a paused torrent is not announced");
    window.click_row(1, cx);
    window.press("ctrl-shift-s", cx);
    window.press("ctrl-s", cx);
    assert_eq!(
        window.sent(),
        [],
        "a downloading torrent is running already"
    );
}

#[gpui_kit::test]
fn toolbar_buttons_follow_availability(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![paused("a"), torrent("b")], cx);

    assert!(window.disabled(START, cx), "nothing is selected");
    assert!(window.disabled(PAUSE, cx));
    window.click(START, cx);
    window.click(PAUSE, cx);
    assert_eq!(window.sent(), []);

    window.click_row(0, cx);
    assert!(!window.disabled(START, cx), "a paused torrent can start");
    assert!(window.disabled(PAUSE, cx), "a paused torrent cannot pause");
    window.click(PAUSE, cx);
    assert_eq!(window.sent(), []);
    window.click(START, cx);
    assert_eq!(window.sent_action().1, Action::Start(hashes(&["a"])));

    window.click_row(1, cx);
    assert!(window.disabled(START, cx), "a seeding torrent runs already");
    assert!(!window.disabled(PAUSE, cx));
    window.click(PAUSE, cx);
    assert_eq!(window.sent_action().1, Action::Stop(hashes(&["b"])));

    // The buttons are named for screen readers, and their tooltips name the keys: a tooltip looks
    // its action's key up in the torrent list's context, as here.
    window.act(cx, |window, _| {
        assert_eq!(window.find(START).label(), Some("Start"));
        assert_eq!(window.find(PAUSE).label(), Some("Pause"));
        let list = KeyContext::parse("TorrentList").expect("a key context");
        for (action, keys) in [
            (&StartTorrents as &dyn gpui_kit::Action, "ctrl-s"),
            (&PauseTorrents, "ctrl-p"),
        ] {
            let binding = window
                .highest_precedence_binding_for_action_in_context(action, list.clone())
                .expect("the action has a key in the list");
            let shown: Vec<String> = binding
                .keystrokes()
                .iter()
                .map(|keystroke| keystroke.unparse(OperatingSystem::Linux))
                .collect();
            assert_eq!(shown, [keys]);
        }
    });
}

#[gpui_kit::test]
fn toolbar_enables_start_when_a_paused_row_gets_selected(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![paused("a"), torrent("b"), torrent("c")], cx);
    window.click_row(1, cx);
    assert!(window.disabled(START, cx));
    let redraws = Rc::new(Cell::new(0));
    let counter = Rc::clone(&redraws);
    let _subscription =
        cx.update(|cx| cx.observe(&window.view, move |_, _| counter.set(counter.get() + 1)));

    // The selection lives in the table; the toolbar is the main view's.
    window.click_row(0, cx);

    assert!(window.availability(cx).start);
    assert!(redraws.get() > 0, "the main view draws its toolbar again");
    assert!(!window.disabled(START, cx));

    // A selection that allows the same actions does not draw the main view again.
    window.click_row(1, cx);
    redraws.set(0);
    window.click_row(2, cx);
    assert_eq!(redraws.get(), 0);
    assert!(window.availability(cx).pause);
}

#[gpui_kit::test]
fn actions_do_nothing_while_disconnected(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![paused("a"), torrent("b")], cx);
    window.press("ctrl-a", cx);
    assert!(window.availability(cx).pause);

    window.send_with(vec![paused("a"), torrent("b")], reconnecting(), cx);

    assert_eq!(
        window.availability(cx),
        Availability {
            copy_hash: true,
            ..Availability::default()
        },
        "only copying the hashes works offline"
    );
    for key in [
        "ctrl-s",
        "ctrl-p",
        "space",
        "ctrl-shift-s",
        "ctrl-shift-r",
        "ctrl-r",
    ] {
        window.press(key, cx);
    }
    assert!(window.disabled(START, cx));
    assert!(window.disabled(PAUSE, cx));
    window.click(START, cx);
    window.click(PAUSE, cx);
    assert_eq!(window.sent(), []);

    // Connected again: the same keys act.
    window.send(vec![paused("a"), torrent("b")], cx);
    window.press("ctrl-p", cx);
    assert_eq!(window.sent_action().1, Action::Stop(hashes(&["a", "b"])));

    // A service that stopped sends nothing more.
    window.events.close();
    cx.run_until_parked();
    window.press("ctrl-p", cx);
    window.click(PAUSE, cx);
    assert_eq!(window.sent(), []);
    assert!(window.disabled(PAUSE, cx));
}

#[gpui_kit::test]
fn delete_and_space_in_the_search_box_edit_text(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![paused("a b"), torrent("b")], cx);
    window.press("ctrl-a", cx);

    window.press("ctrl-f", cx);
    window.act(cx, |window, cx| window.input("a", cx));
    window.press("space", cx);
    window.act(cx, |window, cx| window.input("b", cx));
    window.act(cx, |window, _| {
        assert_eq!(window.find(SEARCH).value(), Some("a b"));
    });
    window.press("home", cx);
    window.press("delete", cx);
    window.act(cx, |window, _| {
        assert_eq!(window.find(SEARCH).value(), Some(" b"));
    });

    assert_eq!(window.sent(), [], "the keys never reached the torrents");
}

#[gpui_kit::test]
fn a_failed_action_is_shown_with_its_count(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![paused("a"), paused("b"), torrent("c")], cx);
    window.press("ctrl-a", cx);
    window.press("ctrl-s", cx);
    let (id, _) = window.sent_action();
    assert_eq!(window.last_failure(cx), None);

    window.finish(
        id,
        Err(ActionError::Rpc(ConnectionError::Server(
            "unrecognized info".to_owned(),
        ))),
        cx,
    );

    assert_eq!(
        window.last_failure(cx).as_deref(),
        Some("Could not start 3 torrents: unrecognized info")
    );
    assert!(window.notified(cx));

    // A success shows nothing: the next snapshot shows what changed.
    window.click_row(2, cx);
    window.press("ctrl-p", cx);
    let (id, _) = window.sent_action();
    window.finish(id, Ok(()), cx);
    assert_eq!(
        window.last_failure(cx).as_deref(),
        Some("Could not start 3 torrents: unrecognized info"),
        "the failure before stays the last one"
    );

    // A refused action: the daemon went away before it was sent.
    window.press("ctrl-shift-r", cx);
    let (id, _) = window.sent_action();
    window.finish(id, Err(ActionError::NotConnected), cx);
    assert_eq!(
        window.last_failure(cx).as_deref(),
        Some("Could not verify the torrent: the server is not connected, so nothing was sent")
    );
}

#[gpui_kit::test]
fn a_timed_out_action_says_it_may_have_happened(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![downloading("a"), torrent("b")], cx);
    window.press("ctrl-a", cx);
    window.press("ctrl-p", cx);
    let (id, _) = window.sent_action();

    window.finish(id, Err(ActionError::Rpc(ConnectionError::Timeout)), cx);

    assert_eq!(
        window.last_failure(cx).as_deref(),
        Some(
            "The server did not answer in time while pausing 2 torrents. They may have been \
             paused anyway; the list will show the result"
        )
    );
    assert!(window.notified(cx));
}
