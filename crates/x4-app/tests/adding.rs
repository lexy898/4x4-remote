//! Headless tests for the add dialog: what a request of a `.torrent` file or a magnet link opens,
//! what Add sends, why Add is disabled, the folder history and the free space, the queue, what
//! becomes of an add (added, a duplicate, unconfirmed, failed) and the torrent selected once it
//! shows, the keys an open dialog keeps, and files that cannot be offered.
//!
//! Each test builds the two channels of a service and the channel of add requests itself, and
//! opens the window with no service behind them, so nothing touches the network: the test sends
//! the requests and the snapshots, reads the commands the window sends and answers them as a
//! service would. The `.torrent` files are made by the tests, in a directory of their own.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, ElementId, Entity, Focusable as _, InputEvent as _,
    KeyDownEvent, KeyUpEvent, Keystroke, OperatingSystem, TestAppContext,
};
use x4_app::adding::AddRequest;
use x4_app::app;
use x4_app::main_view::MainView;
use x4_core::launch::{RejectKind, RejectedArg};
use x4_core::test_support::torrent;
use x4_core::{
    Action, ActionId, AddError, AddItem, Added, Command, ConnectionError, ConnectionState,
    DisplayState, Event, Filter, FreeSpaceAnswer, KbPerSecond, SessionSummary, Snapshot,
    SpeedLimit, Torrent, UiState, ViewState, parse_torrent,
};
use x4_rpc::{AddSource, DownloadDir, HashString, MagnetLink, TorrentAdd};

/// The parts of the dialog.
const TITLE: &str = "add-title";
const NAME: &str = "add-name";
const SIZE: &str = "add-size";
const LINK: &str = "add-link";
const MAGNET_NOTE: &str = "add-magnet-note";
const FOLDER: &str = "add-folder";
const FREE_SPACE: &str = "add-free-space";
const FILES: &str = "add-file-list";
const NO_CHOICE: &str = "add-no-choice";
const START: &str = "add-start";
const REASON: &str = "add-reason";
const SHOW: &str = "add-show";
const CONFIRM: &str = "add-confirm";
const CANCEL: &str = "add-cancel";
/// The parts of a row of a file list.
const WANTED: &str = "file-wanted";
const ROW_SIZE: &str = "file-size";
const PROGRESS: &str = "file-progress";
const PRIORITY: &str = "file-priority";
/// The id of an open menu, and of a notification.
const MENU: &str = "popup-menu";
const NOTIFICATION: &str = "notification";
/// The buttons of the dialog that removes torrents from the list.
const REMOVE_CANCEL: &str = "remove-from-list-cancel";

/// The session's download folder.
const SESSION_DIR: &str = "/data/torrents";
/// The free space the snapshots report for it.
const SESSION_FREE: u64 = 800 << 30;

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;
/// The piece length of the test torrents.
const PIECE: u64 = 16 * KIB;

/// A bencoded string.
fn string(value: &[u8]) -> Vec<u8> {
    let mut out = format!("{}:", value.len()).into_bytes();
    out.extend_from_slice(value);
    out
}

/// The `pieces` of a torrent of `total` bytes: one made-up SHA-1 per piece.
#[expect(
    clippy::expect_used,
    reason = "a test helper: a torrent too large to count its pieces fails the test"
)]
fn pieces(total: u64) -> Vec<u8> {
    let count = usize::try_from(total.div_ceil(PIECE).max(1)).expect("a few pieces");
    string(&vec![7_u8; count * 20])
}

/// A `.torrent` of the folder `name` with `files`, each a path inside it and a length.
fn folder_torrent(name: &str, files: &[(&[&str], u64)]) -> Vec<u8> {
    let mut list = b"l".to_vec();
    for (path, length) in files {
        list.extend_from_slice(format!("d6:lengthi{length}e4:pathl").as_bytes());
        for part in *path {
            list.extend(string(part.as_bytes()));
        }
        list.extend_from_slice(b"ee");
    }
    list.push(b'e');
    let total = files.iter().map(|(_, length)| length).sum();
    let mut info = b"d5:files".to_vec();
    info.extend(list);
    info.extend_from_slice(b"4:name");
    info.extend(string(name.as_bytes()));
    info.extend_from_slice(format!("12:piece lengthi{PIECE}e6:pieces").as_bytes());
    info.extend(pieces(total));
    info.push(b'e');
    let mut torrent = b"d4:info".to_vec();
    torrent.extend(info);
    torrent.push(b'e');
    torrent
}

/// "Show": a folder of two episodes and a sample, 3.5 MiB.
fn show() -> Vec<u8> {
    folder_torrent(
        "Show",
        &[
            (&["a.mkv"], 2 * MIB),
            (&["b.mkv"], MIB),
            (&["Sample", "s.mkv"], 512 * KIB),
        ],
    )
}

/// `bytes` with the first `from` in it replaced by `to`.
#[expect(
    clippy::expect_used,
    reason = "a test helper: content without `from` fails the test"
)]
fn replace(bytes: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let at = bytes
        .windows(from.len())
        .position(|window| window == from)
        .expect("the bytes to replace");
    let (before, rest) = bytes.split_at(at);
    let after = rest
        .get(from.len()..)
        .expect("the bytes to replace end in it");
    [before, to, after].concat()
}

/// The info hash of the `.torrent` content `content`.
#[expect(
    clippy::expect_used,
    reason = "a test helper: a torrent of the tests that does not parse fails the test"
)]
fn hash_of(content: &[u8]) -> HashString {
    parse_torrent(content).expect("a valid torrent").hash
}

/// The element id of the row of the file or folder at `path` in a file list.
fn row(path: &str) -> ElementId {
    ElementId::NamedChild(
        Arc::new(ElementId::Name("file-row".into())),
        path.to_owned().into(),
    )
}

