//! Application startup: the config and the saved state, the service, global actions and key
//! bindings, the main window and the first-frame log.
//!
//! [`run`] is the front door the `4x4` binary uses: it reads the files and starts the service
//! against the configured daemon. [`run_with`] takes a [`Startup`] built by its caller instead,
//! so that a demo or a measurement can run the window against a service of its own.

use std::cell::RefCell;
use std::env;
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use anyhow::Context as _;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Entity, KeyBinding, NoAction, QuitMode,
    TitlebarOptions, WindowBounds, WindowOptions, actions, px, size,
};
use x4_core::service::{self, ServiceHandle};
use x4_core::{Command, Config, ConfigError, EngineConfig, Event, UiState};
use x4_rpc::{Client, DEFAULT_TIMEOUT, UreqTransport};

use crate::PRODUCT_NAME;
use crate::files::{FilesError, read_config, read_ui_state};
use crate::main_view::MainView;
use crate::paths::AppPaths;
use crate::theme::apply_theme;

actions!(
    x4,
    [
        /// Quits the application.
        Quit,
        /// Moves the focus to the next control, out of the torrent table.
        FocusNext,
        /// Moves the focus to the previous control, out of the torrent table.
        FocusPrevious,
    ]
);

/// The key context of gpui-component's table.
const TABLE_CONTEXT: &str = "DataTable";

/// Identifies the application to the desktop environment (the Wayland `app_id`).
const APP_ID: &str = "io.github.lexy898.FourByFour";
/// The id of development runs, so that window rules written for test windows (placing them on
/// a particular screen without focus) never catch the owner's real window.
const DEVEL_APP_ID: &str = "io.github.lexy898.FourByFour.Devel";
/// Set to `1` to give a release build the development id, as `just measure` does.
const DEVEL_ENV: &str = "X4_DEVEL";

/// Everything the app starts with, as plain values: the service, the saved UI state and what
/// went wrong while reading the files.
#[derive(Debug)]
pub struct Startup {
    /// The service thread the window shows, or why it could not be started. On an error the
    /// window still opens and says that the service could not start.
    pub service: io::Result<ServiceHandle>,
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
}

/// Why `config.toml` is not used, as the window tells the user (in their language). The log
/// gets the full report, with the path and the parser's message.
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
    /// `connection.url` is not a plain `http://` address.
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
            // Reading gives only `Read`; the others come from writing the state.
            FilesError::Read { .. }
            | FilesError::CreateDir { .. }
            | FilesError::Serialize { .. }
            | FilesError::Write { .. } => Self::Unreadable,
        }
    }
}

/// Runs the application until it quits: reads the config and the saved state from `paths`
/// (`None` when the home directory is unknown), starts the service against the configured
/// daemon and hands over to [`run_with`].
///
/// The service only starts a thread here; the window opens at once and shows the connection
/// being made, so the first frame never waits for the daemon.
///
/// # Errors
///
/// Returns an error if the main window cannot be opened.
pub fn run(started: Instant, paths: Option<&AppPaths>) -> anyhow::Result<()> {
    let (config, config_problem) = paths.map_or_else(
        || (Config::default(), None),
        |paths| load_config(&paths.config_file),
    );
    let (ui, state_writable) = paths.map_or_else(
        || (UiState::default(), false),
        |paths| load_ui_state(&paths.state_file),
    );
    let client = Client::new(UreqTransport::new(&config.connection.url, DEFAULT_TIMEOUT));
    run_with(
        started,
        Startup {
            service: service::spawn(client, EngineConfig::default()),
            ui,
            state_writable,
            state_path: paths.map(|paths| paths.state_file.clone()),
            config_problem,
        },
    )
}

/// Runs the application until it quits with what `startup` holds: initializes the UI, opens the
/// main window and logs `first frame rendered` with the time elapsed since `started`.
///
/// # Errors
///
/// Returns an error if the main window cannot be opened.
pub fn run_with(started: Instant, startup: Startup) -> anyhow::Result<()> {
    let launch_error = Rc::new(RefCell::new(None));
    let launch_error_slot = Rc::clone(&launch_error);
    gpui_kit::application().run(move |cx| {
        init(cx);
        let opened =
            launch(startup, cx).and_then(|(window, _)| log_first_frame(window, started, cx));
        if let Err(error) = opened {
            launch_error_slot.replace(Some(error));
            // A quit requested during launch is lost: the event loop clears its stop signal
            // when it starts, so quit from inside the loop instead.
            cx.spawn(async |cx| cx.update(|cx| cx.quit())).detach();
        }
    });
    launch_error.take().map_or(Ok(()), Err)
}

