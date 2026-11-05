//! The Connection dialog in the main window: what opens it and what does not, what its Save and
//! Cancel do, and the torrents to add that wait for it. A fake connector hands out channels the
//! test holds, standing in for the service threads, and the config file lives in a temporary
//! directory.

use std::cell::{Cell, Ref, RefCell};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::WindowExt;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, App, Task, TestAppContext};
use x4_core::test_support::torrent;
use x4_core::{AddItem, CheckOutcome, Command, ConnectionState, Event, Snapshot, Torrent, UiState};
use x4_rpc::Credentials;

use super::*;
use crate::adding::{AddRequest, FilePicker, Picked};
use crate::app::{self, ConfigProblem};
use crate::connection::{ConnectionSetup, Connector, ServerCheck, ServiceChannels};
use crate::files::read_config;
use crate::test_log::{Record, records_of};

/// A made-up secret, which must never show.
const PASSWORD: &str = "hunter2";

const TITLE: &str = "connection-title";
const HOST: &str = "connection-host";
const USERNAME: &str = "connection-username";
const PASSWORD_FIELD: &str = "connection-password";
const ADDRESS: &str = "connection-address";
const SAVE: &str = "connection-save";
const CANCEL: &str = "connection-cancel";
const SAVE_ERROR: &str = "connection-save-error";
const TOOLBAR_BUTTON: &str = "toolbar-connection";
const INDICATOR: &str = "status-connection";
const ADD_TITLE: &str = "add-title";
const REMOVAL_TITLE: &str = "removal-title";

/// The test's ends of the channels of one service.
struct FakeService {
    events: async_channel::Sender<Event>,
    commands: crossbeam_channel::Receiver<Command>,
}

impl FakeService {
    /// Sends `event` as the service would, and lets the window take it.
    fn send(&self, event: Event, cx: &mut TestAppContext) {
        self.events
            .send_blocking(event)
            .expect("the window listens");
        cx.run_until_parked();
    }

    /// The commands the window has sent since the last call.
    fn sent(&self) -> Vec<Command> {
        self.commands.try_iter().collect()
    }
}

/// The services the fake connector started, oldest first, with the servers they were for.
type Started = Rc<RefCell<Vec<(ConnectionSettings, FakeService)>>>;

/// A connector that starts no thread: it keeps the far ends of the channels in `started`.
fn fake_connector(started: &Started) -> Connector {
    let started = Rc::clone(started);
    Rc::new(move |settings| {
        let (events_tx, events) = async_channel::unbounded();
        let (commands, commands_rx) = crossbeam_channel::unbounded();
        started.borrow_mut().push((
            settings.clone(),
            FakeService {
                events: events_tx,
                commands: commands_rx,
            },
        ));
        Ok(ServiceChannels {
            events,
            commands,
            handle: None,
        })
    })
}

/// A check that answers at once that the test could not run.
fn idle_check() -> ServerCheck {
    Rc::new(|_, _| Task::ready(CheckOutcome::NotRun))
}

/// A file dialog that counts how often it was shown, and is cancelled at once.
struct CountingPicker(Rc<Cell<usize>>);

impl FilePicker for CountingPicker {
    fn pick(&self, _: &Window, _: &App) -> Task<Picked> {
        self.0.set(self.0.get() + 1);
        Task::ready(Ok(None))
    }
}

fn settings(url: &str, credentials: Option<Credentials>) -> ConnectionSettings {
    ConnectionSettings::from_url(url, credentials).expect("a plain http address")
}

/// The server the window starts with.
fn pi() -> ConnectionSettings {
    settings("http://pi:9091/transmission/rpc", None)
}

/// The server it is set to in the dialog, with a login.
fn nas() -> ConnectionSettings {
    settings(
        "http://nas:9091/transmission/rpc",
        Some(Credentials {
            username: "admin".to_owned(),
            password: PASSWORD.to_owned(),
        }),
    )
}

