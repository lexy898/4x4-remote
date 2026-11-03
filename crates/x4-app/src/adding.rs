//! Adding torrents: the requests that the sources of adds send the main window ([`AddRequest`]),
//! the reading of a `.torrent` file when its turn comes (the `requests` module), and the add
//! dialog that shows one torrent of the queue at a time (the `dialog` module), with its folder
//! field (`folder_field`) and its choice of files (`file_choice`).
//!
//! Nothing is added without the user's Add: every source only queues what it was given, and the
//! main view (its `adding` module) keeps the queue, opens the dialog, sends what the dialog asks
//! for and follows the daemon's answers.

mod dialog;
mod file_choice;
mod folder_field;
mod requests;

pub(crate) use dialog::{AddDialog, AddDialogEvent, open};
pub use requests::{AddRequest, AddRequests};
pub(crate) use requests::{LoadError, load};
