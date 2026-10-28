//! Application startup: global actions and key bindings, the main window and the first-frame log.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use anyhow::Context as _;
use gpui_kit::component::Theme;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Entity, KeyBinding, QuitMode, TitlebarOptions,
    Window, WindowBounds, WindowOptions, actions, px, size,
};

use crate::PRODUCT_NAME;
use crate::root_view::RootView;

/// Identifies the application to the desktop environment (the Wayland `app_id`).
const APP_ID: &str = "io.github.lexy898.FourByFour";

actions!(
    x4,
    [
        /// Quits the application.
        Quit,
    ]
);

/// Runs the application until it quits: initializes the UI, opens the main window and logs
/// `first frame rendered` with the time elapsed since `started`.
///
/// # Errors
///
/// Returns an error if the main window cannot be opened.
pub fn run(started: Instant) -> anyhow::Result<()> {
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
        app_id: Some(APP_ID.to_owned()),
        ..Default::default()
    };
    gpui_kit::open_window(options, cx, |window, cx| {
        follow_system_appearance(window, cx);
        cx.new(|_| RootView)
    })
    .context("failed to open the main window")
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
