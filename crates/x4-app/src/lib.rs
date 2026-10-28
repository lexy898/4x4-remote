//! The 4x4 desktop client: GPUI interface and platform integration.
//!
//! [`app`] starts the application and opens the main window; [`root_view`] is the window's
//! content. The `4x4` binary only sets up logging and calls [`app::run`].

pub mod app;
pub mod root_view;

/// The product name: the only user-visible literal allowed before the i18n layer exists.
const PRODUCT_NAME: &str = "4x4";