/// A snapshot of `torrents` with the daemon connected.
fn snapshot(torrents: Vec<Torrent>) -> Event {
    Event::Snapshot(Arc::new(Snapshot {
        torrents,
        listed: true,
        session: None,
        free_space_bytes: None,
        rate_down: 0,
        rate_up: 0,
        connection: ConnectionState::Connected,
        stale: false,
    }))
}

/// Fails if any of `records` holds the made-up password, in its message or a field.
fn assert_no_password(records: &[Record]) {
    for record in records {
        assert!(!record.message.contains(PASSWORD), "{record:?}");
        assert!(
            record
                .fields
                .iter()
                .all(|(_, value)| !value.contains(PASSWORD)),
            "{record:?}"
        );
    }
}

/// An active main window over a first service whose channels the test holds, with a file dialog
/// that counts its showings.
struct Main {
    handle: AnyWindowHandle,
    view: Entity<MainView>,
    first: FakeService,
    started: Started,
    picks: Rc<Cell<usize>>,
    /// Where the dialog saves, in `_dir`; there is no file at first.
    config: PathBuf,
    _dir: tempfile::TempDir,
}

impl Main {
    /// The window set to [`pi`], which switches with a fake connector, tests with a check that
    /// does not run, and saves to a `config.toml` in a temporary directory.
    fn open(cx: &mut TestAppContext) -> Self {
        Self::open_with(
            |started, config| ConnectionSetup {
                settings: Some(pi()),
                connector: Some(fake_connector(started)),
                check: Some(idle_check()),
                config_file: Some(config.clone()),
            },
            cx,
        )
    }

    /// The window with the connection `setup` makes from the fake connector's record and the
    /// config file's place.
    fn open_with(
        setup: impl FnOnce(&Started, &PathBuf) -> ConnectionSetup,
        cx: &mut TestAppContext,
    ) -> Self {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let config = dir.path().join("config").join("config.toml");
        let (events, events_rx) = async_channel::unbounded();
        let (commands, commands_rx) = crossbeam_channel::unbounded();
        // A dialog taken down goes at once.
        cx.update(|cx| cx.set_reduce_motion(true));
        let (handle, view) = cx
            .update(|cx| {
                app::init(cx);
                app::open_main_window(events_rx, commands, None, &UiState::default(), cx)
            })
            .expect("the window opens");
        let started = Started::default();
        let picks = Rc::new(Cell::new(0));
        let setup = setup(&started, &config);
        let picker = Rc::new(CountingPicker(Rc::clone(&picks)));
        cx.update(|cx| {
            view.update(cx, |view, cx| {
                view.set_connection(setup, cx);
                view.set_file_picker(picker);
                view.read_adds_on_the_executor();
            });
        });
        cx.update_window(handle, |_, window, _| window.activate_window())
            .expect("the window is open");
        cx.run_until_parked();
        Self {
            handle,
            view,
            first: FakeService {
                events,
                commands: commands_rx,
            },
            started,
            picks,
            config,
            _dir: dir,
        }
    }

