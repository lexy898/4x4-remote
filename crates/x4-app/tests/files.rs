//! Headless tests for the Files tab of the details panel: the file tree it draws, opening and
//! closing folders by click and by key, the wanted checkboxes, the priority dropdown and `+`/`-`,
//! the sequential switch, the "Playable from start" part, which torrent and which files an action
//! names, and how the file list's keys and the torrent list's keys stay apart.
//!
//! Each test builds the two channels of a service itself and opens the window with no service
//! behind them, so nothing touches the network: the test reads the commands the window sends and
//! sends the snapshots, the details and the answers a service would send.

use std::sync::Arc;

use gpui_kit::component::table::TableState;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, ElementId, Entity, Focusable as _,
    InputEvent as _, KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, OperatingSystem, Pixels, Point, Role, TestAppContext, px,
};
use x4_app::app;
use x4_app::main_view::MainView;
use x4_app::torrent_table::TorrentTable;
use x4_core::test_support::{details, torrent};
use x4_core::{
    Action, ActionError, ActionId, Command, ConnectionError, ConnectionState, DisplayState, Event,
    KbPerSecond, SessionSummary, Snapshot, SpeedLimit, Torrent, TorrentDetails, UiState,
};
use x4_rpc::{FilePriority, HashString};

/// The file list of the Files tab.
const FILE_LIST: &str = "file-list";
/// The parts of a row of the file list.
const EXPANDER: &str = "file-expander";
const WANTED: &str = "file-wanted";
const PRIORITY: &str = "file-priority";
const PROGRESS: &str = "file-progress";
const NAME: &str = "file-name";
const PLAYABLE: &str = "file-playable";
/// The switch of downloading in order.
const SEQUENTIAL: &str = "file-sequential";
/// What the Files tab says under the switch when the server cannot download in order.
const NEEDS_NEWER: &str = "file-sequential-needs-newer";
/// What the Files tab says while a torrent has no metadata.
const NO_METADATA: &str = "files-no-metadata";
/// What the Files tab says instead of a file list too large to show.
const TOO_LARGE: &str = "files-too-large";
/// What the Files tab says until the details come.
const PENDING: &str = "files-pending";
/// The id of an open menu.
const MENU: &str = "popup-menu";
/// The id of a notification.
const NOTIFICATION: &str = "notification";

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;

/// The element id of the row of the file or folder at `path` inside the torrent.
fn row(path: &str) -> ElementId {
    ElementId::NamedChild(
        Arc::new(ElementId::Name("file-row".into())),
        path.to_owned().into(),
    )
}

/// An open main window and the far ends of its channels, standing in for the service.
struct Window {
    handle: AnyWindowHandle,
    view: Entity<MainView>,
    events: async_channel::Sender<Event>,
    commands: crossbeam_channel::Receiver<Command>,
}

