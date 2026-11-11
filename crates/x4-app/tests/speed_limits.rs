//! Headless tests for the turtle and the global speed limits in the status bar: switching the
//! alternative speed limits, the popover that sets the normal limits, what it sends and what it
//! refuses, and what the controls do while the daemon or its session is not there.
//!
//! Each test builds the two channels of a service itself and opens the window with no service
//! behind them, so nothing touches the network: the test reads the commands the window sends and
//! answers them with the events a service would send.

use std::sync::Arc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Entity, Focusable as _, InputEvent as _, KeyContext,
    KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, OperatingSystem, TestAppContext, VisualTestContext, point, px,
};
use x4_app::app;
use x4_app::keymap::{EditSpeedLimits, ToggleAltSpeed};
use x4_app::main_view::MainView;
use x4_core::test_support::torrent;
use x4_core::{
    Action, ActionError, ActionId, Command, ConnectionError, ConnectionState, DetailsState,
    DetailsTab, Direction, DisplayState, Event, KbPerSecond, SessionSummary, Snapshot, SpeedLimit,
    Torrent, UiState,
};

/// The turtle: the alternative speed limits.
const TURTLE: &str = "alt-speed";
/// The button that shows the normal limits and opens their popover.
const LIMITS: &str = "speed-limits";
/// The download limit's field, its switch and the line under it.
const DOWN: &str = "limit-down";
const DOWN_SWITCH: &str = "limit-down-enabled";
const DOWN_FEEDBACK: &str = "limit-down-feedback";
/// The upload limit's field, its switch and the line under it.
const UP: &str = "limit-up";
const UP_SWITCH: &str = "limit-up-enabled";
const UP_FEEDBACK: &str = "limit-up-feedback";
/// The popover's Apply button.
const APPLY: &str = "limits-apply";
/// The search box.
const SEARCH: &str = "search";
/// The free space in the status bar, a place to click beside the popover.
const FREE_SPACE: &str = "free-space";
/// The id of an open menu: the context menu, or a menu of the toolbar.
const MENU: &str = "popup-menu";
/// The toolbar's Remove button and Columns button, each of which opens a menu.
const REMOVE: &str = "remove-torrents";
const COLUMNS: &str = "column-menu";
/// The id of a notification.
const NOTIFICATION: &str = "notification";
/// The details panel under the table.
const DETAILS: &str = "details";
/// What a field says under it when its text is not a whole number.
const NOT_A_NUMBER: &str = "Enter a whole number of KiB/s";
/// What a field says under it when its number is 0 or too large.
const OUT_OF_RANGE: &str = "Enter a speed from 1 to 1,000,000 KiB/s";
/// The key that selects the whole text of a field. gpui-base's text fields take ⌘A on macOS,
/// where Ctrl+A moves to the start of the line, and Ctrl+A elsewhere.
const SELECT_ALL: &str = if cfg!(target_os = "macos") {
    "cmd-a"
} else {
    "ctrl-a"
};

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
        // A test window opens inactive, and only an active window tells where the focus goes:
        // the popover closes when the focus leaves it.
        cx.update_window(handle, |_, window, _| window.activate_window())
            .expect("the window is open");
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

    /// Sends a snapshot without torrents, with the daemon's `session` and the connection in
    /// `connection`.
    fn send_session(
        &self,
        session: Option<SessionSummary>,
        connection: ConnectionState,
        cx: &mut TestAppContext,
    ) {
        self.send_snapshot(Vec::new(), session, connection, cx);
    }

    /// Sends a snapshot of `torrents`, with the daemon's `session` and the connection in
    /// `connection`.
    fn send_snapshot(
        &self,
        torrents: Vec<Torrent>,
        session: Option<SessionSummary>,
        connection: ConnectionState,
        cx: &mut TestAppContext,
    ) {
        let stale = connection != ConnectionState::Connected;
        self.send_event(
            Event::Snapshot(Arc::new(Snapshot {
                torrents,
                listed: true,
                session,
                free_space_bytes: Some(24_592_841_836),
                rate_down: 2_516_582,
                rate_up: 184_320,
                connection,
                stale,
            })),
            cx,
        );
    }

    /// Sends a snapshot of the connected daemon with the limits `down` and `up`, and the
    /// alternative limits `alt_speed`.
    fn send_limits(
        &self,
        down: SpeedLimit,
        up: SpeedLimit,
        alt_speed: bool,
        cx: &mut TestAppContext,
    ) {
        self.send_session(
            Some(session(down, up, alt_speed)),
            ConnectionState::Connected,
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

    fn click(&self, id: &'static str, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| window.click(id, cx));
    }

    /// Clicks the blank space between the elements `left` and `right`, at the height of `right`.
    #[track_caller]
    fn click_between(&self, left: &'static str, right: &'static str, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| {
            let left = window.find(left).bounds();
            let right = window.find(right).bounds();
            assert!(left.right() < right.left(), "{left:?} {right:?}: no gap");
            let position = point((left.right() + right.left()) / 2., right.center().y);
            let modifiers = Modifiers::default();
            window.dispatch_event(
                MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers,
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseDownEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers,
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseUpEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers,
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
        });
    }

    /// Makes another app's window the active one, as when the user switches away from 4x4.
    fn deactivate(&self, cx: &mut TestAppContext) {
        VisualTestContext::from_window(self.handle, cx).deactivate_window();
        cx.run_until_parked();
    }

    /// Makes the window the active one again, as when the user switches back to 4x4.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a closed window fails the test"
    )]
    fn activate(&self, cx: &mut TestAppContext) {
        cx.update_window(self.handle, |_, window, _| window.activate_window())
            .expect("the window is open");
        cx.run_until_parked();
    }

    /// Sends a key-down of `key` that types its text when nothing takes the key, as a repeat of
    /// a held Space does in a field.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a key that does not parse fails the test"
    )]
    fn key_down_typing(&self, key: &str, cx: &mut TestAppContext) {
        let keystroke = Keystroke::parse_with_os(key, OperatingSystem::Linux).expect("a valid key");
        self.act(cx, |window, cx| window.dispatch_keystroke(keystroke, cx));
    }

    fn press(&self, key: &str, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| window.press(key, cx));
    }

    /// Types `text` into what has the focus.
    fn type_text(&self, text: &str, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| window.input(text, cx));
    }

    /// Replaces the text of the field `id` with `text`, as the user would: a click into it,
    /// [`SELECT_ALL`] and the new text.
    fn retype(&self, id: &'static str, text: &str, cx: &mut TestAppContext) {
        self.click(id, cx);
        self.press(SELECT_ALL, cx);
        if text.is_empty() {
            self.press("backspace", cx);
        } else {
            self.type_text(text, cx);
        }
    }

    /// Sends a key-down of `key`; `held` marks a repeat of a held key, as Wayland sends it.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a key that does not parse fails the test"
    )]
    fn key_down(&self, key: &str, held: bool, cx: &mut TestAppContext) {
        let down = KeyDownEvent {
            keystroke: Keystroke::parse_with_os(key, OperatingSystem::Linux).expect("a valid key"),
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
            keystroke: Keystroke::parse_with_os(key, OperatingSystem::Linux).expect("a valid key"),
        };
        self.act(cx, |window, cx| {
            window.dispatch_event(up.to_platform_input(), cx)
        });
    }

    /// Whether the element `id` is drawn disabled.
    fn disabled(&self, id: &'static str, cx: &mut TestAppContext) -> bool {
        self.act(cx, |window, _| window.find(id).disabled() == Some(true))
    }

    /// Whether the element `id` has the keyboard focus.
    fn focused(&self, id: &'static str, cx: &mut TestAppContext) -> bool {
        self.act(cx, |window, _| window.find(id).focused() == Some(true))
    }

    /// Whether the switch or toggle `id` is on, as a screen reader is told.
    fn checked(&self, id: &'static str, cx: &mut TestAppContext) -> Option<bool> {
        self.act(cx, |window, _| window.find(id).checked())
    }

    /// The text of the field `id`.
    fn value(&self, id: &'static str, cx: &mut TestAppContext) -> Option<String> {
        self.act(cx, |window, _| window.find(id).value().map(str::to_owned))
    }

    /// The accessible name of `id`, which is what a text or a button shows.
    fn label(&self, id: &'static str, cx: &mut TestAppContext) -> Option<String> {
        self.act(cx, |window, _| window.find(id).label().map(str::to_owned))
    }

    /// What the line under the field of `feedback` says; `None` when it says nothing.
    fn feedback(&self, feedback: &'static str, cx: &mut TestAppContext) -> Option<String> {
        self.act(cx, |window, _| {
            window
                .try_find(feedback)
                .and_then(|line| line.label().map(str::to_owned))
        })
    }

    /// Whether the context menu is open.
    fn menu_open(&self, cx: &mut TestAppContext) -> bool {
        self.act(cx, |window, _| window.try_find(MENU).is_some())
    }

    /// Whether the popover of the limits is open.
    fn limits_open(&self, cx: &mut TestAppContext) -> bool {
        self.act(cx, |window, _| window.try_find(DOWN).is_some())
    }

    /// What the window knows of the details panel: whether it is shown, how high, which tab.
    fn details(&self, cx: &mut TestAppContext) -> DetailsState {
        cx.update(|cx| self.view.read(cx).ui_state().details)
    }

    /// Whether the details panel is drawn.
    fn details_shown(&self, cx: &mut TestAppContext) -> bool {
        self.act(cx, |window, _| window.try_find(DETAILS).is_some())
    }

    /// Whether the table has the keyboard focus, so that the list's keys reach it.
    fn table_focused(&self, cx: &mut TestAppContext) -> bool {
        let table = cx.update(|cx| self.view.read(cx).table().clone());
        self.act(cx, |window, cx| table.focus_handle(cx).is_focused(window))
    }

    fn last_failure(&self, cx: &mut TestAppContext) -> Option<String> {
        cx.update(|cx| self.view.read(cx).last_failure().map(str::to_owned))
    }

    /// How many notifications are on screen.
    fn notifications(&self, cx: &mut TestAppContext) -> usize {
        self.act(cx, |window, _| window.find_all(NOTIFICATION).len())
    }

    /// The commands the window has sent so far, other than the window's visibility, the torrent
    /// whose details to read and the tab shown, which the `details` tests cover.
    fn sent(&self) -> Vec<Command> {
        self.commands
            .try_iter()
            .filter(|command| {
                !matches!(
                    command,
                    Command::SetVisibility(_) | Command::Select(_) | Command::ShowTab(_)
                )
            })
            .collect()
    }

    /// The actions the window has sent since the last look, in order.
    fn sent_actions(&self) -> Vec<Action> {
        self.sent()
            .into_iter()
            .filter_map(|command| match command {
                Command::Run(_, action) => Some(action),
                _ => None,
            })
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

fn limit(kbps: u32, enabled: bool) -> SpeedLimit {
    SpeedLimit {
        kbps: KbPerSecond(kbps),
        enabled,
    }
}

/// A limit that is off, with 100 kB/s stored: Transmission's default.
fn off() -> SpeedLimit {
    limit(100, false)
}

/// The daemon's session with the normal limits `down` and `up`, and the alternative limits of
/// 50 kB/s each, applied when `alt_speed` is on.
fn session(down: SpeedLimit, up: SpeedLimit, alt_speed: bool) -> SessionSummary {
    SessionSummary {
        version: "4.1.3".to_owned(),
        download_dir: "/downloads".to_owned(),
        alt_speed_enabled: alt_speed,
        speed_limit_down: down,
        speed_limit_up: up,
        alt_speed_down_kbps: KbPerSecond(50),
        alt_speed_up_kbps: KbPerSecond(50),
        start_added_torrents: true,
        sequential_download_supported: true,
        lists_empty_files: true,
    }
}

fn set_limit(direction: Direction, kbps: Option<u32>) -> Action {
    Action::SetSpeedLimit {
        direction,
        limit: kbps.map(KbPerSecond),
    }
}

fn reconnecting() -> ConnectionState {
    ConnectionState::Reconnecting {
        attempt: 2,
        last_error: ConnectionError::Timeout,
    }
}

#[gpui_kit::test]
fn turtle_click_and_ctrl_t_toggle_alt_speed(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send_limits(off(), off(), false, cx);
    assert_eq!(window.checked(TURTLE, cx), Some(false));
    assert_eq!(
        window.label(TURTLE, cx).as_deref(),
        Some("Alternative speed limits")
    );

    window.click(TURTLE, cx);
    let (id, action) = window.sent_action();
    assert_eq!(action, Action::SetAltSpeed(true));
    assert!(
        window.table_focused(cx),
        "a click hands the keys back to the list"
    );
    // Before the daemon has answered, a second click would only repeat the request.
    window.click(TURTLE, cx);
    assert_eq!(window.sent(), []);
    window.finish(id, Ok(()), cx);

    window.send_limits(off(), off(), true, cx);
    assert_eq!(window.checked(TURTLE, cx), Some(true), "the turtle is on");
    window.press("ctrl-t", cx);
    assert_eq!(window.sent_action().1, Action::SetAltSpeed(false));
}

#[gpui_kit::test]
fn a_held_ctrl_t_switches_the_turtle_once(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send_limits(off(), off(), false, cx);

    window.key_down("ctrl-t", false, cx);
    let (id, action) = window.sent_action();
    assert_eq!(action, Action::SetAltSpeed(true));
    window.finish(id, Ok(()), cx);
    window.send_limits(off(), off(), true, cx);
    // The repeats of the held key come after the turtle went on: they do not switch it off.
    window.key_down("ctrl-t", true, cx);
    window.key_down("ctrl-t", true, cx);
    window.key_up("ctrl-t", cx);
    assert_eq!(window.sent(), []);
}

#[gpui_kit::test]
fn turtle_is_disabled_without_a_session(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let nothing_happens = |window: &Window, cx: &mut TestAppContext| {
        assert!(window.disabled(TURTLE, cx));
        assert!(window.disabled(LIMITS, cx));
        window.click(TURTLE, cx);
        window.click(LIMITS, cx);
        window.press("ctrl-t", cx);
        window.press("ctrl-l", cx);
        assert_eq!(window.sent(), []);
        assert!(!window.limits_open(cx));
    };

    // Connected, before the daemon's settings have come.
    window.send_session(None, ConnectionState::Connected, cx);
    nothing_happens(&window, cx);
    assert_eq!(window.label(LIMITS, cx).as_deref(), Some("Speed limits"));
    // The settings are known, but the daemon does not answer.
    window.send_session(Some(session(off(), off(), false)), reconnecting(), cx);
    nothing_happens(&window, cx);

    window.send_limits(off(), off(), false, cx);
    assert!(!window.disabled(TURTLE, cx));
    assert!(!window.disabled(LIMITS, cx));
}

#[gpui_kit::test]
fn limits_popover_prefills_the_stored_values(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    // On at 1024 kB/s, which are 1000 KiB/s; off, with 100 kB/s stored, which are 98 KiB/s.
    window.send_limits(limit(1024, true), limit(100, false), false, cx);

    window.press("ctrl-l", cx);
    assert!(window.limits_open(cx));
    assert_eq!(window.value(DOWN, cx).as_deref(), Some("1000"));
    assert_eq!(window.checked(DOWN_SWITCH, cx), Some(true));
    assert_eq!(window.value(UP, cx).as_deref(), Some("98"));
    assert_eq!(window.checked(UP_SWITCH, cx), Some(false));
    assert!(
        window.focused(DOWN, cx),
        "the download limit is ready to type"
    );
    assert_eq!(window.feedback(DOWN_FEEDBACK, cx), None);
    assert_eq!(window.feedback(UP_FEEDBACK, cx), None);

    // What is typed replaces the value; Escape closes the popover and sends nothing.
    window.type_text("5", cx);
    assert_eq!(window.value(DOWN, cx).as_deref(), Some("5"));
    window.press("escape", cx);
    assert!(!window.limits_open(cx));
    assert_eq!(window.sent(), []);
    assert!(window.table_focused(cx));

    // A click on the button opens it again, filled from the daemon's values once more.
    window.click(LIMITS, cx);
    assert!(window.limits_open(cx));
    assert_eq!(window.value(DOWN, cx).as_deref(), Some("1000"));
    // Ctrl+L while it is open leaves it as it is.
    window.type_text("7", cx);
    window.press("ctrl-l", cx);
    assert!(window.limits_open(cx));
    assert_eq!(window.value(DOWN, cx).as_deref(), Some("7"));
}

#[gpui_kit::test]
fn applying_one_limit_sends_only_that_direction_in_kbps(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send_limits(off(), off(), false, cx);

    // Typing a speed switches its limit on; 512 KiB/s are 524 kB/s.
    window.press("ctrl-l", cx);
    window.type_text("512", cx);
    assert_eq!(window.checked(DOWN_SWITCH, cx), Some(true));
    window.press("enter", cx);
    assert_eq!(
        window.sent_actions(),
        [set_limit(Direction::Down, Some(524))]
    );
    assert!(!window.limits_open(cx));
    assert!(window.table_focused(cx), "the keys go back to the list");

    // The upload limit, from the Apply button: 1000 KiB/s are 1024 kB/s.
    window.press("ctrl-l", cx);
    window.retype(UP, "1000", cx);
    window.click(APPLY, cx);
    assert_eq!(
        window.sent_actions(),
        [set_limit(Direction::Up, Some(1024))]
    );
    assert!(!window.limits_open(cx));
    assert!(window.table_focused(cx));

    // Untouched, nothing is sent.
    window.press("ctrl-l", cx);
    window.press("enter", cx);
    assert_eq!(window.sent(), []);
    assert!(!window.limits_open(cx));
}

#[gpui_kit::test]
fn turning_a_limit_off_sends_none(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send_limits(limit(1024, true), limit(512, true), false, cx);

    window.press("ctrl-l", cx);
    window.click(DOWN_SWITCH, cx);
    assert_eq!(window.checked(DOWN_SWITCH, cx), Some(false));
    window.click(APPLY, cx);

    assert_eq!(window.sent_actions(), [set_limit(Direction::Down, None)]);
    assert!(!window.limits_open(cx));
}

#[gpui_kit::test]
fn an_invalid_limit_shows_an_error_and_sends_nothing(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send_limits(off(), off(), false, cx);
    window.press("ctrl-l", cx);
    // A valid upload limit does not go either while the download limit is wrong.
    window.retype(UP, "300", cx);

    for (text, message) in [
        ("abc", NOT_A_NUMBER),
        ("0", OUT_OF_RANGE),
        ("1000001", OUT_OF_RANGE),
    ] {
        window.retype(DOWN, text, cx);
        assert_eq!(window.feedback(DOWN_FEEDBACK, cx), None, "{text}");
        window.press("enter", cx);
        assert_eq!(window.sent(), [], "{text}");
        assert!(window.limits_open(cx), "{text}");
        assert_eq!(window.feedback(DOWN_FEEDBACK, cx).as_deref(), Some(message));
        assert_eq!(window.feedback(UP_FEEDBACK, cx), None, "{text}");
    }
    window.click(APPLY, cx);
    assert_eq!(window.sent(), []);

    window.press("escape", cx);
    assert!(!window.limits_open(cx));
    assert_eq!(window.sent(), []);
}

#[gpui_kit::test]
fn a_limit_switched_on_without_a_speed_asks_for_one(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send_limits(off(), off(), false, cx);
    window.press("ctrl-l", cx);

    window.retype(DOWN, "", cx);
    window.click(DOWN_SWITCH, cx);
    assert_eq!(window.checked(DOWN_SWITCH, cx), Some(true));
    window.click(APPLY, cx);

    assert_eq!(window.sent(), []);
    assert!(window.limits_open(cx));
    assert_eq!(
        window.feedback(DOWN_FEEDBACK, cx).as_deref(),
        Some("Enter a speed to turn the limit on")
    );
    // Switched off again, the hint goes, and Apply only closes the popover.
    window.click(DOWN_SWITCH, cx);
    assert_eq!(window.feedback(DOWN_FEEDBACK, cx), None);
    window.click(APPLY, cx);
    assert_eq!(window.sent(), []);
    assert!(!window.limits_open(cx));
}

#[gpui_kit::test]
fn a_held_enter_that_opened_the_popover_applies_nothing(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send_limits(off(), off(), false, cx);
    // From the list, Tab goes to the turtle and then to the limits button.
    window.press("tab", cx);
    window.press("tab", cx);
    assert!(window.focused(LIMITS, cx));

    // Enter on the button opens the popover; the repeats of the held key reach the field. Had
    // they applied it, the popover would have closed.
    window.key_down("enter", false, cx);
    assert!(window.limits_open(cx));
    assert!(window.focused(DOWN, cx));
    window.key_down("enter", true, cx);
    window.key_down("enter", true, cx);
    assert_eq!(window.sent(), []);
    assert!(window.limits_open(cx), "the repeats applied nothing");
    window.key_up("enter", cx);
    window.type_text("512", cx);

    // A new press applies, once.
    window.key_down("enter", false, cx);
    assert_eq!(
        window.sent_actions(),
        [set_limit(Direction::Down, Some(524))]
    );
    window.key_down("enter", true, cx);
    window.key_up("enter", cx);
    assert_eq!(window.sent(), []);
}

#[gpui_kit::test]
fn an_open_popover_applies_nothing_while_the_daemon_is_away(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send_limits(off(), off(), false, cx);
    window.press("ctrl-l", cx);
    window.type_text("512", cx);

    window.send_session(Some(session(off(), off(), false)), reconnecting(), cx);
    assert!(window.limits_open(cx), "the popover stays open");
    assert!(window.disabled(APPLY, cx));
    window.press("enter", cx);
    window.click(APPLY, cx);
    assert_eq!(window.sent(), []);
    assert!(window.limits_open(cx));
    assert_eq!(window.value(DOWN, cx).as_deref(), Some("512"));

    window.send_limits(off(), off(), false, cx);
    assert!(!window.disabled(APPLY, cx));
    window.press("enter", cx);
    assert_eq!(
        window.sent_actions(),
        [set_limit(Direction::Down, Some(524))]
    );
}

#[gpui_kit::test]
fn the_turtle_and_each_limit_fail_in_notifications_of_their_own(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send_limits(off(), off(), false, cx);
    let refused = || {
        Err(ActionError::Rpc(ConnectionError::Server(
            "refused".to_owned(),
        )))
    };

    window.click(TURTLE, cx);
    let (turtle, _) = window.sent_action();
    window.press("ctrl-l", cx);
    window.type_text("512", cx);
    window.retype(UP, "300", cx);
    window.press("enter", cx);
    let ids: Vec<ActionId> = window
        .sent()
        .into_iter()
        .filter_map(|command| match command {
            Command::Run(id, _) => Some(id),
            _ => None,
        })
        .collect();
    let [down, up] = ids.as_slice() else {
        panic!("one action per limit: {ids:?}");
    };

    window.finish(turtle, refused(), cx);
    assert_eq!(
        window.last_failure(cx).as_deref(),
        Some("Could not turn the alternative speed limits on: refused")
    );
    window.finish(*down, refused(), cx);
    assert_eq!(
        window.last_failure(cx).as_deref(),
        Some("Could not change the download speed limit: refused")
    );
    window.finish(*up, refused(), cx);
    assert_eq!(
        window.last_failure(cx).as_deref(),
        Some("Could not change the upload speed limit: refused")
    );
    assert_eq!(window.notifications(cx), 3);
}

#[gpui_kit::test]
fn the_status_bar_shows_the_limits(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    // 512 kB/s are 500 KiB/s.
    window.send_limits(limit(512, true), limit(100, false), false, cx);
    assert_eq!(
        window.label(LIMITS, cx).as_deref(),
        Some("↓ 500 KiB/s · ↑ no limit")
    );
    window.send_limits(off(), limit(1_024_000, true), false, cx);
    assert_eq!(
        window.label(LIMITS, cx).as_deref(),
        Some("↓ no limit · ↑ 1,000,000 KiB/s")
    );

    // The turtle and the limits come before the free space and the speeds, in one row.
    let order = [
        "alt-speed",
        "speed-limits",
        "free-space",
        "rate-down",
        "rate-up",
    ];
    let bounds = window.act(cx, |window, _| order.map(|id| window.find(id).bounds()));
    for (pair, ids) in bounds.windows(2).zip(order.windows(2)) {
        let [left, right] = pair else { continue };
        assert!(left.right() <= right.left(), "{ids:?}: {left:?} {right:?}");
        let rows_apart = (left.center().y - right.center().y).abs();
        assert!(rows_apart < px(1.), "{ids:?}: {left:?} {right:?}");
    }
}

#[gpui_kit::test]
fn the_keys_work_from_the_search_box_and_are_bound_in_the_window(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send_limits(off(), off(), false, cx);

    window.press("ctrl-f", cx);
    window.press("ctrl-t", cx);
    assert_eq!(window.sent_action().1, Action::SetAltSpeed(true));
    window.press("ctrl-l", cx);
    assert!(window.limits_open(cx));
    window.press("escape", cx);

    window.act(cx, |window, _| {
        let main = KeyContext::parse("MainWindow").expect("a key context");
        for (action, keys) in [
            (&ToggleAltSpeed as &dyn gpui_kit::Action, "ctrl-t"),
            (&EditSpeedLimits, "ctrl-l"),
        ] {
            let binding = window
                .highest_precedence_binding_for_action_in_context(action, main.clone())
                .expect("the action has a key in the window");
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
fn the_speed_keys_do_nothing_behind_the_context_menu(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let downloading = |name| Torrent {
        state: DisplayState::Downloading,
        ..torrent(name)
    };
    window.send_snapshot(
        vec![downloading("a"), downloading("b")],
        Some(session(limit(1024, true), off(), false)),
        ConnectionState::Connected,
        cx,
    );
    window.act(cx, |window, cx| window.right_click(("row", 0usize), cx));
    assert!(window.menu_open(cx));

    for key in ["ctrl-l", "ctrl-t"] {
        window.press(key, cx);
        assert!(!window.limits_open(cx), "{key}");
        assert!(window.menu_open(cx), "{key}: the menu keeps the keys");
        assert_eq!(window.sent(), [], "{key}");
    }
    // Pause, behind the menu, does nothing either; closed, the menu gives the keys back.
    window.press("ctrl-p", cx);
    assert_eq!(window.sent(), []);
    window.press("escape", cx);
    assert!(!window.menu_open(cx));
    assert!(window.table_focused(cx));
}

/// The toolbar's menus keep the keys as the context menu does: Ctrl+T and Ctrl+L, which work in
/// the whole window, neither switch the turtle nor open the speed limits behind them.
#[gpui_kit::test]
fn the_speed_keys_do_nothing_behind_a_toolbar_menu(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send_snapshot(
        vec![torrent("a"), torrent("b")],
        Some(session(limit(1024, true), off(), false)),
        ConnectionState::Connected,
        cx,
    );
    // The Remove menu needs a selection.
    window.act(cx, |window, cx| window.click(("row", 0usize), cx));

    for button in [REMOVE, COLUMNS] {
        window.click(button, cx);
        assert!(window.menu_open(cx), "{button}");
        for key in ["ctrl-t", "ctrl-l"] {
            window.press(key, cx);
            assert_eq!(window.sent(), [], "{button}, {key}");
            assert!(!window.limits_open(cx), "{button}, {key}");
            assert!(
                window.menu_open(cx),
                "{button}, {key}: the menu keeps the keys"
            );
        }
        window.press("escape", cx);
        assert!(!window.menu_open(cx), "{button}");
    }

    // Closed, the menus let the keys work again.
    window.press("ctrl-t", cx);
    assert_eq!(window.sent_actions(), [Action::SetAltSpeed(true)]);
}

/// The popover keeps the keys of the details panel, as a menu does: Alt+Enter, Alt+1, Alt+2, Alt+3
/// and Alt+4 change nothing behind it, from a field or from a switch. Each tab key is pressed while
/// the other tabs are open, where it would change something.
#[gpui_kit::test]
fn the_details_keys_do_nothing_in_the_open_popover(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send_limits(limit(1024, true), off(), false, cx);
    assert!(window.details_shown(cx));

    for (open_tab, tab_key, other_tab_keys) in [
        (DetailsTab::General, "alt-1", ["alt-2", "alt-3", "alt-4"]),
        (DetailsTab::Files, "alt-2", ["alt-1", "alt-3", "alt-4"]),
        (DetailsTab::Peers, "alt-3", ["alt-1", "alt-2", "alt-4"]),
        (DetailsTab::Trackers, "alt-4", ["alt-1", "alt-2", "alt-3"]),
    ] {
        window.press(tab_key, cx);
        let before = window.details(cx);
        assert_eq!(before.tab, open_tab);
        window.press("ctrl-l", cx);
        assert!(window.focused(DOWN, cx));
        let typed = window.value(DOWN, cx);
        for place in ["the field", "the switch"] {
            if place == "the switch" {
                window.press("shift-tab", cx);
                assert!(window.focused(DOWN_SWITCH, cx));
            }
            for key in std::iter::once("alt-enter").chain(other_tab_keys) {
                window.press(key, cx);
                let at = format!("{open_tab:?}, {place}, {key}");
                assert!(window.limits_open(cx), "{at}: the popover stays");
                assert!(window.details_shown(cx), "{at}");
                assert_eq!(window.details(cx), before, "{at}");
            }
        }
        assert_eq!(window.value(DOWN, cx), typed, "nothing was typed");
        window.press("escape", cx);
        assert!(!window.limits_open(cx));
    }
    assert_eq!(window.sent(), []);

    // Closed, the popover gives the keys back.
    window.press("alt-1", cx);
    assert_eq!(window.details(cx).tab, DetailsTab::General);
    window.press("alt-enter", cx);
    assert!(!window.details_shown(cx));
}

/// From a switch or Apply, where `/` types nothing, it goes to the search box as anywhere else, so
/// the popover closes and sends nothing; Ctrl+T switches the turtle and leaves the popover open.
#[gpui_kit::test]
fn slash_leaves_the_popover_and_ctrl_t_works_in_it(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send_limits(limit(1024, true), off(), false, cx);

    // The download switch is before its field, Apply three stops after it.
    for (place, key, presses) in [(DOWN_SWITCH, "shift-tab", 1), (APPLY, "tab", 3)] {
        window.press("ctrl-l", cx);
        window.type_text("512", cx);
        for _ in 0..presses {
            window.press(key, cx);
        }
        assert!(window.focused(place, cx), "{place}");
        window.press("/", cx);
        assert!(!window.limits_open(cx), "{place}: / closed the popover");
        assert!(window.focused(SEARCH, cx), "{place}");
        assert_eq!(window.sent(), [], "{place}: nothing was applied");
        window.press("escape", cx);
    }

    window.press("ctrl-l", cx);
    window.press("shift-tab", cx);
    assert!(window.focused(DOWN_SWITCH, cx));
    window.press("ctrl-t", cx);
    assert_eq!(window.sent_action().1, Action::SetAltSpeed(true));
    assert!(window.limits_open(cx), "the popover stays open");
    assert!(window.focused(DOWN_SWITCH, cx));
}

#[gpui_kit::test]
fn the_popover_closes_when_the_focus_leaves_it(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send_limits(limit(1024, true), off(), false, cx);

    // Tab moves through the switches, the fields and Apply, and then out.
    window.press("ctrl-l", cx);
    window.type_text("512", cx);
    for _ in 0..3 {
        window.press("tab", cx);
        assert!(window.limits_open(cx), "the focus is still in the popover");
    }
    assert!(window.focused(APPLY, cx));
    window.press("tab", cx);
    assert!(!window.limits_open(cx), "Tab took the focus out");
    assert_eq!(window.sent(), []);
    assert!(
        window.focused(SEARCH, cx),
        "the focus stays where Tab took it, the first control of the window"
    );

    // Shift+Tab: from the download field to its switch, and then out.
    window.press("ctrl-l", cx);
    window.press("shift-tab", cx);
    assert!(window.limits_open(cx));
    assert!(window.focused(DOWN_SWITCH, cx));
    window.press("shift-tab", cx);
    assert!(!window.limits_open(cx), "Shift+Tab took the focus out");
    assert!(
        window.focused(LIMITS, cx),
        "the focus stays on the button before the popover"
    );
    assert_eq!(window.sent(), []);

    // Ctrl+F: the focus goes to the search box, and stays there.
    window.press("ctrl-l", cx);
    window.press("ctrl-f", cx);
    assert!(!window.limits_open(cx));
    assert!(window.focused(SEARCH, cx), "the focus stays where it went");
    assert_eq!(window.sent(), []);
}

#[gpui_kit::test]
fn an_edit_is_measured_against_the_limits_the_popover_opened_with(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let opened = || session(limit(1024, true), limit(100, false), false);
    // Meanwhile another client switched the download limit off and the upload limit on.
    let changed = || session(limit(1024, false), limit(2048, true), false);

    window.send_session(Some(opened()), ConnectionState::Connected, cx);
    window.press("ctrl-l", cx);
    window.send_session(Some(changed()), ConnectionState::Connected, cx);
    window.click(APPLY, cx);
    assert_eq!(window.sent(), [], "nothing was touched");
    assert!(!window.limits_open(cx));

    window.send_session(Some(opened()), ConnectionState::Connected, cx);
    window.press("ctrl-l", cx);
    window.send_session(Some(changed()), ConnectionState::Connected, cx);
    // 300 KiB/s are 307 kB/s; the untouched download limit stays as the daemon has it now.
    window.retype(UP, "300", cx);
    window.click(APPLY, cx);
    assert_eq!(window.sent_actions(), [set_limit(Direction::Up, Some(307))]);
}

#[gpui_kit::test]
fn a_held_space_that_opened_the_popover_types_nothing(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send_limits(limit(1024, true), off(), false, cx);
    window.press("tab", cx);
    window.press("tab", cx);
    assert!(window.focused(LIMITS, cx));

    window.key_down("space", false, cx);
    assert!(window.limits_open(cx));
    assert!(window.focused(DOWN, cx));
    // The repeats would type spaces over the selected value, which would clear the limit.
    window.key_down_typing("space", cx);
    window.key_down_typing("space", cx);
    window.key_up("space", cx);
    assert_eq!(window.value(DOWN, cx).as_deref(), Some("1000"));
    assert_eq!(window.checked(DOWN_SWITCH, cx), Some(true));

    window.press("enter", cx);
    assert_eq!(window.sent(), [], "the limit is as it was");
    assert!(!window.limits_open(cx));
}

#[gpui_kit::test]
fn clearing_a_field_switches_its_limit_off(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send_limits(limit(1024, true), off(), false, cx);
    window.press("ctrl-l", cx);

    window.retype(DOWN, "", cx);
    assert_eq!(window.checked(DOWN_SWITCH, cx), Some(false));
    window.type_text("2", cx);
    assert_eq!(window.checked(DOWN_SWITCH, cx), Some(true));
    window.press("backspace", cx);
    assert_eq!(window.checked(DOWN_SWITCH, cx), Some(false));
    window.click(APPLY, cx);
    assert_eq!(window.sent_actions(), [set_limit(Direction::Down, None)]);

    // A switch clicked on while its field is empty still asks for a speed.
    window.send_limits(off(), off(), false, cx);
    window.press("ctrl-l", cx);
    window.retype(UP, "", cx);
    window.click(UP_SWITCH, cx);
    window.click(APPLY, cx);
    assert_eq!(window.sent(), []);
    assert_eq!(
        window.feedback(UP_FEEDBACK, cx).as_deref(),
        Some("Enter a speed to turn the limit on")
    );
}

#[gpui_kit::test]
fn an_edit_left_without_applying_is_forgotten(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send_limits(off(), off(), false, cx);

    for close_by in [FREE_SPACE, LIMITS] {
        window.press("ctrl-l", cx);
        window.type_text("512", cx);
        assert_eq!(window.checked(DOWN_SWITCH, cx), Some(true), "{close_by}");
        window.click(close_by, cx);
        assert!(!window.limits_open(cx), "{close_by}");
        assert_eq!(window.sent(), [], "{close_by}");

        window.press("ctrl-l", cx);
        assert_eq!(window.value(DOWN, cx).as_deref(), Some("98"), "{close_by}");
        assert_eq!(window.checked(DOWN_SWITCH, cx), Some(false), "{close_by}");
        window.press("escape", cx);
    }
}

#[gpui_kit::test]
fn a_click_on_blank_space_in_the_popover_keeps_the_field_focused(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send_limits(off(), off(), false, cx);

    // The gap between a switch and its field belongs to the form, which must not take the focus
    // from the field: what is typed still goes into it, and so does Enter.
    window.press("ctrl-l", cx);
    assert!(window.focused(DOWN, cx));
    window.click_between(DOWN_SWITCH, DOWN, cx);
    assert!(window.limits_open(cx));
    assert!(window.focused(DOWN, cx), "the field keeps the focus");
    window.type_text("5", cx);
    window.press("enter", cx);
    assert_eq!(window.sent_actions(), [set_limit(Direction::Down, Some(5))]);
    assert!(!window.limits_open(cx));

    // The same in the upload row.
    window.press("ctrl-l", cx);
    window.click(UP, cx);
    window.click_between(UP_SWITCH, UP, cx);
    assert!(window.focused(UP, cx), "the field keeps the focus");
    window.press(SELECT_ALL, cx);
    window.type_text("7", cx);
    window.press("enter", cx);
    assert_eq!(window.sent_actions(), [set_limit(Direction::Up, Some(7))]);
}

#[gpui_kit::test]
fn the_popover_survives_a_switch_to_another_app(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send_limits(off(), off(), false, cx);
    window.press("ctrl-l", cx);
    window.type_text("512", cx);

    // The focus leaves the window with the activation, which is no reason to close the popover.
    window.deactivate(cx);
    assert!(
        window.limits_open(cx),
        "the popover stays while 4x4 is behind"
    );
    window.activate(cx);
    assert!(window.limits_open(cx));
    assert_eq!(window.value(DOWN, cx).as_deref(), Some("512"));
    assert_eq!(window.checked(DOWN_SWITCH, cx), Some(true));
    assert!(window.focused(DOWN, cx), "the field is ready to type again");
    assert_eq!(window.sent(), []);

    // The user goes on where they left off.
    window.press("enter", cx);
    assert_eq!(
        window.sent_actions(),
        [set_limit(Direction::Down, Some(524))]
    );
}
