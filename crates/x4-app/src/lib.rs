//! The 4x4 desktop client: GPUI interface and platform integration.
//!
//! [`app`] starts the application: it reads the config and the saved state, starts the service of
//! `x4-core` against the daemon set there, if any, and opens the main window, whose content is
//! [`main_view::MainView`]. [`connection`] starts the service for a server, which the window can
//! switch to another without a restart, and checks a server once; the `connection_dialog` module
//! is the dialog where the user sets the server and tests it. The view pumps the service's
//! events without blocking the UI thread, and lays out a toolbar (the `toolbar` module) with the
//! search box (the `search` module), the filter sidebar (the `sidebar` module), the torrent table
//! ([`torrent_table`]), the details panel under it (the `details_panel` module), whose Files tab
//! draws the shared list of a torrent's files (the `file_list` module), whose Peers tab draws a
//! torrent's peers with their flags and whose Trackers tab lists its trackers, and the status bar
//! (the `status_bar` module); the `theme` module holds the light and dark theme and the colors of
//! states, the `icons` module the icons beyond gpui-component's own, and the `flags` module the
//! country flags. [`actions`] holds what can be done to the selected torrents, the `context_menu`
//! module the menu of a right-click on them, the `removal` module the dialogs that ask before they
//! are removed, the `clipboard` module the copies of their hashes and magnet links, and the
//! `key_repeats` module tells a press of their keys from the repeats of a held key. The
//! `speed_limits` module is the popover of the status bar that sets the daemon's speed limits.
//! [`adding`] holds the requests to add torrents and the add dialog that offers them one at a time.
//! [`paths`] finds the config, state and log locations, [`logging`] logs to stderr and a daily file
//! there, and the `files` module reads and writes `config.toml` and `state.toml`; the `saving`
//! module decides when the window's state is written, and the `window_state` module turns the
//! window's bounds into the size to remember and back. [`instance`] keeps one 4x4 per user: a later
//! launch hands its arguments to the first one and exits. The `4x4` binary pins the allocator
//! ([`allocator`]), finds the paths, starts the logging, hands its arguments over or calls
//! [`app::run`] with them. [`keymap`] is the one table of key bindings, with the actions the keys
//! run. Every text the windows show comes from the Fluent messages in `i18n/`, through the `i18n`
//! module.

pub mod actions;
pub mod adding;
pub mod allocator;
pub mod app;
mod clipboard;
pub mod connection;
mod connection_dialog;
mod context_menu;
mod details_panel;
mod file_list;
mod files;
mod flags;
mod geoip;
mod i18n;
mod icons;
pub mod instance;
mod key_repeats;
pub mod keymap;
pub mod logging;
pub mod main_view;
mod own_thread;
pub mod paths;
mod private_dir;
mod removal;
mod saving;
mod search;
mod sidebar;
mod speed_limits;
mod status_bar;
#[cfg(test)]
mod test_dirs;
#[cfg(test)]
mod test_log;
mod theme;
mod toolbar;
pub mod torrent_table;
mod window_state;

/// The product name: the only user-visible literal allowed outside the i18n messages.
const PRODUCT_NAME: &str = "4x4";