/// A magnet link of the hash made of `digit`, named `name` if given.
#[expect(
    clippy::expect_used,
    reason = "a test helper: a link of the tests that does not parse fails the test"
)]
fn magnet(digit: char, name: Option<&str>) -> MagnetLink {
    let hash: String = std::iter::repeat_n(digit, 40).collect();
    let name = name.map(|name| format!("&dn={name}")).unwrap_or_default();
    MagnetLink::parse(&format!("magnet:?xt=urn:btih:{hash}{name}")).expect("a valid link")
}

/// The session the snapshots carry.
fn session() -> SessionSummary {
    let off = SpeedLimit {
        kbps: KbPerSecond(0),
        enabled: false,
    };
    SessionSummary {
        version: "4.1.4".to_owned(),
        download_dir: SESSION_DIR.to_owned(),
        alt_speed_enabled: false,
        speed_limit_down: off,
        speed_limit_up: off,
        alt_speed_down_kbps: KbPerSecond(0),
        alt_speed_up_kbps: KbPerSecond(0),
        start_added_torrents: true,
    }
}

/// The torrent `name` with the hash `hash`, downloading.
fn listed(name: &str, hash: &HashString) -> Torrent {
    Torrent {
        hash: hash.clone(),
        state: DisplayState::Downloading,
        ..torrent(name)
    }
}

/// An open main window, the far ends of its channels, standing in for the service, the sender of
/// its add requests and a directory for the test's files.
struct Window {
    handle: AnyWindowHandle,
    view: Entity<MainView>,
    events: async_channel::Sender<Event>,
    commands: crossbeam_channel::Receiver<Command>,
    adds: async_channel::Sender<AddRequest>,
    dir: tempfile::TempDir,
}