impl Window {
    /// Opens the window, active and with animations off, on the Files tab, making the trees of
    /// big torrents on the test's executor.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a window that does not open fails the test"
    )]
    fn open(cx: &mut TestAppContext) -> Self {
        let (events, events_rx) = async_channel::unbounded();
        let (commands_tx, commands) = crossbeam_channel::unbounded();
        cx.update(|cx| cx.set_reduce_motion(true));
        let mut ui = UiState::default();
        ui.details.tab = x4_core::DetailsTab::Files;
        let (handle, view) = cx
            .update(|cx| {
                app::init(cx);
                app::open_main_window(events_rx, commands_tx, None, &ui, cx)
            })
            .expect("the main window opens");
        cx.update_window(handle, |_, window, _| window.activate_window())
            .expect("the window is open");
        cx.update(|cx| view.update(cx, MainView::make_trees_on_the_executor));
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

    /// Sends a connected snapshot of `torrents` whose daemon's session says whether it can
    /// download in order.
    fn send_session(&self, torrents: Vec<Torrent>, supported: bool, cx: &mut TestAppContext) {
        self.send_event(
            Event::Snapshot(Arc::new(Snapshot {
                torrents,
                listed: true,
                session: Some(SessionSummary {
                    version: "4.0.5".to_owned(),
                    download_dir: "/downloads".to_owned(),
                    alt_speed_enabled: false,
                    speed_limit_down: SpeedLimit {
                        kbps: KbPerSecond(0),
                        enabled: false,
                    },
                    speed_limit_up: SpeedLimit {
                        kbps: KbPerSecond(0),
                        enabled: false,
                    },
                    alt_speed_down_kbps: KbPerSecond(0),
                    alt_speed_up_kbps: KbPerSecond(0),
                    start_added_torrents: true,
                    sequential_download_supported: supported,
                }),
                free_space_bytes: None,
                rate_down: 0,
                rate_up: 0,
                connection: ConnectionState::Connected,
                stale: false,
            })),
            cx,
        );
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

    /// Sends the details of a torrent, as the service does after a read.
    fn send_details(&self, details: TorrentDetails, cx: &mut TestAppContext) {
        self.send_event(Event::Details(Arc::new(details)), cx);
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

    /// Runs `act` on the window as it was drawn last, without drawing it again first.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a closed window fails the test"
    )]
    fn act_on_last_frame(
        &self,
        cx: &mut TestAppContext,
        act: impl FnOnce(&mut gpui_kit::Window, &mut App),
    ) {
        cx.update_window(self.handle, |_, window, cx| act(window, cx))
            .expect("the window is open");
        cx.run_until_parked();
    }

    fn click_row(&self, row: usize, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| window.click(("row", row), cx));
    }

    /// Clicks the part `part` of the row of the file list at `path`.
    fn click_in_row(&self, path: &str, part: &'static str, cx: &mut TestAppContext) {
        let row = row(path);
        self.act(cx, |window, cx| {
            window.within(FILE_LIST).within(row).click(part, cx);
        });
    }

    /// Clicks the row of the file list at `path`, on its name.
    fn click_file(&self, path: &str, cx: &mut TestAppContext) {
        let row = row(path);
        self.act(cx, |window, cx| window.within(FILE_LIST).click(row, cx));
    }

    /// Draws the window and says where the middle of the part `part` of the row at `path` is.
    fn position_of(
        &self,
        path: &str,
        part: &'static str,
        cx: &mut TestAppContext,
    ) -> Point<Pixels> {
        self.bounds_of(path, part, cx).center()
    }

    /// Draws the window and says where the part `part` of the row at `path` is.
    fn bounds_of(&self, path: &str, part: &'static str, cx: &mut TestAppContext) -> Bounds<Pixels> {
        let row = row(path);
        self.act(cx, |window, _| {
            window.within(FILE_LIST).within(row).find(part).bounds()
        })
    }

    /// Draws the window and says where the row at `path` is.
    fn row_bounds(&self, path: &str, cx: &mut TestAppContext) -> Bounds<Pixels> {
        let row = row(path);
        self.act(cx, |window, _| window.within(FILE_LIST).find(row).bounds())
    }

    /// Selects the torrent at `row` of the table, as a caller of the table does, and clicks at
    /// `position` before the window draws again: in one go, so that the click lands on the frame
    /// drawn before the selection moved. gpui draws a window whose state changed once the change
    /// has been carried out, and the main view takes the click after the selection.
    fn select_and_click_without_drawing(
        &self,
        row: usize,
        position: Point<Pixels>,
        cx: &mut TestAppContext,
    ) {
        let table = self.table(cx);
        self.act_on_last_frame(cx, |window, cx| {
            table.update(cx, |table, cx| table.set_selected_row(row, cx));
            let modifiers = Modifiers::none();
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

    /// Whether the element `id` is on screen.
    fn shown(&self, id: impl Into<ElementId>, cx: &mut TestAppContext) -> bool {
        let id = id.into();
        self.act(cx, |window, _| window.try_find(id).is_some())
    }

    /// Whether the row at `path` is drawn in the file list.
    fn row_shown(&self, path: &str, cx: &mut TestAppContext) -> bool {
        let row = row(path);
        self.act(cx, |window, _| {
            window.within(FILE_LIST).try_find(row).is_some()
        })
    }

    /// The accessible label of the element `id`.
    fn label(&self, id: impl Into<ElementId>, cx: &mut TestAppContext) -> Option<String> {
        let id = id.into();
        self.act(cx, |window, _| window.find(id).label().map(str::to_owned))
    }

    /// What the part `part` of the row at `path` says to a screen reader: its role, its label,
    /// its value, whether it is checked, indeterminate, selected, expanded and disabled.
    fn part(&self, path: &str, part: &'static str, cx: &mut TestAppContext) -> Part {
        let row = row(path);
        self.act(cx, |window, _| {
            Part::of(&window.within(FILE_LIST).within(row).find(part))
        })
    }

    /// What the row at `path` says to a screen reader.
    fn row_part(&self, path: &str, cx: &mut TestAppContext) -> Part {
        let row = row(path);
        self.act(cx, |window, _| {
            Part::of(&window.within(FILE_LIST).find(row))
        })
    }

    /// The top of each row at `paths`, as drawn, in the order of `paths`.
    fn tops(&self, paths: &[&str], cx: &mut TestAppContext) -> Vec<Pixels> {
        let rows: Vec<ElementId> = paths.iter().map(|path| row(path)).collect();
        self.act(cx, |window, _| {
            rows.into_iter()
                .map(|row| window.within(FILE_LIST).find(row).bounds().origin.y)
                .collect()
        })
    }

    /// The path of the row the file cursor is on, as the list tells a screen reader.
    fn cursor_row(&self, paths: &[&str], cx: &mut TestAppContext) -> Option<String> {
        let rows: Vec<(String, ElementId)> = paths
            .iter()
            .map(|path| ((*path).to_owned(), row(path)))
            .collect();
        self.act(cx, |window, _| {
            rows.into_iter()
                .find(|(_, row)| {
                    window
                        .within(FILE_LIST)
                        .try_find(row.clone())
                        .is_some_and(|row| row.selected() == Some(true))
                })
                .map(|(path, _)| path)
        })
    }

    /// How many notifications are on screen.
    fn notifications(&self, cx: &mut TestAppContext) -> usize {
        self.act(cx, |window, _| window.find_all(NOTIFICATION).len())
    }

    /// The text of the last failure the window showed.
    fn last_failure(&self, cx: &mut TestAppContext) -> Option<String> {
        cx.update(|cx| self.view.read(cx).last_failure().map(str::to_owned))
    }

    /// Whether the file list has the keyboard focus.
    fn file_list_focused(&self, cx: &mut TestAppContext) -> bool {
        self.act(cx, |window, _| {
            window.find(FILE_LIST).focused() == Some(true)
        })
    }

    fn table(&self, cx: &mut TestAppContext) -> Entity<TableState<TorrentTable>> {
        cx.update(|cx| self.view.read(cx).table().clone())
    }

    /// Whether the table has the keyboard focus, so that the list's keys reach it.
    fn table_focused(&self, cx: &mut TestAppContext) -> bool {
        let table = self.table(cx);
        self.act(cx, |window, cx| table.focus_handle(cx).is_focused(window))
    }

    /// The hashes of the selected torrents.
    fn selected(&self, cx: &mut TestAppContext) -> Vec<HashString> {
        let table = self.table(cx);
        cx.update(|cx| {
            let table = table.read(cx).delegate();
            table
                .selection()
                .targets(table.rows())
                .iter()
                .map(|row| row.hash.clone())
                .collect()
        })
    }

    /// Whether a menu is open.
    fn menu_open(&self, cx: &mut TestAppContext) -> bool {
        self.act(cx, |window, _| window.try_find(MENU).is_some())
    }

    /// Clicks the item `label` of the open menu.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a missing menu item fails the test"
    )]
    fn choose(&self, label: &str, cx: &mut TestAppContext) {
        let item = self.act(cx, |window, _| {
            window
                .within(MENU)
                .find_by_label(label)
                .path()
                .last()
                .cloned()
                .expect("the item has an id")
        });
        self.act(cx, |window, cx| window.within(MENU).click(item, cx));
    }

    /// The actions the window has sent since the last look, in order; other commands are left
    /// out.
    fn actions(&self) -> Vec<Action> {
        self.commands
            .try_iter()
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
        let runs: Vec<(ActionId, Action)> = self
            .commands
            .try_iter()
            .filter_map(|command| match command {
                Command::Run(id, action) => Some((id, action)),
                _ => None,
            })
            .collect();
        match runs.as_slice() {
            [(id, action)] => (*id, action.clone()),
            other => panic!("expected one action, got {other:?}"),
        }
    }
}

