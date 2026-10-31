//! Headless tests for the details panel under the torrent table: what it shows for no torrent,
//! one or several, when it asks the service for a torrent's details, how Alt+Enter, Alt+1 and
//! Alt+2 work, how its height follows the window and the user's drags, what the General tab says
//! and what its copy buttons copy.
//!
//! Each test builds the two channels of a service itself and opens the window with no service
//! behind them, so nothing touches the network: the test reads the commands the window sends and
//! sends the snapshots and the details a service would send. The clipboard is the test platform's
//! own, in memory.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui_kit::component::table::TableState;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, ClipboardItem, Entity, Focusable as _, Pixels,
    TestAppContext, point, px, size,
};
use x4_app::app;
use x4_app::main_view::MainView;
use x4_app::torrent_table::TorrentTable;
use x4_core::test_support::{details, torrent};
use x4_core::{
    Attention, Command, ConnectionState, DetailsState, DetailsTab, Event, MAX_DETAILS_HEIGHT,
    MIN_DETAILS_HEIGHT, MIN_WINDOW_HEIGHT, Ratio, Snapshot, Torrent, TorrentDetails, TrackerAnswer,
    TrackerOutcome, TrackerStat, UiState, ViewState,
};
use x4_rpc::{FilePriority, HashString};

/// The details panel.
const DETAILS: &str = "details";
/// The tab bar of the panel; its tabs are numbered from 0.
const TABS: &str = "details-tabs";
const GENERAL: usize = 0;
const FILES: usize = 1;
/// What the panel says when it shows no torrent.
const PLACEHOLDER: &str = "details-placeholder";
/// The split of the table and the panel, and the handle between them.
const SPLIT: &str = "details-split";
const HANDLE: (&str, usize) = ("resizable-handle", 0usize);
/// The torrent list above the panel.
const LIST: &str = "torrent-list";
/// The connection dot, in the status bar under the panel.
const STATUS_BAR_DOT: &str = "connection";
/// The copy buttons of the General tab.
const COPY_PATH: &str = "copy-path";
const COPY_HASH: &str = "copy-hash";
/// The line of the General tab that words the trackers' worst problem.
const TRACKER_PROBLEM: &str = "tracker-problem";
/// What a row that needs the details says until they come.
const PENDING: &str = "—";
/// The rows of the General tab that only the details fill.
const FROM_DETAILS: &[&str] = &[
    "general-downloaded",
    "general-uploaded",
    "general-created",
    "general-comment",
    "general-private",
    "general-pieces",
    "general-trackers",
];
/// The lowest the table may be: its header and two rows.
const TABLE_MIN_HEIGHT: f32 = 120.;
/// The id of an open menu.
const MENU: &str = "popup-menu";

const GIB: u64 = 1024 * 1024 * 1024;
const MIB: u64 = 1024 * 1024;

/// An open main window and the far ends of its channels, standing in for the service.
struct Window {
    handle: AnyWindowHandle,
    view: Entity<MainView>,
    events: async_channel::Sender<Event>,
    commands: crossbeam_channel::Receiver<Command>,
}

impl Window {
    fn open(cx: &mut TestAppContext) -> Self {
        Self::open_with(&UiState::default(), cx)
    }