impl Window {
    /// Opens the window, active, with animations off and the state `ui`, and lets it listen for
    /// add requests. The daemon is connected, with the session and no torrents.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a window that does not open fails the test"
    )]
    fn open_with(ui: &UiState, cx: &mut TestAppContext) -> Self {
        let (events, events_rx) = async_channel::unbounded();
        let (commands_tx, commands) = crossbeam_channel::unbounded();
        let (adds, adds_rx) = async_channel::unbounded();
        cx.update(|cx| cx.set_reduce_motion(true));
        let (handle, view) = cx
            .update(|cx| {
                app::init(cx);
                app::open_main_window(events_rx, commands_tx, None, ui, cx)
            })
            .expect("the main window opens");
        cx.update_window(handle, |_, window, cx| {
            window.activate_window();
            view.update(cx, |view, cx| view.listen_for_adds(adds_rx, window, cx));
        })
        .expect("the window is open");
        cx.run_until_parked();
        let window = Self {
            handle,
            view,
            events,
            commands,
            adds,
            dir: tempfile::tempdir().expect("a directory for the test's files"),
        };
        window.send(Vec::new(), cx);
        window
    }

    fn open(cx: &mut TestAppContext) -> Self {
        Self::open_with(&UiState::default(), cx)
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

    /// Sends a snapshot of `torrents` with the daemon connected and the session known.
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
                session: Some(session()),
                free_space_bytes: Some(SESSION_FREE),
                rate_down: 0,
                rate_up: 0,
                connection,
                stale,
            })),
            cx,
        );
    }

    /// Sends `request` as a source of adds would and lets the window take it.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a window that stopped listening fails the test"
    )]
    fn request(&self, request: AddRequest, cx: &mut TestAppContext) {
        self.adds
            .send_blocking(request)
            .expect("the window listens");
        cx.run_until_parked();
    }

    /// Writes `content` to the file `name` in the test's directory and returns its path.
    #[expect(
        clippy::expect_used,
        reason = "a test helper: a file that cannot be written fails the test"
    )]
    fn file(&self, name: &str, content: &[u8]) -> PathBuf {
        let path = self.dir.path().join(name);
        fs::write(&path, content).expect("the test's file is written");
        path
    }

    /// Asks to add the `.torrent` files at `paths`.
    fn request_files(&self, paths: &[&Path], cx: &mut TestAppContext) {
        let items = paths
            .iter()
            .map(|path| AddItem::File(path.to_path_buf()))
            .collect();
        self.request(AddRequest::Items(items), cx);
    }

    /// Writes `content` as `name` and asks to add it.
    fn request_torrent(&self, name: &str, content: &[u8], cx: &mut TestAppContext) {
        let path = self.file(name, content);
        self.request_files(&[&path], cx);
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

    fn click(&self, id: impl Into<ElementId>, cx: &mut TestAppContext) {
        let id = id.into();
        self.act(cx, |window, cx| window.click(id, cx));
    }

    /// Clicks the checkbox of the row at `path` of the dialog's file list.
    fn click_wanted(&self, path: &str, cx: &mut TestAppContext) {
        let row = row(path);
        self.act(cx, |window, cx| {
            window.within(FILES).within(row).click(WANTED, cx);
        });
    }

    /// Whether the checkbox of the row at `path` of the dialog's file list is checked, and whether
    /// it is a dash, for a folder of both.
    fn wanted(&self, path: &str, cx: &mut TestAppContext) -> (Option<bool>, Option<bool>) {
        let row = row(path);
        self.act(cx, |window, _| {
            let part = window.within(FILES).within(row).find(WANTED);
            (part.checked(), part.indeterminate())
        })
    }

    /// Whether the part `part` of the row at `path` of the dialog's file list is drawn.
    fn row_part_shown(&self, path: &str, part: &'static str, cx: &mut TestAppContext) -> bool {
        let row = row(path);
        self.act(cx, |window, _| {
            window.within(FILES).within(row).try_find(part).is_some()
        })
    }

    fn press(&self, key: &str, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| window.press(key, cx));
    }

    /// Types `text` into what has the focus.
    fn type_text(&self, text: &str, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| window.input(text, cx));
    }

    /// Replaces the text of the field `id` with `text`, as the user would: a click into it,
    /// Ctrl+A and the new text.
    fn retype(&self, id: &'static str, text: &str, cx: &mut TestAppContext) {
        self.click(id, cx);
        self.press("ctrl-a", cx);
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

    /// Lets `millis` milliseconds pass.
    fn wait(millis: u64, cx: &mut TestAppContext) {
        cx.executor().advance_clock(Duration::from_millis(millis));
        cx.run_until_parked();
    }

    /// Whether a dialog is open.
    fn dialog_open(&self, cx: &mut TestAppContext) -> bool {
        use gpui_kit::component::WindowExt;
        self.act(cx, WindowExt::has_active_dialog)
    }

    /// Whether the element `id` is on screen.
    fn shown(&self, id: impl Into<ElementId>, cx: &mut TestAppContext) -> bool {
        let id = id.into();
        self.act(cx, |window, _| window.try_find(id).is_some())
    }

    /// The accessible name of `id`, which is what a line of text or a button says.
    fn label(&self, id: &'static str, cx: &mut TestAppContext) -> Option<String> {
        self.act(cx, |window, _| {
            window
                .try_find(id)
                .and_then(|found| found.label().map(str::to_owned))
        })
    }

    /// The text of the field `id`.
    fn value(&self, id: &'static str, cx: &mut TestAppContext) -> Option<String> {
        self.act(cx, |window, _| window.find(id).value().map(str::to_owned))
    }

    /// Whether the element `id` is drawn disabled.
    fn disabled(&self, id: &'static str, cx: &mut TestAppContext) -> bool {
        self.act(cx, |window, _| window.find(id).disabled() == Some(true))
    }

    /// Whether the element `id` has the keyboard focus.
    fn focused(&self, id: &'static str, cx: &mut TestAppContext) -> bool {
        self.act(cx, |window, _| window.find(id).focused() == Some(true))
    }

    /// Whether the switch or checkbox `id` is on.
    fn checked(&self, id: &'static str, cx: &mut TestAppContext) -> Option<bool> {
        self.act(cx, |window, _| window.find(id).checked())
    }

    /// How many notifications are on screen.
    fn notifications(&self, cx: &mut TestAppContext) -> usize {
        self.act(cx, |window, _| window.find_all(NOTIFICATION).len())
    }

    /// What the newest notification of the add dialog said.
    fn last_notice(&self, cx: &mut TestAppContext) -> Option<String> {
        cx.update(|cx| self.view.read(cx).last_add_notice().map(str::to_owned))
    }

    /// The names of the key contexts from the window down to the focus.
    fn context_stack(&self, cx: &mut TestAppContext) -> Vec<String> {
        self.act(cx, |window, _| {
            window
                .context_stack()
                .iter()
                .filter_map(|context| context.primary().map(|entry| entry.key.to_string()))
                .collect()
        })
    }

    /// The commands the window has sent since the last call, other than the window's visibility
    /// and the torrent whose details to read.
    fn sent(&self) -> Vec<Command> {
        self.commands
            .try_iter()
            .filter(|command| !matches!(command, Command::SetVisibility(_) | Command::Select(_)))
            .collect()
    }

    /// The adds of `sent`, with their ids.
    fn adds_in(sent: &[Command]) -> Vec<(ActionId, TorrentAdd)> {
        sent.iter()
            .filter_map(|command| match command {
                Command::Run(id, Action::Add(add)) => Some((*id, add.clone())),
                _ => None,
            })
            .collect()
    }

    /// The adds the window has sent since the last read of the commands.
    fn sent_adds(&self) -> Vec<(ActionId, TorrentAdd)> {
        Self::adds_in(&self.sent())
    }

    /// The one add the window has sent since the last read of the commands.
    #[expect(
        clippy::panic,
        reason = "a test helper: anything but one add fails the test"
    )]
    fn sent_add(&self) -> (ActionId, TorrentAdd) {
        let adds = self.sent_adds();
        let [add] = adds.as_slice() else {
            panic!("one add, not {adds:?}");
        };
        add.clone()
    }

    /// The folders asked about in `sent`.
    fn free_space_in(sent: &[Command]) -> Vec<String> {
        sent.iter()
            .filter_map(|command| match command {
                Command::FreeSpace(folder) => Some(folder.as_str().to_owned()),
                _ => None,
            })
            .collect()
    }

    /// Answers the add `id` with `result`.
    fn finish(&self, id: ActionId, result: Result<Added, AddError>, cx: &mut TestAppContext) {
        self.send_event(Event::AddFinished { id, result }, cx);
    }

    /// The hashes of the selected torrents.
    fn selected(&self, cx: &mut TestAppContext) -> Vec<HashString> {
        cx.update(|cx| {
            let table = self.view.read(cx).table().read(cx);
            let delegate = table.delegate();
            delegate
                .rows()
                .iter()
                .filter(|row| delegate.selection().contains(&row.hash))
                .map(|row| row.hash.clone())
                .collect()
        })
    }

    /// The filter and search the table follows.
    fn view_state(&self, cx: &mut TestAppContext) -> ViewState {
        cx.update(|cx| self.view.read(cx).view_state().clone())
    }

    /// The folders the window remembers torrents were added to.
    fn recent_folders(&self, cx: &mut TestAppContext) -> Vec<String> {
        cx.update(|cx| self.view.read(cx).ui_state().add.recent_folders.clone())
    }
}

/// The added torrent `hash`, named `name`, as the daemon reports it.
fn added(hash: &HashString, name: &str, duplicate: bool) -> Added {
    Added {
        hash: hash.clone(),
        name: name.to_owned(),
        duplicate,
    }
}

