//! Application startup: global actions and key bindings, the main window and the first-frame log.

use std::cell::RefCell;
use std::env;
use std::ffi::OsStr;
use std::rc::Rc;
use std::time::Instant;

use anyhow::Context as _;
use gpui_kit::component::Theme;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Entity, KeyBinding, QuitMode, TitlebarOptions,
    Window, WindowBounds, WindowOptions, actions, px, size,
};

use crate::PRODUCT_NAME;
use crate::files::{read_config, read_ui_state};
use crate::paths::AppPaths;
use crate::root_view::RootView;

/// Identifies the application to the desktop environment (the Wayland `app_id`).
const APP_ID: &str = "io.github.lexy898.FourByFour";
/// The id of development runs, so that window rules written for test windows (placing them on
/// a particular screen without focus) never catch the owner's real window.
const DEVEL_APP_ID: &str = "io.github.lexy898.FourByFour.Devel";
/// Set to `1` to give a release build the development id, as `just measure` does.
const DEVEL_ENV: &str = "X4_DEVEL";

actions!(
    x4,
    [
        /// Quits the application.
        Quit,
    ]
);

/// Runs the application until it quits: reads the config and the saved state from `paths`
/// (`None` when the home directory is unknown), initializes the UI, opens the main window and
/// logs `first frame rendered` with the time elapsed since `started`.
///
/// # Errors
///
/// Returns an error if the main window cannot be opened.
pub fn run(started: Instant, paths: Option<&AppPaths>) -> anyhow::Result<()> {
    if let Some(paths) = paths {
        load_files(paths);
    }
    let launch_error = Rc::new(RefCell::new(None));
    let launch_error_slot = Rc::clone(&launch_error);
    gpui_kit::application().run(move |cx| {
        init(cx);
        let opened =
            open_main_window(cx).and_then(|(window, _)| log_first_frame(window, started, cx));
        if let Err(error) = opened {
            launch_error_slot.replace(Some(error));
            // A quit requested during launch is lost: the event loop clears its stop signal
            // when it starts, so quit from inside the loop instead.
            cx.spawn(async |cx| cx.update(|cx| cx.quit())).detach();
        }
    });
    launch_error.take().map_or(Ok(()), Err)
}

/// Reads `config.toml` and `state.toml` and logs what happened. Nothing uses the values yet: the
/// connection and the window state are wired in by later stages. A broken file never stops the
/// app; the defaults are used and the problem is logged.
fn load_files(paths: &AppPaths) {
    match read_config(&paths.config_file) {
        Ok(_) => {
            tracing::info!(path = %paths.config_file.display(), "config loaded");
        }
        Err(error) => {
            tracing::warn!(error = %error.report(), "config not loaded; using the defaults");
        }
    }
    match read_ui_state(&paths.state_file) {
        Ok(loaded) => {
            tracing::info!(path = %paths.state_file.display(), writable = loaded.writable, "UI state loaded");
        }
        Err(error) => {
            tracing::warn!(error = %error.report(), "UI state not loaded; using the defaults");
        }
    }
}

/// Initializes the UI libraries and registers the application-wide actions and key bindings.
pub fn init(cx: &mut App) {
    gpui_kit::init(cx);
    cx.set_quit_mode(QuitMode::LastWindowClosed);
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys([KeyBinding::new("ctrl-q", Quit, None)]);
}

/// Opens the main window, centered, with [`RootView`] as its content. Call [`init`] (or at
/// least `gpui_kit::init`) first.
///
/// # Errors
///
/// Returns an error if the platform cannot create the window.
pub fn open_main_window(cx: &mut App) -> anyhow::Result<(AnyWindowHandle, Entity<RootView>)> {
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
        follow_system_appearance(window, cx);
        cx.new(|_| RootView)
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

/// Matches the light or dark theme to the system appearance now and whenever it changes.
fn follow_system_appearance(window: &mut Window, cx: &mut App) {
    Theme::sync_system_appearance(Some(window), cx);
    window
        .observe_window_appearance(|window, cx| Theme::sync_system_appearance(Some(window), cx))
        .detach();
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
    use super::*;

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