    /// Opens the window from the remembered state `ui`, active and with animations off.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a window that does not open fails the test"
    )]
    fn open_with(ui: &UiState, cx: &mut TestAppContext) -> Self {
        let (events, events_rx) = async_channel::unbounded();
        let (commands_tx, commands) = crossbeam_channel::unbounded();
        cx.update(|cx| cx.set_reduce_motion(true));
        let (handle, view) = cx
            .update(|cx| {
                app::init(cx);
                app::open_main_window(events_rx, commands_tx, None, ui, cx)
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
        self.send_event(
            Event::Snapshot(Arc::new(Snapshot {
                torrents,
                listed: true,
                session: None,
                free_space_bytes: None,
                rate_down: 0,
                rate_up: 0,
                connection: ConnectionState::Connected,
                stale: false,
            })),
            cx,
        );
    }

    /// Sends the details of a torrent, as the service does after a read.
    fn send_details(&self, details: TorrentDetails, cx: &mut TestAppContext) {
        self.send_event(Event::Details(Arc::new(details)), cx);
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
            window.click_with_modifiers(("row", row), gpui_kit::Modifiers::control(), cx);
        });
    }

    fn press(&self, key: &str, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| window.press(key, cx));
    }

    /// Resizes the window to `width` by `height` and lets it lay out again.
    fn resize(&self, width: f32, height: f32, cx: &mut TestAppContext) {
        cx.simulate_window_resize(self.handle, size(px(width), px(height)));
        cx.run_until_parked();
        // A frame at the new size, and the one the fit of the panel asks for.
        self.act(cx, |_, _| ());
        self.act(cx, |_, _| ());
    }

    /// Drags the handle between the table and the panel by `dy` pixels (negative is up).
    fn drag_handle(&self, dy: f32, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| {
            let from = window.within(SPLIT).find(HANDLE).bounds().center();
            window.drag(from, point(from.x, from.y + px(dy)), cx);
        });
    }

    /// Whether the element `id` is on screen.
    fn shown(&self, id: &'static str, cx: &mut TestAppContext) -> bool {
        self.act(cx, |window, _| window.try_find(id).is_some())
    }

    /// Where the element `id` was drawn last.
    fn bounds(&self, id: &'static str, cx: &mut TestAppContext) -> Bounds<Pixels> {
        self.act(cx, |window, _| window.find(id).bounds())
    }

    /// The accessible label of the element `id`.
    fn label(&self, id: &'static str, cx: &mut TestAppContext) -> Option<String> {
        self.act(cx, |window, _| window.find(id).label().map(str::to_owned))
    }

    /// What the row `id` of the General tab says: its accessible value.
    fn value(&self, id: &'static str, cx: &mut TestAppContext) -> Option<String> {
        self.act(cx, |window, _| window.find(id).value().map(str::to_owned))
    }

    /// Whether the tab `tab` of the panel is the selected one.
    fn tab_selected(&self, tab: usize, cx: &mut TestAppContext) -> bool {
        self.act(cx, |window, _| {
            window.within(TABS).find(tab).selected() == Some(true)
        })
    }

    /// Whether a menu is open.
    fn menu_open(&self, cx: &mut TestAppContext) -> bool {
        self.act(cx, |window, _| window.try_find(MENU).is_some())
    }

    fn ui_state(&self, cx: &mut TestAppContext) -> UiState {
        cx.update(|cx| self.view.read(cx).ui_state().clone())
    }

    fn table(&self, cx: &mut TestAppContext) -> Entity<TableState<TorrentTable>> {
        cx.update(|cx| self.view.read(cx).table().clone())
    }

    /// Whether the table has the keyboard focus, so that the list's keys reach it.
    fn table_focused(&self, cx: &mut TestAppContext) -> bool {
        let table = self.table(cx);
        self.act(cx, |window, cx| table.focus_handle(cx).is_focused(window))
    }

    /// The torrents the window has asked the service to read the details of since the last look,
    /// in order; `None` asks for none.
    fn selects(&self) -> Vec<Option<HashString>> {
        self.commands
            .try_iter()
            .filter_map(|command| match command {
                Command::Select(hash) => Some(hash),
                _ => None,
            })
            .collect()
    }
}

/// The text on the clipboard.
fn clipboard(cx: &mut TestAppContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|item| item.text())
}

fn at(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(seconds)
}

/// Whether `text` is a date and time as the panel writes them, such as "2026-10-29 14:05".
fn is_date_time(text: &str) -> bool {
    let shape = "0000-00-00 00:00";
    text.len() == shape.len()
        && text.chars().zip(shape.chars()).all(|(char, expected)| {
            if expected == '0' {
                char.is_ascii_digit()
            } else {
                char == expected
            }
        })
}

