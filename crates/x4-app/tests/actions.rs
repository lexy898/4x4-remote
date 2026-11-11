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
    AnyWindowHandle, App, AppContext as _, Entity, Focusable as _, InputEvent as _, KeyContext,
    KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, TestAppContext,
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

    /// Clicks the row `row` with the key that toggles it in the selection: Ctrl, and on macOS
    /// Command, as gpui's secondary key, which the table reads.
    fn ctrl_click_row(&self, row: usize, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| {
            window.click_with_modifiers(("row", row), Modifiers::secondary_key(cx), cx);
        });
    }

    fn press(&self, key: &str, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| window.press(key, cx));
    }

    /// Sends a key-down of `key`; `held` marks a repeat of a held key, as Wayland sends it.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a key that does not parse fails the test"
    )]
    fn key_down(&self, key: &str, held: bool, cx: &mut TestAppContext) {
        let down = KeyDownEvent {
            keystroke: Keystroke::parse_with_os(key, cx.update(|cx| cx.platform_info().os))
                .expect("a valid key"),
            is_held: held,
            prefer_character_input: false,
        };
        self.act(cx, |window, cx| {
            window.dispatch_event(down.to_platform_input(), cx)
        });
    }

    /// Sends the key-up of `key`.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a key that does not parse fails the test"
    )]
    fn key_up(&self, key: &str, cx: &mut TestAppContext) {
        let up = KeyUpEvent {
            keystroke: Keystroke::parse_with_os(key, cx.update(|cx| cx.platform_info().os))
                .expect("a valid key"),
        };
        self.act(cx, |window, cx| {
            window.dispatch_event(up.to_platform_input(), cx)
        });
    }

    /// Holds `key` down until the system has repeated it `repeats` times, then lets it go.
    fn hold(&self, key: &str, repeats: usize, cx: &mut TestAppContext) {
        self.key_down(key, false, cx);
        for _ in 0..repeats {
            self.key_down(key, true, cx);
        }
        self.key_up(key, cx);
    }

    /// Whether the toolbar button `id` is drawn disabled.
    fn disabled(&self, id: &'static str, cx: &mut TestAppContext) -> bool {
        self.act(cx, |window, _| window.find(id).disabled() == Some(true))
    }

    /// Whether the table has the keyboard focus, so that the list's keys reach it.
    fn table_focused(&self, cx: &mut TestAppContext) -> bool {
        let table = cx.update(|cx| self.view.read(cx).table().clone());
        self.act(cx, |window, cx| table.focus_handle(cx).is_focused(window))
    }

    fn availability(&self, cx: &mut TestAppContext) -> Availability {
        cx.update(|cx| self.view.read(cx).availability())
    }

    fn last_failure(&self, cx: &mut TestAppContext) -> Option<String> {
        cx.update(|cx| self.view.read(cx).last_failure().map(str::to_owned))
    }

    /// Whether a notification is on screen.
    fn notified(&self, cx: &mut TestAppContext) -> bool {
        self.notifications(cx) > 0
    }

    /// How many notifications are on screen.
    fn notifications(&self, cx: &mut TestAppContext) -> usize {
        self.act(cx, |window, _| window.find_all(NOTIFICATION).len())
    }

    /// The commands the window has sent so far, other than the window's visibility and the
    /// torrent whose details to read, which the `details` tests cover.
    fn sent(&self) -> Vec<Command> {
        self.commands
            .try_iter()
            .filter(|command| !matches!(command, Command::SetVisibility(_) | Command::Select(_)))
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

    window.press("secondary-s", cx);

    let (_, action) = window.sent_action();
    assert_eq!(action, Action::Start(hashes(&["a", "c", "d"])));
}