    /// Runs `act` on the view in its window, after drawing a frame, then lets what it started
    /// finish.
    fn act<R>(
        &self,
        cx: &mut TestAppContext,
        act: impl FnOnce(&mut MainView, &mut Window, &mut Context<'_, MainView>) -> R,
    ) -> R {
        let result = cx
            .update_window(self.handle, |_, window, cx| {
                window.render_frame(cx);
                self.view.update(cx, |view, cx| act(view, window, cx))
            })
            .expect("the window is open");
        cx.run_until_parked();
        result
    }

    /// Runs `act` on the window, after drawing a frame, then lets what it started finish.
    fn on_window<R>(
        &self,
        cx: &mut TestAppContext,
        act: impl FnOnce(&mut Window, &mut App) -> R,
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

    fn press(&self, key: &str, cx: &mut TestAppContext) {
        self.on_window(cx, |window, cx| window.press(key, cx));
    }

    fn click(&self, id: &'static str, cx: &mut TestAppContext) {
        self.on_window(cx, |window, cx| window.click(id, cx));
    }

    /// Replaces the text of the field `id` with `text`, as the user would.
    fn retype(&self, id: &'static str, text: &str, cx: &mut TestAppContext) {
        self.click(id, cx);
        self.press("ctrl-a", cx);
        self.on_window(cx, |window, cx| window.input(text, cx));
    }

    /// What the element `id` says; `None` when it is not on screen.
    fn label(&self, id: &'static str, cx: &mut TestAppContext) -> Option<String> {
        self.on_window(cx, |window, _| {
            window
                .try_find(id)
                .and_then(|found| found.label().map(str::to_owned))
        })
    }

    /// Whether the window shows the element `id`.
    fn shows(&self, id: &'static str, cx: &mut TestAppContext) -> bool {
        self.on_window(cx, |window, _| window.try_find(id).is_some())
    }

    /// How many elements `id` the window shows.
    fn count(&self, id: &'static str, cx: &mut TestAppContext) -> usize {
        self.on_window(cx, |window, _| window.find_all(id).len())
    }

    fn focused(&self, id: &'static str, cx: &mut TestAppContext) -> bool {
        self.on_window(cx, |window, _| window.find(id).focused() == Some(true))
    }

    /// Whether the Connection dialog is open.
    fn connection_open(&self, cx: &mut TestAppContext) -> bool {
        self.label(TITLE, cx).is_some()
    }

    fn dialog_open(&self, cx: &mut TestAppContext) -> bool {
        self.on_window(cx, WindowExt::has_active_dialog)
    }

    /// The server the window is set to.
    fn server(&self, cx: &mut TestAppContext) -> Option<ConnectionSettings> {
        cx.update(|cx| self.view.read(cx).connection_settings().cloned())
    }

    /// The service the connector started last.
    fn newest(&self) -> Ref<'_, FakeService> {
        Ref::map(self.started.borrow(), |started| {
            &started.last().expect("a service was started").1
        })
    }

    /// The servers the connector was asked for, oldest first.
    fn switched_to(&self) -> Vec<ConnectionSettings> {
        self.started
            .borrow()
            .iter()
            .map(|(settings, _)| settings.clone())
            .collect()
    }

    /// The names of the rows the table shows, in order.
    fn names(&self, cx: &mut TestAppContext) -> Vec<String> {
        cx.update(|cx| {
            self.view
                .read(cx)
                .table
                .read(cx)
                .delegate()
                .rows()
                .iter()
                .map(|row| row.name.clone())
                .collect()
        })
    }

    /// Shows `name` as the one torrent of a connected daemon, and selects it with a click, which
    /// gives the list the keys.
    fn select_one(&self, name: &str, cx: &mut TestAppContext) {
        self.first.send(snapshot(vec![torrent(name)]), cx);
        self.on_window(cx, |window, cx| window.click(("row", 0_usize), cx));
    }

    /// Clicks Save in the Connection dialog, and gives what that logged.
    fn save(&self, cx: &mut TestAppContext) -> Vec<Record> {
        records_of(|| self.click(SAVE, cx))
    }

    /// Asks to add a magnet link to be typed in, as Ctrl+U does with an empty clipboard.
    fn ask_for_a_magnet(&self, cx: &mut TestAppContext) {
        let requests = cx.update(|cx| self.view.read(cx).add_requests().clone());
        requests
            .send_blocking(AddRequest::Items(vec![AddItem::Magnet(None)]))
            .expect("the window listens");
        cx.run_until_parked();
    }
}

#[gpui_kit::test]
fn ctrl_comma_opens_the_dialog(cx: &mut TestAppContext) {
    let main = Main::open(cx);
    assert!(!main.connection_open(cx));

    main.press("ctrl-,", cx);

    assert_eq!(main.label(TITLE, cx).as_deref(), Some("Connection"));
    assert_eq!(
        main.label(ADDRESS, cx).as_deref(),
        Some("Address: http://pi:9091/transmission/rpc"),
        "the server the window is set to"
    );
    assert!(main.focused(HOST, cx));

    // Escape closes it; from the search box the key works too.
    main.press("escape", cx);
    assert!(!main.dialog_open(cx));
    main.press("ctrl-f", cx);
    main.press("ctrl-,", cx);
    assert!(main.connection_open(cx));
    assert_eq!(main.switched_to(), [], "nothing changed");
}

#[gpui_kit::test]
fn the_toolbar_button_opens_it(cx: &mut TestAppContext) {
    let main = Main::open(cx);
    assert_eq!(
        main.label(TOOLBAR_BUTTON, cx).as_deref(),
        Some("Connection")
    );

    main.click(TOOLBAR_BUTTON, cx);

    assert!(main.connection_open(cx));
    assert!(main.focused(HOST, cx));
    main.click(CANCEL, cx);
    assert!(!main.dialog_open(cx));
    assert_eq!(main.server(cx), Some(pi()), "Cancel changes nothing");
}

#[gpui_kit::test]
fn a_click_on_the_connection_indicator_opens_it(cx: &mut TestAppContext) {
    let main = Main::open(cx);
    main.first.send(snapshot(Vec::new()), cx);
    // The indicator says what it said before it could be clicked.
    assert_eq!(
        main.label(INDICATOR, cx).as_deref(),
        Some("Connected to pi:9091")
    );

    main.click(INDICATOR, cx);

    assert!(main.connection_open(cx));
    assert_eq!(
        main.label(INDICATOR, cx).as_deref(),
        Some("Connected to pi:9091")
    );
}

#[gpui_kit::test]
fn it_does_not_open_over_another_dialog(cx: &mut TestAppContext) {
    let main = Main::open(cx);
    main.select_one("film", cx);
    main.press("delete", cx);
    assert!(main.shows(REMOVAL_TITLE, cx), "the removal asks");

    main.press("ctrl-,", cx);
    // The toolbar and the status bar lie under the dialog's overlay; were they reached, they would
    // ask the view the same.
    main.act(cx, MainView::open_connection);

    assert!(!main.connection_open(cx));
    assert!(main.shows(REMOVAL_TITLE, cx), "the removal still asks");
    assert_eq!(main.count("dialog", cx), 1, "one dialog");

    // Nor over the add dialog.
    main.press("escape", cx);
    assert!(!main.dialog_open(cx));
    main.ask_for_a_magnet(cx);
    assert!(main.label(ADD_TITLE, cx).is_some());
    main.act(cx, MainView::open_connection);
    assert!(!main.connection_open(cx));
    assert!(main.label(ADD_TITLE, cx).is_some());
}

#[gpui_kit::test]
fn save_switches_and_writes_the_file(cx: &mut TestAppContext) {
    let main = Main::open(cx);
    main.first.send(snapshot(vec![torrent("old")]), cx);
    main.act(cx, |view, _, cx| {
        view.show_config_problem(ConfigProblem::UnsupportedUrl, cx);
    });
    assert!(main.label("config-problem", cx).is_some());
    main.press("ctrl-,", cx);
    main.retype(HOST, "nas", cx);
    main.retype(USERNAME, "admin", cx);
    main.retype(PASSWORD_FIELD, PASSWORD, cx);

    let records = main.save(cx);

    assert_no_password(&records);
    assert!(
        records
            .iter()
            .any(|record| record.message == "connection saved"),
        "{records:?}"
    );
    assert!(!main.dialog_open(cx), "the dialog closed");
    let written = read_config(&main.config).expect("a file 4x4 can read");
    assert_eq!(
        written.connection,
        Some(nas()),
        "the file holds the new server"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(&main.config)
            .expect("written")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "private to the owner");
    }
    assert_eq!(main.switched_to(), [nas()], "one switch, to the new server");
    assert_eq!(main.server(cx), Some(nas()));
    assert_eq!(main.label("config-problem", cx), None, "the notice is gone");
    assert!(
        main.first.events.is_closed(),
        "the old service no longer reaches the view"
    );
    assert_eq!(
        main.names(cx),
        [] as [String; 0],
        "the old daemon's rows went"
    );
    assert!(
        main.newest()
            .sent()
            .iter()
            .any(|command| matches!(command, Command::SetVisibility(_))),
        "the new service is told what a service is told at the start"
    );
    main.newest().send(snapshot(vec![torrent("new")]), cx);
    assert_eq!(main.names(cx), ["new"], "the new channels are in use");
    assert_eq!(
        main.label(INDICATOR, cx).as_deref(),
        Some("Connected to nas:9091")
    );