/// A tracker of the details that answered its last announce with `outcome`, or none.
fn tracker(host: &str, outcome: Option<TrackerOutcome>, result: &str) -> TrackerStat {
    TrackerStat {
        host: host.to_owned(),
        last_announce: outcome.map(|outcome| TrackerAnswer {
            outcome,
            result: result.to_owned(),
        }),
        last_scrape: None,
        seeders: None,
        leechers: None,
    }
}

#[gpui_kit::test]
fn the_panel_shows_a_placeholder_for_none_and_many(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![torrent("a"), torrent("b"), torrent("c")], cx);

    assert!(
        window.shown(DETAILS, cx),
        "the panel is open from the first run"
    );
    assert_eq!(
        window.label(PLACEHOLDER, cx).as_deref(),
        Some("Select a torrent")
    );
    assert!(!window.shown("general-hash", cx));

    window.click_row(0, cx);
    assert!(!window.shown(PLACEHOLDER, cx));
    assert_eq!(
        window.value("general-hash", cx),
        Some(torrent("a").hash.as_str().to_owned())
    );

    window.ctrl_click_row(2, cx);
    assert_eq!(
        window.label(PLACEHOLDER, cx).as_deref(),
        Some("2 torrents selected")
    );
    assert!(!window.shown("general-hash", cx));

    // A filter or a search that hides one of them leaves one to show.
    cx.update(|cx| {
        window.view.update(cx, |view, cx| {
            view.set_view(
                ViewState {
                    search: "c".to_owned(),
                    ..view.view_state().clone()
                },
                cx,
            );
        });
    });
    assert_eq!(
        window.value("general-hash", cx),
        Some(torrent("c").hash.as_str().to_owned())
    );

    window.press("escape", cx);
    assert_eq!(
        window.label(PLACEHOLDER, cx).as_deref(),
        Some("Select a torrent")
    );
}

#[gpui_kit::test]
fn one_selected_torrent_sends_select_once_and_none_when_hidden(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let (a, b) = (torrent("a"), torrent("b"));
    window.send(vec![a.clone(), b.clone()], cx);
    assert_eq!(window.selects(), [], "nothing is selected yet");

    window.click_row(0, cx);
    assert_eq!(window.selects(), [Some(a.hash.clone())]);

    // The same torrent again, and a new snapshot of it, ask for nothing more.
    window.click_row(0, cx);
    window.send(vec![a.clone(), b.clone()], cx);
    assert_eq!(window.selects(), []);

    window.click_row(1, cx);
    assert_eq!(window.selects(), [Some(b.hash.clone())]);

    // Several torrents: none is read.
    window.ctrl_click_row(0, cx);
    assert_eq!(window.selects(), [None]);

    // A search that leaves one of them shown reads that one.
    cx.update(|cx| {
        window.view.update(cx, |view, cx| {
            view.set_view(
                ViewState {
                    search: "a".to_owned(),
                    ..view.view_state().clone()
                },
                cx,
            );
        });
    });
    assert_eq!(window.selects(), [Some(a.hash.clone())]);

    // Hidden, the panel reads nothing, whatever is selected.
    window.press("alt-enter", cx);
    assert_eq!(window.selects(), [None]);
    window.press("escape", cx);
    window.click_row(0, cx);
    assert_eq!(window.selects(), []);

    // Shown again, it reads the selected torrent.
    window.press("alt-enter", cx);
    assert_eq!(window.selects(), [Some(a.hash.clone())]);

    // The torrent leaves the list: nothing is selected.
    window.send(vec![b], cx);
    assert_eq!(window.selects(), [None]);
}

#[gpui_kit::test]
fn alt_enter_hides_and_shows_the_panel_keeping_its_height(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![torrent("a")], cx);
    window.drag_handle(-60., cx);
    let height = window.bounds(DETAILS, cx).size.height;
    let list = window.bounds(LIST, cx).size.height;
    assert!(height > px(290.), "dragged to {height:?}");

    window.press("alt-enter", cx);
    assert!(!window.shown(DETAILS, cx));
    assert!(!window.ui_state(cx).details.open);
    assert!(
        window.bounds(LIST, cx).size.height > list + height - px(1.),
        "the table takes the room"
    );

    window.press("alt-enter", cx);
    assert!(window.shown(DETAILS, cx));
    assert!(window.ui_state(cx).details.open);
    assert_eq!(window.bounds(DETAILS, cx).size.height, height);
    assert_eq!(window.bounds(LIST, cx).size.height, list);
    assert_eq!(window.ui_state(cx).details.height, f32::from(height));
}

