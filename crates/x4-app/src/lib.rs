//! The 4x4 desktop client: GPUI interface and platform integration.
//!
//! [`app`] starts the application and opens the main window; [`root_view`] is the window's
//! content. [`paths`] finds the config, state and log locations, [`logging`] logs to stderr and a
//! daily file there, and the `files` module reads `config.toml` and reads and writes
//! `state.toml`. The `4x4` binary finds the paths, starts the logging and calls [`app::run`].

pub mod app;
mod files;
pub mod logging;
pub mod paths;
pub mod root_view;

/// The product name: the only user-visible literal allowed before the i18n layer exists.
const PRODUCT_NAME: &str = "4x4";
