//! Headless tests for removing the selected torrents: the light dialog of Delete ("Remove from
//! list"), the danger dialog of Shift+Delete ("Remove and delete files"), where the focus is, what
//! Enter, Space, Escape and a held key do in them, what they remove when the list changes under
//! them, the toolbar's Remove menu, what they do while the daemon is not connected, and that a
//! click pressed too soon after a dialog opened (the second click of a double-click on the menu
//! item that opened it, however slow) presses nothing, wherever it is let go.
//!
//! Each test builds the two channels of a service itself and opens the window with no service
//! behind them, so nothing touches the network: the test reads the commands the window sends and
//! answers them with the events a service would send.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::test::{ClickOptions, TestWindowExt as _};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Entity, Focusable as _, InputEvent as _,
    KeyContext, KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, TestAppContext,
};
use x4_app::actions::{RemoveTorrents, RemoveTorrentsWithData};
use x4_app::app;
use x4_app::main_view::MainView;
use x4_core::test_support::torrent;
use x4_core::{
    Action, ActionError, ActionId, Command, ConnectionError, ConnectionState, DisplayState, Event,
    Snapshot, Torrent, UiState,
};
use x4_rpc::{HashString, RemoveData};

/// The id of the toolbar's Remove button, whose menu offers both removals.
const REMOVE: &str = "remove-torrents";
/// The buttons of the dialog that removes torrents from the list and keeps their files.
const LIGHT_CANCEL: &str = "remove-from-list-cancel";
const LIGHT_CONFIRM: &str = "remove-from-list-confirm";
/// The buttons of the dialog that removes torrents and deletes their files.
const DANGER_CANCEL: &str = "remove-files-cancel";
const DANGER_CONFIRM: &str = "remove-files-confirm";
/// The id of a name a dialog lists, with its index.
const NAME: &str = "removal-name";
/// The id of the line that counts the torrents a dialog does not name.
const MORE: &str = "removal-more";
/// The ids of a dialog's title and of its line about the files.
const TITLE: &str = "removal-title";
const FILES: &str = "removal-files";
/// The id of a notification.
const NOTIFICATION: &str = "notification";
/// The id of a dialog.
const DIALOG: &str = "dialog";
/// The items of the Remove menu.
const REMOVE_FROM_LIST: &str = "Remove from list…";
const REMOVE_WITH_DATA: &str = "Remove and delete files…";
/// The id of an open menu.
const MENU: &str = "popup-menu";
/// The id of the search box.
const SEARCH: &str = "search";
/// The id of the toolbar's Columns button, whose menu shows and hides columns.
const COLUMNS: &str = "column-menu";
/// The names of the seven torrents that the clicks on a menu item remove.
const SEVEN: [&str; 7] = ["a", "b", "c", "d", "e", "f", "g"];
/// How long after a dialog opens its confirm button takes presses of the pointer, whichever the
/// dialog: the light dialog's Remove from 500 ms on, the danger dialog's "Delete files" from 1 s.
const ARMING: Duration = Duration::from_millis(1000);

/// An open main window and the far ends of its channels, standing in for the service.
struct Window {
    handle: AnyWindowHandle,
    view: Entity<MainView>,
    events: async_channel::Sender<Event>,
    commands: crossbeam_channel::Receiver<Command>,
}