#[gpui_kit::test]
fn a_panel_hidden_last_time_opens_hidden_and_shows_at_its_height(cx: &mut TestAppContext) {
    let ui = UiState {
        details: DetailsState {
            open: false,
            height: 300.,
            tab: DetailsTab::General,
        },
        ..UiState::default()
    };
    let window = Window::open_with(&ui, cx);
    window.send(vec![torrent("a")], cx);
    window.click_row(0, cx);
    assert!(!window.shown(DETAILS, cx));
    assert_eq!(window.selects(), [], "a hidden panel reads nothing");

    window.press("alt-enter", cx);
    assert_eq!(window.bounds(DETAILS, cx).size.height, px(300.));
    assert_eq!(window.selects(), [Some(torrent("a").hash)]);
    assert!(window.ui_state(cx).details.open);
}

#[gpui_kit::test]
fn alt_1_and_alt_2_switch_tabs_and_the_tab_is_remembered(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![torrent("a")], cx);
    window.click_row(0, cx);
    assert!(window.tab_selected(GENERAL, cx));
    assert!(window.shown("general-hash", cx));

    window.press("alt-2", cx);
    assert!(window.tab_selected(FILES, cx));
    assert!(!window.tab_selected(GENERAL, cx));
    assert!(
        !window.shown("general-hash", cx),
        "the Files tab is empty yet"
    );
    assert_eq!(window.ui_state(cx).details.tab, DetailsTab::Files);
    assert!(window.table_focused(cx), "the keys stay in the list");

    window.press("alt-1", cx);
    assert!(window.tab_selected(GENERAL, cx));
    assert!(window.shown("general-hash", cx));
    assert_eq!(window.ui_state(cx).details.tab, DetailsTab::General);

    // A click on a tab does the same.
    window.act(cx, |window, cx| window.within(TABS).click(FILES, cx));
    assert!(window.tab_selected(FILES, cx));
    assert_eq!(window.ui_state(cx).details.tab, DetailsTab::Files);
    assert!(window.table_focused(cx));

    // The next run opens on the tab chosen last.
    let remembered = window.ui_state(cx);
    let next = Window::open_with(&remembered, cx);
    assert!(next.tab_selected(FILES, cx));
}

#[gpui_kit::test]
fn a_restored_height_taller_than_the_window_is_fitted(cx: &mut TestAppContext) {
    let ui = UiState {
        details: DetailsState {
            height: MAX_DETAILS_HEIGHT,
            ..DetailsState::default()
        },
        ..UiState::default()
    };
    let window = Window::open_with(&ui, cx);
    window.send(vec![torrent("a")], cx);
    let fits = |window: &Window, cx: &mut TestAppContext| {
        let panel = window.bounds(DETAILS, cx);
        let list = window.bounds(LIST, cx);
        let status_bar = window.bounds(STATUS_BAR_DOT, cx);
        assert!(
            list.size.height >= px(TABLE_MIN_HEIGHT - 1.),
            "the table keeps its header and two rows: {list:?}"
        );
        assert!(
            panel.size.height >= px(MIN_DETAILS_HEIGHT),
            "the panel is never lower than its minimum: {panel:?}"
        );
        assert!(
            panel.bottom() <= status_bar.top(),
            "the panel {panel:?} ends above the status bar {status_bar:?}"
        );
        panel.size.height
    };

    let fitted = fits(&window, cx);
    assert!(fitted < px(720.), "{fitted:?}");
    assert_eq!(
        window.ui_state(cx).details.height,
        MAX_DETAILS_HEIGHT,
        "a fit is not the user's choice"
    );

    window.resize(1200., 450., cx);
    let smaller = fits(&window, cx);
    assert!(smaller < fitted, "{smaller:?} after {fitted:?}");

    // Grown again, the panel grows back towards the remembered height.
    window.resize(1200., 900., cx);
    let larger = fits(&window, cx);
    assert!(larger > fitted, "{larger:?} after {fitted:?}");
    assert_eq!(window.ui_state(cx).details.height, MAX_DETAILS_HEIGHT);

    // A window that shrank while the panel was hidden fits it when it shows again.
    window.press("alt-enter", cx);
    window.resize(1200., 450., cx);
    window.press("alt-enter", cx);
    window.act(cx, |_, _| ());
    assert_eq!(fits(&window, cx), smaller);
}