/// What a part of the file list says to a screen reader.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Part {
    role: Option<Role>,
    label: Option<String>,
    value: Option<String>,
    checked: Option<bool>,
    indeterminate: Option<bool>,
    selected: Option<bool>,
    expanded: Option<bool>,
    disabled: Option<bool>,
}

impl Part {
    fn of(snapshot: &gpui_kit::test::ElementSnapshot) -> Self {
        Self {
            role: snapshot.role(),
            label: snapshot.label().map(str::to_owned),
            value: snapshot.value().map(str::to_owned),
            checked: snapshot.checked(),
            indeterminate: snapshot.indeterminate(),
            selected: snapshot.selected(),
            expanded: snapshot.expanded(),
            disabled: snapshot.disabled(),
        }
    }
}

fn downloading(name: &str) -> Torrent {
    Torrent {
        state: DisplayState::Downloading,
        ..torrent(name)
    }
}

fn reconnecting() -> ConnectionState {
    ConnectionState::Reconnecting {
        attempt: 2,
        last_error: ConnectionError::Timeout,
    }
}

/// The paths of the show's rows with every folder but the top one closed, in their order.
const SHOW_ROWS: &[&str] = &["Show", "Show/Season 1", "Show/Season 2", "Show/info.nfo"];

/// The details of a show with two seasons, a padding file and a text file:
///
/// - 0: `Season 1/E01.mkv`, complete;
/// - 1: `Season 1/E02.mkv`, 30% done at High, its first 20 MiB playable;
/// - 2: `Season 2/E01.mkv`, not wanted;
/// - 3: `.pad/0`, a padding file, hidden;
/// - 4: `info.nfo`, complete.
///
/// So the top folder is partly wanted and of mixed priority, `Season 1` is wanted and of mixed
/// priority, and `Season 2` is not wanted.
fn show_details(hash: &HashString) -> TorrentDetails {
    let mut details = details(
        hash,
        &[
            (
                "Show/Season 1/E01.mkv",
                100 * MIB,
                100 * MIB,
                true,
                FilePriority::Normal,
            ),
            (
                "Show/Season 1/E02.mkv",
                100 * MIB,
                30 * MIB,
                true,
                FilePriority::High,
            ),
            (
                "Show/Season 2/E01.mkv",
                100 * MIB,
                0,
                false,
                FilePriority::Normal,
            ),
            ("Show/.pad/0", KIB, KIB, true, FilePriority::Normal),
            ("Show/info.nfo", KIB, KIB, true, FilePriority::Normal),
        ],
    );
    if let Some(file) = details.files.get_mut(1) {
        file.playable_from_start = 20 * MIB;
    }
    details
}

/// A window on the Files tab with the torrent `show` selected and its details read.
fn window_with_show(cx: &mut TestAppContext) -> (Window, Torrent) {
    let window = Window::open(cx);
    let show = downloading("show");
    window.send(vec![show.clone()], cx);
    window.click_row(0, cx);
    window.send_details(show_details(&show.hash), cx);
    (window, show)
}

fn wanted(hash: &HashString, files: &[usize], wanted: bool) -> Action {
    Action::SetFilesWanted {
        hash: hash.clone(),
        files: files.to_vec(),
        wanted,
    }
}

fn priority(hash: &HashString, files: &[usize], priority: FilePriority) -> Action {
    Action::SetFilePriority {
        hash: hash.clone(),
        files: files.to_vec(),
        priority,
    }
}