/// The folder `path`, checked.
#[expect(
    clippy::expect_used,
    reason = "a test helper: a folder of the tests that is not valid fails the test"
)]
fn dir(path: &str) -> DownloadDir {
    DownloadDir::new(path).expect("a valid folder")
}

#[gpui_kit::test]
fn a_file_request_opens_the_dialog_with_its_files_all_wanted(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    assert!(!window.dialog_open(cx));

    window.request_torrent("show.torrent", &show(), cx);

    assert!(window.dialog_open(cx));
    assert_eq!(window.label(TITLE, cx).as_deref(), Some("Add torrent"));
    assert_eq!(window.label(NAME, cx).as_deref(), Some("Show"));
    assert_eq!(window.label(SIZE, cx).as_deref(), Some("3.5 MiB"));
    assert_eq!(window.value(FOLDER, cx).as_deref(), Some(SESSION_DIR));
    assert_eq!(
        window.label(FREE_SPACE, cx).as_deref(),
        Some("800 GiB free"),
        "the session's folder has the free space of the snapshot"
    );
    assert_eq!(
        window.checked(START, cx),
        Some(true),
        "the session starts them"
    );
    for path in ["Show", "Show/a.mkv", "Show/b.mkv", "Show/Sample"] {
        assert_eq!(window.wanted(path, cx), (Some(true), Some(false)), "{path}");
    }
    assert!(window.row_part_shown("Show/a.mkv", ROW_SIZE, cx));
    assert!(!window.row_part_shown("Show/a.mkv", PROGRESS, cx));
    assert!(!window.row_part_shown("Show/a.mkv", PRIORITY, cx));
    assert!(!window.shown(LINK, cx));
    assert!(!window.shown(NO_CHOICE, cx));
    assert!(!window.disabled(CONFIRM, cx));
    assert!(window.focused(CONFIRM, cx), "Enter adds it at once");
    assert_eq!(window.label(REASON, cx), None);
    assert_eq!(window.sent(), [], "nothing is sent by opening it");
}

#[gpui_kit::test]
fn add_sends_the_folder_start_and_unwanted_files_once(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let content = show();
    window.request_torrent("show.torrent", &content, cx);

    window.click_wanted("Show/b.mkv", cx);
    assert_eq!(window.wanted("Show", cx), (Some(false), Some(true)));
    window.retype(FOLDER, "/data//shows/", cx);
    window.click(START, cx);
    assert_eq!(window.checked(START, cx), Some(false));
    window.click(CONFIRM, cx);

    let (id, add) = window.sent_add();
    let AddSource::Metainfo(bytes) = &add.source else {
        panic!("a .torrent goes as its content: {add:?}");
    };
    assert_eq!(&bytes[..], &content[..], "the very bytes of the file");
    assert_eq!(add.download_dir, Some(dir("/data/shows")), "normalized");
    assert!(add.paused);
    assert_eq!(add.files_unwanted, [1]);
    assert!(add.priority_high.is_empty() && add.priority_low.is_empty());

    // While it is out, the dialog waits and sends nothing more.
    assert_eq!(window.label(CONFIRM, cx).as_deref(), Some("Adding…"));
    assert!(window.disabled(CONFIRM, cx));
    window.click(CONFIRM, cx);
    window.press("enter", cx);
    assert_eq!(window.sent_adds(), []);

    // A failure leaves the form as it was, and it can be sent again.
    window.finish(
        id,
        Err(AddError::Daemon("unrecognized info".to_owned())),
        cx,
    );
    assert!(window.dialog_open(cx));
    assert_eq!(
        window.last_notice(cx).as_deref(),
        Some("Could not add Show: unrecognized info")
    );
    assert_eq!(window.wanted("Show/b.mkv", cx), (Some(false), Some(false)));
    assert_eq!(window.value(FOLDER, cx).as_deref(), Some("/data//shows/"));
    assert_eq!(window.label(CONFIRM, cx).as_deref(), Some("Add"));
    window.click(CONFIRM, cx);
    let (again, add) = window.sent_add();
    assert_ne!(again, id);
    assert_eq!(add.files_unwanted, [1]);

    // The session's own folder is not named.
    window.finish(again, Err(AddError::NotConnected), cx);
    window.retype(FOLDER, "/data/torrents/", cx);
    window.click(CONFIRM, cx);
    assert_eq!(window.sent_add().1.download_dir, None);
}

#[gpui_kit::test]
fn nothing_wanted_or_a_bad_folder_disables_add_with_the_reason(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.request_torrent("show.torrent", &show(), cx);

    window.click_wanted("Show", cx);
    assert_eq!(window.wanted("Show/a.mkv", cx), (Some(false), Some(false)));
    assert!(window.disabled(CONFIRM, cx));
    assert_eq!(
        window.label(REASON, cx).as_deref(),
        Some("Choose at least one file")
    );
    window.click(CONFIRM, cx);
    window.click_wanted("Show/a.mkv", cx);
    assert!(!window.disabled(CONFIRM, cx));
    assert_eq!(window.label(REASON, cx), None);

    for (folder, reason) in [
        (
            "data/shows",
            "The folder must be a full path on the server, from /",
        ),
        ("", "Name a folder on the server"),
        ("/data/../etc", "The folder cannot have . or .. in its path"),
    ] {
        window.retype(FOLDER, folder, cx);
        assert!(window.disabled(CONFIRM, cx), "{folder:?}");
        assert_eq!(
            window.label(REASON, cx).as_deref(),
            Some(reason),
            "{folder:?}"
        );
        window.press("enter", cx);
    }

    // The daemon away disables it too, and the form is kept.
    window.retype(FOLDER, "/data/shows", cx);
    window.send_with(Vec::new(), ConnectionState::Connecting, cx);
    assert!(window.disabled(CONFIRM, cx));
    assert_eq!(
        window.label(REASON, cx).as_deref(),
        Some("Not connected to the server")
    );
    window.send(Vec::new(), cx);
    assert!(!window.disabled(CONFIRM, cx));
    assert_eq!(
        window.sent_adds(),
        [],
        "nothing went out while it was disabled"
    );
}

