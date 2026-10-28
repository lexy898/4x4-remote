//! The 4x4 desktop client: GPUI interface and platform integration.
//!
//! [`app`] starts the application and opens the main window; [`root_view`] is the window's
//! content. [`paths`] finds the config, state and log locations, [`logging`] logs to stderr and a
//! daily file there, and the `files` module reads `config.toml` and reads and writes
//! `state.toml`. The `4x4` binary finds the paths, starts the logging and calls [`app::run`].
//! Every text the windows show comes from the Fluent messages in `i18n/`, through the `i18n`
//! module.

pub mod app;
mod files;
mod i18n;
pub mod logging;
pub mod paths;
mod private_dir;
pub mod root_view;
#[cfg(test)]
mod test_dirs;

/// The product name: the only user-visible literal allowed outside the i18n messages.
const PRODUCT_NAME: &str = "4x4";