#[gpui_kit::test]
fn files_tab_shows_the_tree_folders_first_with_mixed_states(cx: &mut TestAppContext) {
    let (window, _) = window_with_show(cx);

    assert_eq!(
        window.act(cx, |window, _| {
            let list = window.find(FILE_LIST);
            (list.role(), list.label().map(str::to_owned))
        }),
        (Some(Role::Tree), Some("Files".to_owned()))
    );
    // The top folder is open; the folders come before the file, and the padding file is hidden.
    let tops = window.tops(SHOW_ROWS, cx);
    assert!(tops.windows(2).all(|pair| pair[0] < pair[1]), "{tops:?}");
    assert!(!window.row_shown("Show/.pad", cx));
    assert!(!window.row_shown("Show/.pad/0", cx));
    assert!(!window.row_shown("Show/Season 1/E01.mkv", cx));

    let top = window.row_part("Show", cx);
    assert_eq!(top.role, Some(Role::TreeItem));
    assert_eq!(top.label.as_deref(), Some("Show"));
    assert_eq!(top.expanded, Some(true));
    assert_eq!(window.row_part("Show/Season 1", cx).expanded, Some(false));
    assert_eq!(
        window.row_part("Show/info.nfo", cx).expanded,
        None,
        "a file"
    );

    // Wanted: partly, all, none.
    let top_wanted = window.part("Show", WANTED, cx);
    assert_eq!(top_wanted.role, Some(Role::CheckBox));
    assert_eq!(
        (top_wanted.checked, top_wanted.indeterminate),
        (Some(false), Some(true))
    );
    assert_eq!(top_wanted.label.as_deref(), Some("Download Show"));
    assert_eq!(window.part("Show/Season 1", WANTED, cx).checked, Some(true));
    let skipped = window.part("Show/Season 2", WANTED, cx);
    assert_eq!(
        (skipped.checked, skipped.indeterminate),
        (Some(false), Some(false))
    );

    // Priority: mixed in both folders that hold E02, normal elsewhere.
    assert_eq!(
        window.part("Show", PRIORITY, cx).label.as_deref(),
        Some("Priority of Show: Mixed")
    );
    assert_eq!(
        window.part("Show/Season 1", PRIORITY, cx).label.as_deref(),
        Some("Priority of Season 1: Mixed")
    );
    assert_eq!(
        window.part("Show/Season 2", PRIORITY, cx).label.as_deref(),
        Some("Priority of Season 2: Normal")
    );

    // Progress, of the files shown: Season 1 has 130 of 200 MiB.
    let progress = window.part("Show/Season 1", PROGRESS, cx);
    assert_eq!(progress.role, Some(Role::ProgressIndicator));
    assert_eq!(progress.value.as_deref(), Some("65%"));
    assert_eq!(progress.label.as_deref(), Some("Progress of Season 1"));
    assert_eq!(
        window.part("Show/info.nfo", PROGRESS, cx).value.as_deref(),
        Some("100%")
    );
}

#[gpui_kit::test]
fn expanding_and_collapsing_by_click_and_keys(cx: &mut TestAppContext) {
    let (window, show) = window_with_show(cx);
    assert!(window.table_focused(cx));

    // A click on the expander opens the folder, and the keys go to the file list.
    window.click_in_row("Show/Season 1", EXPANDER, cx);
    assert_eq!(window.row_part("Show/Season 1", cx).expanded, Some(true));
    assert!(window.row_shown("Show/Season 1/E01.mkv", cx));
    assert!(window.row_shown("Show/Season 1/E02.mkv", cx));
    assert!(window.file_list_focused(cx));

    // A new read of the details keeps the folder open.
    window.send_details(show_details(&show.hash), cx);
    assert!(window.row_shown("Show/Season 1/E02.mkv", cx));

    window.click_in_row("Show/Season 1", EXPANDER, cx);
    assert_eq!(window.row_part("Show/Season 1", cx).expanded, Some(false));
    assert!(!window.row_shown("Show/Season 1/E01.mkv", cx));

    // The keys: a click on a row puts the cursor there.
    let all = [
        "Show",
        "Show/Season 1",
        "Show/Season 2",
        "Show/Season 2/E01.mkv",
        "Show/info.nfo",
    ];
    window.click_file("Show/Season 2", cx);
    assert_eq!(
        window.cursor_row(&all, cx).as_deref(),
        Some("Show/Season 2")
    );
    window.press("right", cx);
    assert_eq!(window.row_part("Show/Season 2", cx).expanded, Some(true));
    assert_eq!(
        window.cursor_row(&all, cx).as_deref(),
        Some("Show/Season 2")
    );
    window.press("right", cx);
    assert_eq!(
        window.cursor_row(&all, cx).as_deref(),
        Some("Show/Season 2/E01.mkv")
    );
    window.press("left", cx);
    assert_eq!(
        window.cursor_row(&all, cx).as_deref(),
        Some("Show/Season 2")
    );
    window.press("left", cx);
    assert_eq!(window.row_part("Show/Season 2", cx).expanded, Some(false));
    assert!(!window.row_shown("Show/Season 2/E01.mkv", cx));
    window.press("left", cx);
    assert_eq!(window.cursor_row(&all, cx).as_deref(), Some("Show"));

    window.press("down", cx);
    assert_eq!(
        window.cursor_row(&all, cx).as_deref(),
        Some("Show/Season 1")
    );
    window.press("end", cx);
    assert_eq!(
        window.cursor_row(&all, cx).as_deref(),
        Some("Show/info.nfo")
    );
    window.press("up", cx);
    assert_eq!(
        window.cursor_row(&all, cx).as_deref(),
        Some("Show/Season 2")
    );
    window.press("home", cx);
    assert_eq!(window.cursor_row(&all, cx).as_deref(), Some("Show"));
    window.press("pagedown", cx);
    assert_eq!(
        window.cursor_row(&all, cx).as_deref(),
        Some("Show/info.nfo")
    );
    window.press("pageup", cx);
    assert_eq!(window.cursor_row(&all, cx).as_deref(), Some("Show"));

    // Left on the open top folder closes it.
    window.press("left", cx);
    assert_eq!(window.row_part("Show", cx).expanded, Some(false));
    assert!(!window.row_shown("Show/Season 1", cx));
    assert_eq!(window.actions(), [], "moving around sends nothing");
}

