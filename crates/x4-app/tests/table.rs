//! Headless tests for the torrent table of the main window: what its cells say, sorting by a
//! click on a header, the selection that stays on a torrent across snapshots, the scroll position
//! that snapshots leave alone, hiding, resizing and moving columns, and the keys inside the table.
//!
//! Each test builds the two channels of a service itself and opens the window with no service
//! behind them, so nothing touches the network. The tests drive the table through real pointer,
//! wheel and key events and read it through the accessible labels of its cells and the state of
//! its delegate; pixels and opacity cannot be read.

use std::sync::Arc;

use gpui_kit::component::table::{TableDelegate, TableState};
use gpui_kit::test::{TestQueryExt as _, TestWindowExt};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Entity, Focusable as _, Pixels, Role, ScrollDelta,
    TestAppContext, point, px,
};
use x4_app::app;
use x4_app::main_view::MainView;
use x4_app::torrent_table::TorrentTable;
use x4_core::test_support::torrent;
use x4_core::{
    ColumnId, Command, ConnectionState, DisplayState, Event, Filter, Snapshot, SortDirection,
    SortKey, SortState, Torrent, UiState, ViewState,
};

/// The positions of the columns in the default layout.
const NAME: usize = 0;
const SIZE: usize = 1;
const PROGRESS: usize = 2;
const STATE: usize = 3;
const PEERS: usize = 8;

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
            .send_blocking(Event::Snapshot(Arc::new(snapshot(torrents))))
            .expect("the window listens");
        cx.run_until_parked();
    }

    fn table(&self, cx: &mut TestAppContext) -> Entity<TableState<TorrentTable>> {
        cx.update(|cx| self.view.read(cx).table().clone())
    }

    /// Reads the delegate of the table.
    fn read<R>(&self, cx: &mut TestAppContext, read: impl FnOnce(&TorrentTable, &App) -> R) -> R {
        let table = self.table(cx);
        cx.update(|cx| read(table.read(cx).delegate(), cx))
    }

    /// The names of the rows, in the order shown.
    fn names(&self, cx: &mut TestAppContext) -> Vec<String> {
        self.read(cx, |table, _| {
            table.rows().iter().map(|row| row.name.clone()).collect()
        })
    }

    fn selected_name(&self, cx: &mut TestAppContext) -> Option<String> {
        self.read(cx, |table, _| {
            let selected = table.selected()?;
            table
                .rows()
                .iter()
                .find(|row| &row.hash == selected)
                .map(|row| row.name.clone())
        })
    }

    /// Runs `act` on the window, after drawing a frame.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a closed window fails the test"
    )]
    fn act<R>(
        &self,
        cx: &mut TestAppContext,
        act: impl FnOnce(&mut gpui_kit::Window, &mut App) -> R,
    ) -> R {
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            act(window, cx)
        })
        .expect("the window is open")
    }

    /// Opens the column menu and clicks the entry of the column at `index` in
    /// [`ColumnId::ALL`].
    fn click_in_column_menu(&self, index: usize, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| window.click("column-menu", cx));
        cx.run_until_parked();
        self.click_menu_entry(index, cx);
    }

    /// Clicks the entry of the column at `index` in [`ColumnId::ALL`] in the open column menu.
    fn click_menu_entry(&self, index: usize, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| {
            window.within("popup-menu").click(index, cx);
        });
        cx.run_until_parked();
    }

    /// Whether the column menu is open.
    fn menu_is_open(&self, cx: &mut TestAppContext) -> bool {
        self.act(cx, |window, _| {
            !window.find_all_by_role(Role::Menu).is_empty()
        })
    }

    /// The commands the window has sent so far.
    fn sent(&self) -> Vec<Command> {
        self.commands.try_iter().collect()
    }
}

