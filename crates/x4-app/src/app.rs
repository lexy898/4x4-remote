//! Application startup: the config and the saved state, the service, the global Quit action and
//! the key bindings of [`keymap`](crate::keymap), the main window and the first-frame log.
//!
//! [`run`] is the front door the `4x4` binary uses: it reads the files, starts the service
//! against the server `config.toml` sets (none is built in: without one, no service starts and the
//! window says that no server is set), queues the requests of the binary's own arguments and, in
//! the first 4x4 of the user, answers the launches after it ([`instance`](crate::instance)).
//! [`run_with`] takes a [`Startup`] built by its caller instead, so that a demo or a measurement
//! can run the window against a service of its own, and [`run_driven`] also hands it the open
//! window to drive. [`launch`] opens the window of a [`Startup`] in an application that runs
//! already, such as a test's.

use std::cell::RefCell;
use std::env;
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use anyhow::Context as _;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Entity, QuitMode, TitlebarOptions,
    WindowOptions, px, size,
};
use x4_core::service::ServiceHandle;
use x4_core::{
    Command, Config, ConfigError, ConnectionSettings, Event, MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH,
    UiState,
};

use crate::PRODUCT_NAME;
use crate::adding::{AddRequest, AddRequests};
use crate::connection::{
    ConnectionSetup, ServiceChannels, real_connector, real_server_check, spawn_service,
};
use crate::files::{FilesError, read_config, read_ui_state};
use crate::icons::AppAssets;
use crate::instance::{Listener, Serving};
use crate::keymap::{Quit, key_bindings};
use crate::main_view::MainView;
use crate::paths::AppPaths;
use crate::theme::apply_theme;
use crate::window_state::opening_bounds;

/// Identifies the application to the desktop environment: the Wayland `app_id`, the X11
/// `WM_CLASS`, the name of the `.desktop` file, and the names of the single instance's lock and
/// socket.
pub const APP_ID: &str = "io.github.lexy898.FourByFour";
/// The id of development runs, so that window rules written for test windows (placing them on
/// a particular screen without focus) never catch the owner's real window, and so that a
/// development run never hands its arguments to the owner's 4x4.
pub const DEVEL_APP_ID: &str = "io.github.lexy898.FourByFour.Devel";
/// Set to `1` to give a release build the development id, as `just measure` does.
const DEVEL_ENV: &str = "X4_DEVEL";

/// Everything the app starts with, as plain values: the service, how the window reaches other
/// servers, the saved UI state and what went wrong while reading the files.
///
/// [`Startup::from_state_file`] reads the UI state the way [`run`] does.
#[derive(Debug)]
pub struct Startup {
    /// The service thread the window shows, why it could not be started, or that no server is
    /// set. Without a service the window still opens and says why.
    pub service: ServiceStart,
    /// The server the window is set to, and how it switches to another one, checks one and saves
    /// the choice. The default cannot switch.
    pub connection: ConnectionSetup,
    /// The UI state to start from: the saved one, or the defaults.
    pub ui: UiState,
    /// Whether the UI state may be saved over `state_path`. It is not when the file there could
    /// not be read, is from a newer version or could not be moved out of the way.
    pub state_writable: bool,
    /// Where the UI state is saved; `None` means it is never saved.
    pub state_path: Option<PathBuf>,
    /// Why `config.toml` is not used, if it is not. The window shows it until the user
    /// dismisses it.
    pub config_problem: Option<ConfigProblem>,
    /// The channel of the requests to add torrents. The window listens on its receiver and keeps
    /// its sender for the sources inside it
    /// ([`MainView::add_requests`](crate::main_view::MainView::add_requests)); the sources outside
    /// it, such as the launch arguments and later launches, send into clones of its sender, taken
    /// before the start.
    pub adds: AddRequests,
}

impl Startup {
    /// A start with `service`, the UI state saved at `state_path`, no config problem and a
    /// window that cannot switch servers ([`ConnectionSetup::default`]).
    ///
    /// The file is read as [`run`] reads it: a missing file gives the defaults, which are saved
    /// there once the user changes something; a corrupt one is moved aside first. A file that
    /// cannot be read, or that a newer version wrote, gives the defaults, and nothing is saved over
    /// it. With no `state_path`, the state is the defaults and is never saved.
    #[must_use]
    pub fn from_state_file(service: ServiceStart, state_path: Option<PathBuf>) -> Self {
        let (ui, state_writable) = state_path
            .as_deref()
            .map_or_else(|| (UiState::default(), false), load_ui_state);
        Self {
            service,
            connection: ConnectionSetup::default(),
            ui,
            state_writable,
            state_path,
            config_problem: None,
            adds: AddRequests::new(),
        }
    }

    /// Where the window saves its state: `state_path`, if it may be saved there.
    fn state_file(&self) -> Option<PathBuf> {
        match (&self.state_path, self.state_writable) {
            (Some(path), true) => Some(path.clone()),
            (Some(path), false) => {
                tracing::info!(path = %path.display(), "the UI state file is left as it is; changes in this session are not saved");
                None
            }
            (None, _) => {
                tracing::info!(
                    "there is no place for the UI state; changes in this session are not saved"
                );
                None
            }
        }
    }
}

