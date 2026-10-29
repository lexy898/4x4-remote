//! Headless tests for selecting several torrents in the main window's table: Ctrl+click,
//! Shift+click, Ctrl+A, Escape and Ctrl+Shift+A, the arrow and page keys with and without Shift,
//! how every selected row is drawn and kept in view, what a new snapshot, a filter or a search
//! does to the selection, and the table of key bindings.
//!
//! Each test builds the two channels of a service itself and opens the window with no service
//! behind them, so nothing touches the network. The tests drive the table through real clicks and
//! key presses, and read it through the delegate's selection, the table's own selected row and the
//! elements drawn for the rows.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::table::TableState;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Entity, Modifiers, ScrollDelta, TestAppContext, point,
    px,
};
use x4_app::app;
use x4_app::keymap::{BINDINGS, key_bindings};
use x4_app::main_view::MainView;
use x4_app::torrent_table::TorrentTable;
use x4_core::test_support::torrent;
use x4_core::{
    Command, ConnectionState, DisplayState, Event, Filter, Snapshot, Torrent, UiState, ViewState,
};

/// The id of the search box.
const SEARCH: &str = "search";
/// The id of what marks a selected row that the table does not mark itself.
const ROW_SELECTION: &str = "row-selection";

/// The names of keys that gpui's Linux platform reports, besides the keys that type one
/// character (`platform.rs`, `keystroke_from_xkb`). gpui parses any other word as a key name too,
/// so a misspelled one would never match a key press.
const NAMED_KEYS: &[&str] = &[
    "enter",
    "escape",
    "tab",
    "space",
    "backspace",
    "delete",
    "insert",
    "up",
    "down",
    "left",
    "right",
    "home",
    "end",
    "pageup",
    "pagedown",
    "f1",
    "f2",
    "f3",
    "f4",
    "f5",
    "f6",
    "f7",
    "f8",
    "f9",
    "f10",
    "f11",
    "f12",
];

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

    /// Sends a snapshot of `torrents` as the service would and lets the window take it.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a window that stopped listening fails the test"
    )]
    fn send(&self, torrents: Vec<Torrent>, cx: &mut TestAppContext) {
        self.events
            .send_blocking(Event::Snapshot(Arc::new(Snapshot {
                torrents,
                listed: true,
                session: None,
                free_space_bytes: None,
                rate_down: 0,
                rate_up: 0,
                connection: ConnectionState::Connected,
                stale: false,
            })))
            .expect("the window listens");
        cx.run_until_parked();
    }

    /// Shows the torrents that pass `filter` and match `search`, by name.
    fn show(&self, filter: Filter, search: &str, cx: &mut TestAppContext) {
        cx.update(|cx| {
            self.view.update(cx, |view, cx| {
                view.set_view(
                    ViewState {
                        filter,
                        search: search.to_owned(),
                        ..ViewState::default()
                    },
                    cx,
                );
            });
        });
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

    fn click(&self, row: usize, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| window.click(("row", row), cx));
    }

    fn ctrl_click(&self, row: usize, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| {
            window.click_with_modifiers(("row", row), Modifiers::control(), cx);
        });
    }

    fn shift_click(&self, row: usize, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| {
            window.click_with_modifiers(("row", row), Modifiers::shift(), cx);
        });
    }

    fn press(&self, key: &str, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| window.press(key, cx));
    }

    fn table(&self, cx: &mut TestAppContext) -> Entity<TableState<TorrentTable>> {
        cx.update(|cx| self.view.read(cx).table().clone())
    }

    /// Reads the delegate of the table.
    fn read<R>(&self, cx: &mut TestAppContext, read: impl FnOnce(&TorrentTable) -> R) -> R {
        let table = self.table(cx);
        cx.update(|cx| read(table.read(cx).delegate()))
    }

    /// The names of the selected rows among those shown, in the order shown.
    fn selected(&self, cx: &mut TestAppContext) -> Vec<String> {
        self.read(cx, |table| {
            table
                .selection()
                .targets(table.rows())
                .iter()
                .map(|row| row.name.clone())
                .collect()
        })
    }

    /// How many torrents are selected, shown or not.
    fn selected_count(&self, cx: &mut TestAppContext) -> usize {
        self.read(cx, |table| table.selection().len())
    }

    /// The table's own selected row, which the table paints and moves with its keys.
    fn table_row(&self, cx: &mut TestAppContext) -> Option<usize> {
        let table = self.table(cx);
        cx.update(|cx| table.read(cx).selected_row())
    }

    /// The indices of the selected rows among those shown.
    fn selected_rows(&self, cx: &mut TestAppContext) -> Vec<usize> {
        self.read(cx, |table| {
            let rows = table.rows();
            rows.iter()
                .enumerate()
                .filter(|(_, row)| table.selection().contains(&row.hash))
                .map(|(row_ix, _)| row_ix)
                .collect()
        })
    }

    /// The rows on screen drawn as selected, either by the table (its own selected row) or by
    /// the mark we add to the other selected rows.
    fn drawn_selected(&self, cx: &mut TestAppContext) -> Vec<usize> {
        let count = self.read(cx, |table| table.rows().len());
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

    /// Asserts that only the row at `row_ix` is selected, that it is the table's own row and that
    /// it is drawn selected on screen, alone.
    #[track_caller]
    fn assert_only(&self, row_ix: usize, cx: &mut TestAppContext) {
        let name = self.read(cx, |table| {
            table.rows().get(row_ix).map(|row| row.name.clone())
        });
        assert_eq!(self.selected(cx), Vec::from_iter(name), "row {row_ix}");
        assert_eq!(self.selected_rows(cx), [row_ix]);
        assert_eq!(self.table_row(cx), Some(row_ix));
        assert_eq!(self.drawn_selected(cx), [row_ix], "on screen and drawn");
    }

    /// How many rows Page Up and Page Down move by: the rows on screen, but the one that is
    /// usually cut off at an edge.
    fn page(&self, cx: &mut TestAppContext) -> usize {
        let table = self.table(cx);
        cx.update(|cx| table.read(cx).visible_range().rows().len())
            .saturating_sub(1)
            .max(1)
    }

    /// Scrolls the table by `pixels` (negative is down) with the mouse wheel.
    fn scroll(&self, pixels: f32, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| {
            window.scroll("table", ScrollDelta::Pixels(point(px(0.), px(pixels))), cx);
        });
    }

    /// Whether the row at `row_ix` is on screen.
    fn on_screen(&self, row_ix: usize, cx: &mut TestAppContext) -> bool {
        self.act(cx, |window, _| window.try_find(("row", row_ix)).is_some())
    }

    /// The commands the window has sent so far, other than the window's visibility.
    fn sent(&self) -> Vec<Command> {
        self.commands
            .try_iter()
            .filter(|command| !matches!(command, Command::SetVisibility(_)))
            .collect()
    }
}

