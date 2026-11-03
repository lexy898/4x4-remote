//! Adding torrents: the requests that the sources of adds send the main window ([`AddRequest`]),
//! the reading of a `.torrent` file when its turn comes (the `requests` module), the file dialog
//! that chooses `.torrent` files ([`FilePicker`], the `picker` module), and the add dialog that
//! shows one torrent of the queue at a time (the `dialog` module), with its folder field
//! (`folder_field`) and its choice of files (`file_choice`). The main window and the dialog are
//! highlighted alike while files hover over them, and take a drop after a key press too
//! (`drop_target`).
//!
//! Nothing is added without the user's Add: every source only queues what it was given, and the
//! main view (its `adding` module) keeps the queue, opens the dialog, sends what the dialog asks
//! for and follows the daemon's answers.

mod dialog;
mod drop_target;
mod file_choice;
mod folder_field;
mod picker;
mod requests;

pub(crate) use dialog::{AddDialog, AddDialogEvent, open};
pub(crate) use drop_target::{drop_highlight, hover_while_dragging};
#[cfg(target_os = "linux")]
pub(crate) use picker::PortalPicker;
pub(crate) use picker::PromptPicker;
pub use picker::{FilePicker, PickError, Picked};
pub use requests::{AddRequest, AddRequests};
pub(crate) use requests::{LoadError, load};