#[gpui_kit::test]
fn space_toggles_wanted_once_per_press_on_the_cursor_row(cx: &mut TestAppContext) {
    let (window, show) = window_with_show(cx);
    window.click_file("Show/Season 1", cx);
    assert!(window.file_list_focused(cx));

    window.key_down("space", false, cx);
    let (id, action) = window.sent_action();
    assert_eq!(action, wanted(&show.hash, &[0, 1], false));
    window.finish(id, Ok(()), cx);
    assert_eq!(
        window.part("Show/Season 1", WANTED, cx).checked,
        Some(true),
        "the box follows the daemon, not the press"
    );

    // The read after the action shows the season skipped while the key is still down: its
    // repeats do not want it again, which would flip it back and forth for as long as it is held.
    let mut skipped = show_details(&show.hash);
    for file in skipped.files.iter_mut().take(2) {
        file.wanted = false;
    }
    window.send_details(skipped, cx);
    assert_eq!(
        window.part("Show/Season 1", WANTED, cx).checked,
        Some(false)
    );
    for _ in 0..3 {
        window.key_down("space", true, cx);
    }
    window.key_up("space", cx);
    assert_eq!(window.actions(), []);

    // The next press wants it again; another before the answer asks for the same again, which
    // is on its way.
    window.press("space", cx);
    assert_eq!(window.sent_action().1, wanted(&show.hash, &[0, 1], true));
    window.press("space", cx);
    assert_eq!(window.actions(), []);
    // The torrent was not paused: Space in the file list is the file list's.
    assert_eq!(window.actions(), []);
}

#[gpui_kit::test]
fn a_folder_checkbox_sends_all_its_indices_including_pad_files(cx: &mut TestAppContext) {
    let (window, show) = window_with_show(cx);

    // Partly wanted: a click wants all of it, the padding file too.
    window.click_in_row("Show", WANTED, cx);
    assert_eq!(
        window.sent_action().1,
        wanted(&show.hash, &[0, 1, 2, 3, 4], true)
    );
    assert!(window.file_list_focused(cx), "the keys go to the file list");
    assert_eq!(
        window.cursor_row(SHOW_ROWS, cx).as_deref(),
        Some("Show"),
        "the cursor goes to the row clicked"
    );

    // A file wanted: a click skips it alone.
    window.click_in_row("Show/info.nfo", WANTED, cx);
    assert_eq!(window.sent_action().1, wanted(&show.hash, &[4], false));
    // Not wanted at all: a click wants it.
    window.click_in_row("Show/Season 2", WANTED, cx);
    assert_eq!(window.sent_action().1, wanted(&show.hash, &[2], true));
}

#[gpui_kit::test]
fn priority_dropdown_and_plus_minus_send_set_file_priority(cx: &mut TestAppContext) {
    let (window, show) = window_with_show(cx);

    window.click_in_row("Show/Season 1", PRIORITY, cx);
    assert!(window.menu_open(cx));
    // The menu keeps the keys while it is open.
    window.press("+", cx);
    window.press("space", cx);
    assert_eq!(window.actions(), []);
    window.choose("High", cx);
    assert_eq!(
        window.sent_action().1,
        priority(&show.hash, &[0, 1], FilePriority::High)
    );
    assert!(!window.menu_open(cx));
    assert!(
        window.file_list_focused(cx),
        "the keys are back in the file list"
    );

    // `+` and `-` step the priority of the row under the cursor, once per press of the key.
    let mut low = show_details(&show.hash);
    if let Some(file) = low.files.get_mut(4) {
        file.priority = FilePriority::Low;
    }
    window.send_details(low, cx);
    window.click_file("Show/info.nfo", cx);
    window.key_down("+", false, cx);
    let (id, action) = window.sent_action();
    assert_eq!(action, priority(&show.hash, &[4], FilePriority::Normal));
    // The read after the answer shows it Normal while the key is still down: its repeats do not
    // raise it again.
    window.finish(id, Ok(()), cx);
    window.send_details(show_details(&show.hash), cx);
    for _ in 0..2 {
        window.key_down("+", true, cx);
    }
    window.key_up("+", cx);
    assert_eq!(window.actions(), []);
    window.press("-", cx);
    assert_eq!(
        window.sent_action().1,
        priority(&show.hash, &[4], FilePriority::Low)
    );
    // A folder of mixed priority goes to Normal either way.
    window.press("home", cx);
    window.press("-", cx);
    assert_eq!(
        window.sent_action().1,
        priority(&show.hash, &[0, 1, 2, 3, 4], FilePriority::Normal)
    );
}

#[gpui_kit::test]
fn a_failed_file_action_shows_a_notification_of_its_kind(cx: &mut TestAppContext) {
    let (window, show) = window_with_show(cx);
    window.click_file("Show/info.nfo", cx);
    window.press("space", cx);
    let (skip, action) = window.sent_action();
    assert_eq!(action, wanted(&show.hash, &[4], false));
    window.press("+", cx);
    let (raise, action) = window.sent_action();
    assert_eq!(action, priority(&show.hash, &[4], FilePriority::High));

    let busy = Err(ActionError::Rpc(ConnectionError::Server("busy".to_owned())));
    window.finish(skip, busy, cx);
    assert_eq!(window.notifications(cx), 1);
    assert_eq!(
        window.last_failure(cx).as_deref(),
        Some("Could not skip files: busy")
    );
    window.finish(raise, Err(ActionError::Rpc(ConnectionError::Timeout)), cx);
    assert_eq!(window.notifications(cx), 2, "one for each kind of change");
    assert_eq!(
        window.last_failure(cx).as_deref(),
        Some(
            "The server did not answer while changing the priority of files; it may have \
             changed. The Files tab will show the result"
        )
    );
}

