//! Headless tests for the context menu of the torrent list and for copying torrents: a right-click
//! selects as a file manager does; the menu's items act on the torrents the menu took when it
//! opened, show their keys, are disabled while the daemon is not connected (all but Copy hash) and
//! hand the keys back to the list; the list's keys do nothing while the menu is open; Ctrl+Shift+C
//! copies the hashes at once and Ctrl+C the magnet links, once the daemon has sent them.
//!
//! Each test builds the two channels of a service itself and opens the window with no service
//! behind them, so nothing touches the network: the test reads the commands the window sends and
//! answers them with the events a service would send. The clipboard is the test platform's own,
//! in memory. The window is made active, as the user's window is when they press a key: links
//! that arrive while 4x4 is not in focus are not written.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::table::TableState;
use gpui_kit::test::ClickOptions;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, ClipboardItem, Entity, Focusable as _, InputEvent as _,
    KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, OperatingSystem, Pixels, Point, TestAppContext, VisualTestContext, point, px,
};
use x4_app::app;
use x4_app::main_view::MainView;
use x4_app::torrent_table::TorrentTable;
use x4_core::test_support::torrent;
use x4_core::{
    Action, ActionError, ActionId, Command, ConnectionError, ConnectionState, DisplayState, Event,
    QueueKind, Snapshot, SortDirection, Torrent, UiState, ViewState,
};
use x4_rpc::HashString;

