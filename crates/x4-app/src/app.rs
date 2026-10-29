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
    AnyWindowHandle, App, AppContext as _, Bounds, Entity, KeyBinding, QuitMode, SharedString,
    TitlebarOptions, WindowBounds, WindowOptions, actions, px, size,
};
use x4_core::service::{self, ServiceHandle};
use x4_core::{Command, Config, EngineConfig, Event, UiState};
use x4_rpc::{Client, DEFAULT_TIMEOUT, UreqTransport};

use crate::PRODUCT_NAME;
use crate::files::{read_config, read_ui_state};
use crate::main_view::MainView;
use crate::paths::AppPaths;
use crate::theme::apply_theme;

actions!(
    x4,
    [
        /// Quits the application.
        Quit,
    ]
);

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
    /// Why `config.toml` is not used, if it is not, in words that show nothing from the file.
    /// The window shows it until the user dismisses it.
    pub config_problem: Option<String>,
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

/// Reads `config.toml`. A file that cannot be used gives the defaults and a display-safe
/// description of the problem, which is logged too; the URL is never logged, as it may hold a
/// password.
fn load_config(path: &Path) -> (Config, Option<String>) {
    match read_config(path) {
        Ok(config) => {
            tracing::info!(path = %path.display(), "config loaded");
            (config, None)
        }
        Err(error) => {
            let problem = error.report();
            tracing::warn!(error = %problem, "config not loaded; using the defaults");
            (Config::default(), Some(problem))
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
            view.show_config_problem(SharedString::from(problem), cx);
        }
    });
    Ok((window, view))
}

/// Initializes the UI libraries and registers the application-wide actions and key bindings.
pub fn init(cx: &mut App) {
    gpui_kit::init(cx);
    cx.set_quit_mode(QuitMode::LastWindowClosed);
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys([KeyBinding::new("ctrl-q", Quit, None)]);
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
    let bounds = Bounds::centered(None, size(px(1100.), px(700.)), cx);
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
    use gpui_kit::TestAppContext;
    use gpui_kit::test::TestWindowExt as _;

    use super::*;

    /// A start whose service could not be started, with `config_problem`.
    fn failed_start(config_problem: Option<&str>) -> Startup {
        Startup {
            service: Err(io::Error::other("no more threads")),
            ui: UiState::default(),
            state_writable: false,
            state_path: None,
            config_problem: config_problem.map(str::to_owned),
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

    #[gpui_kit::test]
    fn a_service_that_cannot_start_still_opens_the_window(cx: &mut TestAppContext) {
        let (window, view) = launched(failed_start(None), cx);

        let (text, stale) = cx.update(|cx| {
            let view = view.read(cx);
            (view.connection_text(), view.status().stale)
        });
        assert_eq!(text, "Service could not start");
        assert!(stale);
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
        let (window, _) = launched(failed_start(Some("invalid config file /x/config.toml")), cx);

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                window.find("config-problem").label(),
                Some("Config file not used")
            );

            window.click("config-problem-dismiss", cx);
            window.render_frame(cx);
            assert!(window.try_find("config-problem").is_none());
        })
        .expect("the window is open");
    }

    #[gpui_kit::test]
    fn the_saved_state_reaches_the_window(cx: &mut TestAppContext) {
        let ui = UiState {
            theme: x4_core::ThemeChoice::Dark,
            filter: x4_core::Filter::Seeding,
            ..UiState::default()
        };

        let (_, view) = launched(
            Startup {
                ui: ui.clone(),
                ..failed_start(None)
            },
            cx,
        );

        assert_eq!(cx.update(|cx| view.read(cx).ui_state().clone()), ui);
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