fn snapshot(torrents: Vec<Torrent>) -> Snapshot {
    Snapshot {
        torrents,
        session: None,
        free_space_bytes: None,
        rate_down: 0,
        rate_up: 0,
        connection: ConnectionState::Connected,
        stale: false,
    }
}

fn named(names: &[&str]) -> Vec<Torrent> {
    names.iter().map(|name| torrent(name)).collect()
}

fn sized(name: &str, size_when_done: u64) -> Torrent {
    Torrent {
        size_when_done,
        ..torrent(name)
    }
}

/// `count` torrents, "t000" to "t099" and on, which sort in that order by name.
fn many(count: usize) -> Vec<Torrent> {
    (0..count).map(|ix| torrent(&format!("t{ix:03}"))).collect()
}

/// The vertical scroll offset of the table's rows.
fn scroll_offset(table: &Entity<TableState<TorrentTable>>, cx: &App) -> Pixels {
    table
        .read(cx)
        .vertical_scroll_handle
        .0
        .borrow()
        .base_handle
        .offset()
        .y
}

/// The accessible names of the column headers, in the order shown.
fn header_labels(window: &gpui_kit::Window) -> Vec<String> {
    let mut headers = window.find_all_by_role(Role::ColumnHeader);
    headers.sort_by(|a, b| f32::from(a.bounds().left()).total_cmp(&f32::from(b.bounds().left())));
    headers
        .iter()
        .filter_map(|header| header.label().map(str::to_owned))
        .collect()
}

#[gpui_kit::test]
fn rows_render_with_labels(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(
        vec![
            Torrent {
                state: DisplayState::Downloading,
                progress: 0.63,
                size_when_done: 24_592_841_836,
                rate_down: 2_516_582,
                active: true,
                ..torrent("Alpha")
            },
            Torrent {
                state: DisplayState::Paused,
                ..torrent("Beta")
            },
            torrent("Gamma"),
        ],
        cx,
    );

    assert_eq!(window.names(cx), ["Alpha", "Beta", "Gamma"]);
    let labels = window.read(cx, |table, cx| {
        [(0, NAME), (0, PROGRESS), (1, STATE)]
            .map(|(row, column)| table.cell_label(row, column, cx))
    });
    assert_eq!(
        labels,
        [
            Some("Alpha".into()),
            Some("63% of 22.9 GiB".into()),
            Some("Paused".into())
        ]
    );
    window.act(cx, |window, _| {
        assert_eq!(window.find("table-cell-0:0").label(), Some("Alpha"));
        assert_eq!(
            window.find("table-cell-0:2").label(),
            Some("63% of 22.9 GiB")
        );
        assert_eq!(window.find("table-cell-0:1").label(), Some("22.9 GiB"));
        assert_eq!(window.find("table-cell-0:4").label(), Some("2.4 MiB/s"));
        assert_eq!(window.find("table-cell-2:2").label(), Some("100% of 0 B"));
        assert_eq!(window.find("table-cell-2:3").label(), Some("Seeding"));
        assert_eq!(
            header_labels(window),
            [
                "Name",
                "Size",
                "Progress",
                "State",
                "Download speed",
                "Upload speed",
                "ETA",
                "Ratio",
                "Peers"
            ]
        );
        assert_eq!(window.find("torrent-list").label(), Some("3 torrents"));
    });
}

#[gpui_kit::test]
fn the_empty_table_says_why(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.act(cx, |window, _| {
        assert_eq!(window.find("table-empty").label(), Some("No torrents"));
    });

    window.send(named(&["Alpha"]), cx);
    cx.update(|cx| {
        window.view.update(cx, |view, cx| {
            view.set_view(
                ViewState {
                    filter: Filter::Paused,
                    ..ViewState::default()
                },
                cx,
            );
        });
    });

    window.act(cx, |window, _| {
        assert_eq!(
            window.find("table-empty").label(),
            Some("No torrents match")
        );
    });
}