#[gpui_kit::test]
fn only_a_user_drag_saves_the_height(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![torrent("a")], cx);
    let first = window.bounds(DETAILS, cx).size.height;
    assert_eq!(first, px(DetailsState::default().height));

    // A window too low for the panel shrinks it, and the remembered height stays.
    window.resize(1200., 330., cx);
    assert!(window.bounds(DETAILS, cx).size.height < first);
    assert_eq!(
        window.ui_state(cx).details.height,
        DetailsState::default().height
    );
    window.resize(1200., 720., cx);
    assert_eq!(window.bounds(DETAILS, cx).size.height, first);

    // A drag is remembered: the height it leaves the panel at.
    window.drag_handle(-80., cx);
    let dragged = window.bounds(DETAILS, cx).size.height;
    assert!(dragged > first + px(70.), "{dragged:?}");
    assert_eq!(window.ui_state(cx).details.height, f32::from(dragged));

    // Hiding and showing the panel, and resizing the window, keep it.
    window.press("alt-enter", cx);
    window.press("alt-enter", cx);
    window.resize(1200., 800., cx);
    assert_eq!(window.ui_state(cx).details.height, f32::from(dragged));
    assert_eq!(window.bounds(DETAILS, cx).size.height, dragged);
}

/// A torrent with what the list knows of it, and none of its files done.
fn show() -> Torrent {
    Torrent {
        download_dir: "/data/torrents".to_owned(),
        total_size: 3 * GIB,
        size_when_done: 2 * GIB,
        ratio: Ratio::Value(1.5),
        added: Some(at(1_700_000_000)),
        done: None,
        ..torrent("a-show")
    }
}

/// The details of [`show`], with its metainfo, its totals and two trackers, one of which did not
/// answer in time.
fn show_details() -> TorrentDetails {
    TorrentDetails {
        comment: "from the tracker".to_owned(),
        creator: "mktorrent".to_owned(),
        date_created: Some(at(1_600_000_000)),
        source: Some("PT".to_owned()),
        is_private: true,
        piece_count: 1500,
        piece_size: 4 * MIB,
        downloaded_ever: 5 * GIB,
        uploaded_ever: 7 * GIB,
        trackers: vec![
            tracker(
                "good.example.org:443",
                Some(TrackerOutcome::Succeeded),
                "Success",
            ),
            tracker(
                "tracker.example.org:6969",
                Some(TrackerOutcome::TimedOut),
                "",
            ),
        ],
        ..details(
            &show().hash,
            &[("a-show/e01.mkv", 3 * GIB, 0, true, FilePriority::Normal)],
        )
    }
}