impl Window {
    /// Opens the window with animations off, so that a dialog is gone as soon as it closes.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a window that does not open fails the test"
    )]
    fn open(cx: &mut TestAppContext) -> Self {
        let (events, events_rx) = async_channel::unbounded();
        let (commands_tx, commands) = crossbeam_channel::unbounded();
        cx.update(|cx| cx.set_reduce_motion(true));
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

    /// Sends a snapshot of `torrents` with the daemon connected.
    fn send(&self, torrents: Vec<Torrent>, cx: &mut TestAppContext) {
        self.send_with(torrents, ConnectionState::Connected, cx);
    }

    /// Sends a snapshot of `torrents` with the connection in `connection`, and lets the window
    /// take it.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a window that stopped listening fails the test"
    )]
    fn send_with(
        &self,
        torrents: Vec<Torrent>,
        connection: ConnectionState,
        cx: &mut TestAppContext,
    ) {
        let stale = connection != ConnectionState::Connected;
        self.events
            .send_blocking(Event::Snapshot(Arc::new(Snapshot {
                torrents,
                listed: true,
                session: None,
                free_space_bytes: None,
                rate_down: 0,
                rate_up: 0,
                connection,
                stale,
            })))
            .expect("the window listens");
        cx.run_until_parked();
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

    /// Lets the time pass after which the confirm buttons of both dialogs take presses of the
    /// pointer.
    fn let_the_buttons_arm(cx: &mut TestAppContext) {
        cx.executor().advance_clock(ARMING);
        cx.run_until_parked();
    }

    /// Clicks the left button at `position`, as the `count`th click of a series (2 is the second
    /// click of a double-click), whatever is there.
    fn click_at(&self, position: Point<Pixels>, count: usize, cx: &mut TestAppContext) {
        self.mouse_down_at(position, count, cx);
        self.mouse_up_at(position, count, cx);
    }

    /// Moves the pointer to `position` and presses the left button there, as the `count`th click
    /// of a series, whatever is there. The button stays down until `mouse_up_at`.
    fn mouse_down_at(&self, position: Point<Pixels>, count: usize, cx: &mut TestAppContext) {
        let modifiers = Modifiers::none();
        self.act(cx, |window, cx| {
            window.dispatch_event(
                MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers,
                }
                .to_platform_input(),
                cx,
            );
            window.render_frame(cx);
            window.dispatch_event(
                MouseDownEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers,
                    click_count: count,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
        });
    }

    /// Lets the left button go at `position`, ending the `count`th click of a series.
    fn mouse_up_at(&self, position: Point<Pixels>, count: usize, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| {
            window.dispatch_event(
                MouseUpEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers: Modifiers::none(),
                    click_count: count,
                }
                .to_platform_input(),
                cx,
            );
        });
    }

    /// Where the element `id` was drawn last.
    fn bounds(
        &self,
        id: impl Into<gpui_kit::ElementId>,
        cx: &mut TestAppContext,
    ) -> Bounds<Pixels> {
        let id = id.into();
        self.act(cx, |window, _| window.find(id).bounds())
    }

    /// Where the item `label` of the open menu was drawn last.
    fn item_bounds(&self, label: &str, cx: &mut TestAppContext) -> Bounds<Pixels> {
        self.act(cx, |window, _| {
            window.within(MENU).find_by_label(label).bounds()
        })
    }

    /// Opens the context menu so that its item `label` lies over the button `button` of the
    /// dialog the item opens, as a double-click on the item would find them, and returns the
    /// middle of the button, which is on both. The button is found first in the dialog that the
    /// key `key` opens for the same torrents; the menu's place, from a first right-click.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a layout where the item cannot lie over the button fails the test"
    )]
    fn item_over_button(
        &self,
        key: &str,
        button: &'static str,
        label: &str,
        cx: &mut TestAppContext,
    ) -> Point<Pixels> {
        self.press(key, cx);
        let target = self.bounds(button, cx).center();
        self.press("escape", cx);
        assert_eq!(self.dialogs(cx), 0);

        let pressed_at = self.bounds(("row", 0_usize), cx).center();
        self.act(cx, |window, cx| window.right_click(("row", 0_usize), cx));
        let item_offset = self.item_bounds(label, cx).center() - pressed_at;
        self.press("escape", cx);

        let press_at = target - item_offset;
        let rows: Vec<Bounds<Pixels>> = self.act(cx, |window, _| {
            (0_usize..)
                .map_while(|row| window.try_find(("row", row)).map(|row| row.bounds()))
                .collect()
        });
        let (row, row_bounds) = rows
            .into_iter()
            .enumerate()
            .find(|(_, bounds)| bounds.contains(&press_at))
            .expect("a row where a right-click opens the menu with the item over the button");
        let row_origin = row_bounds.origin;
        self.act(cx, |window, cx| {
            window.click_with_options(
                ("row", row),
                ClickOptions::new()
                    .with_button(MouseButton::Right)
                    .with_offset(press_at - row_origin),
                cx,
            );
        });
        assert!(
            self.item_bounds(label, cx).contains(&target),
            "the item lies over the button"
        );
        target
    }

    /// Clicks the row `row` with the key that toggles it in the selection: Ctrl, and on macOS
    /// Command, as gpui's secondary key, which the table reads.
    fn ctrl_click_row(&self, row: usize, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| {
            window.click_with_modifiers(("row", row), Modifiers::secondary_key(cx), cx);
        });
    }

    /// Presses `key`: a key-down and a key-up, so that a focused button is pressed too.
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

    /// Opens the toolbar's Remove menu and chooses the item `label`.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a missing menu item fails the test"
    )]
    fn choose(&self, label: &str, cx: &mut TestAppContext) {
        self.click(REMOVE, cx);
        self.act(cx, |window, cx| {
            let mut menu = window.within("popup-menu");
            let item = menu
                .find_by_label(label)
                .path()
                .last()
                .cloned()
                .expect("the item has an id");
            menu.click(item, cx);
        });
    }

    /// Whether the element `id` is on screen.
    fn shown(&self, id: &'static str, cx: &mut TestAppContext) -> bool {
        self.act(cx, |window, _| window.try_find(id).is_some())
    }

    /// Whether the element `id` has the keyboard focus.
    fn focused(&self, id: &'static str, cx: &mut TestAppContext) -> bool {
        self.act(cx, |window, _| window.find(id).focused() == Some(true))
    }

    /// Whether the button `id` is drawn disabled.
    fn disabled(&self, id: &'static str, cx: &mut TestAppContext) -> bool {
        self.act(cx, |window, _| window.find(id).disabled() == Some(true))
    }

    /// What a screen reader says for the element `id`.
    fn label(&self, id: &'static str, cx: &mut TestAppContext) -> Option<String> {
        self.act(cx, |window, _| window.find(id).label().map(str::to_owned))
    }

    /// How many dialogs are open.
    fn dialogs(&self, cx: &mut TestAppContext) -> usize {
        self.act(cx, |window, _| window.find_all(DIALOG).len())
    }

    /// How many torrents the open dialog names (it names at most five).
    fn names(&self, cx: &mut TestAppContext) -> usize {
        self.act(cx, |window, _| {
            (0_usize..10)
                .take_while(|index| window.try_find((NAME, *index)).is_some())
                .count()
        })
    }

    /// Whether the table has the keyboard focus, so that the list's keys reach it.
    fn table_focused(&self, cx: &mut TestAppContext) -> bool {
        let table = cx.update(|cx| self.view.read(cx).table().clone());
        self.act(cx, |window, cx| table.focus_handle(cx).is_focused(window))
    }

    /// The commands the window has sent so far, other than the window's visibility and the
    /// torrent whose details to read, which the `details` tests cover.
    fn sent(&self) -> Vec<Command> {
        self.commands
            .try_iter()
            .filter(|command| !matches!(command, Command::SetVisibility(_) | Command::Select(_)))
            .collect()
    }

    /// The one action the window has sent since the last look.
    #[track_caller]
    fn sent_action(&self) -> Action {
        self.sent_run().1
    }

    /// The one action the window has sent since the last look, with its id.
    #[track_caller]
    #[expect(
        clippy::panic,
        reason = "a test helper: anything but one action fails the test"
    )]
    fn sent_run(&self) -> (ActionId, Action) {
        match self.sent().as_slice() {
            [Command::Run(id, action)] => (*id, action.clone()),
            other => panic!("expected one action, got {other:?}"),
        }
    }

    /// Answers the action `id` with `result`, as the service does once the daemon has answered.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a window that stopped listening fails the test"
    )]
    fn finish(&self, id: ActionId, result: Result<(), ActionError>, cx: &mut TestAppContext) {
        self.events
            .send_blocking(Event::ActionFinished { id, result })
            .expect("the window listens");
        cx.run_until_parked();
    }

    /// The text of the dialog's line `id`, as a screen reader reads it.
    fn text(&self, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) -> Option<String> {
        let id = id.into();
        self.act(cx, |window, _| window.find(id).value().map(str::to_owned))
    }

    /// The value of the search box.
    fn search_text(&self, cx: &mut TestAppContext) -> Option<String> {
        self.act(cx, |window, _| {
            window.find("search").value().map(str::to_owned)
        })
    }

    /// The height of the element `id` as it was drawn last.
    fn height(&self, id: impl Into<gpui_kit::ElementId>, cx: &mut TestAppContext) -> f32 {
        let id = id.into();
        self.act(cx, |window, _| {
            f32::from(window.find(id).bounds().size.height)
        })
    }

    fn last_failure(&self, cx: &mut TestAppContext) -> Option<String> {
        cx.update(|cx| self.view.read(cx).last_failure().map(str::to_owned))
    }

    /// How many notifications are on screen.
    fn notifications(&self, cx: &mut TestAppContext) -> usize {
        self.act(cx, |window, _| window.find_all(NOTIFICATION).len())
    }
}

