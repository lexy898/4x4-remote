//! The reading of a `.torrent` file on a thread of its own, one thread per file, which ends with
//! the read.
//!
//! A read on a thread of GPUI's background pool left its memory with that thread: glibc keeps an
//! arena of freed memory for each thread that allocates, and a pool thread never ends, so each
//! one that read a large torrent kept its own (five files of 2000 entries held 5.5 MiB more than
//! one). A thread that ends gives its arena back for the next one to take, so the reads share
//! about one, as long as no thread of the pool takes it first: the answer is awaited on the UI
//! thread. And a read that never ends, as on a network folder that hangs, holds only its own
//! thread, where it held one of the pool's for good: the thread is left to it when the user skips
//! the file, and its late answer goes nowhere.

use std::io;
use std::thread;

use gpui_kit::{App, Task};
use x4_core::{AddItem, LoadedItem};

use super::LoadError;

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
    let (answer, answered) = async_channel::bounded(1);
    let started = thread::Builder::new()
        .name(READER_THREAD.to_owned())
        .stack_size(READER_STACK)
        .spawn(move || {
            if answer.send_blocking(load(item)).is_err() {
                tracing::debug!("a torrent file was read after it was skipped");
            }
        });
    if let Err(error) = started {
        tracing::error!(%error, "no thread could be started to read a torrent file");
        return Task::ready(Err(LoadError::Unreadable(error.kind())));
    }
    // The answer is awaited on the UI thread. On a thread of GPUI's pool, the wait would allocate,
    // and a pool thread that had not allocated yet would take the arena the last reader left, and
    // keep it for good: each read would then leave its memory in an arena of its own.
    cx.spawn(async move |_| {
        answered.recv().await.unwrap_or_else(|_| {
            tracing::error!("the thread that read a torrent file ended without an answer");
            Err(LoadError::Unreadable(io::ErrorKind::Other))
        })
    })
}