#[gpui_kit::test]
fn space_pauses_when_any_target_runs(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![paused("a"), downloading("b"), paused("c")], cx);

    window.press("secondary-a", cx);
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
    window.press("secondary-a", cx);

    window.press("secondary-shift-s", cx);
    let (start_now, action) = window.sent_action();
    assert_eq!(action, Action::StartNow(hashes(&["a", "b", "c"])));
    window.press("secondary-shift-r", cx);
    let (verify, action) = window.sent_action();
    assert_eq!(action, Action::Verify(hashes(&["a", "b", "c"])));
    window.press("secondary-r", cx);
    let (reannounce, action) = window.sent_action();
    assert_eq!(action, Action::Reannounce(hashes(&["a", "b", "c"])));
    window.press("secondary-p", cx);
    let (pause, action) = window.sent_action();
    assert_eq!(action, Action::Stop(hashes(&["a", "b", "c"])));

    let ids: HashSet<ActionId> = [start_now, verify, reannounce, pause].into();
    assert_eq!(ids.len(), 4, "every action has its own id: {ids:?}");

    // Reannouncing reaches only torrents that run, and starting now only those that do not.
    window.click_row(0, cx);
    window.press("secondary-r", cx);
    assert_eq!(window.sent(), [], "a paused torrent is not announced");
    window.click_row(1, cx);
    window.press("secondary-shift-s", cx);
    window.press("secondary-s", cx);
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
    window.act(cx, |window, cx| {
        assert_eq!(window.find(START).label(), Some("Start"));
        assert_eq!(window.find(PAUSE).label(), Some("Pause"));
        let list = KeyContext::parse("TorrentList").expect("a key context");
        for (action, key) in [
            (&StartTorrents as &dyn gpui_kit::Action, "s"),
            (&PauseTorrents, "p"),
        ] {
            let binding = window
                .highest_precedence_binding_for_action_in_context(action, list.clone())
                .expect("the action has a key in the list");
            let shown: Vec<String> = binding
                .keystrokes()
                .iter()
                .map(|keystroke| keystroke.unparse(cx.platform_info().os))
                .collect();
            let modifier = if cfg!(target_os = "macos") {
                "cmd"
            } else {
                "ctrl"
            };
            assert_eq!(shown, [format!("{modifier}-{key}")]);
        }
    });
}

#[gpui_kit::test]
fn a_toolbar_button_hands_the_keys_back_to_the_list(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![downloading("a"), torrent("b")], cx);
    window.click_row(0, cx);

    // A click leaves the focus in the list.
    window.click(PAUSE, cx);
    let (id, action) = window.sent_action();
    assert_eq!(action, Action::Stop(hashes(&["a"])));
    assert!(window.table_focused(cx));
    window.finish(id, Ok(()), cx);

    // Pressed with the keyboard, the button gives the focus back to the list. From the list, Tab
    // passes the controls of the details panel and the status bar, and the search box; how many
    // there are is not the point, so it goes on until Pause, within a sane number of presses.
    let pause_focused =
        |cx: &mut TestAppContext| window.act(cx, |window, _| window.find(PAUSE).focused());
    let mut presses = 0;
    while pause_focused(cx) != Some(true) {
        assert!(presses < 20, "Tab never reached Pause");
        window.press("tab", cx);
        presses += 1;
    }
    window.press("space", cx);
    let (id, action) = window.sent_action();
    assert_eq!(action, Action::Stop(hashes(&["a"])));
    assert!(window.table_focused(cx), "the list has the focus again");
    window.finish(id, Ok(()), cx);

    // Space now reaches the list: with the torrent paused, it starts it.
    window.send(vec![paused("a"), torrent("b")], cx);
    assert!(window.disabled(PAUSE, cx));
    window.press("space", cx);
    assert_eq!(window.sent_action().1, Action::Start(hashes(&["a"])));

    // A click on a disabled button does not take the keys either.
    window.click(PAUSE, cx);
    assert!(window.table_focused(cx));
    window.press("down", cx);
    window.press("secondary-p", cx);
    assert_eq!(window.sent_action().1, Action::Stop(hashes(&["b"])));
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
    window.press("secondary-a", cx);
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
        "secondary-s",
        "secondary-p",
        "space",
        "secondary-shift-s",
        "secondary-shift-r",
        "secondary-r",
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
    window.press("secondary-p", cx);
    assert_eq!(window.sent_action().1, Action::Stop(hashes(&["a", "b"])));

    // A service that stopped sends nothing more.
    window.events.close();
    cx.run_until_parked();
    window.press("secondary-p", cx);
    window.click(PAUSE, cx);
    assert_eq!(window.sent(), []);
    assert!(window.disabled(PAUSE, cx));
}