#[gpui_kit::test]
fn general_shows_metainfo_dates_and_the_tracker_problem(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let (show, other) = (show(), torrent("b-other"));
    window.send(vec![show.clone(), other.clone()], cx);
    window.click_row(0, cx);

    // What the list knows is there at once.
    assert_eq!(window.label("general-path", cx).as_deref(), Some("Path"));
    assert_eq!(
        window.value("general-path", cx).as_deref(),
        Some("/data/torrents/a-show")
    );
    assert_eq!(
        window.value("general-hash", cx).as_deref(),
        Some(show.hash.as_str())
    );
    assert_eq!(
        window.value("general-size", cx).as_deref(),
        Some("3.0 GiB (2.0 GiB wanted)")
    );
    assert_eq!(window.value("general-ratio", cx).as_deref(), Some("1.50"));
    let added = window.value("general-added", cx).unwrap_or_default();
    assert!(is_date_time(&added), "{added:?}");
    assert_eq!(window.value("general-done", cx).as_deref(), Some("not yet"));
    // What only the details know waits for them.
    for id in FROM_DETAILS {
        assert_eq!(window.value(id, cx).as_deref(), Some(PENDING), "{id}");
    }
    assert!(!window.shown(TRACKER_PROBLEM, cx));

    // The details of another torrent are not shown for this one.
    window.send_details(
        TorrentDetails {
            comment: "not this one".to_owned(),
            ..details(&other.hash, &[])
        },
        cx,
    );
    assert_eq!(
        window.value("general-comment", cx).as_deref(),
        Some(PENDING)
    );

    window.send_details(show_details(), cx);
    assert_eq!(
        window.value("general-comment", cx).as_deref(),
        Some("from the tracker")
    );
    let created = window.value("general-created", cx).unwrap_or_default();
    let date = created.strip_prefix("by mktorrent on ").unwrap_or_default();
    assert!(is_date_time(date), "{created:?}");
    assert_eq!(window.value("general-source", cx).as_deref(), Some("PT"));
    assert_eq!(window.value("general-private", cx).as_deref(), Some("Yes"));
    assert_eq!(
        window.value("general-pieces", cx).as_deref(),
        Some("1,500 pieces of 4.0 MiB")
    );
    assert_eq!(
        window.value("general-downloaded", cx).as_deref(),
        Some("5.0 GiB")
    );
    assert_eq!(
        window.value("general-uploaded", cx).as_deref(),
        Some("7.0 GiB")
    );
    assert_eq!(
        window.value("general-trackers", cx).as_deref(),
        Some("1 of 2 working")
    );
    assert_eq!(
        window.label(TRACKER_PROBLEM, cx).as_deref(),
        Some("Tracker tracker.example.org: connection timed out")
    );
}

#[gpui_kit::test]
fn general_follows_newer_details_and_the_errors_of_the_list(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let show = show();
    window.send(vec![show.clone()], cx);
    window.click_row(0, cx);
    window.send_details(show_details(), cx);

    // A tracker that refused names the daemon's text, as sent.
    window.send_details(
        TorrentDetails {
            trackers: vec![tracker(
                "tracker.example.org:443",
                Some(TrackerOutcome::Failed),
                "Unregistered torrent",
            )],
            ..details(&show.hash, &[])
        },
        cx,
    );
    assert_eq!(
        window.label(TRACKER_PROBLEM, cx).as_deref(),
        Some("Tracker tracker.example.org: Unregistered torrent")
    );
    // Without a source, the row is not there; an empty comment says so.
    assert!(!window.shown("general-source", cx));
    assert_eq!(window.value("general-comment", cx).as_deref(), Some("—"));
    assert_eq!(window.value("general-private", cx).as_deref(), Some("No"));
    // These details have no pieces, as a magnet link before its metadata.
    assert_eq!(window.value("general-pieces", cx).as_deref(), Some("—"));
    assert!(!window.shown("general-error", cx));

    // An error the daemon reports for the torrent comes first, in its own words.
    window.send(
        vec![Torrent {
            attention: Some(Attention::LocalError("No space left on device".to_owned())),
            ..show
        }],
        cx,
    );
    assert_eq!(window.label("general-error", cx).as_deref(), Some("Error"));
    assert_eq!(
        window.value("general-error", cx).as_deref(),
        Some("No space left on device")
    );
    let (error, path) = (
        window.bounds("general-error", cx),
        window.bounds("general-path", cx),
    );
    assert!(error.bottom() <= path.top(), "{error:?} above {path:?}");
}