/// The items of the context menu, as their labels read.
const START: &str = "Start";
const PAUSE: &str = "Pause";
const START_NOW: &str = "Start now";
const VERIFY: &str = "Verify";
const REANNOUNCE: &str = "Reannounce";
const COPY_MAGNET: &str = "Copy magnet link";
const COPY_HASH: &str = "Copy hash";
const REMOVE_FROM_LIST: &str = "Remove from list…";
const REMOVE_WITH_DATA: &str = "Remove and delete files…";
/// The id of an open menu.
const MENU: &str = "popup-menu";
/// The confirm button of the dialog that removes torrents from the list.
const LIGHT_CONFIRM: &str = "remove-from-list-confirm";
/// The buttons of the dialog that removes torrents and deletes their files.
const DANGER_CANCEL: &str = "remove-files-cancel";
const DANGER_CONFIRM: &str = "remove-files-confirm";
/// The id of a dialog.
const DIALOG: &str = "dialog";
/// The id of a notification.
const NOTIFICATION: &str = "notification";
/// The id of the search box.
const SEARCH: &str = "search";
/// The id of gpui-component's table, the header and the rows.
const TABLE: &str = "table-inner";
/// The id of what marks a selected row that the table does not mark itself.
const ROW_SELECTION: &str = "row-selection";

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

    /// Answers the read of magnet links `id` with `result`, as the service does.
    fn answer(
        &self,
        id: ActionId,
        result: Result<Vec<(HashString, String)>, ActionError>,
        cx: &mut TestAppContext,
    ) {
        self.send_event(Event::MagnetLinks { id, result }, cx);
    }

    /// Answers the action `id` with `result`.
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

    /// Right-clicks the row `row`, which opens the context menu on the next frame.
    fn right_click_row(&self, row: usize, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| window.right_click(("row", row), cx));
    }

    /// Where the middle of the row at `row` was drawn last.
    fn row_middle(&self, row: usize, cx: &mut TestAppContext) -> Point<Pixels> {
        self.act(cx, |window, _| window.find(("row", row)).bounds().center())
    }

    /// Lets `change` change the rows of the view, then right-clicks at `position` before a new
    /// frame is drawn, as when an update comes between the last frame and the press: what the user
    /// right-clicks is the row they saw there. (The test platform draws a frame as soon as the
    /// effects of an update have run, so the change and the press go in one update.)
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a closed window fails the test"
    )]
    fn right_click_as_drawn(
        &self,
        position: Point<Pixels>,
        change: impl FnOnce(&mut MainView, &mut gpui_kit::Context<'_, MainView>),
        cx: &mut TestAppContext,
    ) {
        let modifiers = Modifiers::none();
        let view = self.view.clone();
        cx.update_window(self.handle, |_, window, cx| {
            view.update(cx, change);
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
                    button: MouseButton::Right,
                    position,
                    modifiers,
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
        })
        .expect("the window is open");
        cx.run_until_parked();
        self.act(cx, |window, cx| {
            window.dispatch_event(
                MouseUpEvent {
                    button: MouseButton::Right,
                    position,
                    modifiers,
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
        });
    }

    /// Right-clicks the table below its rows, 40 pixels above its bottom edge.
    fn right_click_below_the_rows(&self, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| {
            let height = window.find(TABLE).bounds().size.height;
            window.click_with_options(
                TABLE,
                ClickOptions::new()
                    .with_button(MouseButton::Right)
                    .with_offset(point(px(100.), height - px(40.))),
                cx,
            );
        });
    }

    /// Right-clicks the table's header, 10 pixels below its top edge.
    fn right_click_header(&self, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| {
            window.click_with_options(
                TABLE,
                ClickOptions::new()
                    .with_button(MouseButton::Right)
                    .with_offset(point(px(100.), px(10.))),
                cx,
            );
        });
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

    /// Makes another app's window the active one, as when the user switches away from 4x4.
    fn deactivate(&self, cx: &mut TestAppContext) {
        VisualTestContext::from_window(self.handle, cx).deactivate_window();
        cx.run_until_parked();
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

    /// The element id of the open menu's item `label`.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a missing menu item fails the test"
    )]
    fn item(&self, label: &str, cx: &mut TestAppContext) -> gpui_kit::ElementId {
        self.act(cx, |window, _| {
            window
                .within(MENU)
                .find_by_label(label)
                .path()
                .last()
                .cloned()
                .expect("the item has an id")
        })
    }

    /// Clicks the item `label` of the open menu.
    fn choose(&self, label: &str, cx: &mut TestAppContext) {
        let item = self.item(label, cx);
        self.act(cx, |window, cx| window.within(MENU).click(item, cx));
    }

    /// Points at the item `label` of the open menu, which highlights it for Enter.
    fn point_at(&self, label: &str, cx: &mut TestAppContext) {
        let item = self.item(label, cx);
        self.act(cx, |window, cx| window.within(MENU).hover(item, cx));
    }

    /// Whether a menu is open.
    fn menu_open(&self, cx: &mut TestAppContext) -> bool {
        self.act(cx, |window, _| window.try_find(MENU).is_some())
    }

    /// Whether the last frame shows the key hint `selector`, `kbd:` and the keys in gpui's
    /// spelling, such as `kbd:ctrl-s`.
    fn shows_key(&self, selector: &'static str, cx: &mut TestAppContext) -> bool {
        // A frame with the menu as it is now.
        self.act(cx, |_, _| ());
        VisualTestContext::from_window(self.handle, cx)
            .debug_bounds(selector)
            .is_some()
    }

    /// Whether the element `id` is on screen.
    fn shown(&self, id: &'static str, cx: &mut TestAppContext) -> bool {
        self.act(cx, |window, _| window.try_find(id).is_some())
    }

    /// Whether the element `id` has the keyboard focus.
    fn focused(&self, id: &'static str, cx: &mut TestAppContext) -> bool {
        self.act(cx, |window, _| window.find(id).focused() == Some(true))
    }

    /// How many dialogs are open.
    fn dialogs(&self, cx: &mut TestAppContext) -> usize {
        self.act(cx, |window, _| window.find_all(DIALOG).len())
    }

    /// How many notifications are on screen.
    fn notifications(&self, cx: &mut TestAppContext) -> usize {
        self.act(cx, |window, _| window.find_all(NOTIFICATION).len())
    }

    /// How many notifications are on screen once the ones that were closed have finished leaving:
    /// a closed notification fades out first, and counts until it is gone.
    fn notifications_left(&self, cx: &mut TestAppContext) -> usize {
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        self.notifications(cx)
    }

    fn last_failure(&self, cx: &mut TestAppContext) -> Option<String> {
        cx.update(|cx| self.view.read(cx).last_failure().map(str::to_owned))
    }

    /// Whether the table has the keyboard focus, so that the list's keys reach it.
    fn table_focused(&self, cx: &mut TestAppContext) -> bool {
        let table = self.table(cx);
        self.act(cx, |window, cx| table.focus_handle(cx).is_focused(window))
    }

    fn table(&self, cx: &mut TestAppContext) -> Entity<TableState<TorrentTable>> {
        cx.update(|cx| self.view.read(cx).table().clone())
    }

    /// The names of the selected rows among those shown, in the order shown.
    fn selected(&self, cx: &mut TestAppContext) -> Vec<String> {
        let table = self.table(cx);
        cx.update(|cx| {
            let table = table.read(cx).delegate();
            table
                .selection()
                .targets(table.rows())
                .iter()
                .map(|row| row.name.clone())
                .collect()
        })
    }

    /// The table's own selected row, which the table paints and moves with its keys.
    fn table_row(&self, cx: &mut TestAppContext) -> Option<usize> {
        let table = self.table(cx);
        cx.update(|cx| table.read(cx).selected_row())
    }

    /// The rows on screen drawn as selected, by the table or by our mark.
    fn drawn_selected(&self, cx: &mut TestAppContext) -> Vec<usize> {
        let table = self.table(cx);
        let count = cx.update(|cx| table.read(cx).delegate().rows().len());
        self.act(cx, |window, _| {
            (0..count)
                .filter(|&row| {
                    window
                        .try_find(("row", row))
                        .and_then(|drawn| drawn.selected())
                        == Some(true)
                        || window.try_find((ROW_SELECTION, row)).is_some()
                })
                .collect()
        })
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

    /// The one read of magnet links the window has sent since the last look, with its id.
    #[track_caller]
    #[expect(
        clippy::panic,
        reason = "a test helper: anything but one read fails the test"
    )]
    fn sent_read(&self) -> (ActionId, Vec<HashString>) {
        match self.sent().as_slice() {
            [Command::ReadMagnetLinks(id, hashes)] => (*id, hashes.clone()),
            other => panic!("expected one read of magnet links, got {other:?}"),
        }
    }
}