/// How the service of a start came out.
#[derive(Debug)]
pub enum ServiceStart {
    /// The service thread runs against the server set.
    Started(ServiceHandle),
    /// The service thread could not be started: the window says so, and no data will come.
    Failed(io::Error),
    /// No server is set, so no service was started: the window says so, and the server can be
    /// set in the Connection dialog.
    NotConfigured,
}

impl From<io::Result<ServiceHandle>> for ServiceStart {
    /// A started service, or why it could not be started.
    fn from(started: io::Result<ServiceHandle>) -> Self {
        started.map_or_else(Self::Failed, Self::Started)
    }
}

/// Why `config.toml` is not used, as the window tells the user (in their language). The log
/// gets the full report, with the path and the parser's message. Such a file sets no server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigProblem {
    /// The file is there but cannot be read.
    Unreadable,
    /// The file is not valid TOML, or a value has the wrong type.
    Invalid {
        /// The line of the error, from 1.
        line: u32,
        /// The column of the error, from 1, in characters.
        column: u32,
    },
    /// The file is not valid, and the parser cannot say where.
    Malformed,
    /// `connection.url` cannot be used: it is not a plain `http://` address, or it holds the user
    /// name or password that belong in their own keys; or `connection.username` holds a `:`. The
    /// log says which.
    UnsupportedUrl,
}

impl ConfigProblem {
    /// The problem behind an error of [`read_config`].
    fn of(error: &FilesError) -> Self {
        match error {
            FilesError::Config {
                source: ConfigError::Parse { position, .. },
                ..
            } => position.map_or(Self::Malformed, |position| Self::Invalid {
                line: position.line,
                column: position.column,
            }),
            FilesError::Config {
                source: ConfigError::InvalidUrl(_),
                ..
            } => Self::UnsupportedUrl,
            // Reading gives only `Read`; the others come from writing.
            FilesError::Read { .. }
            | FilesError::CreateDir { .. }
            | FilesError::Serialize { .. }
            | FilesError::MoveAside { .. }
            | FilesError::Write { .. } => Self::Unreadable,
        }
    }
}

/// Runs the application until it quits: reads the config and the saved state from `paths`
/// (`None` when the home directory is unknown), starts the service against the server the config
/// sets (if it sets one), and runs the window as [`run_with`] does, with `requests`,
/// those of the binary's own arguments, waiting for it. With `instance`, the place of the first
/// 4x4 of the user, it answers the launches after it from the start: their requests join the same
/// channel, after `requests`. When it quits, it removes the socket of `instance` and then lets go
/// of its lock.
///
/// The service only starts a thread here; the window opens at once and shows the connection
/// being made, so the first frame never waits for the daemon.
///
/// # Errors
///
/// Returns an error if the main window cannot be opened.
pub fn run(
    started: Instant,
    paths: Option<&AppPaths>,
    requests: Vec<AddRequest>,
    instance: Option<Listener>,
) -> anyhow::Result<()> {
    let startup = startup_from_files(paths);
    // The channel keeps them until the window listens.
    for request in requests {
        if startup.adds.sender.try_send(request).is_err() {
            tracing::error!("the window's channel closed before it opened; an argument is lost");
        }
    }
    run_app(started, startup, instance, |_, _, _| {})
}

/// The start [`run`] makes from the files of `paths` (`None` when the home directory is unknown):
/// the server `config.toml` sets and its service, the saved UI state, the app's own connector and
/// check, and `config.toml` as the place the Connection dialog saves to.
fn startup_from_files(paths: Option<&AppPaths>) -> Startup {
    let (config, config_problem) = paths.map_or_else(
        || (Config::default(), None),
        |paths| load_config(&paths.config_file),
    );
    let settings = config.connection;
    // Spawning only starts the thread; the state file is read while it connects.
    let service = start_service(settings.as_ref());
    Startup {
        config_problem,
        connection: ConnectionSetup {
            settings,
            connector: Some(real_connector()),
            check: Some(real_server_check()),
            config_file: paths.map(|paths| paths.config_file.clone()),
        },
        ..Startup::from_state_file(service, paths.map(|paths| paths.state_file.clone()))
    }
}

/// Starts the service against `connection`. Without one no thread starts: no server is set.
fn start_service(connection: Option<&ConnectionSettings>) -> ServiceStart {
    let Some(settings) = connection else {
        tracing::info!("no server set; no service started");
        return ServiceStart::NotConfigured;
    };
    spawn_service(settings).into()
}

/// Runs the application until it quits with what `startup` holds: initializes the UI, opens the
/// main window and logs `first frame rendered` with the time elapsed since `started`.
///
/// # Errors
///
/// Returns an error if the main window cannot be opened.
pub fn run_with(started: Instant, startup: Startup) -> anyhow::Result<()> {
    run_driven(started, startup, |_, _, _| {})
}