/// The clock of a test that says when each press and release comes, counted from when the clock
/// started.
struct Timeline(Instant);

impl Timeline {
    fn start(cx: &TestAppContext) -> Self {
        Self(cx.executor().now())
    }

    /// Lets the time pass until `millis` after the clock started.
    #[track_caller]
    fn at(&self, millis: u64, cx: &mut TestAppContext) {
        let then = self.0 + Duration::from_millis(millis);
        let now = cx.executor().now();
        assert!(now <= then, "the test is already past {millis} ms");
        cx.executor().advance_clock(then - now);
        cx.run_until_parked();
    }
}

fn downloading(name: &str) -> Torrent {
    Torrent {
        state: DisplayState::Downloading,
        ..torrent(name)
    }
}

/// The hashes of the torrents called `names`, in that order.
fn hashes(names: &[&str]) -> Vec<HashString> {
    names.iter().map(|name| torrent(name).hash).collect()
}

fn remove(names: &[&str], data: RemoveData) -> Action {
    Action::Remove {
        hashes: hashes(names),
        data,
    }
}

/// A torrent with 1 GiB downloaded.
fn gibibyte(name: &str) -> Torrent {
    Torrent {
        size_when_done: 1 << 30,
        left_until_done: 0,
        ..torrent(name)
    }
}

fn reconnecting() -> ConnectionState {
    ConnectionState::Reconnecting {
        attempt: 2,
        last_error: ConnectionError::Timeout,
    }
}

#[gpui_kit::test]
fn delete_opens_the_light_dialog_and_enter_removes_from_list(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![torrent("a"), torrent("b"), torrent("c")], cx);
    window.click_row(0, cx);
    window.ctrl_click_row(2, cx);

    window.press("delete", cx);

    assert_eq!(window.sent(), [], "the dialog asks first");
    assert!(
        !window.shown(DANGER_CONFIRM, cx),
        "not the dialog that deletes"
    );
    assert!(window.focused(LIGHT_CONFIRM, cx), "Remove has the focus");
    assert_eq!(window.label(LIGHT_CONFIRM, cx).as_deref(), Some("Remove"));
    assert_eq!(window.label(LIGHT_CANCEL, cx).as_deref(), Some("Cancel"));
    assert_eq!(window.names(cx), 2);
    assert_eq!(
        window.text(TITLE, cx).as_deref(),
        Some("Remove 2 torrents from the list?")
    );
    assert_eq!(window.text((NAME, 1_usize), cx).as_deref(), Some("c"));
    assert_eq!(
        window.text(FILES, cx).as_deref(),
        Some("The downloaded files stay on the server.")
    );
    assert!(!window.shown(MORE, cx));

    window.press("enter", cx);

    assert_eq!(window.sent_action(), remove(&["a", "c"], RemoveData::Keep));
    assert!(!window.shown(LIGHT_CONFIRM, cx), "the dialog closed");
    assert!(window.table_focused(cx), "the list has the keys again");

    // Enter presses the button that has the focus: on Cancel, it cancels.
    window.press("delete", cx);
    window.press("shift-tab", cx);
    assert!(window.focused(LIGHT_CANCEL, cx));
    window.press("enter", cx);
    assert_eq!(window.sent(), []);
    assert!(!window.shown(LIGHT_CONFIRM, cx));
    assert!(window.table_focused(cx));
}

