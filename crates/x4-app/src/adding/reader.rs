//! The reading of a `.torrent` file on a thread of its own, one thread per file, which ends with
//! the read ([`on_a_thread`](crate::own_thread::on_a_thread), which says why). A read that never
//! ends, as on a network folder that hangs, holds only its own thread: the thread is left to it
//! when the user skips the file, and its late answer goes nowhere.

use gpui_kit::{App, Task};
use x4_core::{AddItem, LoadedItem};

use super::LoadError;
use crate::own_thread::on_a_thread;

/// The name of the thread a `.torrent` file is read on.
const READER_THREAD: &str = "torrent-reader";

/// The stack of that thread. The parser keeps its own stack of at most 31 nested values, and
/// nothing else recurses deeply; a stack is mapped, not filled, so only what is used takes memory.
const READER_STACK: usize = 256 * 1024;

/// What reads one item.
pub(crate) type Load = fn(AddItem) -> Result<LoadedItem, LoadError>;

/// Reads `item` with `load` on a thread of its own, named `torrent-reader`; the task, on the UI
/// thread, gives what the read gave. Dropping the task lets go of the answer: the thread still ends
/// with its read, and its answer is dropped. A thread that cannot be started is an unreadable file.
pub(crate) fn read_on_a_thread(
    item: AddItem,
    load: Load,
    cx: &App,
) -> Task<Result<LoadedItem, LoadError>> {
    on_a_thread(
        READER_THREAD,
        READER_STACK,
        move || load(item),
        |kind| Err(LoadError::Unreadable(kind)),
        cx,
    )
}