#[gpui_kit::test]
fn header_click_sorts(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(
        vec![
            sized("b", 10),
            sized("small", 1),
            sized("a", 10),
            sized("big", 500),
        ],
        cx,
    );

    window.act(cx, |window, cx| window.click(("col-header", SIZE), cx));
    cx.run_until_parked();
    assert_eq!(window.names(cx), ["big", "a", "b", "small"]);
    let table = window.table(cx);
    assert_eq!(
        cx.update(|cx| table.read(cx).selected_col()),
        None,
        "a click on a header selects no column"
    );
    window.act(cx, |window, _| {
        let mark = window.within(("col-header", SIZE)).find("sort-mark");
        assert_eq!(mark.label(), Some("▼"));
        assert!(
            window
                .within(("col-header", NAME))
                .try_find("sort-mark")
                .is_none(),
            "only the sorted column has a mark"
        );
    });

    window.act(cx, |window, cx| window.click(("col-header", SIZE), cx));
    cx.run_until_parked();
    assert_eq!(window.names(cx), ["small", "a", "b", "big"]);
    window.act(cx, |window, _| {
        let mark = window.within(("col-header", SIZE)).find("sort-mark");
        assert_eq!(mark.label(), Some("▲"));
    });

    let (sort, delegate_sort) = (
        cx.update(|cx| window.view.read(cx).ui_state().sort),
        window.read(cx, |table, _| table.sort()),
    );
    assert_eq!(
        sort,
        SortState {
            key: SortKey::Size,
            direction: SortDirection::Ascending
        },
        "the sort order is remembered"
    );
    assert_eq!(delegate_sort, (SortKey::Size, SortDirection::Ascending));
    assert!(
        window
            .sent()
            .iter()
            .all(|command| matches!(command, Command::SetVisibility(_))),
        "sorting sends nothing to the daemon"
    );
}

#[gpui_kit::test]
fn selection_follows_the_hash_after_resort(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![sized("a", 1), sized("b", 2), sized("c", 3)], cx);
    window.act(cx, |window, cx| window.click(("row", 0usize), cx));
    assert_eq!(window.selected_name(cx).as_deref(), Some("a"));

    // A new torrent sorts before the selected one.
    window.send(
        vec![
            sized("0 new", 4),
            sized("a", 1),
            sized("b", 2),
            sized("c", 3),
        ],
        cx,
    );
    let table = window.table(cx);
    assert_eq!(window.read(cx, |table, _| table.selected_index()), Some(1));
    assert_eq!(cx.update(|cx| table.read(cx).selected_row()), Some(1));
    assert_eq!(window.selected_name(cx).as_deref(), Some("a"));
    window.act(cx, |window, _| {
        assert_eq!(window.find(("row", 1usize)).selected(), Some(true));
        assert_eq!(window.find(("row", 0usize)).selected(), Some(false));
    });

    // Sorting by size, largest first, puts it last.
    window.act(cx, |window, cx| window.click(("col-header", SIZE), cx));
    cx.run_until_parked();
    assert_eq!(window.names(cx), ["0 new", "c", "b", "a"]);
    assert_eq!(window.read(cx, |table, _| table.selected_index()), Some(3));
    assert_eq!(cx.update(|cx| table.read(cx).selected_row()), Some(3));

    // The keyboard moves the selection on from there.
    window.act(cx, |window, cx| window.press("up", cx));
    assert_eq!(window.selected_name(cx).as_deref(), Some("b"));
}