/// [`run_with`], which also hands the main window and its view to `drive` once the window is
/// open, before its first frame, so that a demo or a measurement can drive the real window.
///
/// On Linux the window asks for `.torrent` files through the desktop's own file dialog, with a
/// filter for them (the portal's); [`launch`] alone leaves it gpui's, which a test answers.
///
/// # Errors
///
/// Returns an error if the main window cannot be opened.
pub fn run_driven(
    started: Instant,
    startup: Startup,
    drive: impl FnOnce(AnyWindowHandle, Entity<MainView>, &mut App) + 'static,
) -> anyhow::Result<()> {
    run_app(started, startup, None, drive)
}

/// [`run_driven`], which also answers the launches after this one with `instance`, the place of
/// the first 4x4 of the user, until the application quits.
fn run_app(
    started: Instant,
    startup: Startup,
    instance: Option<Listener>,
    drive: impl FnOnce(AnyWindowHandle, Entity<MainView>, &mut App) + 'static,
) -> anyhow::Result<()> {
    let launch_error = Rc::new(RefCell::new(None));
    let launch_error_slot = Rc::clone(&launch_error);
    let serving = Rc::new(RefCell::new(None));
    let serving_slot = Rc::clone(&serving);
    // The component icons (the sidebar's, the search box's, the toolbar's, the menus' checks) and
    // the few the app adds; not the whole catalog.
    gpui_kit::application()
        .with_assets(AppAssets)
        .run(move |cx| {
            init(cx);
            // Only now that the platform exists: creating it took the activation token out of
            // the environment, which is safe only while no other thread may read it.
            if let Some(listener) = instance {
                serving_slot.replace(serve_later_launches(listener, &startup.adds));
            }
            let opened = launch(startup, cx).and_then(|(window, view)| {
                use_desktop_file_dialog(&view, cx);
                log_first_frame(window, started, cx)?;
                drive(window, view, cx);
                Ok(())
            });
            if let Err(error) = opened {
                launch_error_slot.replace(Some(error));
                // A quit requested during launch is lost: the event loop clears its stop signal
                // when it starts, so quit from inside the loop instead.
                cx.spawn(async |cx| cx.update(|cx| cx.quit())).detach();
            }
        });
    // Removes the socket, then lets go of the lock, so that the next launch takes over at once.
    drop(serving.take());
    launch_error.take().map_or(Ok(()), Err)
}

/// Has `listener` answer the launches after this one, which send their requests into `adds`.
/// If it cannot, they run on their own.
fn serve_later_launches(listener: Listener, adds: &AddRequests) -> Option<Serving> {
    listener
        .serve(adds.sender.clone())
        .inspect_err(|error| {
            tracing::warn!(%error, "later launches cannot hand over to this 4x4 and run on their own");
        })
        .ok()
}

/// Has `view` ask for `.torrent` files through the desktop's own file dialog, where 4x4 has one:
/// the `FileChooser` portal on Linux, which filters them and has a title of 4x4's. Elsewhere gpui's
/// dialog stays.
fn use_desktop_file_dialog(view: &Entity<MainView>, cx: &mut App) {
    #[cfg(target_os = "linux")]
    view.update(cx, |view, _| {
        view.set_file_picker(Rc::new(crate::adding::PortalPicker));
    });
    #[cfg(not(target_os = "linux"))]
    let _ = (view, cx);
}

/// Reads `config.toml`. A file that cannot be used gives the defaults, which set no server, and
/// the problem for the window; the full report goes to the log, which never shows the URL or
/// the password.
fn load_config(path: &Path) -> (Config, Option<ConfigProblem>) {
    match read_config(path) {
        Ok(config) => {
            tracing::info!(path = %path.display(), "config loaded");
            (config, None)
        }
        Err(error) => {
            tracing::warn!(error = %error.report(), "config not loaded; no server is set");
            (Config::default(), Some(ConfigProblem::of(&error)))
        }
    }
}

/// Reads `state.toml`: the state to start from and whether it may be saved over the file. A
/// file that cannot be read gives the defaults, which are then never saved over it.
fn load_ui_state(path: &Path) -> (UiState, bool) {
    match read_ui_state(path) {
        Ok(loaded) => {
            tracing::info!(path = %path.display(), writable = loaded.writable, "UI state loaded");
            (loaded.state, loaded.writable)
        }
        Err(error) => {
            tracing::warn!(error = %error.report(), "UI state not loaded; using the defaults and not saving");
            (UiState::default(), false)
        }
    }
}