    // The dialog opens again on the new server.
    main.press("ctrl-,", cx);
    assert_eq!(
        main.label(ADDRESS, cx).as_deref(),
        Some("Address: http://nas:9091/transmission/rpc")
    );
}

#[gpui_kit::test]
fn saving_the_same_settings_only_closes(cx: &mut TestAppContext) {
    let main = Main::open(cx);
    main.first.send(snapshot(vec![torrent("film")]), cx);
    main.press("ctrl-,", cx);

    main.click(SAVE, cx);

    assert!(!main.dialog_open(cx));
    assert_eq!(main.switched_to(), [], "the connector was not called");
    assert!(!main.config.exists(), "no file was written");
    assert!(
        main.config.parent().is_some_and(|dir| !dir.exists()),
        "nor its directory"
    );
    assert_eq!(main.server(cx), Some(pi()));
    main.first.send(snapshot(vec![torrent("still")]), cx);
    assert_eq!(
        main.names(cx),
        ["still"],
        "the first service still reaches it"
    );
}

#[gpui_kit::test]
fn a_failed_write_keeps_the_dialog_open(cx: &mut TestAppContext) {
    let main = Main::open(cx);
    // A directory where the file should be: it can be neither read nor replaced.
    fs::create_dir_all(&main.config).expect("a directory");
    main.press("ctrl-,", cx);
    main.retype(HOST, "nas", cx);
    main.retype(USERNAME, "admin", cx);
    main.retype(PASSWORD_FIELD, PASSWORD, cx);

    let records = main.save(cx);

    assert_no_password(&records);
    assert!(
        records
            .iter()
            .any(|record| record.level == tracing::Level::ERROR),
        "the log says the save failed: {records:?}"
    );
    assert!(main.connection_open(cx), "the dialog stays open");
    let error = main.label(SAVE_ERROR, cx).expect("the dialog says why");
    // The messages of the language, with the path and the system's own words.
    assert!(
        error.starts_with(&format!(
            "The settings could not be saved: {} cannot be read (",
            main.config.display()
        )),
        "{error}"
    );
    assert!(error.ends_with(')'), "{error}");
    assert!(
        !error.contains("cannot read"),
        "not the log's report: {error}"
    );
    assert!(!error.contains(PASSWORD), "{error}");
    assert_eq!(main.switched_to(), [], "nothing switched");
    assert_eq!(main.server(cx), Some(pi()));
    assert!(main.config.is_dir(), "the file system is as it was");

    // Cancel then changes nothing either.
    main.click(CANCEL, cx);
    assert!(!main.dialog_open(cx));
    assert_eq!(main.server(cx), Some(pi()));
}