#[gpui_kit::test]
fn shift_delete_only_opens_the_danger_dialog(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![torrent("a"), torrent("b")], cx);
    window.click_row(1, cx);

    window.press("shift-delete", cx);

    assert_eq!(window.sent(), [], "Shift+Delete only asks");
    assert!(window.shown(DANGER_CONFIRM, cx));
    assert!(!window.shown(LIGHT_CONFIRM, cx));
    assert_eq!(
        window.label(DANGER_CONFIRM, cx).as_deref(),
        Some("Delete files")
    );
    assert_eq!(window.label(DANGER_CANCEL, cx).as_deref(), Some("Cancel"));
    assert_eq!(window.names(cx), 1);

    // With nothing selected, there is nothing to ask about.
    window.press("escape", cx);
    window.press("escape", cx);
    window.press("shift-delete", cx);
    window.press("delete", cx);
    assert_eq!(window.dialogs(cx), 0);
    assert_eq!(window.sent(), []);
}

#[gpui_kit::test]
fn cancel_has_focus_in_the_danger_dialog(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![torrent("a"), torrent("b")], cx);
    window.press("secondary-a", cx);

    window.press("shift-delete", cx);

    assert!(window.focused(DANGER_CANCEL, cx));
    assert!(!window.focused(DANGER_CONFIRM, cx));

    // The dialog is drawn again at every update of the list; the focus stays where the user put
    // it.
    window.press("tab", cx);
    assert!(
        window.focused(DANGER_CONFIRM, cx),
        "Tab reaches Delete files"
    );
    window.send(vec![downloading("a"), torrent("b")], cx);
    assert!(window.focused(DANGER_CONFIRM, cx));
    window.press("shift-tab", cx);
    window.send(vec![torrent("a"), torrent("b")], cx);
    assert!(window.focused(DANGER_CANCEL, cx));

    window.click(DANGER_CANCEL, cx);

    assert!(!window.shown(DANGER_CONFIRM, cx), "the dialog closed");
    assert_eq!(window.sent(), []);
    assert!(window.table_focused(cx), "the list has the keys again");
}

#[gpui_kit::test]
fn enter_in_the_danger_dialog_never_deletes(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![torrent("a"), torrent("b")], cx);
    window.click_row(0, cx);

    // Enter presses Cancel, which has the focus.
    window.press("shift-delete", cx);
    window.press("enter", cx);
    assert_eq!(window.sent(), []);
    assert!(
        !window.shown(DANGER_CONFIRM, cx),
        "Cancel closed the dialog"
    );
    assert!(window.table_focused(cx));

    // So does Space, and so does a held Enter, once.
    window.press("shift-delete", cx);
    window.press("space", cx);
    assert_eq!(window.sent(), []);
    assert!(!window.shown(DANGER_CONFIRM, cx));
    window.press("shift-delete", cx);
    window.hold("enter", 5, cx);
    assert_eq!(window.sent(), []);
    assert!(!window.shown(DANGER_CONFIRM, cx));

    // Only the danger button deletes: Tab to it, then Enter.
    window.press("shift-delete", cx);
    window.press("tab", cx);
    window.press("enter", cx);
    assert_eq!(window.sent_action(), remove(&["a"], RemoveData::Delete));
    assert!(!window.shown(DANGER_CONFIRM, cx));
    assert!(window.table_focused(cx));
}

#[gpui_kit::test]
fn escape_cancels_the_danger_dialog(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![torrent("a"), torrent("b")], cx);
    window.click_row(0, cx);

    window.press("shift-delete", cx);
    window.press("escape", cx);

    assert!(!window.shown(DANGER_CONFIRM, cx));
    assert_eq!(window.sent(), []);
    assert!(window.table_focused(cx));

    // The light dialog too.
    window.press("delete", cx);
    window.press("escape", cx);
    assert!(!window.shown(LIGHT_CONFIRM, cx));
    assert_eq!(window.sent(), []);

    // The list has its keys back: Escape there clears the selection, so there is nothing to
    // remove.
    window.press("escape", cx);
    window.press("shift-delete", cx);
    assert_eq!(window.dialogs(cx), 0);
}

#[gpui_kit::test]
fn clicking_delete_files_sends_remove_with_data_for_the_captured_hashes(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let names = ["a", "b", "c", "d", "e", "f", "g"];
    window.send(names.into_iter().map(gibibyte).collect(), cx);
    window.press("secondary-a", cx);
    window.press("shift-delete", cx);
    assert_eq!(window.names(cx), 5, "five names, and how many more");

    // Meanwhile one of them is removed elsewhere, and another one changes.
    window.send(
        vec![
            Torrent {
                state: DisplayState::Downloading,
                ..gibibyte("a")
            },
            gibibyte("c"),
            gibibyte("d"),
            gibibyte("e"),
            gibibyte("f"),
            gibibyte("g"),
        ],
        cx,
    );

    // The dialog says what it said when it opened.
    assert_eq!(
        window.text(TITLE, cx).as_deref(),
        Some("Delete 7 torrents and their files?")
    );
    assert_eq!(window.names(cx), 5);
    for (index, name) in ["a", "b", "c", "d", "e"].into_iter().enumerate() {
        assert_eq!(window.text((NAME, index), cx).as_deref(), Some(name));
    }
    assert_eq!(window.text(MORE, cx).as_deref(), Some("and 2 more"));
    assert_eq!(
        window.text(FILES, cx).as_deref(),
        Some("7.0 GiB of downloaded data will be deleted from the server. This cannot be undone.")
    );

    Window::let_the_buttons_arm(cx);
    window.click(DANGER_CONFIRM, cx);

    assert_eq!(window.sent_action(), remove(&names, RemoveData::Delete));
    assert!(!window.shown(DANGER_CONFIRM, cx), "the dialog closed");
    assert!(window.table_focused(cx));
}