#[gpui_kit::test]
fn a_file_action_names_the_drawn_hash_even_if_the_selection_changed(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let (a, b) = (downloading("a"), downloading("b"));
    window.send(vec![a.clone(), b.clone()], cx);
    window.click_row(0, cx);
    window.send_details(show_details(&a.hash), cx);
    let position = window.position_of("Show/info.nfo", WANTED, cx);

    // The selection moves to b, whose files have the same paths, and a click lands on a's box
    // before the window draws again. The click names a, which is no longer the panel's torrent
    // when the main view takes it, so it is refused, and never sent for b with a's files.
    window.select_and_click_without_drawing(1, position, cx);
    assert_eq!(window.selected(cx), std::slice::from_ref(&b.hash));
    assert_eq!(window.actions(), []);

    // b's details come and are drawn: the same box is b's now.
    window.send_details(show_details(&b.hash), cx);
    window.click_in_row("Show/info.nfo", WANTED, cx);
    assert_eq!(window.sent_action().1, wanted(&b.hash, &[4], false));
}

#[gpui_kit::test]
fn a_folder_with_only_pad_files_offers_no_toggle(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let film = downloading("film");
    window.send(vec![film.clone()], cx);
    window.click_row(0, cx);
    window.send_details(
        details(
            &film.hash,
            &[
                ("Film/film.mkv", 100 * MIB, 0, true, FilePriority::Normal),
                ("Film/.pad/0", KIB, 0, true, FilePriority::Normal),
                (
                    "Film/extras/_____padding_file_0",
                    KIB,
                    0,
                    true,
                    FilePriority::Normal,
                ),
                (
                    "Film/extras/_____padding_file_1",
                    KIB,
                    0,
                    true,
                    FilePriority::Normal,
                ),
            ],
        ),
        cx,
    );

    // Neither folder of padding has a row, a box or a priority to act on.
    assert!(window.row_shown("Film/film.mkv", cx));
    for hidden in [
        "Film/.pad",
        "Film/extras",
        "Film/.pad/0",
        "Film/extras/_____padding_file_0",
    ] {
        assert!(!window.row_shown(hidden, cx), "{hidden}");
    }
    // The keys never reach them: the last row is the film.
    window.click_file("Film", cx);
    window.press("end", cx);
    window.press("space", cx);
    assert_eq!(window.sent_action().1, wanted(&film.hash, &[0], false));
    // The torrent's folder reaches every file, padding included.
    window.click_in_row("Film", WANTED, cx);
    assert_eq!(
        window.sent_action().1,
        wanted(&film.hash, &[0, 1, 2, 3], false)
    );
}

#[gpui_kit::test]
fn the_sequential_switch_sends_set_sequential(cx: &mut TestAppContext) {
    let (window, show) = window_with_show(cx);
    let switch = window.act(cx, |window, _| Part::of(&window.find(SEQUENTIAL)));
    assert_eq!(switch.role, Some(Role::Switch));
    assert_eq!(
        switch.label.as_deref(),
        Some("Download in order (sequential)")
    );

    window.act(cx, |window, cx| window.click(SEQUENTIAL, cx));
    let (id, action) = window.sent_action();
    assert_eq!(
        action,
        Action::SetSequential {
            hash: show.hash.clone(),
            enabled: true
        }
    );
    assert!(window.file_list_focused(cx), "the keys go to the file list");
    window.finish(id, Ok(()), cx);

    // The list says it is on now; a click turns it off.
    window.send(
        vec![Torrent {
            sequential: true,
            ..show.clone()
        }],
        cx,
    );
    window.act(cx, |window, cx| window.click(SEQUENTIAL, cx));
    assert_eq!(
        window.sent_action().1,
        Action::SetSequential {
            hash: show.hash,
            enabled: false
        }
    );
}

#[gpui_kit::test]
fn the_sequential_switch_says_the_server_needs_4_1(cx: &mut TestAppContext) {
    let (window, show) = window_with_show(cx);
    window.send_session(vec![show.clone()], false, cx);

    assert!(window.shown(SEQUENTIAL, cx), "the switch is still drawn");
    window.act(cx, |window, cx| window.click(SEQUENTIAL, cx));
    assert!(
        window
            .commands
            .try_iter()
            .all(|command| !matches!(command, Command::Run(..))),
        "a click sends no action"
    );
    assert!(window.shown(NEEDS_NEWER, cx));
    assert_eq!(
        window.label(NEEDS_NEWER, cx).as_deref(),
        Some("Downloading in order needs Transmission 4.1 or newer on the server")
    );

    window.send_session(vec![show], true, cx);
    assert!(!window.shown(NEEDS_NEWER, cx));
    window.act(cx, |window, cx| window.click(SEQUENTIAL, cx));
    assert!(matches!(
        window.sent_action().1,
        Action::SetSequential { enabled: true, .. }
    ));
}

#[gpui_kit::test]
fn until_the_details_come_the_files_tab_says_they_are_pending(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let show = downloading("Show");
    window.send(vec![show.clone()], cx);
    window.click_row(0, cx);

    assert_eq!(window.label(PENDING, cx).as_deref(), Some("—"));
    assert!(!window.shown(FILE_LIST, cx));
    assert!(window.shown(SEQUENTIAL, cx), "the switch is there already");

    window.send_details(show_details(&show.hash), cx);
    assert!(!window.shown(PENDING, cx));
    assert!(window.row_shown("Show", cx));
}