/// Opens the main window for `startup`, at the remembered size and maximized if it was. A service
/// that could not start, no server set and a config problem are shown in the status bar. The
/// window saves its UI state to `startup.state_path` if it may, takes the requests to add torrents
/// that arrive on `startup.adds`, and reaches other servers as `startup.connection` says. Call
/// [`init`] first.
///
/// # Errors
///
/// Returns an error if the platform cannot create the window.
pub fn launch(
    startup: Startup,
    cx: &mut App,
) -> anyhow::Result<(AnyWindowHandle, Entity<MainView>)> {
    let state_file = startup.state_file();
    let Startup {
        service,
        connection,
        ui,
        state_writable: _,
        state_path: _,
        config_problem,
        adds,
    } = startup;
    let (channels, failed, not_configured) = match service {
        ServiceStart::Started(service) => (ServiceChannels::from(service), false, false),
        ServiceStart::Failed(error) => {
            tracing::error!(%error, "the service could not start; the window has no data to show");
            (ServiceChannels::disconnected(), true, false)
        }
        // Said in the log when it was decided; it is no error.
        ServiceStart::NotConfigured => (ServiceChannels::disconnected(), false, true),
    };
    let ServiceChannels {
        events,
        commands,
        handle,
    } = channels;
    let (window, view) = open_window(events, commands, handle, &ui, state_file, cx)?;
    window.update(cx, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.set_connection(connection, cx);
            if failed {
                view.service_failed(cx);
            }
            if not_configured {
                view.show_not_configured(cx);
            }
            if let Some(problem) = config_problem {
                view.show_config_problem(problem, cx);
            }
            view.listen_for_adds(adds, window, cx);
        });
    })?;
    Ok((window, view))
}

/// Initializes the UI libraries and registers the application-wide actions and the key bindings
/// of [`keymap`](crate::keymap).
///
/// The bindings come after the libraries' own, so that ours win where both bind a key in the same
/// context.
pub fn init(cx: &mut App) {
    gpui_kit::init(cx);
    cx.set_quit_mode(QuitMode::LastWindowClosed);
    cx.on_action(|_: &Quit, cx| cx.quit());
    let bindings = key_bindings(cx);
    cx.bind_keys(bindings);
}

/// Opens the main window, centered, at the size and in the theme `ui` asks for, with a
/// [`MainView`] of the service whose channels are `events` and `commands` as its content. The
/// view keeps `service` so that closing the window stops it; pass `None` when there is no thread
/// behind the channels. The window never saves its UI state; [`launch`] opens one that does.
/// Call [`init`] (or at least `gpui_kit::init`) first.
///
/// # Errors
///
/// Returns an error if the platform cannot create the window.
pub fn open_main_window(
    events: async_channel::Receiver<Event>,
    commands: crossbeam_channel::Sender<Command>,
    service: Option<ServiceHandle>,
    ui: &UiState,
    cx: &mut App,
) -> anyhow::Result<(AnyWindowHandle, Entity<MainView>)> {
    open_window(events, commands, service, ui, None, cx)
}

/// [`open_main_window`], whose view saves its state to `state_file` (none: never).
fn open_window(
    events: async_channel::Receiver<Event>,
    commands: crossbeam_channel::Sender<Command>,
    service: Option<ServiceHandle>,
    ui: &UiState,
    state_file: Option<PathBuf>,
    cx: &mut App,
) -> anyhow::Result<(AnyWindowHandle, Entity<MainView>)> {
    let options = main_window_options(ui, cx);
    gpui_kit::open_window(options, cx, |window, cx| {
        // Before the view exists, so that the first frame is drawn in the right theme.
        apply_theme(ui.theme, window, cx);
        let ui = ui.clone();
        cx.new(|cx| MainView::new(events, commands, service, ui, state_file, window, cx))
    })
    .context("failed to open the main window")
}

/// How the main window opens for the remembered state `ui`: centered on the primary display, at
/// the remembered size and maximized if it was, never smaller than core's minimum, with the
/// product name as its title and the app id of this run.
fn main_window_options(ui: &UiState, cx: &App) -> WindowOptions {
    let bounds = Bounds::centered(None, size(px(ui.window.width), px(ui.window.height)), cx);
    WindowOptions {
        window_bounds: Some(opening_bounds(ui.window, bounds)),
        // The lower bound of a remembered size. The compositor then never makes a size that
        // `WindowState::normalized` would reset; that bound only guards a hand-edited file.
        window_min_size: Some(size(px(MIN_WINDOW_WIDTH), px(MIN_WINDOW_HEIGHT))),
        titlebar: Some(TitlebarOptions {
            title: Some(PRODUCT_NAME.into()),
            ..Default::default()
        }),
        app_id: Some(app_id().to_owned()),
        ..Default::default()
    }
}

/// The Wayland `app_id` of this run: the development id in debug builds and when `X4_DEVEL=1`,
/// the real one otherwise. The single instance's lock and socket are named after it.
#[must_use]
pub fn app_id() -> &'static str {
    app_id_for(cfg!(debug_assertions), env::var_os(DEVEL_ENV).as_deref())
}

/// The id for a debug or release build and the value of `X4_DEVEL`, if it is set. Only the value
/// `1` asks for the development id.
fn app_id_for(debug_build: bool, devel_env: Option<&OsStr>) -> &'static str {
    if debug_build || devel_env == Some(OsStr::new("1")) {
        DEVEL_APP_ID
    } else {
        APP_ID
    }
}