#[gpui_kit::test]
fn a_second_shift_delete_does_not_open_a_second_dialog(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![torrent("a"), torrent("b")], cx);
    window.click_row(0, cx);

    // Held down, the key repeats; then it is pressed again, and Delete too.
    window.hold("shift-delete", 3, cx);
    window.press("shift-delete", cx);
    window.press("delete", cx);

    assert_eq!(window.dialogs(cx), 1);
    assert!(window.shown(DANGER_CONFIRM, cx));
    assert!(!window.shown(LIGHT_CONFIRM, cx));
    assert!(window.focused(DANGER_CANCEL, cx));
    assert_eq!(window.sent(), []);

    window.press("escape", cx);
    assert_eq!(window.dialogs(cx), 0, "one Escape closes it");

    // A held Delete opens one light dialog.
    window.hold("delete", 3, cx);
    assert_eq!(window.dialogs(cx), 1);
    assert!(window.shown(LIGHT_CONFIRM, cx));
    assert_eq!(window.sent(), []);

    // Should the list get the focus back while the dialog is open, its keys still open no second
    // dialog over it.
    let table = cx.update(|cx| window.view.read(cx).table().clone());
    window.act(cx, |window, cx| window.focus(&table.focus_handle(cx), cx));
    assert!(window.table_focused(cx));
    window.press("shift-delete", cx);
    window.press("delete", cx);
    assert_eq!(window.dialogs(cx), 1);
    assert!(window.shown(LIGHT_CONFIRM, cx));
    assert!(!window.shown(DANGER_CONFIRM, cx));
    assert_eq!(window.sent(), []);
}

#[gpui_kit::test]
fn the_repeats_of_a_held_delete_open_nothing(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![torrent("a"), torrent("b")], cx);

    // Delete goes down with nothing selected, and a torrent is clicked while it is held: its
    // repeats do not ask to remove it.
    window.key_down("shift-delete", false, cx);
    window.click_row(0, cx);
    window.key_down("shift-delete", true, cx);
    window.key_down("shift-delete", true, cx);
    assert_eq!(window.dialogs(cx), 0);
    window.key_up("shift-delete", cx);

    window.press("escape", cx);
    window.key_down("delete", false, cx);
    window.click_row(1, cx);
    window.key_down("delete", true, cx);
    assert_eq!(window.dialogs(cx), 0);
    window.key_up("delete", cx);

    // Let go and pressed again, the key asks.
    window.press("delete", cx);
    assert!(window.shown(LIGHT_CONFIRM, cx));
    assert_eq!(window.sent(), []);
}

#[gpui_kit::test]
fn remove_is_disabled_while_disconnected(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![torrent("a"), torrent("b")], cx);
    window.click_row(0, cx);
    assert!(!window.disabled(REMOVE, cx));

    window.send_with(vec![torrent("a"), torrent("b")], reconnecting(), cx);

    assert!(window.disabled(REMOVE, cx));
    window.press("delete", cx);
    window.press("shift-delete", cx);
    window.click(REMOVE, cx);
    assert_eq!(window.dialogs(cx), 0, "nothing asks");
    assert!(
        !window.act(cx, |window, _| window.try_find("popup-menu").is_some()),
        "the menu stays closed"
    );
    assert_eq!(window.sent(), []);

    // The connection is lost while the dialog is open: it stays, so that it can be cancelled, but
    // its button is disabled until the connection is back.
    window.send(vec![torrent("a"), torrent("b")], cx);
    window.press("shift-delete", cx);
    assert!(!window.disabled(DANGER_CONFIRM, cx));
    window.send_with(vec![torrent("a"), torrent("b")], reconnecting(), cx);
    assert!(window.disabled(DANGER_CONFIRM, cx));
    assert!(!window.disabled(DANGER_CANCEL, cx));
    Window::let_the_buttons_arm(cx);
    window.click(DANGER_CONFIRM, cx);
    assert_eq!(window.sent(), []);
    assert!(window.shown(DANGER_CONFIRM, cx), "the dialog stays open");
    window.send(vec![torrent("a"), torrent("b")], cx);
    assert!(!window.disabled(DANGER_CONFIRM, cx), "enabled again");
    window.click(DANGER_CONFIRM, cx);
    assert_eq!(window.sent_action(), remove(&["a"], RemoveData::Delete));

    // Remove keeps the focus while it is disabled: Enter does nothing, and Escape still cancels.
    window.click_row(1, cx);
    window.press("delete", cx);
    window.send_with(vec![torrent("a"), torrent("b")], reconnecting(), cx);
    assert!(window.disabled(LIGHT_CONFIRM, cx));
    assert!(window.focused(LIGHT_CONFIRM, cx));
    window.press("enter", cx);
    assert_eq!(window.sent(), []);
    assert!(window.shown(LIGHT_CONFIRM, cx));
    window.press("escape", cx);
    assert_eq!(window.dialogs(cx), 0);
    assert!(window.table_focused(cx));
}