fn named(names: &[&str]) -> Vec<Torrent> {
    names.iter().map(|name| torrent(name)).collect()
}

/// `count` torrents, "t000" to "t099" and on, which sort in that order by name.
fn many(count: usize) -> Vec<Torrent> {
    (0..count).map(|ix| torrent(&format!("t{ix:03}"))).collect()
}

fn paused(name: &str) -> Torrent {
    Torrent {
        state: DisplayState::Paused,
        ..torrent(name)
    }
}

#[gpui_kit::test]
fn ctrl_click_adds_rows_to_the_selection(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(named(&["a", "b", "c", "d"]), cx);

    window.click(0, cx);
    window.ctrl_click(2, cx);
    assert_eq!(window.selected(cx), ["a", "c"]);
    window.ctrl_click(3, cx);
    assert_eq!(window.selected(cx), ["a", "c", "d"]);
    assert_eq!(window.table_row(cx), Some(3), "the lead is the table's row");

    // A Ctrl+click on a selected row takes it out.
    window.ctrl_click(0, cx);
    assert_eq!(window.selected(cx), ["c", "d"]);

    // A plain click selects only that row, also when it is the lead of the selection.
    window.ctrl_click(3, cx);
    window.ctrl_click(3, cx);
    assert_eq!(window.selected(cx), ["c", "d"]);
    window.click(3, cx);
    assert_eq!(window.selected(cx), ["d"]);
    assert_eq!(window.table_row(cx), Some(3));
    assert_eq!(window.sent(), [], "selecting sends nothing");
}

