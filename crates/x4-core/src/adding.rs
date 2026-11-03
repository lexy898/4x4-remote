//! The add dialog, decided here so the app only draws it.
//!
//! - An [`AddQueue`] holds what the user asked to add, as [`AddItem`]s: paths and magnet links,
//!   never the contents of a file. Its head is the one shown.
//! - The app reads the head's `.torrent` file and core parses it: a [`LoadedItem`], whose
//!   [`LoadedTorrent`] keeps the parse together with the bytes it was made from and the tree of
//!   its files. A list the daemon could number otherwise, or too large to choose from, gets no
//!   tree there: the torrent is added with all its files ([`NoChoiceReason`]).
//! - An [`AddForm`] is the dialog for that item: the folder, "Start immediately" and, for a
//!   `.torrent`, the choice of files through the same [`FileTree`](crate::FileTree) as the
//!   details panel.
//! - [`AddForm::check`] gives the [`AddCheck`] that tells whether Add may send it, and the
//!   [`TorrentAdd`](x4_rpc::TorrentAdd) it sends.
//! - [`folder_picks`] lists the folders the folder field offers; [`FormDefaults`] gives each form
//!   of a run its folder and "Start immediately"; [`FreeSpaceCache`] decides when the free space
//!   of the folder is asked for and keeps the answers.
//! - [`add_outcome`] says what follows the daemon's answer to an add (an [`AddOutcome`]), and
//!   [`stopped_outcome`] what follows when the service stops before it; a [`PendingSelection`]
//!   waits for the torrent to select once it is listed, unless the user's [`ListChoices`] changed.
//!
//! Nothing here sends anything: the app sends the request of an [`AddCheck::Ready`] only when the
//! user presses Add.

mod choice;
mod defaults;
mod folders;
mod form;
mod free_space;
mod outcome;
mod queue;
mod selection;

pub use defaults::{FormDefaults, TakenDefaults};
pub use folders::{MAX_FOLDER_PICKS, folder_picks};
pub use form::{AddCheck, AddForm, LoadedItem, LoadedTorrent, NoChoiceReason};
pub use free_space::{FREE_SPACE_DELAY, FreeSpaceCache, FreeSpaceFollow};
pub use outcome::{AddNotice, AddOutcome, FormStep, SentAdd, add_outcome, stopped_outcome};
pub use queue::{AddItem, AddQueue, QUEUE_CAP};
pub use selection::{ListChoices, PENDING_SELECTION_LIFE, PendingSelection, Reveal};