#[gpui_kit::test]
fn the_remove_button_asks_either_way_from_its_menu(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![torrent("a"), torrent("b")], cx);
    assert!(window.disabled(REMOVE, cx), "nothing is selected");
    window.click_row(0, cx);
    assert!(!window.disabled(REMOVE, cx));
    assert_eq!(window.label(REMOVE, cx).as_deref(), Some("Remove"));

    window.choose(REMOVE_FROM_LIST, cx);
    assert!(window.shown(LIGHT_CONFIRM, cx));
    assert!(window.focused(LIGHT_CONFIRM, cx));
    window.press("escape", cx);
    assert!(window.table_focused(cx), "the list has the keys again");

    window.choose(REMOVE_WITH_DATA, cx);
    assert!(window.shown(DANGER_CONFIRM, cx));
    assert!(!window.shown(LIGHT_CONFIRM, cx));
    assert!(window.focused(DANGER_CANCEL, cx));
    window.click(DANGER_CANCEL, cx);
    assert!(window.table_focused(cx));
    assert_eq!(window.sent(), []);

    window.choose(REMOVE_FROM_LIST, cx);
    Window::let_the_buttons_arm(cx);
    window.click(LIGHT_CONFIRM, cx);
    assert_eq!(window.sent_action(), remove(&["a"], RemoveData::Keep));
    assert!(window.table_focused(cx));

    // The tooltip names the key: it looks the action's key up in the torrent list's context.
    window.act(cx, |window, cx| {
        let list = KeyContext::parse("TorrentList").expect("a key context");
        // The Mac removes with ⌘⌫ and ⌥⌘⌫, which are bound after Delete and win; gpui writes the
        // modifiers of the second as alt-cmd.
        let (remove, remove_with_data) = if cfg!(target_os = "macos") {
            ("cmd-backspace", "alt-cmd-backspace")
        } else {
            ("delete", "shift-delete")
        };
        for (action, keys) in [
            (&RemoveTorrents as &dyn gpui_kit::Action, remove),
            (&RemoveTorrentsWithData, remove_with_data),
        ] {
            let binding = window
                .highest_precedence_binding_for_action_in_context(action, list.clone())
                .expect("the action has a key in the list");
            let shown: Vec<String> = binding
                .keystrokes()
                .iter()
                .map(|keystroke| keystroke.unparse(cx.platform_info().os))
                .collect();
            assert_eq!(shown, [keys]);
        }
    });
}

/// An open menu of the toolbar keeps the keys until it closes, as the context menu does: no key
/// takes the focus elsewhere and leaves the menu open, so that the keys of the list or the search
/// box would act behind it. The Columns menu is the toolbar's other one.
#[gpui_kit::test]
fn focus_keys_in_an_open_toolbar_menu_do_nothing(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![torrent("a"), torrent("b")], cx);
    window.click_row(0, cx);

    for button in [REMOVE, COLUMNS] {
        window.click(button, cx);
        assert!(window.shown(MENU, cx), "{button}");
        for key in ["tab", "shift-tab", "secondary-f", "/"] {
            window.press(key, cx);
            assert!(
                window.shown(MENU, cx),
                "{button}, {key}: the menu stays open"
            );
            assert!(!window.focused(SEARCH, cx), "{button}, {key}");
            assert!(
                !window.table_focused(cx),
                "{button}, {key}: the keys stay in the menu"
            );
        }
        window.press("space", cx);
        assert_eq!(window.sent(), [], "{button}: nothing acts behind the menu");

        window.press("escape", cx);
        assert!(!window.shown(MENU, cx), "{button}");
    }
    window.press("secondary-f", cx);
    assert!(
        window.focused(SEARCH, cx),
        "closed, the menus let Ctrl+F work again"
    );
}

#[gpui_kit::test]
fn delete_in_the_search_box_edits_text_and_opens_no_dialog(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![torrent("ab"), torrent("b")], cx);
    window.press("secondary-a", cx);

    window.press("secondary-f", cx);
    window.act(cx, |window, cx| window.input("ab", cx));
    window.press("home", cx);
    window.press("delete", cx);
    assert_eq!(window.search_text(cx).as_deref(), Some("b"));
    window.press("shift-delete", cx);
    assert_eq!(window.search_text(cx).as_deref(), Some(""));

    assert_eq!(window.dialogs(cx), 0);
    assert_eq!(window.sent(), []);
}

#[gpui_kit::test]
fn a_dialog_closed_with_the_mouse_lets_the_key_ask_again(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![torrent("a"), torrent("b")], cx);
    window.click_row(0, cx);

    window.press("shift-delete", cx);
    window.click(DANGER_CANCEL, cx);
    window.press("shift-delete", cx);
    assert_eq!(window.dialogs(cx), 1, "Shift+Delete asks again");
    assert!(window.shown(DANGER_CONFIRM, cx));
    window.click(DANGER_CANCEL, cx);

    window.press("delete", cx);
    Window::let_the_buttons_arm(cx);
    window.click(LIGHT_CONFIRM, cx);
    assert_eq!(window.sent_action(), remove(&["a"], RemoveData::Keep));
    window.click_row(1, cx);
    window.press("delete", cx);
    assert_eq!(window.dialogs(cx), 1, "Delete asks again");
    assert!(window.shown(LIGHT_CONFIRM, cx));
}

#[gpui_kit::test]
fn an_enter_held_on_the_menu_opens_the_dialog_but_does_not_confirm_it(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![torrent("a"), torrent("b")], cx);
    window.click_row(0, cx);

    // "Remove from list…" chosen with Enter, held: its repeats land on the focused Remove.
    window.click(REMOVE, cx);
    window.press("down", cx);
    window.key_down("enter", false, cx);
    assert!(window.focused(LIGHT_CONFIRM, cx), "the dialog opened");
    window.key_down("enter", true, cx);
    window.key_down("enter", true, cx);
    window.key_up("enter", cx);
    assert_eq!(window.sent(), [], "the held Enter did not confirm");
    assert!(window.shown(LIGHT_CONFIRM, cx));

    // X11 does not mark its repeats.
    window.press("escape", cx);
    window.click(REMOVE, cx);
    window.press("down", cx);
    window.key_down("enter", false, cx);
    window.key_down("enter", false, cx);
    window.key_up("enter", cx);
    assert_eq!(window.sent(), []);
    assert!(window.shown(LIGHT_CONFIRM, cx));

    // A fresh press confirms.
    window.press("enter", cx);
    assert_eq!(window.sent_action(), remove(&["a"], RemoveData::Keep));
}