#[gpui_kit::test]
fn a_magnet_request_shows_its_name_and_never_a_file_list(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let link = magnet('a', Some("Film"));
    let unnamed = magnet('b', None);
    window.request(
        AddRequest::Items(vec![
            AddItem::Magnet(Some(link.clone())),
            AddItem::Magnet(Some(unnamed.clone())),
            AddItem::Magnet(None),
        ]),
        cx,
    );

    assert_eq!(
        window.label(TITLE, cx).as_deref(),
        Some("Add torrent (1 of 3)")
    );
    assert_eq!(window.label(NAME, cx).as_deref(), Some("Film"));
    assert_eq!(window.value(LINK, cx).as_deref(), Some(link.as_str()));
    assert!(!window.shown(FILES, cx));
    assert!(!window.shown(SIZE, cx));
    assert!(window.shown(MAGNET_NOTE, cx));
    assert!(window.focused(CONFIRM, cx), "a valid link is ready to add");
    window.click(CONFIRM, cx);
    let (id, add) = window.sent_add();
    assert_eq!(add.source, AddSource::Magnet(link.clone()));
    assert_eq!(add.files_unwanted, [] as [usize; 0]);
    assert_eq!(add.download_dir, None);
    window.finish(id, Ok(added(link.hash(), "Film", false)), cx);

    // A link without a name shows its hash.
    assert_eq!(
        window.label(TITLE, cx).as_deref(),
        Some("Add torrent (2 of 3)")
    );
    assert_eq!(
        window.label(NAME, cx).as_deref(),
        Some(unnamed.hash().as_str())
    );
    window.click(CANCEL, cx);

    // The empty form asks for a link, and checks it as it is typed.
    assert_eq!(window.value(LINK, cx).as_deref(), Some(""));
    assert!(window.focused(LINK, cx));
    assert!(window.disabled(CONFIRM, cx));
    assert_eq!(
        window.label(REASON, cx).as_deref(),
        Some("Paste a magnet link")
    );
    window.type_text("https://example.org/film.torrent", cx);
    assert_eq!(
        window.label(REASON, cx).as_deref(),
        Some("This is not a magnet link: it must start with magnet:?")
    );
    let typed = magnet('c', Some("Typed"));
    window.retype(LINK, typed.as_str(), cx);
    assert_eq!(window.label(NAME, cx).as_deref(), Some("Typed"));
    assert!(!window.disabled(CONFIRM, cx));
    assert!(!window.shown(FILES, cx));
    window.click(CONFIRM, cx);
    assert_eq!(window.sent_add().1.source, AddSource::Magnet(typed));
}

#[gpui_kit::test]
fn the_folder_history_opens_with_alt_down_and_picks_by_keyboard(cx: &mut TestAppContext) {
    let mut ui = UiState::default();
    ui.add.recent_folders = vec!["/data/films".to_owned(), "/data/music".to_owned()];
    let window = Window::open_with(&ui, cx);
    window.request_torrent("show.torrent", &show(), cx);
    window.click(FOLDER, cx);
    assert!(!window.shown(MENU, cx));

    window.press("alt-down", cx);

    assert!(window.shown(MENU, cx));
    let tops: Vec<Option<f32>> = window.act(cx, |window, _| {
        [SESSION_DIR, "/data/films", "/data/music"]
            .iter()
            .map(|folder| {
                window
                    .within(MENU)
                    .try_find_by_label(folder)
                    .map(|item| f32::from(item.bounds().origin.y))
            })
            .collect()
    });
    assert!(
        tops.iter().all(Option::is_some) && tops.is_sorted(),
        "the session's folder, then the history: {tops:?}"
    );
    window.press("down", cx);
    window.press("down", cx);
    window.press("enter", cx);

    assert!(!window.shown(MENU, cx));
    assert_eq!(window.value(FOLDER, cx).as_deref(), Some("/data/films"));
    assert!(window.focused(FOLDER, cx), "the keys go back to the field");
    assert!(
        window.dialog_open(cx),
        "Enter in the menu picks, and adds nothing"
    );
    assert_eq!(window.sent_adds(), []);

    // Escape closes only the menu.
    window.press("alt-down", cx);
    assert!(window.shown(MENU, cx));
    window.press("escape", cx);
    assert!(!window.shown(MENU, cx));
    assert!(window.dialog_open(cx));
    assert_eq!(window.value(FOLDER, cx).as_deref(), Some("/data/films"));
}