#[gpui_kit::test]
fn a_torrent_without_metadata_says_its_files_come_later(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let magnet = downloading("magnet");
    window.send(vec![magnet.clone()], cx);
    window.click_row(0, cx);
    assert!(
        !window.shown(NO_METADATA, cx),
        "nothing is said before the details come"
    );

    window.send_details(details(&magnet.hash, &[]), cx);
    assert_eq!(
        window.label(NO_METADATA, cx).as_deref(),
        Some("The file list arrives with the torrent's metadata")
    );
    assert!(!window.shown(FILE_LIST, cx));
    // Downloading in order can be chosen already.
    window.act(cx, |window, cx| window.click(SEQUENTIAL, cx));
    assert_eq!(
        window.sent_action().1,
        Action::SetSequential {
            hash: magnet.hash.clone(),
            enabled: true
        }
    );

    // The metadata arrives: the tree appears.
    window.send_details(
        details(
            &magnet.hash,
            &[("Album/a.flac", MIB, 0, true, FilePriority::Normal)],
        ),
        cx,
    );
    assert!(!window.shown(NO_METADATA, cx));
    assert!(window.row_shown("Album/a.flac", cx));
}

#[gpui_kit::test]
fn a_file_list_too_large_to_show_says_so(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let deep = downloading("deep");
    window.send(vec![deep.clone()], cx);
    window.click_row(0, cx);

    // 20 000 folders one in the other: a tree of some 400 MiB, which is not made.
    let path = "a/".repeat(20_000) + "f";
    window.send_details(
        details(
            &deep.hash,
            &[(path.as_str(), MIB, 0, true, FilePriority::Normal)],
        ),
        cx,
    );
    assert_eq!(
        window.label(TOO_LARGE, cx).as_deref(),
        Some("This torrent's file list is too large to show")
    );
    assert!(!window.shown(FILE_LIST, cx));
    assert!(!window.shown(NO_METADATA, cx));
    // Downloading in order can still be chosen.
    window.act(cx, |window, cx| window.click(SEQUENTIAL, cx));
    assert_eq!(
        window.sent_action().1,
        Action::SetSequential {
            hash: deep.hash.clone(),
            enabled: true
        }
    );

    // A list that fits is shown again.
    window.send_details(
        details(
            &deep.hash,
            &[("Album/a.flac", MIB, 0, true, FilePriority::Normal)],
        ),
        cx,
    );
    assert!(!window.shown(TOO_LARGE, cx));
    assert!(window.row_shown("Album/a.flac", cx));
}

#[gpui_kit::test]
fn more_than_50_000_files_are_too_large_to_show(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let big = downloading("big");
    window.send(vec![big.clone()], cx);
    window.click_row(0, cx);

    let paths: Vec<String> = (0..=50_000).map(|n| format!("Big/{n}.bin")).collect();
    let files: Vec<(&str, u64, u64, bool, FilePriority)> = paths
        .iter()
        .map(|path| (path.as_str(), KIB, 0, true, FilePriority::Normal))
        .collect();
    window.send_details(details(&big.hash, &files), cx);

    assert_eq!(
        window.label(TOO_LARGE, cx).as_deref(),
        Some("This torrent's file list is too large to show")
    );
    assert!(!window.shown(FILE_LIST, cx));
    assert!(window.shown(SEQUENTIAL, cx));

    // 50 000 files are shown.
    window.send_details(details(&big.hash, files.get(..50_000).unwrap()), cx);
    assert!(!window.shown(TOO_LARGE, cx));
    assert!(window.row_shown("Big", cx));
}

#[gpui_kit::test]
fn the_playable_part_shows_for_wanted_incomplete_files(cx: &mut TestAppContext) {
    let (window, _) = window_with_show(cx);
    window.click_in_row("Show/Season 1", EXPANDER, cx);
    window.click_in_row("Show/Season 2", EXPANDER, cx);

    assert_eq!(
        window
            .part("Show/Season 1/E02.mkv", PLAYABLE, cx)
            .label
            .as_deref(),
        Some("Playable from start: 20.0 MiB (20%)")
    );
    // Complete, not wanted, a folder: none.
    for path in [
        "Show/Season 1/E01.mkv",
        "Show/Season 2/E01.mkv",
        "Show/Season 1",
        "Show",
    ] {
        let row = row(path);
        assert!(
            window.act(cx, |window, _| {
                window
                    .within(FILE_LIST)
                    .within(row)
                    .try_find(PLAYABLE)
                    .is_none()
            }),
            "{path}"
        );
    }
}

#[gpui_kit::test]
fn a_row_is_one_line_with_the_playable_part_after_the_name(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let film = downloading("a");
    window.send(vec![film.clone()], cx);
    window.click_row(0, cx);
    let short = "Show/E01.mkv";
    let long = format!("Show/{}.mkv", "A very long name of an episode ".repeat(20));
    let mut both = details(
        &film.hash,
        &[
            (short, 100 * MIB, 30 * MIB, true, FilePriority::Normal),
            (&long, 100 * MIB, 30 * MIB, true, FilePriority::Normal),
        ],
    );
    for file in &mut both.files {
        file.playable_from_start = 20 * MIB;
    }
    window.send_details(both, cx);

    let height = window.row_bounds(short, cx).size.height;
    assert!(height <= px(28.), "a row is one line: {height:?}");
    for path in [short, long.as_str()] {
        let name = window.bounds_of(path, NAME, cx);
        let playable = window.bounds_of(path, PLAYABLE, cx);
        assert!(
            playable.top() >= name.top() && playable.bottom() <= name.bottom(),
            "on the line of the name: {name:?}, {playable:?}"
        );
        assert!(
            playable.left() >= name.right(),
            "after the name: {name:?}, {playable:?}"
        );
    }
    // The long name is cut short, not the part after it, which says the same as the short one's.
    assert_eq!(
        window.bounds_of(&long, PLAYABLE, cx).size.width,
        window.bounds_of(short, PLAYABLE, cx).size.width
    );
}