#[gpui_kit::test]
fn ctrl_click_on_the_selected_lead_unpaints_it(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(named(&["a", "b", "c", "d"]), cx);
    window.click(0, cx);
    window.ctrl_click(2, cx);
    assert_eq!(window.drawn_selected(cx), [0, 2]);

    // Row 2 stays the lead, but it is no longer selected, so the table must not paint it.
    window.ctrl_click(2, cx);

    assert_eq!(window.selected(cx), ["a"]);
    assert_eq!(window.table_row(cx), None);
    assert_eq!(window.drawn_selected(cx), [0]);
    window.act(cx, |window, _| {
        assert_eq!(window.find(("row", 2usize)).selected(), Some(false));
    });

    // The arrow keys go on from the lead, not from the top of the list.
    window.press("down", cx);
    assert_eq!(window.selected(cx), ["d"]);
    window.ctrl_click(2, cx);
    window.ctrl_click(2, cx);
    assert_eq!(window.table_row(cx), None);
    window.press("up", cx);
    assert_eq!(window.selected(cx), ["b"]);
    assert_eq!(window.table_row(cx), Some(1));
}

#[gpui_kit::test]
fn shift_click_selects_a_range(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(named(&["a", "b", "c", "d", "e"]), cx);

    window.click(1, cx);
    window.shift_click(3, cx);
    assert_eq!(window.selected(cx), ["b", "c", "d"]);
    assert_eq!(window.table_row(cx), Some(3));

    // The range starts where the plain click was; a new one replaces the old.
    window.shift_click(0, cx);
    assert_eq!(window.selected(cx), ["a", "b"]);
    assert_eq!(window.table_row(cx), Some(0));
    assert_eq!(window.drawn_selected(cx), [0, 1]);
    assert_eq!(window.sent(), []);
}

#[gpui_kit::test]
fn ctrl_a_selects_every_visible_row_and_only_those(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(
        vec![paused("a"), torrent("b"), paused("c"), torrent("d")],
        cx,
    );
    window.show(Filter::Paused, "", cx);
    assert_eq!(window.read(cx, |table| table.rows().len()), 2);

    window.press("ctrl-a", cx);

    assert_eq!(window.selected(cx), ["a", "c"]);
    assert_eq!(
        window.selected_count(cx),
        2,
        "the hidden rows are not selected"
    );
    assert_eq!(window.drawn_selected(cx), [0, 1]);

    // Showing every torrent again does not select the others.
    window.show(Filter::All, "", cx);
    assert_eq!(window.selected(cx), ["a", "c"]);
    assert_eq!(window.sent(), []);
}