#[gpui_kit::test]
fn names_are_shown_in_full_however_long(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let long = |year: u32| {
        format!(
            "Some.Very.Long.Movie.Title.With.Many.Words.In.It.Directors.Cut.Remastered.\
             1080p.BluRay.x264-GROUP ({year})"
        )
    };
    window.send(
        vec![torrent(&long(2019)), torrent(&long(2020)), torrent("x")],
        cx,
    );
    window.press("secondary-a", cx);
    window.press("shift-delete", cx);

    assert_eq!(window.text((NAME, 0_usize), cx), Some(long(2019)));
    assert_eq!(window.text((NAME, 1_usize), cx), Some(long(2020)));
    let (long_height, short_height) = (
        window.height((NAME, 0_usize), cx),
        window.height((NAME, 2_usize), cx),
    );
    assert!(
        long_height > short_height * 1.5,
        "a long name wraps over more lines instead of being cut: {long_height} vs {short_height}"
    );
}

#[gpui_kit::test]
fn a_failed_removal_is_shown(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![torrent("a"), torrent("b")], cx);
    window.press("secondary-a", cx);
    window.press("shift-delete", cx);
    Window::let_the_buttons_arm(cx);
    window.click(DANGER_CONFIRM, cx);
    let (id, action) = window.sent_run();
    assert_eq!(action, remove(&["a", "b"], RemoveData::Delete));

    window.finish(
        id,
        Err(ActionError::Rpc(ConnectionError::Server("busy".to_owned()))),
        cx,
    );

    assert_eq!(
        window.last_failure(cx).as_deref(),
        Some("Could not remove 2 torrents and delete their files: busy")
    );
    assert_eq!(window.notifications(cx), 1);
}

/// The second click of a double-click on "Remove and delete files…" lands on whatever the dialog
/// it opened has put under the pointer, which can be "Delete files": it must press nothing, and nor
/// may any click so soon after the dialog opened.
#[gpui_kit::test]
fn a_double_click_on_the_menu_item_does_not_delete_files(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let names = ["a", "b", "c", "d", "e", "f", "g"];
    window.send(names.into_iter().map(torrent).collect(), cx);
    window.press("secondary-a", cx);
    let point = window.item_over_button("shift-delete", DANGER_CONFIRM, REMOVE_WITH_DATA, cx);

    window.click_at(point, 1, cx);
    assert!(
        window.shown(DANGER_CONFIRM, cx),
        "the item opened the dialog"
    );
    assert!(
        window.bounds(DANGER_CONFIRM, cx).contains(&point),
        "the second click lands on Delete files"
    );
    window.click_at(point, 2, cx);

    assert_eq!(window.sent(), [], "the double-click deleted nothing");
    assert!(window.shown(DANGER_CONFIRM, cx), "the dialog stays open");

    // Later on too, a click that is the second of a double-click is not a press.
    Window::let_the_buttons_arm(cx);
    window.click_at(point, 2, cx);
    assert_eq!(window.sent(), []);
    window.click_at(point, 1, cx);
    assert_eq!(window.sent_action(), remove(&names, RemoveData::Delete));
}

/// Opens the dialog of the menu item `label` for seven selected torrents by clicking the item from
/// 0 to 100 ms on the test's clock, with the item lying over the dialog's `button` (found in the
/// dialog that `key` opens). Returns the window, the point on both, and the clock.
fn open_with_a_click_on_the_item(
    key: &str,
    button: &'static str,
    label: &str,
    cx: &mut TestAppContext,
) -> (Window, Point<Pixels>, Timeline) {
    let window = Window::open(cx);
    window.send(SEVEN.into_iter().map(torrent).collect(), cx);
    window.press("secondary-a", cx);
    let point = window.item_over_button(key, button, label, cx);

    let clock = Timeline::start(cx);
    window.mouse_down_at(point, 1, cx);
    clock.at(100, cx);
    window.mouse_up_at(point, 1, cx);
    assert!(window.shown(button, cx), "the item opened the dialog");
    assert!(
        window.bounds(button, cx).contains(&point),
        "the next press lands on the button"
    );
    (window, point, clock)
}

/// A slow double-click on "Remove and delete files…": its second press comes too late after the
/// first for the system to count it as the second click of a double-click, so it is a click of its
/// own, pressed on "Delete files" at `pressed` and let go there at `released`. It was pressed
/// before the button was armed, so it deletes nothing.
fn a_slow_double_click_on_delete_files(pressed: u64, released: u64, cx: &mut TestAppContext) {
    let (window, point, clock) =
        open_with_a_click_on_the_item("shift-delete", DANGER_CONFIRM, REMOVE_WITH_DATA, cx);

    clock.at(pressed, cx);
    window.mouse_down_at(point, 1, cx);
    clock.at(released, cx);
    window.mouse_up_at(point, 1, cx);

    assert_eq!(window.sent(), [], "the slow double-click deleted nothing");
    assert!(window.shown(DANGER_CONFIRM, cx), "the dialog stays open");
}

#[gpui_kit::test]
fn a_slow_double_click_on_the_menu_item_does_not_delete_files(cx: &mut TestAppContext) {
    a_slow_double_click_on_delete_files(450, 650, cx);
}

#[gpui_kit::test]
fn half_a_second_between_the_clicks_on_the_menu_item_deletes_no_files(cx: &mut TestAppContext) {
    a_slow_double_click_on_delete_files(600, 720, cx);
}