/// Reads `config.toml`. A file that cannot be used gives the defaults and the problem for the
/// window; the full report goes to the log, which never shows the URL, as it may hold a
/// password.
fn load_config(path: &Path) -> (Config, Option<ConfigProblem>) {
    match read_config(path) {
        Ok(config) => {
            tracing::info!(path = %path.display(), "config loaded");
            (config, None)
        }
        Err(error) => {
            tracing::warn!(error = %error.report(), "config not loaded; using the defaults");
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

/// Opens the main window for `startup`. A service that could not start, and a config problem,
/// are shown in the status bar.
fn launch(startup: Startup, cx: &mut App) -> anyhow::Result<(AnyWindowHandle, Entity<MainView>)> {
    let Startup {
        service,
        ui,
        // Saving the UI state is wired in with the window state.
        state_writable: _,
        state_path: _,
        config_problem,
    } = startup;
    let (events, commands, service) = match service {
        Ok(service) => (service.events().clone(), service.commands(), Some(service)),
        Err(error) => {
            tracing::error!(%error, "the service could not start; the window has no data to show");
            // Channels with nobody at the other end: the events are closed at once and the
            // commands go nowhere.
            let (_, events) = async_channel::unbounded();
            let (commands, _) = crossbeam_channel::unbounded();
            (events, commands, None)
        }
    };
    let failed = service.is_none();
    let (window, view) = open_main_window(events, commands, service, &ui, cx)?;
    view.update(cx, |view, cx| {
        if failed {
            view.service_failed(cx);
        }
        if let Some(problem) = config_problem {
            view.show_config_problem(problem, cx);
        }
    });
    Ok((window, view))
}

/// Initializes the UI libraries and registers the application-wide actions and key bindings.
///
/// This is the one table of key bindings. They come after the libraries' own, so that ours win
/// where both bind a key in the same context.
pub fn init(cx: &mut App) {
    gpui_kit::init(cx);
    cx.set_quit_mode(QuitMode::LastWindowClosed);
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys([
        KeyBinding::new("ctrl-q", Quit, None),
        // The table binds Tab and the side arrows to moving between its columns. A list of
        // torrents has rows only: Tab leaves it, and the side arrows do nothing.
        KeyBinding::new("tab", FocusNext, Some(TABLE_CONTEXT)),
        KeyBinding::new("shift-tab", FocusPrevious, Some(TABLE_CONTEXT)),
        KeyBinding::new("left", NoAction, Some(TABLE_CONTEXT)),
        KeyBinding::new("right", NoAction, Some(TABLE_CONTEXT)),
    ]);
}

/// Opens the main window, centered, in the theme `ui` asks for, with a [`MainView`] of the
/// service whose channels are `events` and `commands` as its content. The view keeps `service`
/// so that closing the window stops it; pass `None` when there is no thread behind the channels.
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
    let bounds = Bounds::centered(None, size(px(ui.window.width), px(ui.window.height)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some(PRODUCT_NAME.into()),
            ..Default::default()
        }),
        app_id: Some(app_id().to_owned()),
        ..Default::default()
    };
    gpui_kit::open_window(options, cx, |window, cx| {
        // Before the view exists, so that the first frame is drawn in the right theme.
        apply_theme(ui.theme, window, cx);
        let ui = ui.clone();
        cx.new(|cx| MainView::new(events, commands, service, ui, window, cx))
    })
    .context("failed to open the main window")
}

/// The Wayland `app_id` of this run: the development id in debug builds and when `X4_DEVEL=1`,
/// the real one otherwise.
pub(crate) fn app_id() -> &'static str {
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
    use gpui_kit::test::TestWindowExt as _;
    use x4_core::test_support::torrent;
    use x4_core::{
        ConnectionState, DisplayState, Filter, Snapshot, TextPosition, ThemeChoice, Torrent,
    };

    use super::*;

    /// A start whose service could not be started, with `config_problem`.
    fn failed_start(config_problem: Option<ConfigProblem>) -> Startup {
        Startup {
            service: Err(io::Error::other("no more threads")),
            ui: UiState::default(),
            state_writable: false,
            state_path: None,
            config_problem,
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

        let (text, stale, shown_ui) = cx.update(|cx| {
            let view = view.read(cx);
            (
                view.connection_text(),
                view.status().stale,
                view.ui_state().clone(),
            )
        });
        assert_eq!(text, "Service could not start");
        assert!(stale);
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
                Some("config.toml has an error at line 2, column 7, so the defaults are used")
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
}