#[gpui_kit::test]
fn escape_and_ctrl_shift_a_clear_the_selection(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(named(&["a", "b", "c"]), cx);

    window.click(0, cx);
    window.ctrl_click(2, cx);
    window.press("escape", cx);
    assert_eq!(window.selected(cx), [] as [&str; 0]);
    assert_eq!(window.selected_count(cx), 0);
    assert_eq!(window.table_row(cx), None);
    assert_eq!(window.drawn_selected(cx), [] as [usize; 0]);

    // Also when the table has no row of its own, because the lead was taken out.
    window.click(0, cx);
    window.ctrl_click(2, cx);
    window.ctrl_click(2, cx);
    assert_eq!(window.table_row(cx), None);
    window.press("escape", cx);
    assert_eq!(window.selected_count(cx), 0);
    assert_eq!(window.drawn_selected(cx), [] as [usize; 0]);

    window.press("ctrl-a", cx);
    assert_eq!(window.selected(cx), ["a", "b", "c"]);
    window.press("ctrl-shift-a", cx);
    assert_eq!(window.selected_count(cx), 0);
    assert_eq!(window.table_row(cx), None);
    assert_eq!(window.drawn_selected(cx), [] as [usize; 0]);
}

#[gpui_kit::test]
fn shift_down_extends_from_the_lead(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(named(&["a", "b", "c", "d"]), cx);

    window.click(1, cx);
    window.press("shift-down", cx);
    window.press("shift-down", cx);
    assert_eq!(window.selected(cx), ["b", "c", "d"]);
    assert_eq!(window.table_row(cx), Some(3), "the lead moved down");
    window.press("shift-down", cx);
    assert_eq!(
        window.selected(cx),
        ["b", "c", "d"],
        "it stops at the last row"
    );

    window.press("shift-up", cx);
    assert_eq!(window.selected(cx), ["b", "c"]);
    assert_eq!(window.table_row(cx), Some(2));

    // The range runs from the last Ctrl+click.
    window.click(0, cx);
    window.ctrl_click(2, cx);
    window.press("shift-down", cx);
    assert_eq!(window.selected(cx), ["c", "d"]);

    // A plain Down after a range selects only the next row.
    window.press("up", cx);
    assert_eq!(window.selected(cx), ["c"]);
    assert_eq!(window.sent(), []);
}

#[gpui_kit::test]
fn home_end_and_page_keys_select_only_the_row_they_land_on(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(many(60), cx);
    let last = 59;

    window.click(0, cx);
    window.ctrl_click(2, cx);
    window.ctrl_click(5, cx);
    window.press("end", cx);
    window.assert_only(last, cx);

    // End on the lead of a range that ends there.
    window.click(last - 2, cx);
    window.press("shift-down", cx);
    window.press("shift-down", cx);
    assert_eq!(window.selected_rows(cx), [last - 2, last - 1, last]);
    assert_eq!(window.table_row(cx), Some(last));
    window.press("end", cx);
    window.assert_only(last, cx);

    window.press("home", cx);
    window.assert_only(0, cx);

    // Home on the lead, row 0, of several selected rows.
    window.ctrl_click(1, cx);
    window.ctrl_click(3, cx);
    window.ctrl_click(0, cx);
    window.ctrl_click(0, cx);
    assert_eq!(window.selected_rows(cx), [0, 1, 3]);
    assert_eq!(window.table_row(cx), Some(0));
    window.press("home", cx);
    window.assert_only(0, cx);

    // Page Down a page below the lead.
    window.ctrl_click(2, cx);
    let page = window.page(cx);
    assert!(page > 1, "the window shows more than two rows: {page}");
    window.press("pagedown", cx);
    window.assert_only(2 + page, cx);

    // Page Down on the lead, the last row, of several selected rows; then Page Up.
    window.press("end", cx);
    window.ctrl_click(last - 3, cx);
    window.ctrl_click(last, cx);
    window.ctrl_click(last, cx);
    assert_eq!(window.selected_rows(cx), [last - 3, last]);
    assert_eq!(window.table_row(cx), Some(last));
    window.press("pagedown", cx);
    window.assert_only(last, cx);
    let page = window.page(cx);
    window.press("pageup", cx);
    window.assert_only(last - page, cx);

    // Shift+Home and Shift+End extend the range from the anchor to either end.
    window.press("home", cx);
    window.click(3, cx);
    window.press("shift-home", cx);
    assert_eq!(window.selected_rows(cx), [0, 1, 2, 3]);
    assert_eq!(window.table_row(cx), Some(0));
    window.press("shift-end", cx);
    assert_eq!(window.selected_rows(cx), (3..=last).collect::<Vec<_>>());
    assert_eq!(window.table_row(cx), Some(last));
    assert_eq!(window.sent(), []);
}