#[gpui_kit::test]
fn save_without_a_config_file_switches_without_writing(cx: &mut TestAppContext) {
    let main = Main::open_with(
        |started, _| ConnectionSetup {
            settings: Some(pi()),
            connector: Some(fake_connector(started)),
            ..ConnectionSetup::default()
        },
        cx,
    );
    main.press("ctrl-,", cx);
    main.retype(HOST, "nas", cx);

    main.click(SAVE, cx);

    assert!(!main.dialog_open(cx));
    let nas = settings("http://nas:9091/transmission/rpc", None);
    assert_eq!(main.switched_to(), std::slice::from_ref(&nas));
    assert_eq!(main.server(cx), Some(nas));
    assert!(!main.config.exists());
}

#[gpui_kit::test]
fn a_window_that_cannot_switch_closes_the_dialog_and_stays_as_it_is(cx: &mut TestAppContext) {
    let main = Main::open_with(|_, _| ConnectionSetup::default(), cx);
    main.first.send(snapshot(vec![torrent("film")]), cx);
    main.press("ctrl-,", cx);
    assert!(main.connection_open(cx), "it opens in any window");
    main.ask_for_a_magnet(cx);
    main.retype(HOST, "nas", cx);

    main.click(SAVE, cx);

    assert!(!main.connection_open(cx));
    assert_eq!(main.server(cx), None);
    assert_eq!(main.names(cx), ["film"], "the first service stays");
    assert!(
        main.label(ADD_TITLE, cx).is_some(),
        "the torrent that waited is offered"
    );
}