/// "Delete files" takes a click only if it was pressed once the button was armed, a second after
/// the dialog opened: a press that came before it does nothing, however late it is let go.
#[gpui_kit::test]
fn delete_files_takes_only_a_press_that_comes_once_it_is_armed(cx: &mut TestAppContext) {
    let (window, point, clock) =
        open_with_a_click_on_the_item("shift-delete", DANGER_CONFIRM, REMOVE_WITH_DATA, cx);

    clock.at(900, cx);
    window.mouse_down_at(point, 1, cx);
    clock.at(1100, cx);
    window.mouse_up_at(point, 1, cx);
    assert_eq!(window.sent(), [], "pressed before the button was armed");
    assert!(window.shown(DANGER_CONFIRM, cx), "the dialog stays open");

    window.mouse_down_at(point, 1, cx);
    clock.at(1200, cx);
    window.mouse_up_at(point, 1, cx);
    assert_eq!(window.sent_action(), remove(&SEVEN, RemoveData::Delete));
    assert!(!window.shown(DANGER_CONFIRM, cx), "the dialog closed");
}

/// Remove in the light dialog waits half a second, measured from the press too.
#[gpui_kit::test]
fn a_slow_double_click_on_the_menu_item_does_not_remove_from_list(cx: &mut TestAppContext) {
    let (window, point, clock) =
        open_with_a_click_on_the_item("delete", LIGHT_CONFIRM, REMOVE_FROM_LIST, cx);

    clock.at(450, cx);
    window.mouse_down_at(point, 1, cx);
    clock.at(650, cx);
    window.mouse_up_at(point, 1, cx);
    assert_eq!(window.sent(), [], "pressed before the button was armed");
    assert!(window.shown(LIGHT_CONFIRM, cx), "the dialog stays open");

    clock.at(700, cx);
    window.mouse_down_at(point, 1, cx);
    clock.at(800, cx);
    window.mouse_up_at(point, 1, cx);
    assert_eq!(window.sent_action(), remove(&SEVEN, RemoveData::Keep));
    assert!(!window.shown(LIGHT_CONFIRM, cx), "the dialog closed");
}

#[gpui_kit::test]
fn delete_files_ignores_clicks_right_after_the_dialog_opens(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![torrent("a"), torrent("b")], cx);
    window.click_row(0, cx);
    let clock = Timeline::start(cx);
    window.press("shift-delete", cx);

    window.click(DANGER_CONFIRM, cx);
    assert_eq!(window.sent(), [], "too soon after the dialog opened");
    clock.at(500, cx);
    window.click(DANGER_CONFIRM, cx);
    assert_eq!(window.sent(), [], "half a second is not enough here");
    clock.at(999, cx);
    window.click(DANGER_CONFIRM, cx);
    assert_eq!(window.sent(), [], "still too soon");
    assert!(window.shown(DANGER_CONFIRM, cx), "the dialog stays open");

    clock.at(1000, cx);
    window.click(DANGER_CONFIRM, cx);
    assert_eq!(window.sent_action(), remove(&["a"], RemoveData::Delete));
    assert!(!window.shown(DANGER_CONFIRM, cx), "the dialog closed");
}

/// The light dialog's Remove takes no double-click either.
#[gpui_kit::test]
fn a_double_click_on_the_menu_item_does_not_remove_from_list(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let names = ["a", "b", "c", "d", "e", "f", "g"];
    window.send(names.into_iter().map(torrent).collect(), cx);
    window.press("secondary-a", cx);
    let point = window.item_over_button("delete", LIGHT_CONFIRM, REMOVE_FROM_LIST, cx);

    window.click_at(point, 1, cx);
    assert!(
        window.shown(LIGHT_CONFIRM, cx),
        "the item opened the dialog"
    );
    assert!(
        window.bounds(LIGHT_CONFIRM, cx).contains(&point),
        "the second click lands on Remove"
    );
    window.click_at(point, 2, cx);

    assert_eq!(window.sent(), [], "the double-click removed nothing");
    assert!(window.shown(LIGHT_CONFIRM, cx), "the dialog stays open");

    Window::let_the_buttons_arm(cx);
    window.click_at(point, 2, cx);
    assert_eq!(window.sent(), []);
    window.click_at(point, 1, cx);
    assert_eq!(window.sent_action(), remove(&names, RemoveData::Keep));
}

#[gpui_kit::test]
fn remove_from_list_ignores_clicks_right_after_the_dialog_opens(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![torrent("a"), torrent("b")], cx);
    window.click_row(0, cx);
    let clock = Timeline::start(cx);
    window.press("delete", cx);

    window.click(LIGHT_CONFIRM, cx);
    assert_eq!(window.sent(), [], "too soon after the dialog opened");
    clock.at(250, cx);
    window.click(LIGHT_CONFIRM, cx);
    assert_eq!(window.sent(), [], "still too soon");
    clock.at(499, cx);
    window.click(LIGHT_CONFIRM, cx);
    assert_eq!(window.sent(), [], "still too soon");
    assert!(window.shown(LIGHT_CONFIRM, cx), "the dialog stays open");

    clock.at(500, cx);
    window.click(LIGHT_CONFIRM, cx);
    assert_eq!(window.sent_action(), remove(&["a"], RemoveData::Keep));
    assert!(!window.shown(LIGHT_CONFIRM, cx), "the dialog closed");

    // A key needs no wait: Enter presses the focused Remove at once.
    window.click_row(1, cx);
    window.press("delete", cx);
    window.press("enter", cx);
    assert_eq!(window.sent_action(), remove(&["b"], RemoveData::Keep));
}
