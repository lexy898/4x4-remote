//! The add dialog, decided here so the app only draws it.
//!
//! - An [`AddQueue`] holds what the user asked to add, as [`AddItem`]s: paths and magnet links,
//!   never the contents of a file. Its head is the one shown.
//! - The app reads the head's `.torrent` file and core parses it: a [`LoadedItem`].
//! - An [`AddForm`] is the dialog for that item: the folder, "Start immediately" and, for a
//!   `.torrent`, the choice of files through the same [`FileTree`](crate::FileTree) as the
//!   details panel.
//! - [`AddForm::check`] gives the [`AddCheck`] that tells whether Add may send it, and the
//!   [`TorrentAdd`](x4_rpc::TorrentAdd) it sends.
//! - [`folder_picks`] lists the folders the folder field offers.
//!
//! Nothing here sends anything: the app sends the request of an [`AddCheck::Ready`] only when the
//! user presses Add.

mod choice;
mod folders;
mod form;
mod queue;

pub use folders::{MAX_FOLDER_PICKS, folder_picks};
pub use form::{AddCheck, AddForm, LoadedItem};
pub use queue::{AddItem, AddQueue, QUEUE_CAP};
