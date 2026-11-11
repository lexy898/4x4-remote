//! Headless tests for the filter sidebar and the search box of the main window: the counts the
//! sidebar shows, a click on a filter, the keys that focus and clear the search, and searching by
//! name.
//!
//! Each test builds the two channels of a service itself and opens the window with no service
//! behind them, so nothing touches the network. The tests drive the window through real clicks,
//! key presses and typed text, and read it through the accessible labels of its elements, the view
//! model and the state of the table.

use std::collections::BTreeSet;
use std::sync::Arc;

use gpui_kit::component::table::TableState;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Entity, Focusable as _, OperatingSystem, PlatformInfo,
    TestAppContext,
};
use x4_app::app;
use x4_app::main_view::MainView;
use x4_app::torrent_table::TorrentTable;
use x4_core::test_support::torrent;
use x4_core::{
    Attention, Command, ConnectionState, DisplayState, Event, Filter, QueueKind, Snapshot, Torrent,
    UiState, ViewState, counts,
};

/// The id of the search box.
const SEARCH: &str = "search";

/// An open main window and the far ends of its channels, standing in for the service.
struct Window {
    handle: AnyWindowHandle,
    view: Entity<MainView>,
    events: async_channel::Sender<Event>,
    commands: crossbeam_channel::Receiver<Command>,
}

impl Window {
    fn open(cx: &mut TestAppContext) -> Self {
        Self::open_as(None, cx)
    }

    /// Opens the window on the OS `os`, or on the host's when it is `None`: the keys are bound
    /// for the OS the app is told it runs on.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a window that does not open fails the test"
    )]
    fn open_as(os: Option<OperatingSystem>, cx: &mut TestAppContext) -> Self {
        if let Some(os) = os {
            let info = cx.update(|cx| cx.platform_info());
            cx.set_platform_info(PlatformInfo { os, ..info });
        }
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

    fn table(&self, cx: &mut TestAppContext) -> Entity<TableState<TorrentTable>> {
        cx.update(|cx| self.view.read(cx).table().clone())
    }

    /// The names of the rows the table shows, in order.
    fn names(&self, cx: &mut TestAppContext) -> Vec<String> {
        let table = self.table(cx);
        cx.update(|cx| {
            table
                .read(cx)
                .delegate()
                .rows()
                .iter()
                .map(|row| row.name.clone())
                .collect()
        })
    }

    fn view_state(&self, cx: &mut TestAppContext) -> ViewState {
        cx.update(|cx| self.view.read(cx).view_state().clone())
    }

    /// Whether the search box has the keyboard focus.
    fn search_focused(&self, cx: &mut TestAppContext) -> bool {
        self.act(cx, |window, _| window.find(SEARCH).focused() == Some(true))
    }

    /// Whether the torrent table has the keyboard focus.
    fn table_focused(&self, cx: &mut TestAppContext) -> bool {
        let table = self.table(cx);
        self.act(cx, |window, cx| table.focus_handle(cx).is_focused(window))
    }

    /// The text in the search box.
    fn search_text(&self, cx: &mut TestAppContext) -> String {
        self.act(cx, |window, _| {
            window.find(SEARCH).value().unwrap_or_default().to_owned()
        })
    }

    /// The count the sidebar shows next to `filter`.
    fn shown_count(&self, filter: Filter, cx: &mut TestAppContext) -> Option<String> {
        let id = count_id(filter);
        self.act(cx, |window, _| window.find(id).label().map(str::to_owned))
    }

    /// Clicks `filter` in the sidebar.
    fn click_filter(&self, filter: Filter, cx: &mut TestAppContext) {
        let id = count_id(filter);
        self.act(cx, |window, cx| window.click(id, cx));
    }

    fn press(&self, key: &str, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| window.press(key, cx));
    }

    fn type_text(&self, text: &str, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| window.input(text, cx));
    }

    /// The commands the window has sent so far.
    fn sent(&self) -> Vec<Command> {
        self.commands.try_iter().collect()
    }
}

/// The id of the count next to `filter` in the sidebar.
#[expect(
    clippy::expect_used,
    reason = "a test helper: every filter is in the sidebar"
)]
fn count_id(filter: Filter) -> (&'static str, usize) {
    let index = Filter::ALL
        .iter()
        .position(|listed| *listed == filter)
        .expect("the sidebar lists every filter");
    ("filter-count", index)
}

