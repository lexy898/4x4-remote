//! The 4x4 desktop client: GPUI interface and platform integration.
//!
//! [`app`] starts the application: it reads the config and the saved state, starts the service
//! of `x4-core` against the daemon and opens the main window, whose content is
//! [`main_view::MainView`]. The view pumps the service's events without blocking the UI thread,
//! and lays out a toolbar, the sidebar, the torrent table ([`torrent_table`]) and the status bar
//! (the `status_bar` module); the `theme` module holds the light and dark theme and the colors of
//! states. [`paths`] finds the config, state and log locations, [`logging`] logs to stderr and a
//! daily file there, and the `files` module reads `config.toml` and reads and writes
//! `state.toml`. The `4x4` binary finds the paths, starts the logging and calls [`app::run`].
//! Every text the windows show comes from the Fluent messages in `i18n/`, through the `i18n`
//! module.

pub mod app;
mod files;
mod i18n;
pub mod logging;
pub mod main_view;
pub mod paths;
mod private_dir;
mod status_bar;
#[cfg(test)]
mod test_dirs;
mod theme;
pub mod torrent_table;

/// The product name: the only user-visible literal allowed outside the i18n messages.
const PRODUCT_NAME: &str = "4x4";