#[gpui_kit::test]
fn file_keys_do_not_move_the_torrent_selection_and_back(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let (a, b) = (downloading("a"), downloading("b"));
    window.send(vec![a.clone(), b.clone()], cx);
    window.click_row(0, cx);
    window.send_details(show_details(&a.hash), cx);

    // The file list's keys move the file cursor only.
    window.click_file("Show", cx);
    for key in [
        "down", "down", "up", "end", "home", "pagedown", "pageup", "right", "left",
    ] {
        window.press(key, cx);
        assert_eq!(window.selected(cx), std::slice::from_ref(&a.hash), "{key}");
    }
    assert!(window.file_list_focused(cx));

    // The table's keys move the selection only. With one torrent shown, the selection stays, and
    // so must the file cursor.
    window.press("down", cx);
    let cursor = window.cursor_row(SHOW_ROWS, cx);
    assert_eq!(cursor.as_deref(), Some("Show/Season 1"));
    cx.update(|cx| {
        window.view.update(cx, |view, cx| {
            view.set_view(
                x4_core::ViewState {
                    search: "a".to_owned(),
                    ..view.view_state().clone()
                },
                cx,
            );
        });
    });
    window.click_row(0, cx);
    assert!(window.table_focused(cx));
    for key in [
        "down", "up", "end", "home", "pagedown", "pageup", "right", "left",
    ] {
        window.press(key, cx);
        assert_eq!(window.cursor_row(SHOW_ROWS, cx), cursor, "{key}");
    }
    // Space in the table pauses the torrent; it does not skip a file.
    window.press("space", cx);
    assert_eq!(window.sent_action().1, Action::Stop(vec![a.hash.clone()]));

    // And Down in the table moves the selection, whatever the file list had.
    cx.update(|cx| {
        window.view.update(cx, |view, cx| {
            view.set_view(
                x4_core::ViewState {
                    search: String::new(),
                    ..view.view_state().clone()
                },
                cx,
            );
        });
    });
    window.press("down", cx);
    assert_eq!(window.selected(cx), [b.hash]);
}

#[gpui_kit::test]
fn file_actions_do_nothing_while_disconnected(cx: &mut TestAppContext) {
    let (window, show) = window_with_show(cx);
    window.click_file("Show/info.nfo", cx);

    // The connection breaks: the controls are drawn disabled, and the keys send nothing either.
    // A click that lands before the frame shows them disabled is the `file_actions` unit test's.
    window.send_with(vec![show.clone()], reconnecting(), cx);
    assert_eq!(
        window.part("Show/info.nfo", WANTED, cx).disabled,
        Some(true)
    );
    assert_eq!(
        window.part("Show/info.nfo", PRIORITY, cx).disabled,
        Some(true)
    );
    assert_eq!(
        window.act(cx, |window, _| window.find(SEQUENTIAL).disabled()),
        Some(true)
    );
    assert!(window.file_list_focused(cx));
    for key in ["space", "+", "-"] {
        window.press(key, cx);
    }
    window.act(cx, |window, cx| window.click(SEQUENTIAL, cx));
    assert_eq!(window.actions(), []);

    // Connected again, they work.
    window.send(vec![show.clone()], cx);
    window.click_file("Show/info.nfo", cx);
    window.press("space", cx);
    assert_eq!(window.sent_action().1, wanted(&show.hash, &[4], false));
}

#[gpui_kit::test]
fn a_2000_file_torrent_renders_only_visible_rows(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let album = downloading("album");
    window.send(vec![album.clone()], cx);
    window.click_row(0, cx);
    let names: Vec<String> = (1..=2000)
        .map(|track| format!("Album/Track {track:04}.flac"))
        .collect();
    let files: Vec<(&str, u64, u64, bool, FilePriority)> = names
        .iter()
        .map(|name| (name.as_str(), MIB, 0, true, FilePriority::Normal))
        .collect();
    window.send_details(details(&album.hash, &files), cx);

    // 2001 rows; only those in sight are built.
    let drawn = window.act(cx, |window, _| {
        window.within(FILE_LIST).find_all(WANTED).len()
    });
    assert!((2..60).contains(&drawn), "{drawn} rows drawn");
    assert!(window.row_shown("Album/Track 0001.flac", cx));
    assert!(!window.row_shown("Album/Track 2000.flac", cx));

    // End takes the cursor to the last row, and the list scrolls to it.
    window.click_file("Album", cx);
    window.press("end", cx);
    assert!(window.row_shown("Album/Track 2000.flac", cx));
    assert!(!window.row_shown("Album", cx));
    assert_eq!(
        window.row_part("Album/Track 2000.flac", cx).selected,
        Some(true)
    );
    let drawn = window.act(cx, |window, _| {
        window.within(FILE_LIST).find_all(WANTED).len()
    });
    assert!((2..60).contains(&drawn), "{drawn} rows drawn");
}