fn in_state(name: &str, state: DisplayState) -> Torrent {
    Torrent {
        state,
        ..torrent(name)
    }
}

/// Torrents in every state, so that each filter counts a different number of them.
fn zoo() -> Vec<Torrent> {
    vec![
        Torrent {
            active: true,
            ..in_state("downloading", DisplayState::Downloading)
        },
        in_state("queued one", DisplayState::Queued(QueueKind::Download)),
        in_state("queued two", DisplayState::Queued(QueueKind::Download)),
        in_state("seeding one", DisplayState::Seeding),
        in_state("seeding two", DisplayState::Seeding),
        Torrent {
            active: true,
            ..in_state("seeding three", DisplayState::Seeding)
        },
        in_state("seeding four", DisplayState::Seeding),
        in_state("seeding five", DisplayState::Seeding),
        in_state("paused one", DisplayState::Paused),
        in_state("paused two", DisplayState::Paused),
        in_state("paused three", DisplayState::Paused),
        in_state("paused four", DisplayState::Paused),
        in_state("checking", DisplayState::Checking),
        Torrent {
            attention: Some(Attention::LocalError("disk full".to_owned())),
            ..in_state("broken", DisplayState::Error)
        },
    ]
}

/// Whether every command sent is a visibility report, the only one this stage may send.
fn only_visibility(commands: &[Command]) -> bool {
    commands
        .iter()
        .all(|command| matches!(command, Command::SetVisibility(_)))
}

#[gpui_kit::test]
fn sidebar_counts_match_snapshot(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let torrents = zoo();
    let expected = counts(&torrents);
    window.send(torrents, cx);

    for filter in Filter::ALL {
        assert_eq!(
            window.shown_count(filter, cx),
            Some(expected.get(filter).to_string()),
            "{filter:?}"
        );
    }
    // Every filter counts a different number, so a count shown next to the wrong filter fails
    // the comparison above.
    let numbers = Filter::ALL.map(|filter| expected.get(filter));
    assert_eq!(numbers, [14, 3, 5, 4, 2, 1]);
    assert_eq!(
        numbers.into_iter().collect::<BTreeSet<_>>().len(),
        Filter::ALL.len(),
        "the counts are distinct"
    );

    // Neither the filter nor the search changes the counts.
    window.click_filter(Filter::Paused, cx);
    window.press("secondary-f", cx);
    window.type_text("one", cx);
    assert_eq!(window.names(cx), ["paused one"]);
    for filter in Filter::ALL {
        assert_eq!(
            window.shown_count(filter, cx),
            Some(expected.get(filter).to_string()),
            "{filter:?}, filtered"
        );
    }

    // A new snapshot brings new counts.
    window.send(vec![in_state("paused", DisplayState::Paused)], cx);
    assert_eq!(window.shown_count(Filter::All, cx).as_deref(), Some("1"));
    assert_eq!(window.shown_count(Filter::Paused, cx).as_deref(), Some("1"));
    assert_eq!(window.shown_count(Filter::Error, cx).as_deref(), Some("0"));
}

#[gpui_kit::test]
fn the_sidebar_shows_no_counts_before_the_list_has_come(cx: &mut TestAppContext) {
    let window = Window::open(cx);

    for filter in Filter::ALL {
        let id = count_id(filter);
        assert!(
            window.act(cx, |window, _| window.try_find(id).is_none()),
            "{filter:?}"
        );
    }

    window.send(Vec::new(), cx);
    for filter in Filter::ALL {
        assert_eq!(
            window.shown_count(filter, cx).as_deref(),
            Some("0"),
            "{filter:?}"
        );
    }
}