#[gpui_kit::test]
fn free_space_is_asked_once_after_typing_stops(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.request_torrent("show.torrent", &show(), cx);
    assert_eq!(
        Window::free_space_in(&window.sent()),
        Vec::<String>::new(),
        "the session's folder is known from the snapshot"
    );

    window.retype(FOLDER, "/data/ne", cx);
    Window::wait(300, cx);
    window.type_text("w", cx);
    Window::wait(499, cx);
    assert_eq!(Window::free_space_in(&window.sent()), Vec::<String>::new());
    assert_eq!(window.label(FREE_SPACE, cx), None, "nothing known yet");
    Window::wait(1, cx);
    assert_eq!(Window::free_space_in(&window.sent()), ["/data/new"]);
    Window::wait(2000, cx);
    assert_eq!(Window::free_space_in(&window.sent()), Vec::<String>::new());

    // An answer for a folder the field has left is not shown.
    window.send_event(
        Event::FreeSpace {
            path: dir("/data/old"),
            answer: FreeSpaceAnswer::Free { bytes: MIB },
        },
        cx,
    );
    assert_eq!(window.label(FREE_SPACE, cx), None);
    window.send_event(
        Event::FreeSpace {
            path: dir("/data/new"),
            answer: FreeSpaceAnswer::Missing {
                parent: Some(dir("/data")),
                free: Some(3 << 30),
            },
        },
        cx,
    );
    assert_eq!(
        window.label(FREE_SPACE, cx).as_deref(),
        Some("New folder; 3.0 GiB free in /data")
    );

    // Back to a folder asked about, nothing is asked again.
    window.retype(FOLDER, SESSION_DIR, cx);
    Window::wait(600, cx);
    assert_eq!(
        window.label(FREE_SPACE, cx).as_deref(),
        Some("800 GiB free")
    );
    window.retype(FOLDER, "/data/new/", cx);
    Window::wait(600, cx);
    assert_eq!(Window::free_space_in(&window.sent()), Vec::<String>::new());
    assert_eq!(
        window.label(FREE_SPACE, cx).as_deref(),
        Some("New folder; 3.0 GiB free in /data")
    );

    // A folder not known, and then known once the daemon is back.
    window.retype(FOLDER, "/srv", cx);
    Window::wait(500, cx);
    assert_eq!(Window::free_space_in(&window.sent()), ["/srv"]);
    window.send_event(
        Event::FreeSpace {
            path: dir("/srv"),
            answer: FreeSpaceAnswer::Unknown,
        },
        cx,
    );
    assert_eq!(
        window.label(FREE_SPACE, cx).as_deref(),
        Some("Free space unknown")
    );
    window.send_with(Vec::new(), ConnectionState::Connecting, cx);
    assert_eq!(Window::free_space_in(&window.sent()), Vec::<String>::new());
    window.send(Vec::new(), cx);
    assert_eq!(
        Window::free_space_in(&window.sent()),
        ["/srv"],
        "an unknown answer is asked again once the daemon is back"
    );
}

#[gpui_kit::test]
fn the_queue_shows_its_position_and_escape_skips(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let first = window.file("first.torrent", &folder_torrent("First", &[(&["a"], MIB)]));
    let second = window.file(
        "second.torrent",
        &folder_torrent("Second", &[(&["b"], MIB)]),
    );
    window.request_files(&[&first, &second], cx);
    window.request(
        AddRequest::Items(vec![AddItem::Magnet(Some(magnet('d', Some("Third"))))]),
        cx,
    );

    assert_eq!(
        window.label(TITLE, cx).as_deref(),
        Some("Add torrent (1 of 3)")
    );
    assert_eq!(window.label(NAME, cx).as_deref(), Some("First"));
    window.press("escape", cx);
    assert_eq!(
        window.label(TITLE, cx).as_deref(),
        Some("Add torrent (2 of 3)")
    );
    assert_eq!(window.label(NAME, cx).as_deref(), Some("Second"));
    window.click(CANCEL, cx);
    assert_eq!(
        window.label(TITLE, cx).as_deref(),
        Some("Add torrent (3 of 3)")
    );
    assert_eq!(window.label(NAME, cx).as_deref(), Some("Third"));
    window.press("escape", cx);

    assert!(!window.dialog_open(cx));
    assert_eq!(window.sent_adds(), []);
    assert_eq!(window.notifications(cx), 0);

    // A new request starts a new run.
    window.request_files(&[&first], cx);
    assert_eq!(window.label(TITLE, cx).as_deref(), Some("Add torrent"));
}

#[gpui_kit::test]
fn a_duplicate_is_shown_and_selected(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let content = show();
    let hash = hash_of(&content);
    let other = torrent("Other");
    window.send(vec![other.clone()], cx);
    window.request_torrent("show.torrent", &content, cx);
    window.retype(FOLDER, "/data/shows", cx);
    window.click(CONFIRM, cx);
    let (id, _) = window.sent_add();

    window.finish(id, Ok(added(&hash, "Show", true)), cx);

    assert!(!window.dialog_open(cx));
    assert_eq!(
        window.last_notice(cx).as_deref(),
        Some("Show is in the list already")
    );
    assert_eq!(window.notifications(cx), 1);
    assert_eq!(
        window.recent_folders(cx),
        Vec::<String>::new(),
        "a duplicate is no add"
    );
    window.send(vec![other.clone(), listed("Show", &hash)], cx);
    assert_eq!(window.selected(cx), std::slice::from_ref(&hash));

    // A torrent in the list is offered to show, and nothing is sent.
    window.request_torrent("again.torrent", &content, cx);
    assert!(window.disabled(CONFIRM, cx));
    assert_eq!(
        window.label(REASON, cx).as_deref(),
        Some("This torrent is in the list already")
    );
    window.click(CANCEL, cx);
    window.click(("row", 0_usize), cx);
    assert_eq!(window.selected(cx), std::slice::from_ref(&other.hash));
    let _ = window.sent();
    window.request_torrent("again2.torrent", &content, cx);
    window.click(SHOW, cx);
    assert!(!window.dialog_open(cx));
    assert_eq!(window.selected(cx), [hash]);
    assert_eq!(window.sent_adds(), []);
}

#[gpui_kit::test]
fn an_unconfirmed_add_keeps_waiting_for_the_hash(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let content = show();
    let hash = hash_of(&content);
    window.request_torrent("show.torrent", &content, cx);
    window.retype(FOLDER, "/data/shows", cx);
    window.click(CONFIRM, cx);
    let (id, _) = window.sent_add();

    window.finish(id, Err(AddError::Unconfirmed), cx);

    assert!(!window.dialog_open(cx));
    assert_eq!(
        window.last_notice(cx).as_deref(),
        Some("The server has not confirmed Show yet; it shows in the list once it is added")
    );
    assert_eq!(
        window.recent_folders(cx),
        Vec::<String>::new(),
        "not known to be added"
    );
    let other = torrent("Other");
    window.send(vec![other.clone()], cx);
    assert_eq!(window.selected(cx), []);
    window.send(vec![other, listed("Show", &hash)], cx);
    assert_eq!(window.selected(cx), [hash]);
}