#[gpui_kit::test]
fn copy_hash_and_path_buttons_write_the_clipboard(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let show = Torrent {
        download_dir: "/data/torrents/".to_owned(),
        ..torrent("Show")
    };
    window.send(vec![show.clone()], cx);
    window.click_row(0, cx);

    window.click(COPY_HASH, cx);
    assert_eq!(clipboard(cx).as_deref(), Some(show.hash.as_str()));
    assert!(window.table_focused(cx), "the keys stay in the list");
    assert_eq!(window.label(COPY_HASH, cx).as_deref(), Some("Copy hash"));

    window.click(COPY_PATH, cx);
    assert_eq!(clipboard(cx).as_deref(), Some("/data/torrents/Show"));
    assert!(window.table_focused(cx));
    assert_eq!(window.label(COPY_PATH, cx).as_deref(), Some("Copy path"));

    // Wherever the keys were, a copy hands them back to the list, as a toolbar button does.
    window.press("ctrl-f", cx);
    assert!(!window.table_focused(cx));
    cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
    window.click(COPY_HASH, cx);
    assert_eq!(clipboard(cx).as_deref(), Some(show.hash.as_str()));
    assert!(window.table_focused(cx));

    // Tab from the list reaches the copy buttons. One that has the keys when the panel hides
    // goes with it, and the list takes the keys, so that the window's keys still work.
    window.press("tab", cx);
    window.act(cx, |window, _| {
        assert_eq!(window.find(COPY_PATH).focused(), Some(true));
    });
    window.press("alt-enter", cx);
    assert!(!window.shown(DETAILS, cx));
    assert!(window.table_focused(cx));
    window.press("alt-enter", cx);
    assert!(window.shown(DETAILS, cx));
}

/// Presses the keys of the panel with a menu open, which keeps them, then closes the menu.
fn the_panel_keys_do_nothing_in_the_open_menu(window: &Window, cx: &mut TestAppContext) {
    assert!(window.menu_open(cx));
    for key in ["alt-enter", "alt-2", "alt-1"] {
        window.press(key, cx);
        assert!(window.menu_open(cx), "{key}: the menu keeps the keys");
    }
    assert!(window.shown(DETAILS, cx));
    assert!(window.tab_selected(GENERAL, cx));
    assert_eq!(window.ui_state(cx).details, DetailsState::default());
    window.press("escape", cx);
    assert!(!window.menu_open(cx));
}

#[gpui_kit::test]
fn window_keys_do_nothing_with_a_menu_open(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![torrent("a"), torrent("b")], cx);
    window.click_row(0, cx);

    // The context menu of the list.
    window.act(cx, |window, cx| window.right_click(("row", 0usize), cx));
    the_panel_keys_do_nothing_in_the_open_menu(&window, cx);
    // A menu of the toolbar.
    window.click("column-menu", cx);
    the_panel_keys_do_nothing_in_the_open_menu(&window, cx);

    // Closed, the menus give the keys back.
    window.press("alt-2", cx);
    assert!(window.tab_selected(FILES, cx));
    window.press("alt-enter", cx);
    assert!(!window.shown(DETAILS, cx));
}

#[gpui_kit::test]
fn a_window_lower_than_both_minimums_keeps_them_and_recovers(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![torrent("a")], cx);
    window.click_row(0, cx);
    let heights = |window: &Window, cx: &mut TestAppContext| {
        (
            window.bounds(LIST, cx).size.height,
            window.bounds(DETAILS, cx).size.height,
        )
    };

    // The lowest window the app allows still has room for both.
    window.resize(1200., MIN_WINDOW_HEIGHT, cx);
    let (list, panel) = heights(&window, cx);
    assert!(list >= px(TABLE_MIN_HEIGHT), "{list:?}");
    assert!(panel >= px(MIN_DETAILS_HEIGHT), "{panel:?}");
    assert!(window.bounds(DETAILS, cx).bottom() <= window.bounds(STATUS_BAR_DOT, cx).top());

    // A lower one, which a compositor may still make, keeps both at their minimums.
    window.resize(1200., 200., cx);
    assert_eq!(
        heights(&window, cx),
        (px(TABLE_MIN_HEIGHT), px(MIN_DETAILS_HEIGHT))
    );

    window.resize(1200., 720., cx);
    assert_eq!(
        window.bounds(DETAILS, cx).size.height,
        px(DetailsState::default().height)
    );
    assert_eq!(
        window.ui_state(cx).details.height,
        DetailsState::default().height
    );
}