#[gpui_kit::test]
fn up_with_nothing_selected_selects_the_first_row(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(named(&["a", "b", "c"]), cx);

    window.press("up", cx);
    window.assert_only(0, cx);

    window.press("escape", cx);
    window.press("shift-up", cx);
    window.assert_only(0, cx);

    window.press("ctrl-shift-a", cx);
    window.press("down", cx);
    window.assert_only(0, cx);
}

#[gpui_kit::test]
fn end_with_nothing_selected_selects_the_last_row(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(named(&["a", "b", "c"]), cx);

    window.press("end", cx);
    window.assert_only(2, cx);

    window.press("escape", cx);
    window.press("shift-end", cx);
    window.assert_only(2, cx);

    window.press("ctrl-shift-a", cx);
    window.press("home", cx);
    window.assert_only(0, cx);
}

#[gpui_kit::test]
fn selection_keys_that_change_nothing_do_not_redraw(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(named(&["a", "b", "c"]), cx);
    let table = window.table(cx);
    let redraws = Rc::new(Cell::new(0));
    let counter = Rc::clone(&redraws);
    let _subscription =
        cx.update(|cx| cx.observe(&table, move |_, _| counter.set(counter.get() + 1)));

    window.press("escape", cx);
    window.press("ctrl-shift-a", cx);
    assert_eq!(redraws.get(), 0, "nothing was selected");

    window.click(2, cx);
    redraws.set(0);
    window.press("down", cx);
    window.press("down", cx);
    window.press("shift-down", cx);
    window.press("end", cx);
    assert_eq!(redraws.get(), 0, "the lead is on the last row, on screen");

    window.press("ctrl-a", cx);
    redraws.set(0);
    window.press("ctrl-a", cx);
    assert_eq!(redraws.get(), 0, "everything was selected");

    window.press("escape", cx);
    assert!(redraws.get() > 0, "a change redraws");
}

#[gpui_kit::test]
fn a_key_at_either_end_brings_the_lead_back_into_view(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(many(100), cx);

    window.click(0, cx);
    window.scroll(-1500., cx);
    assert!(!window.on_screen(0, cx));
    window.press("up", cx);
    window.assert_only(0, cx);

    window.press("end", cx);
    window.scroll(1500., cx);
    assert!(!window.on_screen(99, cx));
    window.press("down", cx);
    window.assert_only(99, cx);
}

#[gpui_kit::test]
fn every_selected_row_is_drawn_selected(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let downloading = Torrent {
        state: DisplayState::Downloading,
        ..torrent("a")
    };
    // Every row but the first is an idle seeder, which is dimmed.
    window.send(
        vec![
            downloading,
            torrent("b"),
            torrent("c"),
            torrent("d"),
            torrent("e"),
        ],
        cx,
    );

    window.click(0, cx);
    window.ctrl_click(2, cx);
    window.ctrl_click(4, cx);

    assert_eq!(window.selected(cx), ["a", "c", "e"]);
    assert_eq!(window.drawn_selected(cx), [0, 2, 4]);
    window.act(cx, |window, _| {
        // The table paints the lead itself; marking it as well would lay the color twice.
        assert_eq!(window.find(("row", 4usize)).selected(), Some(true));
        assert!(window.try_find((ROW_SELECTION, 4usize)).is_none());
        // The mark covers the whole row.
        for row in [0usize, 2] {
            let bounds = window.find(("row", row)).bounds();
            let mark = window.find((ROW_SELECTION, row)).bounds();
            assert_eq!(mark.origin, bounds.origin, "row {row}");
            assert_eq!(mark.size.width, bounds.size.width, "row {row}");
            assert!(
                mark.size.height >= bounds.size.height - px(1.),
                "row {row}: {mark:?} in {bounds:?}"
            );
        }
    });
}