#[gpui_kit::test]
fn the_new_torrent_is_selected_and_shown_when_it_appears(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let content = show();
    let hash = hash_of(&content);
    let other = torrent("Other");
    window.send(vec![other.clone()], cx);
    window.click(("row", 0_usize), cx);
    cx.update(|cx| {
        window.view.update(cx, |view, cx| {
            view.set_view(
                ViewState {
                    filter: Filter::Paused,
                    search: "zzz".to_owned(),
                    ..ViewState::default()
                },
                cx,
            );
        });
    });
    window.request_torrent("show.torrent", &content, cx);
    window.retype(FOLDER, "/data/shows", cx);

    // Cancel while it is out lets the dialog go, and the add still counts.
    window.click(CONFIRM, cx);
    let (id, _) = window.sent_add();
    window.click(CANCEL, cx);
    assert!(!window.dialog_open(cx));
    window.finish(id, Ok(added(&hash, "Show", false)), cx);

    assert_eq!(window.recent_folders(cx), ["/data/shows"]);
    window.send(vec![other.clone(), listed("Show", &hash)], cx);
    let view = window.view_state(cx);
    assert_eq!((view.filter, view.search.as_str()), (Filter::All, ""));
    assert_eq!(window.value("search", cx).as_deref(), Some(""));
    assert_eq!(window.selected(cx), std::slice::from_ref(&hash));

    // Once selected, it is not selected again over the user's choice.
    window.click(("row", 0_usize), cx);
    window.send(vec![other.clone(), listed("Show", &hash)], cx);
    assert_eq!(window.selected(cx), std::slice::from_ref(&other.hash));

    // An add to the session's folder names none, so it adds nothing to the history.
    let film = folder_torrent("Film", &[(&["film.mkv"], MIB)]);
    window.request_torrent("film.torrent", &film, cx);
    window.retype(FOLDER, SESSION_DIR, cx);
    window.click(CONFIRM, cx);
    let (id, add) = window.sent_add();
    assert_eq!(add.download_dir, None);
    window.finish(id, Ok(added(&hash_of(&film), "Film", false)), cx);
    assert_eq!(window.recent_folders(cx), ["/data/shows"]);
}

#[gpui_kit::test]
fn the_open_dialog_keeps_every_window_key(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let other = torrent("Other");
    window.send(vec![other], cx);
    window.click(("row", 0_usize), cx);
    let _ = window.sent();
    window.request_torrent("show.torrent", &show(), cx);

    assert_eq!(window.context_stack(cx), ["Root", "Dialog", "AddTorrent"]);
    window.click(FOLDER, cx);
    assert_eq!(
        window.context_stack(cx),
        ["Root", "Dialog", "AddTorrent", "FolderField", "Input"]
    );
    window.press("alt-down", cx);
    assert_eq!(
        window.context_stack(cx),
        [
            "Root",
            "Dialog",
            "AddTorrent",
            "FolderField",
            "Popover",
            "PopoverContent",
            "PopupMenu"
        ]
    );
    window.press("escape", cx);
    window.act(cx, |window, cx| {
        window.within(FILES).click(row("Show/a.mkv"), cx);
    });
    assert_eq!(
        window.context_stack(cx),
        ["Root", "Dialog", "AddTorrent", "FileList"]
    );

    // The window's keys and the list's do nothing behind the dialog.
    for key in [
        "ctrl-f",
        "delete",
        "ctrl-p",
        "alt-enter",
        "ctrl-l",
        "ctrl-a",
    ] {
        window.press(key, cx);
        assert!(window.dialog_open(cx), "{key}");
        assert_eq!(window.sent(), [], "{key}");
        assert!(!window.shown(REMOVE_CANCEL, cx), "{key}");
    }
    assert!(!window.focused("search", cx));
    // The file list's own keys work there.
    window.press("space", cx);
    assert_eq!(window.wanted("Show/a.mkv", cx), (Some(false), Some(false)));
}

#[gpui_kit::test]
fn enter_in_a_field_adds_and_enter_elsewhere_does_nothing(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.request_torrent("show.torrent", &show(), cx);

    // In the file list, Enter does nothing.
    window.act(cx, |window, cx| {
        window.within(FILES).click(row("Show/a.mkv"), cx);
    });
    window.press("enter", cx);
    assert!(window.dialog_open(cx));
    assert_eq!(window.sent_adds(), []);

    // In the folder field it adds, once for a held key.
    window.click(FOLDER, cx);
    window.key_down("enter", false, cx);
    window.key_down("enter", true, cx);
    window.key_down("enter", true, cx);
    window.key_up("enter", cx);
    let (id, _) = window.sent_add();
    window.finish(id, Err(AddError::Failed(ConnectionError::Timeout)), cx);
    assert!(window.dialog_open(cx), "a failure keeps the form");

    // So it does in the link field of a magnet.
    window.click(CANCEL, cx);
    window.request(
        AddRequest::Items(vec![AddItem::Magnet(Some(magnet('e', Some("Linked"))))]),
        cx,
    );
    window.click(LINK, cx);
    window.press("enter", cx);
    assert_eq!(window.sent_adds().len(), 1);
}