#[gpui_kit::test]
fn clicking_a_filter_filters_rows(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(zoo(), cx);
    assert_eq!(window.names(cx).len(), 14);

    window.click_filter(Filter::Seeding, cx);

    assert_eq!(
        window.names(cx),
        [
            "seeding five",
            "seeding four",
            "seeding one",
            "seeding three",
            "seeding two"
        ]
    );
    assert_eq!(window.view_state(cx).filter, Filter::Seeding);
    let ui = cx.update(|cx| window.view.read(cx).ui_state().clone());
    assert_eq!(ui.filter, Filter::Seeding, "the filter is kept to be saved");

    window.click_filter(Filter::Error, cx);
    assert_eq!(window.names(cx), ["broken"]);

    window.click_filter(Filter::All, cx);
    assert_eq!(window.names(cx).len(), 14);
    assert_eq!(
        cx.update(|cx| window.view.read(cx).ui_state().filter),
        Filter::All
    );

    // A click leaves the keyboard where it was, so the window's keys still work.
    assert!(window.table_focused(cx));
    assert!(
        only_visibility(&window.sent()),
        "filtering asks the daemon for nothing"
    );
}

/// On the Mac the key to search is ⌘F, and Ctrl+F is no key of the app there.
#[gpui_kit::test]
fn command_f_focuses_the_search_box_on_the_mac(cx: &mut TestAppContext) {
    let window = Window::open_as(Some(OperatingSystem::Mac), cx);
    window.send(zoo(), cx);
    assert!(window.table_focused(cx), "the table has the focus at first");

    window.press("ctrl-f", cx);
    assert!(!window.search_focused(cx), "Ctrl+F does nothing on the Mac");
    assert!(window.table_focused(cx));

    window.press("cmd-f", cx);
    assert!(window.search_focused(cx));
}

#[gpui_kit::test]
fn ctrl_f_and_slash_focus_search(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(zoo(), cx);
    assert!(window.table_focused(cx), "the table has the focus at first");

    window.press("secondary-f", cx);
    assert!(window.search_focused(cx));

    window.press("escape", cx);
    assert!(!window.search_focused(cx));
    assert!(window.table_focused(cx), "escape hands the focus back");

    window.press("/", cx);
    assert!(window.search_focused(cx));
    assert_eq!(window.search_text(cx), "", "the slash only moved the focus");

    // Ctrl+F in the box selects what is there, so that typing replaces it.
    window.type_text("old", cx);
    window.press("secondary-f", cx);
    assert!(window.search_focused(cx));
    window.type_text("new", cx);
    assert_eq!(window.search_text(cx), "new");
}

#[gpui_kit::test]
fn slash_in_search_box_types_a_slash(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(zoo(), cx);

    window.press("secondary-f", cx);
    window.type_text("a/b", cx);

    assert_eq!(window.search_text(cx), "a/b");
    assert_eq!(window.view_state(cx).search, "a/b");
    assert!(window.search_focused(cx));
}

#[gpui_kit::test]
fn search_filters_case_insensitively(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(
        vec![
            torrent("Тед Лассо S01"),
            torrent("Ted Lasso S02"),
            torrent("Другое"),
        ],
        cx,
    );

    window.press("secondary-f", cx);
    window.type_text("лассо", cx);

    assert_eq!(window.names(cx), ["Тед Лассо S01"]);
    assert_eq!(window.view_state(cx).search, "лассо");

    window.press("escape", cx);

    assert_eq!(window.search_text(cx), "");
    assert_eq!(window.view_state(cx).search, "");
    assert_eq!(window.names(cx).len(), 3);
    assert!(window.table_focused(cx));
    assert!(
        only_visibility(&window.sent()),
        "searching asks the daemon for nothing"
    );
}

#[gpui_kit::test]
fn an_empty_search_box_that_loses_the_focus_changes_nothing(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(zoo(), cx);
    window.click_filter(Filter::Paused, cx);

    window.press("/", cx);
    window.press("tab", cx);

    assert!(!window.search_focused(cx));
    assert_eq!(
        window.view_state(cx),
        ViewState {
            filter: Filter::Paused,
            ..ViewState::default()
        }
    );
    assert_eq!(
        window.names(cx),
        ["paused four", "paused one", "paused three", "paused two"]
    );
}

#[gpui_kit::test]
fn the_search_box_has_a_placeholder(cx: &mut TestAppContext) {
    let window = Window::open(cx);

    let placeholder = window.act(cx, |window, _| {
        window.find(SEARCH).placeholder().map(str::to_owned)
    });

    assert_eq!(
        placeholder.as_deref(),
        Some(if cfg!(target_os = "macos") {
            "Search (⌘F)"
        } else {
            "Search (Ctrl+F)"
        })
    );
}