#[gpui_kit::test]
fn a_new_snapshot_keeps_the_selection_by_hash(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(named(&["a", "b", "c", "d"]), cx);
    window.click(1, cx);
    window.ctrl_click(3, cx);

    // A new torrent sorts first and moves every row down.
    window.send(named(&["0 new", "a", "b", "c", "d"]), cx);

    assert_eq!(window.selected(cx), ["b", "d"]);
    assert_eq!(window.table_row(cx), Some(4), "the lead's new row");
    assert_eq!(window.drawn_selected(cx), [2, 4]);

    // A torrent that is gone leaves the selection, and does not come back with the torrent.
    window.send(named(&["0 new", "a", "c", "d"]), cx);
    assert_eq!(window.selected(cx), ["d"]);
    assert_eq!(window.selected_count(cx), 1);
    window.send(named(&["0 new", "a", "b", "c", "d"]), cx);
    assert_eq!(window.selected(cx), ["d"]);
    assert_eq!(window.drawn_selected(cx), [4]);
}

#[gpui_kit::test]
fn a_filter_change_drops_hidden_rows_from_the_selection(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![paused("a"), torrent("b"), paused("c")], cx);
    window.press("ctrl-a", cx);
    assert_eq!(window.selected_count(cx), 3);

    window.show(Filter::Paused, "", cx);
    assert_eq!(window.selected(cx), ["a", "c"]);
    assert_eq!(window.selected_count(cx), 2, "the hidden row is dropped");

    // A search drops the rows it hides too.
    window.show(Filter::Paused, "c", cx);
    assert_eq!(window.selected_count(cx), 1);
    assert_eq!(window.table_row(cx), Some(0), "the lead, c, is still shown");

    // Nothing comes back when the rows do.
    window.show(Filter::All, "", cx);
    assert_eq!(window.selected(cx), ["c"]);
    assert_eq!(window.drawn_selected(cx), [2]);
}

#[gpui_kit::test]
fn ctrl_a_in_the_search_box_selects_text_not_torrents(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(named(&["alpha", "beta"]), cx);
    window.click(0, cx);

    window.press("ctrl-f", cx);
    window.act(cx, |window, cx| window.input("a", cx));
    assert_eq!(window.read(cx, |table| table.rows().len()), 2);
    window.press("ctrl-a", cx);

    assert_eq!(
        window.selected(cx),
        ["alpha"],
        "the torrents stay as they were"
    );
    // The text was selected: what is typed next replaces it.
    window.act(cx, |window, cx| window.input("l", cx));
    window.act(cx, |window, _| {
        assert_eq!(window.find(SEARCH).value(), Some("l"));
    });
    assert_eq!(window.selected(cx), ["alpha"]);
}

#[gpui_kit::test]
fn every_binding_parses(cx: &mut TestAppContext) {
    let bindings = cx.update(|cx| key_bindings(cx));

    assert_eq!(
        bindings.len(),
        BINDINGS.len(),
        "an entry that does not parse is left out"
    );
    for (binding, entry) in bindings.iter().zip(BINDINGS) {
        assert_eq!(
            binding.action().name(),
            (entry.action)().name(),
            "{}",
            entry.keys
        );
        for keystroke in binding.keystrokes() {
            let key = keystroke.key();
            assert!(
                key.chars().count() == 1 || NAMED_KEYS.contains(&key),
                "{}: gpui knows no key {key:?}",
                entry.keys
            );
        }
    }
    // A binding without a context would also fire in the search box.
    let global: Vec<&str> = BINDINGS
        .iter()
        .filter(|entry| entry.context.is_none())
        .map(|entry| entry.keys)
        .collect();
    assert_eq!(global, ["ctrl-q"]);
}