/// The text on the test platform's clipboard.
fn clipboard(cx: &TestAppContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|item| item.text())
}

/// Puts `text` on the clipboard, as another app would.
fn put_on_clipboard(text: &str, cx: &TestAppContext) {
    cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
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

/// Downloading torrents called `names`, which sort in that order.
fn downloading_all(names: &[&str]) -> Vec<Torrent> {
    names.iter().map(|name| downloading(name)).collect()
}

/// `count` torrents, "t000" to "t099" and on, which sort in that order by name.
fn many(count: usize) -> Vec<Torrent> {
    (0..count).map(|ix| torrent(&format!("t{ix:03}"))).collect()
}

/// The hashes of the torrents called `names`, in that order.
fn hashes(names: &[&str]) -> Vec<HashString> {
    names.iter().map(|name| torrent(name).hash).collect()
}

/// The hashes of the torrents called `names`, one per line, as Copy hash writes them.
fn hash_lines(names: &[&str]) -> String {
    hashes(names)
        .iter()
        .map(HashString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The made-up magnet link of the torrent `name`.
fn magnet(name: &str) -> String {
    format!("magnet:?xt=urn:btih:{}&dn={name}", torrent(name).hash)
}

/// The daemon's answer to a read of the magnet links of `names`.
fn links(names: &[&str]) -> Vec<(HashString, String)> {
    names
        .iter()
        .map(|name| (torrent(name).hash, magnet(name)))
        .collect()
}

/// The magnet links of `names`, one per line, as Copy magnet link writes them.
fn magnet_lines(names: &[&str]) -> String {
    names
        .iter()
        .map(|name| magnet(name))
        .collect::<Vec<_>>()
        .join("\n")
}

fn reconnecting() -> ConnectionState {
    ConnectionState::Reconnecting {
        attempt: 2,
        last_error: ConnectionError::Timeout,
    }
}

#[gpui_kit::test]
fn right_click_on_an_unselected_row_selects_only_it(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(downloading_all(&["a", "b", "c", "d"]), cx);
    window.click_row(0, cx);
    window.ctrl_click_row(1, cx);

    window.right_click_row(3, cx);

    assert_eq!(window.selected(cx), ["d"]);
    assert_eq!(
        window.table_row(cx),
        Some(3),
        "the table marks it as its row"
    );
    assert_eq!(window.drawn_selected(cx), [3]);
    assert!(window.menu_open(cx));
    assert_eq!(window.sent(), [], "opening the menu sends nothing");
    window.choose(PAUSE, cx);
    assert_eq!(window.sent_action().1, Action::Stop(hashes(&["d"])));
    assert!(!window.menu_open(cx));
}

#[gpui_kit::test]
fn right_click_inside_the_selection_keeps_it(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(downloading_all(&["a", "b", "c", "d"]), cx);
    window.click_row(0, cx);
    window.ctrl_click_row(2, cx);

    window.right_click_row(0, cx);

    assert_eq!(window.selected(cx), ["a", "c"]);
    assert_eq!(window.table_row(cx), Some(2), "the lead stays where it was");
    assert_eq!(window.drawn_selected(cx), [0, 2]);
    window.choose(PAUSE, cx);
    assert_eq!(window.sent_action().1, Action::Stop(hashes(&["a", "c"])));

    // The lead itself, too.
    window.right_click_row(2, cx);
    assert_eq!(window.selected(cx), ["a", "c"]);
    window.choose(REANNOUNCE, cx);
    assert_eq!(
        window.sent_action().1,
        Action::Reannounce(hashes(&["a", "c"]))
    );
}

/// A right-click selects the torrent the user saw under the pointer, also when the rows have been
/// re-sorted since the last frame was drawn, and opens the menu for it.
#[gpui_kit::test]
fn a_right_click_selects_the_torrent_that_was_pressed_even_if_rows_moved(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(downloading_all(&["a", "b"]), cx);

    let position = window.row_middle(0, cx);
    // Sorted the other way, "a" moves down a row before the next frame is drawn.
    window.right_click_as_drawn(
        position,
        |view, cx| {
            let sorted = ViewState {
                sort_direction: SortDirection::Descending,
                ..view.view_state().clone()
            };
            view.set_view(sorted, cx);
        },
        cx,
    );

    assert_eq!(window.selected(cx), ["a"]);
    assert_eq!(window.table_row(cx), Some(1), "the table's row follows");
    assert_eq!(window.drawn_selected(cx), [1]);
    assert!(window.menu_open(cx));
    window.choose(PAUSE, cx);
    assert_eq!(window.sent_action().1, Action::Stop(hashes(&["a"])));

    // A torrent right-clicked that has left the list by then (here, hidden by a search) opens no
    // menu, and the one that took its place is not right-clicked.
    let position = window.row_middle(0, cx);
    window.right_click_as_drawn(
        position,
        |view, cx| {
            let searched = ViewState {
                search: "a".to_owned(),
                ..view.view_state().clone()
            };
            view.set_view(searched, cx);
        },
        cx,
    );
    assert!(!window.menu_open(cx));
    assert_eq!(window.selected(cx), ["a"], "the selection stays as it was");
    assert!(window.table_focused(cx), "the list keeps the keys");
    window.press("ctrl-r", cx);
    assert_eq!(window.sent_action().1, Action::Reannounce(hashes(&["a"])));
}

#[gpui_kit::test]
fn menu_items_dispatch_their_actions(cx: &mut TestAppContext) {
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
    let all = hashes(&["a", "b", "c"]);

    // Each item names its key, which it looks up in the list, even when the keys were elsewhere,
    // in the search box, as the menu opened.
    window.press("ctrl-f", cx);
    window.right_click_row(1, cx);
    for hint in [
        "kbd:ctrl-s",
        "kbd:ctrl-p",
        "kbd:ctrl-shift-s",
        "kbd:ctrl-shift-r",
        "kbd:ctrl-r",
        "kbd:ctrl-c",
        "kbd:ctrl-shift-c",
        "kbd:delete",
        "kbd:shift-delete",
    ] {
        assert!(window.shows_key(hint, cx), "the menu shows {hint}");
    }
    window.press("escape", cx);
    assert!(!window.menu_open(cx));
    assert!(
        window.focused(SEARCH, cx),
        "Escape closes the menu and leaves the keys where they were"
    );

    for (label, action) in [
        (START, Action::Start(all.clone())),
        (PAUSE, Action::Stop(all.clone())),
        (START_NOW, Action::StartNow(all.clone())),
        (VERIFY, Action::Verify(all.clone())),
        (REANNOUNCE, Action::Reannounce(all.clone())),
    ] {
        // The search box has the keys; a right-click leaves them there, and the item hands them
        // to the list, as a toolbar button does.
        window.press("ctrl-f", cx);
        window.right_click_row(1, cx);
        window.choose(label, cx);
        assert_eq!(window.sent_action().1, action, "{label}");
        assert!(window.table_focused(cx), "{label}: the list has the keys");
    }

    window.press("ctrl-f", cx);
    window.right_click_row(0, cx);
    window.choose(COPY_MAGNET, cx);
    let (id, read) = window.sent_read();
    assert_eq!(read, all);
    assert!(window.table_focused(cx));
    window.answer(id, Ok(links(&["a", "b", "c"])), cx);
    assert_eq!(
        clipboard(cx).as_deref(),
        Some(magnet_lines(&["a", "b", "c"]).as_str())
    );

    window.press("ctrl-f", cx);
    window.right_click_row(2, cx);
    window.choose(COPY_HASH, cx);
    assert_eq!(
        clipboard(cx).as_deref(),
        Some(hash_lines(&["a", "b", "c"]).as_str())
    );
    assert_eq!(window.sent(), []);
    assert!(window.table_focused(cx));

    // The removal items open their dialogs, which ask first.
    window.right_click_row(0, cx);
    window.choose(REMOVE_FROM_LIST, cx);
    assert!(window.focused(LIGHT_CONFIRM, cx));
    window.press("escape", cx);
    assert!(window.table_focused(cx), "the dialog gave the keys back");
    window.right_click_row(0, cx);
    window.choose(REMOVE_WITH_DATA, cx);
    assert!(window.shown(DANGER_CONFIRM, cx));
    assert!(window.focused(DANGER_CANCEL, cx));
    window.press("escape", cx);
    assert_eq!(window.sent(), [], "nothing is removed without a confirm");
}

#[gpui_kit::test]
fn menu_items_are_disabled_while_disconnected_except_copy_hash(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![paused("a"), downloading("b")], cx);
    window.press("ctrl-a", cx);
    window.send_with(vec![paused("a"), downloading("b")], reconnecting(), cx);

    window.right_click_row(0, cx);
    for label in [
        START,
        PAUSE,
        START_NOW,
        VERIFY,
        REANNOUNCE,
        COPY_MAGNET,
        REMOVE_FROM_LIST,
        REMOVE_WITH_DATA,
    ] {
        window.choose(label, cx);
        assert!(
            window.menu_open(cx),
            "{label} is disabled: a click leaves the menu open"
        );
    }
    assert_eq!(window.sent(), []);
    assert_eq!(window.dialogs(cx), 0);

    // Pointed at and chosen with Enter, a disabled item still does nothing.
    window.point_at(PAUSE, cx);
    window.press("enter", cx);
    assert_eq!(window.sent(), []);

    window.right_click_row(0, cx);
    put_on_clipboard("other", cx);
    window.choose(COPY_HASH, cx);
    assert!(!window.menu_open(cx), "Copy hash works offline");
    assert_eq!(
        clipboard(cx).as_deref(),
        Some(hash_lines(&["a", "b"]).as_str())
    );

    // A menu opened while connected re-reads the connection when an item is chosen.
    window.send(vec![paused("a"), downloading("b")], cx);
    window.right_click_row(1, cx);
    window.send_with(vec![paused("a"), downloading("b")], reconnecting(), cx);
    window.choose(PAUSE, cx);
    assert_eq!(
        window.sent(),
        [],
        "the daemon went away while the menu was open"
    );
}

#[gpui_kit::test]
fn delete_with_the_menu_open_does_nothing_to_torrents(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(downloading_all(&["a", "b"]), cx);
    window.click_row(0, cx);
    window.right_click_row(0, cx);

    for key in ["delete", "shift-delete", "space", "ctrl-p", "ctrl-a"] {
        window.press(key, cx);
        assert!(window.menu_open(cx), "{key}");
    }

    assert_eq!(window.sent(), []);
    assert_eq!(window.dialogs(cx), 0);
    assert_eq!(window.selected(cx), ["a"]);

    // Closed, the menu gives the keys back to the list.
    window.press("escape", cx);
    assert!(!window.menu_open(cx));
    window.press("delete", cx);
    assert!(window.shown(LIGHT_CONFIRM, cx));
}

#[gpui_kit::test]
fn page_keys_with_the_menu_open_do_not_move_the_selection(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(many(60), cx);
    window.click_row(5, cx);
    window.right_click_row(5, cx);

    for key in ["home", "end", "pageup", "pagedown", "down", "up"] {
        window.press(key, cx);
        assert_eq!(window.selected(cx), ["t005"], "{key}");
        assert_eq!(window.table_row(cx), Some(5), "{key}");
    }
    assert!(window.menu_open(cx));

    window.press("escape", cx);
    window.press("end", cx);
    assert_eq!(
        window.selected(cx),
        ["t059"],
        "closed, the keys move it again"
    );
}

#[gpui_kit::test]
fn focus_keys_with_the_menu_open_do_nothing(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(downloading_all(&["a", "b"]), cx);
    window.click_row(0, cx);
    window.right_click_row(0, cx);

    for key in ["tab", "shift-tab", "ctrl-f", "/"] {
        window.press(key, cx);
        assert!(window.menu_open(cx), "{key}");
        assert!(
            !window.table_focused(cx),
            "{key}: the keys stay in the menu"
        );
        assert!(!window.focused(SEARCH, cx), "{key}");
    }
    window.press("space", cx);
    window.press("ctrl-p", cx);
    assert_eq!(window.sent(), [], "nothing acts behind the menu");

    window.press("escape", cx);
    assert!(window.table_focused(cx));
    window.press("ctrl-f", cx);
    assert!(
        window.focused(SEARCH, cx),
        "closed, the menu lets Ctrl+F work again"
    );
}

#[gpui_kit::test]
fn a_right_click_below_the_rows_opens_no_menu(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(downloading_all(&["a", "b"]), cx);

    window.right_click_below_the_rows(cx);
    assert!(!window.menu_open(cx));

    // Also after a right-click on a row, which the table goes on marking.
    window.right_click_row(0, cx);
    assert!(window.menu_open(cx));
    window.press("escape", cx);
    window.press("ctrl-a", cx);
    window.right_click_below_the_rows(cx);
    assert!(!window.menu_open(cx));
    assert_eq!(window.selected(cx), ["a", "b"]);
    assert!(window.table_focused(cx), "the list keeps the keys");
    window.press("ctrl-p", cx);
    assert_eq!(
        window.sent_action().1,
        Action::Stop(hashes(&["a", "b"])),
        "the keys still reach the list"
    );

    // The common case: after a left click the table has forgotten the row it was right-clicked
    // on, and the empty menu that gpui-component opens must not keep the keys.
    window.click_row(0, cx);
    window.right_click_below_the_rows(cx);
    assert!(!window.menu_open(cx));
    assert!(window.table_focused(cx), "the list keeps the keys");
    window.press("ctrl-p", cx);
    assert_eq!(
        window.sent_action().1,
        Action::Stop(hashes(&["a"])),
        "the keys still reach the list"
    );

    // The header is no row either.
    window.right_click_header(cx);
    assert!(!window.menu_open(cx));
    assert!(window.table_focused(cx), "the list keeps the keys");

    // Where the focus was stays where it was: the search box keeps it.
    window.press("ctrl-f", cx);
    assert!(window.focused(SEARCH, cx));
    window.right_click_below_the_rows(cx);
    assert!(!window.menu_open(cx));
    assert!(window.focused(SEARCH, cx), "the search box keeps the keys");

    // A row still opens it.
    window.right_click_row(1, cx);
    assert!(window.menu_open(cx));
}

#[gpui_kit::test]
fn an_item_disabled_when_the_menu_opened_does_nothing_on_enter(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(downloading_all(&["a"]), cx);
    window.click_row(0, cx);
    window.send_with(downloading_all(&["a"]), reconnecting(), cx);
    window.right_click_row(0, cx);

    // The daemon is back while the menu is open; Pause was drawn disabled.
    window.send(downloading_all(&["a"]), cx);
    assert!(window.menu_open(cx));
    window.point_at(PAUSE, cx);
    window.press("enter", cx);

    assert_eq!(window.sent(), []);
}

#[gpui_kit::test]
fn a_snapshot_while_the_menu_is_open_does_not_change_what_it_acts_on(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(downloading_all(&["a", "b", "c"]), cx);
    window.click_row(1, cx);
    window.ctrl_click_row(2, cx);
    window.right_click_row(1, cx);

    // Two new torrents sort first and move the lead, c, down; b is gone.
    window.send(downloading_all(&["0 new", "00 newer", "a", "c"]), cx);
    assert_eq!(window.table_row(cx), Some(3), "the table's row followed c");
    assert!(window.menu_open(cx));

    window.choose(PAUSE, cx);
    assert_eq!(
        window.sent_action().1,
        Action::Stop(hashes(&["b", "c"])),
        "the torrents of the moment the menu opened"
    );
}

#[gpui_kit::test]
fn ctrl_shift_c_copies_hashes_in_display_order(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(downloading_all(&["a", "b", "c", "d"]), cx);
    // Selected from the bottom up: they are copied as the list shows them.
    window.click_row(3, cx);
    window.ctrl_click_row(0, cx);
    window.ctrl_click_row(2, cx);

    window.press("ctrl-shift-c", cx);

    let copied = hash_lines(&["a", "c", "d"]);
    assert_eq!(clipboard(cx).as_deref(), Some(copied.as_str()));
    assert_eq!(window.sent(), [], "the hashes are in the list already");

    // Offline too.
    window.send_with(downloading_all(&["a", "b", "c", "d"]), reconnecting(), cx);
    put_on_clipboard("other", cx);
    window.press("ctrl-shift-c", cx);
    assert_eq!(clipboard(cx).as_deref(), Some(copied.as_str()));

    // A held key copies once.
    put_on_clipboard("other", cx);
    window.key_down("ctrl-shift-c", false, cx);
    assert_eq!(clipboard(cx).as_deref(), Some(copied.as_str()));
    put_on_clipboard("other", cx);
    window.key_down("ctrl-shift-c", true, cx);
    window.key_down("ctrl-shift-c", true, cx);
    window.key_up("ctrl-shift-c", cx);
    assert_eq!(clipboard(cx).as_deref(), Some("other"));

    // Nothing selected, nothing copied.
    window.press("escape", cx);
    window.press("ctrl-shift-c", cx);
    assert_eq!(clipboard(cx).as_deref(), Some("other"));
    assert_eq!(window.sent(), []);
}

#[gpui_kit::test]
fn copy_magnet_writes_the_links_when_they_arrive(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(downloading_all(&["a", "b", "c"]), cx);
    window.click_row(2, cx);
    window.ctrl_click_row(0, cx);

    window.press("ctrl-c", cx);
    let (id, read) = window.sent_read();
    assert_eq!(read, hashes(&["a", "c"]), "in the order shown");
    assert_eq!(clipboard(cx), None, "nothing until the links arrive");

    window.answer(id, Ok(links(&["a", "c"])), cx);
    assert_eq!(
        clipboard(cx).as_deref(),
        Some(magnet_lines(&["a", "c"]).as_str())
    );
    assert_eq!(window.last_failure(cx), None);
    assert_eq!(window.sent(), []);

    // A failed read is shown as a failed action is, and copies nothing.
    window.press("ctrl-c", cx);
    let (id, _) = window.sent_read();
    window.answer(id, Err(ActionError::Rpc(ConnectionError::Timeout)), cx);
    assert_eq!(
        window.last_failure(cx).as_deref(),
        Some("Could not copy the magnet links of 2 torrents: timed out")
    );
    assert_eq!(window.notifications(cx), 1);
    assert_eq!(
        clipboard(cx).as_deref(),
        Some(magnet_lines(&["a", "c"]).as_str())
    );

    // The daemon has none of them any more: a failure, not an empty clipboard.
    window.click_row(1, cx);
    window.press("ctrl-c", cx);
    let (id, _) = window.sent_read();
    window.answer(id, Ok(Vec::new()), cx);
    assert_eq!(
        window.last_failure(cx).as_deref(),
        Some("Could not copy the magnet link: the torrent is no longer on the server")
    );
    assert_eq!(window.notifications(cx), 1, "a newer failure replaces it");
    assert_eq!(
        clipboard(cx).as_deref(),
        Some(magnet_lines(&["a", "c"]).as_str())
    );

    // The links are read from the daemon: offline, nothing is asked.
    window.send_with(downloading_all(&["a", "b", "c"]), reconnecting(), cx);
    window.press("ctrl-c", cx);
    assert_eq!(window.sent(), []);
}

#[gpui_kit::test]
fn a_partial_magnet_answer_copies_what_came_and_says_so(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(downloading_all(&["a", "b", "c"]), cx);
    window.press("ctrl-a", cx);
    window.press("ctrl-c", cx);
    let (id, _) = window.sent_read();

    // b is gone from the daemon.
    window.answer(id, Ok(links(&["a", "c"])), cx);

    assert_eq!(
        clipboard(cx).as_deref(),
        Some(magnet_lines(&["a", "c"]).as_str())
    );
    assert_eq!(
        window.last_failure(cx).as_deref(),
        Some("Copied 2 of 3 magnet links; 1 torrent is no longer on the server")
    );
    assert_eq!(window.notifications(cx), 1);
}

#[gpui_kit::test]
fn links_that_arrive_while_4x4_is_not_in_focus_ask_for_another_press(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(downloading_all(&["a", "b"]), cx);
    window.click_row(0, cx);
    window.press("ctrl-c", cx);
    let (id, _) = window.sent_read();

    // The user switched to another app before the daemon answered: on Wayland only the window in
    // focus may set the clipboard.
    window.deactivate(cx);
    put_on_clipboard("other", cx);
    window.answer(id, Ok(links(&["a"])), cx);

    assert_eq!(clipboard(cx).as_deref(), Some("other"));
    assert_eq!(
        window.last_failure(cx).as_deref(),
        Some("The magnet links are ready, but 4x4 was not in focus — copy them again")
    );
    assert_eq!(window.notifications(cx), 1);
}

#[gpui_kit::test]
fn a_magnet_copy_that_succeeds_removes_the_notices_of_the_ones_that_did_not(
    cx: &mut TestAppContext,
) {
    let window = Window::open(cx);
    window.send(downloading_all(&["a", "b", "c"]), cx);
    window.press("ctrl-a", cx);

    // "Copied 2 of 3" stays until a copy of all of them replaces it.
    window.press("ctrl-c", cx);
    let (id, _) = window.sent_read();
    window.answer(id, Ok(links(&["a", "c"])), cx);
    assert_eq!(window.notifications(cx), 1);
    window.press("ctrl-c", cx);
    let (id, _) = window.sent_read();
    window.answer(id, Ok(links(&["a", "b", "c"])), cx);
    assert_eq!(
        window.notifications_left(cx),
        0,
        "the partial copy is old news"
    );
    assert_eq!(
        clipboard(cx).as_deref(),
        Some(magnet_lines(&["a", "b", "c"]).as_str())
    );

    // "Copy them again" goes once the user has.
    window.press("ctrl-c", cx);
    let (id, _) = window.sent_read();
    window.deactivate(cx);
    window.answer(id, Ok(links(&["a", "b", "c"])), cx);
    assert_eq!(window.notifications(cx), 1);
    window.activate(cx);
    window.press("ctrl-c", cx);
    let (id, _) = window.sent_read();
    window.answer(id, Ok(links(&["a", "b", "c"])), cx);
    assert_eq!(
        window.notifications_left(cx),
        0,
        "the user pressed it again"
    );

    // A failed read goes the same way.
    window.press("ctrl-c", cx);
    let (id, _) = window.sent_read();
    window.answer(id, Err(ActionError::Rpc(ConnectionError::Timeout)), cx);
    assert_eq!(window.notifications(cx), 1);
    window.press("ctrl-c", cx);
    let (id, _) = window.sent_read();
    window.answer(id, Ok(links(&["a", "b", "c"])), cx);
    assert_eq!(window.notifications_left(cx), 0, "the failure is old news");
}

#[gpui_kit::test]
fn a_newer_magnet_copy_supersedes_a_pending_one(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(downloading_all(&["a", "b"]), cx);
    window.click_row(0, cx);
    window.press("ctrl-c", cx);
    let (older, _) = window.sent_read();

    // Pressed again before the answer: the same read is on its way.
    window.press("ctrl-c", cx);
    assert_eq!(window.sent(), []);

    window.click_row(1, cx);
    window.press("ctrl-c", cx);
    let (newer, read) = window.sent_read();
    assert_eq!(read, hashes(&["b"]));
    assert_ne!(newer, older);

    // The older answer comes first, and is not what the user asked for last.
    window.answer(older, Ok(links(&["a"])), cx);
    assert_eq!(clipboard(cx), None);
    window.answer(newer, Ok(links(&["b"])), cx);
    assert_eq!(clipboard(cx).as_deref(), Some(magnet("b").as_str()));

    // An older answer that comes late changes nothing, a failure neither.
    window.press("ctrl-c", cx);
    let (pending, _) = window.sent_read();
    window.click_row(0, cx);
    window.press("ctrl-c", cx);
    let (newest, _) = window.sent_read();
    window.answer(newest, Ok(links(&["a"])), cx);
    window.answer(pending, Err(ActionError::Rpc(ConnectionError::Timeout)), cx);
    assert_eq!(clipboard(cx).as_deref(), Some(magnet("a").as_str()));
    assert_eq!(window.last_failure(cx), None);

    // A held Ctrl+C reads once.
    window.key_down("ctrl-c", false, cx);
    let (held, _) = window.sent_read();
    window.answer(held, Ok(links(&["a"])), cx);
    for _ in 0..3 {
        window.key_down("ctrl-c", true, cx);
    }
    window.key_up("ctrl-c", cx);
    assert_eq!(window.sent(), [], "the repeats of a held key read nothing");
}

#[gpui_kit::test]
fn a_magnet_copy_between_two_pauses_keeps_the_pause_from_repeating(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(downloading_all(&["a", "b"]), cx);
    window.click_row(0, cx);
    window.press("ctrl-p", cx);
    let (pause, _) = window.sent_action();
    window.press("ctrl-c", cx);
    let (read, _) = window.sent_read();
    assert_ne!(read, pause, "a read and an action never share an id");

    // The pause sent last still waits, whatever was read in between.
    window.press("ctrl-p", cx);
    assert_eq!(window.sent(), []);

    window.finish(pause, Ok(()), cx);
    window.press("ctrl-p", cx);
    let (again, action) = window.sent_action();
    assert_eq!(action, Action::Stop(hashes(&["a"])));
    assert_ne!(again, read);
}

#[gpui_kit::test]
fn ctrl_c_in_the_search_box_copies_text(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(downloading_all(&["abc", "b"]), cx);
    window.click_row(0, cx);

    window.press("ctrl-f", cx);
    window.act(cx, |window, cx| window.input("abc", cx));
    window.press("ctrl-a", cx);
    window.press("ctrl-c", cx);

    assert_eq!(clipboard(cx).as_deref(), Some("abc"));
    window.act(cx, |window, _| {
        assert_eq!(window.find(SEARCH).value(), Some("abc"));
    });
    window.press("ctrl-shift-c", cx);
    assert_eq!(clipboard(cx).as_deref(), Some("abc"));
    assert_eq!(window.sent(), [], "the keys never reached the torrents");
}