#[gpui_kit::test]
fn a_request_waits_while_the_removal_dialog_is_open(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.send(vec![torrent("Other")], cx);
    window.click(("row", 0_usize), cx);
    window.press("delete", cx);
    assert!(window.shown(REMOVE_CANCEL, cx));

    window.request_torrent("show.torrent", &show(), cx);

    assert!(!window.shown(TITLE, cx), "it waits");
    assert!(window.shown(REMOVE_CANCEL, cx));
    window.press("escape", cx);
    assert!(!window.shown(REMOVE_CANCEL, cx));
    assert_eq!(window.label(NAME, cx).as_deref(), Some("Show"));

    // And so after the removal dialog's own Cancel button.
    window.press("escape", cx);
    window.click(("row", 0_usize), cx);
    window.press("delete", cx);
    window.request_torrent("again.torrent", &show(), cx);
    assert!(!window.shown(TITLE, cx));
    window.click(REMOVE_CANCEL, cx);
    assert_eq!(window.label(NAME, cx).as_deref(), Some("Show"));
}

#[gpui_kit::test]
fn an_unreadable_or_oversized_file_is_skipped_with_a_notification(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let missing = window.dir.path().join("missing.torrent");
    let folder = window.dir.path().join("folder.torrent");
    fs::create_dir(&folder).expect("a directory");
    let huge = window.file("huge.torrent", b"");
    fs::File::options()
        .write(true)
        .open(&huge)
        .and_then(|file| file.set_len(17 * MIB))
        .expect("a sparse file of 17 MiB");
    let text = window.file("notes.torrent", b"these are not bencoded");
    let v2 = window.file(
        "v2.torrent",
        b"d4:infod9:file treed1:ad0:d6:lengthi1eeee12:meta versioni2e4:name1:a12:piece lengthi16384eee",
    );
    let good = window.file("show.torrent", &show());

    window.request_files(&[&missing, &folder, &huge, &text, &v2, &good], cx);

    assert_eq!(window.notifications(cx), 5);
    assert_eq!(window.label(NAME, cx).as_deref(), Some("Show"));
    assert_eq!(
        window.label(TITLE, cx).as_deref(),
        Some("Add torrent (6 of 6)")
    );
    window.press("escape", cx);
    for (file, reason) in [
        (&missing, "it does not exist"),
        (&folder, "it is not a file"),
        (&huge, "it is larger than 16.0 MiB"),
        (&text, "it is not a torrent file"),
        (
            &v2,
            "it is a BitTorrent v2 torrent, which the server cannot add",
        ),
    ] {
        window.request_files(&[file], cx);
        assert!(!window.dialog_open(cx));
        let name = file
            .file_name()
            .and_then(|name| name.to_str())
            .expect("a name");
        assert_eq!(
            window.last_notice(cx),
            Some(format!("Could not open {name}: {reason}")),
            "{name}"
        );
    }
}

#[gpui_kit::test]
fn rejected_arguments_make_one_notification_and_raise_activates(cx: &mut TestAppContext) {
    let window = Window::open(cx);

    window.request(
        AddRequest::Rejected(vec![
            RejectedArg {
                kind: RejectKind::Url,
            },
            RejectedArg {
                kind: RejectKind::Option,
            },
        ]),
        cx,
    );

    assert_eq!(window.notifications(cx), 1);
    assert_eq!(
        window.last_notice(cx).as_deref(),
        Some(
            "Could not open 2 of the items 4x4 was given; the first is a web address, and only \
             magnet links and .torrent files can be added"
        )
    );
    assert!(!window.dialog_open(cx));

    gpui_kit::VisualTestContext::from_window(window.handle, cx).deactivate_window();
    cx.run_until_parked();
    window.request(AddRequest::Raise, cx);
    assert!(window.act(cx, |window, _| window.is_window_active()));
}

#[gpui_kit::test]
fn a_full_queue_says_how_many_were_left_out(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    let items: Vec<AddItem> = (0..105_u8)
        .map(|n| AddItem::File(window.dir.path().join(format!("{n}.torrent"))))
        .collect();
    let good = window.file("0.torrent", &show());
    assert!(good.exists());

    window.request(AddRequest::Items(items), cx);

    assert_eq!(
        window.label(TITLE, cx).as_deref(),
        Some("Add torrent (1 of 100)")
    );
    let notices = window.notifications(cx);
    assert_eq!(notices, 1, "only the overflow; the first file was read");
    assert_eq!(
        window.last_notice(cx).as_deref(),
        Some("5 more torrents were left out: at most 100 wait to be added")
    );
}

#[gpui_kit::test]
fn a_torrent_without_a_choice_says_why(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    // The keys of a file out of order: the daemon could read the list otherwise, so no choice.
    let sorted = folder_torrent("Twins", &[(&["a"], MIB), (&["b"], MIB)]);
    let ambiguous = replace(
        &sorted,
        b"d6:lengthi1048576e4:pathl1:aee",
        b"d4:pathl1:ae6:lengthi1048576ee",
    );
    window.request_torrent("twins.torrent", &ambiguous, cx);

    assert!(!window.shown(FILES, cx));
    assert_eq!(
        window.label(NO_CHOICE, cx).as_deref(),
        Some("All files are added: the list of files could be read in more than one way")
    );
    assert!(!window.disabled(CONFIRM, cx));
    window.click(CONFIRM, cx);
    assert_eq!(window.sent_add().1.files_unwanted, [] as [usize; 0]);
}

#[gpui_kit::test]
fn the_dialog_keeps_the_focus_held_key_from_the_table(cx: &mut TestAppContext) {
    let window = Window::open(cx);
    window.request_torrent("show.torrent", &show(), cx);
    let focus = cx.update(|cx| window.view.read(cx).table().focus_handle(cx));
    assert!(!window.act(cx, |window, _| focus.is_focused(window)));
    window.press("escape", cx);
    assert!(
        window.act(cx, |window, _| focus.is_focused(window)),
        "the table has it back"
    );
}