#[gpui_kit::test]
fn the_add_dialog_waits_for_the_connection_dialog(cx: &mut TestAppContext) {
    let main = Main::open(cx);
    main.press("ctrl-,", cx);

    main.ask_for_a_magnet(cx);

    assert!(main.connection_open(cx));
    assert_eq!(main.label(ADD_TITLE, cx), None, "the add waits");
    main.press("escape", cx);
    assert!(!main.connection_open(cx));
    assert_eq!(
        main.label(ADD_TITLE, cx).as_deref(),
        Some("Add torrent"),
        "shown once the dialog is cancelled"
    );

    // After a Save that switches, the switch offers it, once.
    main.press("escape", cx);
    assert!(!main.dialog_open(cx));
    main.press("ctrl-,", cx);
    main.ask_for_a_magnet(cx);
    assert_eq!(main.label(ADD_TITLE, cx), None);
    main.retype(HOST, "nas", cx);
    main.click(SAVE, cx);
    assert!(!main.connection_open(cx));
    assert_eq!(main.label(ADD_TITLE, cx).as_deref(), Some("Add torrent"));
    assert_eq!(main.count("add-dialog", cx), 1, "one add dialog");

    // After a Save that only closes, too.
    main.press("escape", cx);
    main.press("ctrl-,", cx);
    main.ask_for_a_magnet(cx);
    main.click(SAVE, cx);
    assert!(!main.connection_open(cx));
    assert_eq!(main.label(ADD_TITLE, cx).as_deref(), Some("Add torrent"));
}

/// The dialog keeps the keys of the main window: Ctrl+O and Delete, which act in the window, do
/// nothing while it is open, wherever its focus is.
#[gpui_kit::test]
fn the_dialog_keeps_the_main_windows_keys(cx: &mut TestAppContext) {
    let main = Main::open(cx);
    main.select_one("film", cx);
    // Outside the dialog, both act.
    main.press("ctrl-o", cx);
    assert_eq!(main.picks.get(), 1, "Ctrl+O shows the file dialog");
    main.press("delete", cx);
    assert!(main.shows(REMOVAL_TITLE, cx), "Delete asks");
    main.press("escape", cx);
    assert!(!main.dialog_open(cx));

    main.press("ctrl-,", cx);
    assert!(main.focused(HOST, cx));
    main.press("ctrl-o", cx);
    main.press("delete", cx);
    // On a button of the dialog, where no field takes Delete.
    for _ in 0..8 {
        if main.focused(CANCEL, cx) {
            break;
        }
        main.press("shift-tab", cx);
    }
    assert!(main.focused(CANCEL, cx));
    main.press("delete", cx);
    main.press("ctrl-o", cx);

    assert_eq!(main.picks.get(), 1, "no file dialog");
    assert!(!main.shows(REMOVAL_TITLE, cx), "no removal");
    assert!(main.connection_open(cx));
    assert_eq!(main.count("dialog", cx), 1);
}