#[gpui_kit::test]
fn holding_space_sends_one_command(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![downloading("a"), torrent("b")], cx);
    window.click_row(0, cx);

    window.key_down("space", false, cx);
    let (id, action) = window.sent_action();
    assert_eq!(action, Action::Stop(hashes(&["a"])));
    // The daemon answers before the key repeats, and the list still shows the torrent running,
    // so each repeat would pause it again.
    window.finish(id, Ok(()), cx);
    for _ in 0..5 {
        window.key_down("space", true, cx);
    }
    assert_eq!(window.sent(), [], "the repeats of a held key send nothing");
    window.key_up("space", cx);

    // Let go and pressed again, the key acts again.
    window.press("space", cx);
    let (id, action) = window.sent_action();
    assert_eq!(action, Action::Stop(hashes(&["a"])));
    window.finish(id, Ok(()), cx);

    // X11 does not mark its repeats, but sends no key-up between them either.
    window.key_down("secondary-shift-r", false, cx);
    let (id, action) = window.sent_action();
    assert_eq!(action, Action::Verify(hashes(&["a"])));
    window.finish(id, Ok(()), cx);
    window.key_down("secondary-shift-r", false, cx);
    assert_eq!(
        window.sent(),
        [],
        "every action key works so, not only Space"
    );
    window.key_up("secondary-shift-r", cx);

    // Keys that are not actions still repeat: a held Down moves on with every repeat.
    window.send(
        vec![downloading("a"), torrent("b"), torrent("c"), torrent("d")],
        cx,
    );
    window.hold("down", 2, cx);
    window.press("secondary-r", cx);
    assert_eq!(window.sent_action().1, Action::Reannounce(hashes(&["d"])));
}

#[gpui_kit::test]
fn a_key_let_go_out_of_the_lists_sight_does_not_hold_the_next_press(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    // A test window opens inactive, and an inactive window has no focus to move.
    cx.update_window(window.handle, |_, window, _| window.activate_window())
        .expect("the window is open");
    window.send(vec![downloading("a"), torrent("b")], cx);
    window.click_row(0, cx);
    let pause_and_answer = |cx: &mut TestAppContext| {
        window.key_down("space", false, cx);
        let (id, action) = window.sent_action();
        assert_eq!(action, Action::Stop(hashes(&["a"])));
        window.finish(id, Ok(()), cx);
    };

    // Something outside the main view, such as a dialog, has the focus when the key goes up.
    pause_and_answer(cx);
    let elsewhere = cx.update(|cx| cx.focus_handle());
    window.act(cx, |window, cx| window.focus(&elsewhere, cx));
    window.key_up("space", cx);
    let table = cx.update(|cx| window.view.read(cx).table().clone());
    window.act(cx, |window, cx| window.focus(&table.focus_handle(cx), cx));
    assert!(window.table_focused(cx), "back in the list");
    pause_and_answer(cx);

    // The key goes up in another window.
    let other = cx.add_window(|_, _| gpui_kit::Empty);
    cx.update_window(other.into(), |_, other, _| other.activate_window())
        .expect("the other window is open");
    cx.update_window(window.handle, |_, window, _| window.activate_window())
        .expect("the window is open");
    cx.run_until_parked();
    pause_and_answer(cx);
}

#[gpui_kit::test]
fn an_identical_pending_request_is_not_sent_again(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![downloading("a"), downloading("b"), paused("c")], cx);
    window.click_row(0, cx);
    window.ctrl_click_row(1, cx);
    window.press("secondary-p", cx);
    let (first, action) = window.sent_action();
    assert_eq!(action, Action::Stop(hashes(&["a", "b"])));

    // Pressed again, or Pause clicked twice, before the daemon has answered: the same pause is on
    // its way already.
    window.press("secondary-p", cx);
    window.press("space", cx);
    window.click(PAUSE, cx);
    window.click(PAUSE, cx);
    assert_eq!(window.sent(), []);

    // Another action on the same torrents goes, and so does the same action on others.
    window.press("secondary-r", cx);
    assert_eq!(
        window.sent_action().1,
        Action::Reannounce(hashes(&["a", "b"]))
    );
    window.click_row(0, cx);
    window.press("secondary-p", cx);
    assert_eq!(window.sent_action().1, Action::Stop(hashes(&["a"])));

    // Once the daemon has answered, the same pause can go again.
    window.finish(first, Ok(()), cx);
    window.ctrl_click_row(1, cx);
    window.press("secondary-p", cx);
    assert_eq!(window.sent_action().1, Action::Stop(hashes(&["a", "b"])));
}