#[gpui_kit::test]
fn selected_torrent_removed_clears(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(named(&["a", "b", "c"]), cx);
    window.act(cx, |window, cx| window.click(("row", 1usize), cx));
    assert_eq!(window.selected_name(cx).as_deref(), Some("b"));

    window.send(named(&["a", "c"]), cx);

    let table = window.table(cx);
    assert_eq!(window.read(cx, |table, _| table.selected().cloned()), None);
    assert_eq!(window.read(cx, |table, _| table.selected_index()), None);
    assert_eq!(cx.update(|cx| table.read(cx).selected_row()), None);
    window.act(cx, |window, _| {
        assert_eq!(window.find(("row", 1usize)).selected(), Some(false));
    });
    let sent = window.sent();
    assert!(
        sent.iter()
            .all(|command| matches!(command, Command::SetVisibility(_))),
        "selecting sends nothing: {sent:?}"
    );

    // It does not come back when the torrent does.
    window.send(named(&["a", "b", "c"]), cx);
    assert_eq!(window.read(cx, |table, _| table.selected().cloned()), None);
}

#[gpui_kit::test]
fn new_snapshot_keeps_scroll_offset(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(many(100), cx);
    let table = window.table(cx);
    window.act(cx, |window, cx| window.click(("row", 0usize), cx));
    window.act(cx, |window, cx| {
        window.scroll("table", ScrollDelta::Pixels(point(px(0.), px(-1500.))), cx);
    });
    let scrolled = cx.update(|cx| scroll_offset(&table, cx));
    assert!(scrolled < px(-1000.), "{scrolled:?}");

    // The selected torrent moves to the second row, far above the view; the view stays.
    let mut torrents = many(100);
    torrents.push(torrent("0 first"));
    window.send(torrents, cx);
    window.act(cx, TestWindowExt::render_frame);

    assert_eq!(cx.update(|cx| scroll_offset(&table, cx)), scrolled);
    assert_eq!(window.read(cx, |table, _| table.selected_index()), Some(1));
    assert_eq!(cx.update(|cx| table.read(cx).selected_row()), Some(1));

    // A snapshot that only drops rows below the view does not move it either.
    window.send(many(90), cx);
    window.act(cx, TestWindowExt::render_frame);
    assert_eq!(cx.update(|cx| scroll_offset(&table, cx)), scrolled);
}

#[gpui_kit::test]
fn hidden_column_is_not_rendered(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(named(&["a"]), cx);
    assert_eq!(window.read(cx, TableDelegate::columns_count), 9);

    window.click_in_column_menu(PEERS, cx);

    assert_eq!(window.read(cx, TableDelegate::columns_count), 8);
    window.act(cx, |window, _| {
        let headers = header_labels(window);
        assert_eq!(headers.len(), 8);
        assert!(!headers.contains(&"Peers".to_owned()), "{headers:?}");
        assert!(window.try_find("table-cell-0:8").is_none());
    });
    let remembered = cx.update(|cx| window.view.read(cx).ui_state().columns.clone());
    assert!(
        remembered
            .iter()
            .any(|column| column.id == ColumnId::Peers && !column.visible),
        "{remembered:?}"
    );

    // The name column's entry is there but does nothing, and the menu stays open.
    window.click_in_column_menu(NAME, cx);
    assert_eq!(window.read(cx, TableDelegate::columns_count), 8);
    assert!(window.menu_is_open(cx));
    window.act(cx, |window, _| {
        assert_eq!(
            header_labels(window).first().map(String::as_str),
            Some("Name")
        );
    });

    // Showing the column again puts it back where it was, and closes the menu.
    window.click_menu_entry(PEERS, cx);
    assert!(!window.menu_is_open(cx));
    assert_eq!(window.read(cx, TableDelegate::columns_count), 9);
    window.act(cx, |window, _| {
        assert_eq!(
            header_labels(window).last().map(String::as_str),
            Some("Peers")
        );
    });
}

