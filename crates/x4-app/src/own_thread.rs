//! Work done on a thread of its own, one thread for each piece of work, which ends with it: the
//! reading of a `.torrent` file (`adding::read_on_a_thread`) and the making of a big torrent's
//! file tree for the Files tab.
//!
//! Work done on a thread of GPUI's background pool left its memory with that thread: glibc keeps an
//! arena of freed memory for each thread that allocates, and a pool thread never ends, so each one
//! that did such work kept its own. Five `.torrent` files of 2000 entries read on the pool held
//! 5.5 MiB more than one, and the file trees of the synthetic demo's 2000-file and 50 000-file
//! torrents, selected in turn five times, a median of 128 MiB of private memory against 87 on
//! threads of their own (README, Performance). A thread that ends gives its arena back for the
//! next one to take, so the pieces of work share about one, as long as no thread of the pool takes
//! it first: the answer is awaited on the UI thread. And work that never ends, as a read on a
//! network folder that hangs, holds only its own thread, where it held one of the pool's for good.

use std::io;
use std::thread;

use gpui_kit::{App, Task};

/// Runs `work` on a thread of its own, named `name`, with a stack of `stack` bytes, which ends
/// with it; the task, on the UI thread, gives what `work` gave. A thread that cannot be started,
/// or that ends without an answer, gives what `lost` makes of the kind of the error, which is
/// logged. Dropping the task lets go of the answer: the thread still ends with its work, and its
/// answer is dropped there.
pub(crate) fn on_a_thread<R: Send + 'static>(
    name: &'static str,
    stack: usize,
    work: impl FnOnce() -> R + Send + 'static,
    lost: fn(io::ErrorKind) -> R,
    cx: &App,
) -> Task<R> {
    let (answer, answered) = async_channel::bounded(1);
    let started = thread::Builder::new()
        .name(name.to_owned())
        .stack_size(stack)
        .spawn(move || {
            if answer.send_blocking(work()).is_err() {
                tracing::debug!(thread = name, "the work was done after it was dropped");
            }
        });
    if let Err(error) = started {
        tracing::error!(thread = name, %error, "no thread could be started for the work");
        return Task::ready(lost(error.kind()));
    }
    // The answer is awaited on the UI thread. On a thread of GPUI's pool, the wait would allocate,
    // and a pool thread that had not allocated yet would take the arena the last thread of its own
    // left, and keep it for good: each piece of work would then leave its memory in an arena of
    // its own.
    cx.spawn(async move |_| {
        answered.recv().await.unwrap_or_else(|_| {
            tracing::error!(thread = name, "the thread ended without an answer");
            lost(io::ErrorKind::Other)
        })
    })
}