#[gpui_kit::test]
fn a_repeated_choice_after_a_different_action_is_sent(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![downloading("a"), paused("b")], cx);
    window.press("secondary-a", cx);

    // Pause, then Start, then Pause again, before the daemon has answered any of them: the last
    // press is what the user wants, and the daemon runs the commands in order.
    window.press("secondary-p", cx);
    window.press("secondary-s", cx);
    window.press("secondary-p", cx);

    let sent: Vec<Action> = window
        .sent()
        .into_iter()
        .filter_map(|command| match command {
            Command::Run(_, action) => Some(action),
            _ => None,
        })
        .collect();
    let both = hashes(&["a", "b"]);
    assert_eq!(
        sent,
        [
            Action::Stop(both.clone()),
            Action::Start(both.clone()),
            Action::Stop(both)
        ]
    );
}

#[gpui_kit::test]
fn a_selected_row_the_search_hides_is_not_acted_on(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![paused("alpha"), paused("beta")], cx);
    window.press("secondary-a", cx);

    window.press("secondary-f", cx);
    window.act(cx, |window, cx| window.input("beta", cx));
    window.click(START, cx);

    assert_eq!(window.sent_action().1, Action::Start(hashes(&["beta"])));
}

#[gpui_kit::test]
fn delete_and_space_in_the_search_box_edit_text(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![paused("a b"), torrent("b")], cx);
    window.press("secondary-a", cx);

    window.press("secondary-f", cx);
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
    window.press("secondary-a", cx);
    window.press("secondary-s", cx);
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
    window.press("secondary-p", cx);
    let (id, _) = window.sent_action();
    window.finish(id, Ok(()), cx);
    assert_eq!(
        window.last_failure(cx).as_deref(),
        Some("Could not start 3 torrents: unrecognized info"),
        "the failure before stays the last one"
    );

    // A refused action: the daemon went away before it was sent.
    window.press("secondary-shift-r", cx);
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
    window.press("secondary-a", cx);
    window.press("secondary-p", cx);
    let (id, _) = window.sent_action();

    window.finish(id, Err(ActionError::Rpc(ConnectionError::Timeout)), cx);

    assert_eq!(
        window.last_failure(cx).as_deref(),
        Some(
            "The server did not answer while pausing 2 torrents; they may have been paused. The \
             list will show the result"
        )
    );
    assert!(window.notified(cx));
}

#[gpui_kit::test]
fn a_pause_failure_and_a_verify_failure_are_both_shown(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![downloading("a"), torrent("b")], cx);
    window.click_row(0, cx);
    window.press("secondary-p", cx);
    let (pause, _) = window.sent_action();
    window.press("secondary-shift-r", cx);
    let (verify, _) = window.sent_action();

    let busy = || Err(ActionError::Rpc(ConnectionError::Server("busy".to_owned())));
    window.finish(pause, busy(), cx);
    window.finish(verify, busy(), cx);

    assert_eq!(window.notifications(cx), 2, "one for each kind of action");
    assert_eq!(
        window.last_failure(cx).as_deref(),
        Some("Could not verify the torrent: busy")
    );

    // A newer failure of a pause takes the place of the older one.
    window.press("secondary-p", cx);
    let (pause, _) = window.sent_action();
    window.finish(pause, Err(ActionError::Rpc(ConnectionError::Timeout)), cx);
    assert_eq!(window.notifications(cx), 2);
    assert_eq!(
        window.last_failure(cx).as_deref(),
        Some(
            "The server did not answer while pausing the torrent; it may have been paused. The \
             list will show the result"
        )
    );
}

#[gpui_kit::test]
fn an_action_the_service_cannot_take_says_nothing_was_sent(cx: &mut TestAppContext) {
    let mut window = Window::open(cx);
    window.send(vec![downloading("a"), downloading("b")], cx);
    window.press("secondary-a", cx);
    // The service's end of the commands is gone, while the window still thinks it connected.
    drop(std::mem::replace(
        &mut window.commands,
        crossbeam_channel::never(),
    ));
    assert!(window.availability(cx).pause);

    window.press("secondary-p", cx);

    assert_eq!(
        window.last_failure(cx).as_deref(),
        Some("Could not pause 2 torrents: the server is not connected, so nothing was sent")
    );
    assert!(window.notified(cx));
}