#[gpui_kit::test]
fn widths_survive_refresh(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(named(&["a"]), cx);
    let table = window.table(cx);
    let before = cx.update(|cx| table.read(cx).col_width(SIZE));

    // Drag the resize handle on the right edge of the size column 60 px to the right.
    window.act(cx, |window, cx| {
        let header = window.find(("col-header", SIZE)).bounds();
        let from = point(header.right() - px(2.), header.center().y);
        window.drag(from, point(from.x + px(60.), from.y), cx);
    });
    cx.run_until_parked();
    let resized = cx.update(|cx| table.read(cx).col_width(SIZE));
    assert!(
        resized
            .zip(before)
            .is_some_and(|(resized, before)| resized > before + px(40.)),
        "{before:?} -> {resized:?}"
    );
    let stored = |cx: &mut TestAppContext| {
        window.read(cx, |table, _| {
            table
                .columns()
                .iter()
                .find(|column| column.id == ColumnId::Size)
                .map(|column| px(column.width))
        })
    };
    assert_eq!(stored(cx), resized, "the delegate keeps the new width");

    window.click_in_column_menu(PEERS, cx);

    assert_eq!(cx.update(|cx| table.read(cx).col_width(SIZE)), resized);
    assert_eq!(stored(cx), resized);
    let remembered = cx.update(|cx| {
        window
            .view
            .read(cx)
            .ui_state()
            .columns
            .iter()
            .find(|column| column.id == ColumnId::Size)
            .map(|column| px(column.width))
    });
    assert_eq!(remembered, resized, "the width is remembered between runs");
}

#[gpui_kit::test]
fn moving_a_column_is_remembered(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(named(&["a"]), cx);

    // Drag the size header onto the right half of the state header.
    window.act(cx, |window, cx| {
        let from = window.find(("col-header", SIZE)).bounds().center();
        let target = window.find(("col-header", STATE)).bounds();
        window.drag(from, point(target.right() - px(10.), target.center().y), cx);
    });
    cx.run_until_parked();

    let order = |columns: &[x4_core::ColumnState]| -> Vec<ColumnId> {
        columns.iter().map(|column| column.id).take(4).collect()
    };
    let expected = [
        ColumnId::Name,
        ColumnId::Progress,
        ColumnId::State,
        ColumnId::Size,
    ];
    assert_eq!(window.read(cx, |table, _| order(table.columns())), expected);
    assert_eq!(
        cx.update(|cx| order(&window.view.read(cx).ui_state().columns)),
        expected
    );
    window.act(cx, |window, _| {
        assert_eq!(
            header_labels(window).get(..4),
            Some(&["Name", "Progress", "State", "Size"].map(str::to_owned)[..])
        );
        assert_eq!(window.find("table-cell-0:3").label(), Some("0 B"));
    });
}

#[gpui_kit::test]
fn tab_leaves_the_table(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(named(&["a", "b"]), cx);
    let table = window.table(cx);

    window.act(cx, |window, cx| {
        assert!(
            table.focus_handle(cx).is_focused(window),
            "the table has the focus from the start"
        );
        window.press("right", cx);
        window.press("left", cx);
    });
    assert_eq!(
        cx.update(|cx| table.read(cx).selected_col()),
        None,
        "left and right select no column"
    );

    window.act(cx, |window, cx| {
        window.press("tab", cx);
        assert!(!table.focus_handle(cx).is_focused(window), "tab leaves");
    });
    assert_eq!(cx.update(|cx| table.read(cx).selected_col()), None);

    window.act(cx, |window, cx| {
        table.focus_handle(cx).focus(window, cx);
        window.render_frame(cx);
        window.press("shift-tab", cx);
        assert!(
            !table.focus_handle(cx).is_focused(window),
            "shift-tab leaves too"
        );
    });
}

#[gpui_kit::test]
fn down_selects_the_first_row_and_stops_at_the_last(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(named(&["a", "b"]), cx);

    window.act(cx, |window, cx| window.press("down", cx));
    assert_eq!(window.selected_name(cx).as_deref(), Some("a"));
    window.act(cx, |window, cx| {
        window.press("down", cx);
        window.press("down", cx);
    });
    assert_eq!(
        window.selected_name(cx).as_deref(),
        Some("b"),
        "the selection does not wrap around"
    );
}