fn log_first_frame(window: AnyWindowHandle, started: Instant, cx: &mut App) -> anyhow::Result<()> {
    window.update(cx, |_, window, _| {
        // Next-frame callbacks run when the platform asks for a frame, before it is drawn.
        // The first hop runs ahead of the first frame; the second runs once it has been
        // presented and the platform asks for the next one.
        window.on_next_frame(move |window, _| {
            window.on_next_frame(move |_, _| {
                let elapsed_ms = started.elapsed().as_millis();
                tracing::info!(elapsed_ms, "first frame rendered");
            });
        });
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Arc;

    use gpui_kit::TestAppContext;
    use gpui_kit::WindowBounds;
    use gpui_kit::test::TestWindowExt as _;
    use x4_core::test_support::torrent;
    use x4_core::{
        AddItem, ConnectionState, DisplayState, Filter, Snapshot, TextPosition, ThemeChoice,
        Torrent, WindowState,
    };

    use super::*;
    use crate::adding::AddRequest;
    use crate::test_log::{Record, records_of};

    /// A start whose service could not be started, with `config_problem`.
    fn failed_start(config_problem: Option<ConfigProblem>) -> Startup {
        Startup {
            service: ServiceStart::Failed(io::Error::other("no more threads")),
            connection: ConnectionSetup::default(),
            ui: UiState::default(),
            state_writable: false,
            state_path: None,
            config_problem,
            adds: AddRequests::new(),
        }
    }

    fn launched(startup: Startup, cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<MainView>) {
        let opened = cx
            .update(|cx| {
                init(cx);
                launch(startup, cx)
            })
            .expect("the window opens");
        cx.run_until_parked();
        opened
    }

    /// The problem `load_config` reports for a `config.toml` holding `text`.
    fn problem_with(text: &str) -> Option<ConfigProblem> {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("config.toml");
        fs::write(&path, text).expect("written");
        let (config, problem) = load_config(&path);
        if problem.is_some() {
            assert_eq!(
                config,
                Config::default(),
                "a broken file gives the defaults"
            );
        }
        problem
    }

    /// The places of the app's files in `dir`, none of which exists yet.
    fn paths_in(dir: &Path) -> AppPaths {
        AppPaths {
            config_file: dir.join("config").join("config.toml"),
            state_file: dir.join("state").join("state.toml"),
            log_dir: dir.join("logs"),
        }
    }

    /// The start [`run`] makes from the files of `paths`, and what it logged.
    fn start_from(paths: &AppPaths) -> (Startup, Vec<Record>) {
        let mut startup = None;
        let records = records_of(|| startup = Some(startup_from_files(Some(paths))));
        (startup.expect("a start"), records)
    }

    /// Whether `records` hold an event at `level` or worse.
    fn logs_at(level: tracing::Level, records: &[Record]) -> bool {
        records.iter().any(|record| record.level <= level)
    }

    #[test]
    fn no_server_set_starts_no_service() {
        assert!(matches!(start_service(None), ServiceStart::NotConfigured));
    }

    #[gpui_kit::test]
    fn no_server_set_starts_no_service_and_says_so(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let paths = paths_in(dir.path());

        let (startup, records) = start_from(&paths);

        assert!(matches!(startup.service, ServiceStart::NotConfigured));
        assert_eq!(startup.config_problem, None, "no file is no problem");
        assert_eq!(startup.connection.settings, None);
        assert!(startup.connection.connector.is_some(), "it can switch");
        assert!(startup.connection.check.is_some(), "and test a server");
        assert_eq!(
            startup.connection.config_file.as_deref(),
            Some(paths.config_file.as_path()),
            "the dialog saves where the config is read"
        );
        assert!(
            records
                .iter()
                .any(|record| record.message == "no server set; no service started"),
            "{records:?}"
        );
        assert!(
            !logs_at(tracing::Level::WARN, &records),
            "a first run is no problem: {records:?}"
        );

        let mut opened = None;
        let records = records_of(|| opened = Some(launched(startup, cx)));
        let (window, view) = opened.expect("the window opens");

        // The libraries may warn about the test machine's fonts; the window logs no error.
        assert!(!logs_at(tracing::Level::ERROR, &records), "{records:?}");
        let (text, service, stale, settings) = cx.update(|cx| {
            let view = view.read(cx);
            (
                view.connection_text(),
                view.status().service,
                view.status().stale,
                view.connection_settings().cloned(),
            )
        });
        assert_eq!(text, "No server set");
        assert_eq!(service, x4_core::ServiceStatus::NotConfigured);
        assert!(!stale, "there is no data to be old");
        assert_eq!(settings, None);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(window.find("connection").label(), Some("No server set"));
            assert!(window.try_find("config-problem").is_none());
        })
        .expect("the window is open");
    }

    #[gpui_kit::test]
    fn a_broken_config_sets_no_server_and_shows_its_notice(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let paths = paths_in(dir.path());
        let text = "[connection]\nurl = 5\n";
        fs::create_dir_all(paths.config_file.parent().expect("a folder")).expect("created");
        fs::write(&paths.config_file, text).expect("written");

        let (startup, _) = start_from(&paths);

        assert!(matches!(startup.service, ServiceStart::NotConfigured));
        assert_eq!(startup.connection.settings, None);
        assert_eq!(
            startup.config_problem,
            Some(ConfigProblem::Invalid { line: 2, column: 7 })
        );
        let (window, _) = launched(startup, cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(window.find("connection").label(), Some("No server set"));
            let notice = window.find("config-problem");
            assert_eq!(notice.label(), Some("Config file not used"));
            assert_eq!(
                notice.description(),
                Some("config.toml has an error at line 2, column 7, so no server is set")
            );
        })
        .expect("the window is open");
        assert_eq!(
            fs::read_to_string(&paths.config_file).expect("still there"),
            text,
            "reading never changes the file"
        );
    }

    #[test]
    fn a_server_set_in_the_config_starts_its_service() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let paths = paths_in(dir.path());
        fs::create_dir_all(paths.config_file.parent().expect("a folder")).expect("created");
        // A closed port on this machine: the service only starts a thread, which gets nowhere.
        fs::write(
            &paths.config_file,
            "[connection]\nurl = \"http://127.0.0.1:1/rpc\"\n",
        )
        .expect("written");

        let (startup, records) = start_from(&paths);

        assert!(matches!(startup.service, ServiceStart::Started(_)));
        assert_eq!(
            startup.connection.settings.map(|settings| settings.url()),
            Some("http://127.0.0.1:1/rpc".to_owned())
        );
        let connecting = records
            .iter()
            .find(|record| record.message == "connecting")
            .expect("the log says where it connects");
        assert_eq!(connecting.field("server"), Some("127.0.0.1:1"));
    }

    #[test]
    fn a_usable_config_has_no_problem() {
        assert_eq!(
            problem_with("[connection]\nurl = \"http://pi/rpc\"\n"),
            None
        );
        let dir = tempfile::tempdir().expect("a temporary directory");
        assert_eq!(load_config(&dir.path().join("missing.toml")).1, None);
    }

    #[test]
    fn config_problems_name_the_kind_and_the_place() {
        assert_eq!(
            problem_with("[connection]\nurl = 5\n"),
            Some(ConfigProblem::Invalid { line: 2, column: 7 })
        );
        assert_eq!(
            problem_with("[connection]\nurl = \"https://user:secret@pi/rpc\"\n"),
            Some(ConfigProblem::UnsupportedUrl)
        );
        assert_eq!(
            problem_with("[connection]\nurl = \"http://user:secret@pi/rpc\"\n"),
            Some(ConfigProblem::UnsupportedUrl)
        );
        assert_eq!(
            problem_with("[connection]\nurl = \"http://pi/rpc\"\nusername = \"a:b\"\n"),
            Some(ConfigProblem::UnsupportedUrl),
            "a user name with a colon"
        );

        let dir = tempfile::tempdir().expect("a temporary directory");
        let path = dir.path().join("config.toml");
        fs::create_dir(&path).expect("a directory");
        assert_eq!(load_config(&path).1, Some(ConfigProblem::Unreadable));
    }

    #[test]
    fn a_parse_error_without_a_place_is_malformed() {
        let error = |position| FilesError::Config {
            path: "/x/config.toml".into(),
            source: ConfigError::Parse {
                position,
                message: "boom".to_owned(),
            },
        };

        assert_eq!(ConfigProblem::of(&error(None)), ConfigProblem::Malformed);
        assert_eq!(
            ConfigProblem::of(&error(Some(TextPosition { line: 3, column: 1 }))),
            ConfigProblem::Invalid { line: 3, column: 1 }
        );
    }

    #[gpui_kit::test]
    fn a_service_that_cannot_start_still_opens_the_window(cx: &mut TestAppContext) {
        let ui = UiState {
            theme: ThemeChoice::Dark,
            filter: Filter::Paused,
            ..UiState::default()
        };

        let (window, view) = launched(
            Startup {
                ui: ui.clone(),
                ..failed_start(None)
            },
            cx,
        );

        let (text, stale, table_stale, shown_ui) = cx.update(|cx| {
            let view = view.read(cx);
            (
                view.connection_text(),
                view.status().stale,
                view.table().read(cx).delegate().stale(),
                view.ui_state().clone(),
            )
        });
        assert_eq!(text, "Service could not start");
        assert!(stale);
        assert!(table_stale, "the table draws its rows stale");
        assert_eq!(shown_ui, ui, "the saved state reaches the window");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                window.find("connection").label(),
                Some("Service could not start")
            );
            assert!(window.try_find("config-problem").is_none());
        })
        .expect("the window is open");
    }

    #[gpui_kit::test]
    fn a_config_problem_is_shown_until_dismissed(cx: &mut TestAppContext) {
        let (window, _) = launched(
            failed_start(Some(ConfigProblem::Invalid { line: 2, column: 7 })),
            cx,
        );
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let notice = window.find("config-problem");
            assert_eq!(notice.label(), Some("Config file not used"));
            assert_eq!(
                notice.description(),
                Some("config.toml has an error at line 2, column 7, so no server is set")
            );
        })
        .expect("the window is open");

        // The redraw itself is checked in `main_view`: the click redraws the view anyway, for
        // the button's hover and press styles.
        cx.update_window(window, |_, window, cx| {
            window.click("config-problem-dismiss", cx);
            window.render_frame(cx);
            assert!(window.try_find("config-problem").is_none());
        })
        .expect("the window is open");
    }

    #[gpui_kit::test]
    fn a_request_sent_into_the_startup_channel_opens_the_add_dialog(cx: &mut TestAppContext) {
        let startup = failed_start(None);
        let adds = startup.adds.sender.clone();
        let (window, _) = launched(startup, cx);

        adds.send_blocking(AddRequest::Items(vec![AddItem::Magnet(None)]))
            .expect("the window listens");
        cx.run_until_parked();

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(window.find("add-title").label(), Some("Add torrent"));
            assert!(window.try_find("add-link").is_some(), "the magnet form");
        })
        .expect("the window is open");
    }

    #[gpui_kit::test]
    fn the_sources_inside_the_window_send_into_the_startup_channel(cx: &mut TestAppContext) {
        let startup = failed_start(None);
        let outside = startup.adds.sender.clone();
        let (window, view) = launched(startup, cx);
        let inside = cx.update(|cx| view.read(cx).add_requests().clone());

        assert!(inside.same_channel(&outside), "one channel, the startup's");
        assert_eq!(outside.receiver_count(), 1, "which the window drains");
        inside
            .send_blocking(AddRequest::Items(vec![AddItem::Magnet(None)]))
            .expect("the window listens");
        cx.run_until_parked();

        assert!(outside.is_empty(), "taken");
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(window.find("add-title").label(), Some("Add torrent"));
        })
        .expect("the window is open");
    }

    #[gpui_kit::test]
    fn the_saved_state_reaches_the_window(cx: &mut TestAppContext) {
        let ui = UiState {
            filter: Filter::Seeding,
            ..UiState::default()
        };
        let (events, events_rx) = async_channel::unbounded();
        let (commands, _commands_rx) = crossbeam_channel::unbounded();
        let (_, view) = cx
            .update(|cx| {
                init(cx);
                open_main_window(events_rx, commands, None, &ui, cx)
            })
            .expect("the window opens");

        events
            .send_blocking(Event::Snapshot(Arc::new(Snapshot {
                torrents: vec![
                    torrent("seeding"),
                    Torrent {
                        state: DisplayState::Paused,
                        ..torrent("paused")
                    },
                ],
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

        let (names, shown_ui) = cx.update(|cx| {
            let view = view.read(cx);
            let names: Vec<String> = view
                .model()
                .rows
                .iter()
                .map(|row| row.name.clone())
                .collect();
            (names, view.ui_state().clone())
        });
        assert_eq!(names, ["seeding"], "the saved filter applies");
        assert_eq!(shown_ui, ui);
    }

    #[test]
    fn a_start_from_the_state_file_saves_only_where_it_may() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let no_service = || ServiceStart::Failed(io::Error::other("no service in this test"));

        let nowhere = Startup::from_state_file(no_service(), None);
        assert_eq!(nowhere.ui, UiState::default());
        assert!(!nowhere.state_writable);
        assert_eq!(nowhere.state_file(), None);

        let first_run = dir.path().join("missing.toml");
        let fresh = Startup::from_state_file(no_service(), Some(first_run.clone()));
        assert_eq!(fresh.ui, UiState::default());
        assert_eq!(fresh.state_file(), Some(first_run), "a first run saves");

        let newer = dir.path().join("newer.toml");
        fs::write(&newer, "version = 99\n").expect("written");
        let kept = Startup::from_state_file(no_service(), Some(newer.clone()));
        assert!(!kept.state_writable);
        assert_eq!(kept.state_path, Some(newer));
        assert_eq!(kept.state_file(), None, "a newer file is left alone");
        assert_eq!(kept.config_problem, None);
    }

    #[gpui_kit::test]
    fn the_main_window_opens_at_the_remembered_size_and_never_smaller(cx: &mut TestAppContext) {
        let at = |maximized| UiState {
            window: WindowState {
                width: 1000.,
                height: 650.,
                maximized,
            },
            ..UiState::default()
        };
        let remembered = size(px(1000.), px(650.));
        let minimum = Some(size(px(400.), px(300.)));

        let (maximized, windowed) = cx.update(|cx| {
            (
                main_window_options(&at(true), cx),
                main_window_options(&at(false), cx),
            )
        });

        match maximized.window_bounds {
            Some(WindowBounds::Maximized(bounds)) => assert_eq!(bounds.size, remembered),
            other => panic!("a maximized window opens maximized, not {other:?}"),
        }
        match windowed.window_bounds {
            Some(WindowBounds::Windowed(bounds)) => assert_eq!(bounds.size, remembered),
            other => panic!("a normal window opens as one, not {other:?}"),
        }
        for options in [&maximized, &windowed] {
            assert_eq!(options.window_min_size, minimum);
            assert_eq!(
                options.window_min_size,
                Some(size(px(MIN_WINDOW_WIDTH), px(MIN_WINDOW_HEIGHT))),
                "the minimum is core's"
            );
            assert_eq!(options.app_id.as_deref(), Some(app_id()));
        }
    }

    #[test]
    fn app_id_is_devel_in_debug_builds() {
        assert_eq!(app_id_for(true, None), "io.github.lexy898.FourByFour.Devel");
        if cfg!(debug_assertions) {
            assert_eq!(app_id(), "io.github.lexy898.FourByFour.Devel");
        }
    }

    #[test]
    fn app_id_for_covers_every_build_and_environment() {
        let one = Some(OsStr::new("1"));

        assert_eq!(app_id_for(true, one), DEVEL_APP_ID);
        assert_eq!(app_id_for(true, None), DEVEL_APP_ID);
        assert_eq!(app_id_for(false, one), DEVEL_APP_ID);
        assert_eq!(app_id_for(false, None), APP_ID);
    }

    #[test]
    fn only_x4_devel_one_makes_a_release_build_a_development_run() {
        for other in ["0", "", "true", "yes", " 1", "11"] {
            assert_eq!(
                app_id_for(false, Some(OsStr::new(other))),
                APP_ID,
                "{other:?}"
            );
        }
    }

    #[test]
    fn the_two_ids_differ_only_in_the_devel_suffix() {
        assert_eq!(APP_ID, "io.github.lexy898.FourByFour");
        assert_eq!(DEVEL_APP_ID, format!("{APP_ID}.Devel"));
    }

    /// The keys of the `[Desktop Entry]` group of the desktop file named after `id`, with their
    /// values, in the file's order.
    fn desktop_entry(id: &str) -> Vec<(String, String)> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../packaging/linux")
            .join(format!("{id}.desktop"));
        let text = fs::read_to_string(&path).expect("the desktop file is named after the app id");
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some("[Desktop Entry]"), "one group, first");
        lines
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| {
                assert!(!line.starts_with('['), "no other group: {line}");
                let (key, value) = line.split_once('=').expect("key=value");
                (key.to_owned(), value.to_owned())
            })
            .collect()
    }

    /// The value of `key` in a desktop entry, which may give each key once.
    fn entry_value<'a>(entry: &'a [(String, String)], key: &str) -> Option<&'a str> {
        let values: Vec<&str> = entry
            .iter()
            .filter(|(found, _)| found == key)
            .map(|(_, value)| value.as_str())
            .collect();
        assert!(values.len() <= 1, "{key} once");
        values.first().copied()
    }

    #[test]
    fn the_desktop_file_names_the_app_id_exec_and_mime_types() {
        let entry = desktop_entry(APP_ID);
        let value = |key: &str| entry_value(&entry, key);

        assert_eq!(value("Type"), Some("Application"));
        assert_eq!(value("Name"), Some(PRODUCT_NAME));
        assert_eq!(
            value("Exec"),
            Some("4x4 %U"),
            "one process for every file and link"
        );
        assert_eq!(value("TryExec"), Some("4x4"));
        assert_eq!(
            value("MimeType"),
            Some("application/x-bittorrent;x-scheme-handler/magnet;")
        );
        assert_eq!(value("StartupWMClass"), Some(APP_ID));
        assert_eq!(value("Icon"), Some(APP_ID));
        assert_eq!(value("Categories"), Some("Network;FileTransfer;P2P;"));
        assert_eq!(value("SingleMainWindow"), Some("true"));
        assert_eq!(value("Terminal"), Some("false"));
        for key in ["GenericName", "Comment"] {
            let text = value(key).unwrap_or_default();
            assert!(
                !text.is_empty() && text.is_ascii(),
                "{key}: English text, {text:?}"
            );
        }
        assert_eq!(value("DBusActivatable"), None, "launched by Exec only");
        assert!(
            entry.iter().all(|(key, _)| !key.contains('[')),
            "no translations yet"
        );
    }

    /// KDE and GNOME find a window's icon through the desktop file named after its app id, so the
    /// windows of `just run` need one of their own. It lends them the installed icon and nothing
    /// else: hidden from menus and search, never a handler of files or links, and a new start
    /// from it hands over to the running debug build, as a second launch of 4x4 does.
    #[test]
    fn the_debug_builds_desktop_file_lends_its_windows_the_icon_only() {
        let entry = desktop_entry(DEVEL_APP_ID);
        let value = |key: &str| entry_value(&entry, key);

        assert_eq!(value("Type"), Some("Application"));
        assert!(
            value("Name")
                .is_some_and(|name| name.starts_with(PRODUCT_NAME) && name != PRODUCT_NAME),
            "told apart from 4x4: {:?}",
            value("Name")
        );
        assert_eq!(value("Icon"), Some(APP_ID), "the icon the package installs");
        assert_eq!(value("StartupWMClass"), Some(DEVEL_APP_ID));
        assert_eq!(value("NoDisplay"), Some("true"), "never in menus or search");
        assert_eq!(value("MimeType"), None, "never opens files or links");
        assert_eq!(
            value("Exec"),
            Some(format!("env {DEVEL_ENV}=1 4x4 %U").as_str()),
            "the development id, so a new start hands over to the debug build"
        );
        assert_eq!(value("TryExec"), Some("4x4"));
        assert_eq!(value("Terminal"), Some("false"));
        assert_eq!(value("SingleMainWindow"), Some("true"));
        assert_eq!(value("DBusActivatable"), None, "launched by Exec only");
    }
}
