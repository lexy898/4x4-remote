//! One 4x4 per user: the first launch listens, and a later one hands its arguments over to it and
//! is done ([`claim`]).
//!
//! The first 4x4 of a user holds the lock `<app id>.lock` in the user's runtime directory and
//! listens on the Unix socket `<app id>.sock` beside it. A later launch finds the lock taken,
//! connects and sends one [`Frame`]: its arguments, its working directory and the tokens the
//! desktop gave it to bring a window to the front. Then it waits for the answer ([`Reply`]):
//! - `ok`: the requests are queued, and the launch exits;
//! - `closing`, from a 4x4 that is quitting and queued nothing: it tries again, and takes the lock
//!   itself once that 4x4 has gone. A quitting 4x4 removes its socket first, then answers this to
//!   the launches still waiting in its queue, and only then lets go of the lock;
//! - `bad`, or no answer within 2 seconds, from a 4x4 that hangs: it runs on its own and opens a
//!   window of its own, so that the user's click is never lost.
//!
//! A frame once written is never sent again, so that a 4x4 that was only slow never shows it
//! twice. The lock goes with the process, however it ends, and only the holder of the lock
//! removes or binds the socket: a socket left by a 4x4 that crashed is replaced by the next one to
//! take the lock, and two launches at once make exactly one first 4x4. The names come from the
//! app id ([`app_id`](crate::app::app_id)), so debug and release runs never hand over to each
//! other, and a session with a runtime directory of its own (`just virtual`, a test) never reaches
//! the 4x4 of another. Without a runtime directory of the user's own, a launch runs on its own
//! ([`Claim::Standalone`]).
//!
//! The first 4x4 answers on a thread of its own ([`Listener::serve`]), so that a launch gets its
//! answer in microseconds whatever the window does. It turns each frame into the requests of its
//! arguments, then [`AddRequest::Raise`](crate::adding::AddRequest::Raise), which the window shows
//! in turn. That work is bounded there, so that the window only ever gets a queueful: the frame
//! takes at most 4 MiB with a working directory of at most 4096 bytes, and only the first
//! [`QUEUE_CAP`](x4_core::QUEUE_CAP) items of its arguments are told apart
//! ([`launch_requests`](crate::adding::launch_requests)); the others are counted. Nothing is added
//! by them: they only fill the add dialog's queue. The tokens are kept in the frame and logged:
//! gpui-pre 0.3.11 has no way to raise an open window with a token from another process, so the
//! window asks for its own.
//!
//! Unix only: elsewhere every launch runs on its own.

#[cfg(not(unix))]
mod stub;
#[cfg(unix)]
mod unix;
#[cfg(unix)]
mod wire;

use std::env;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(not(unix))]
use stub as imp;
#[cfg(not(unix))]
pub use stub::{Listener, Serving};
#[cfg(unix)]
use unix as imp;
#[cfg(unix)]
pub use unix::{Listener, Serving};

/// What [`claim`] made of this launch.
#[derive(Debug)]
pub enum Claim {
    /// This is the first 4x4 of the user: it holds the lock and has bound the socket. Later
    /// launches wait in the socket's queue until it [serves](Listener::serve) them.
    Primary(Listener),
    /// The running 4x4 took the arguments: this launch is done.
    HandedOff,
    /// This launch runs on its own and takes no arguments of later launches, for this reason.
    Standalone(StandaloneReason),
}

/// Why a launch runs on its own.
#[derive(Debug, thiserror::Error)]
pub enum StandaloneReason {
    /// `$XDG_RUNTIME_DIR` is not set, or not to an absolute path.
    #[error("there is no runtime directory: XDG_RUNTIME_DIR is unset or not an absolute path")]
    NoRuntimeDir,
    /// The runtime directory is not a directory, or others may use it.
    #[error("the runtime directory is not a directory of this user's alone (mode {mode:o})")]
    UnsafeDir {
        /// Its mode bits, without the file type.
        mode: u32,
    },
    /// The socket's path does not fit a Unix socket address: 107 bytes on Linux, 103 on macOS.
    #[error("the socket path would have {bytes} bytes, more than a Unix socket address holds")]
    PathTooLong {
        /// The length of the path in bytes.
        bytes: usize,
    },
    /// The frame cannot be written: an argument or a token holds a NUL byte, the frame would take
    /// more than 4 MiB, or its working directory is relative or longer than 4096 bytes (the frame
    /// of this process, [`Frame::of_this_process`], sends an unknown one instead of a longer one).
    #[error(
        "the arguments cannot be handed over: they hold a NUL byte, take more than 4 MiB, or come \
         with a working directory that is relative or longer than 4096 bytes"
    )]
    Unencodable,
    /// A file or socket operation failed.
    #[error("could not {what}: {source}")]
    Io {
        /// What was attempted.
        what: &'static str,
        /// Why it failed.
        source: io::Error,
    },
    /// The running 4x4 took the frame and did not answer in time: it hangs, or is stopped.
    #[error("the running 4x4 did not answer in time")]
    NoAnswer,
    /// The running 4x4 could not read the frame.
    #[error("the running 4x4 could not read the arguments")]
    Refused,
    /// No 4x4 took the frame or let go of the lock in time.
    #[error("no running 4x4 took the arguments or let go of its lock within {0:?}")]
    Timeout(Duration),
    /// There is no single instance on this system.
    #[error("one 4x4 per user is not supported on this system")]
    Unsupported,
}

/// The longest working directory a frame carries, in bytes: Linux's `PATH_MAX`. glibc's `getcwd`
/// gives longer ones too; [`Frame::of_this_process`] sends those as unknown.
pub(crate) const MAX_CWD_BYTES: usize = 4096;

/// What a launch hands over to the running 4x4: where it ran, its arguments, and what the desktop
/// gave it to bring a window to the front.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frame {
    /// The working directory of the launch, which its relative paths are relative to. Empty when
    /// it is unknown; its relative paths are then refused.
    pub cwd: PathBuf,
    /// `XDG_ACTIVATION_TOKEN`, the token of a Wayland launch.
    pub activation_token: Option<OsString>,
    /// `DESKTOP_STARTUP_ID`, the token of an X11 launch.
    pub desktop_startup_id: Option<OsString>,
    /// The arguments, as given, without the program's name.
    pub args: Vec<OsString>,
}

impl Frame {
    /// The frame of this process: its arguments, its working directory and its tokens. The tokens
    /// are read from the environment and left there, for gpui to take.
    ///
    /// A working directory longer than a frame carries, 4096 bytes, goes as unknown: the
    /// running 4x4 then refuses the relative arguments, and the magnet links and absolute paths
    /// still hand over, where the frame would otherwise not be written and this launch would open
    /// a window of its own.
    #[must_use]
    pub fn of_this_process() -> Self {
        let cwd = env::current_dir().unwrap_or_else(|error| {
            tracing::debug!(%error, "the working directory is unknown");
            PathBuf::new()
        });
        Self {
            cwd: frame_cwd(cwd),
            activation_token: env::var_os("XDG_ACTIVATION_TOKEN"),
            desktop_startup_id: env::var_os("DESKTOP_STARTUP_ID"),
            args: env::args_os().skip(1).collect(),
        }
    }
}

/// How long the two sides of a hand-over wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    /// How long a launch tries to reach the running 4x4, or to take the lock from one that quits.
    pub budget: Duration,
    /// How long a launch waits for the answer once its frame is on the way.
    pub reply: Duration,
    /// The pause between two tries.
    pub retry: Duration,
    /// How long the running 4x4 waits for the whole frame of a launch, so that a launch that
    /// never finishes it holds the listener no longer.
    pub read: Duration,
}

impl Default for Timeouts {
    /// 2 seconds to reach the running 4x4, 2 for its answer and 2 for a frame, and a try every
    /// 50 ms.
    fn default() -> Self {
        Self {
            budget: Duration::from_secs(2),
            reply: Duration::from_secs(2),
            retry: Duration::from_millis(50),
            read: Duration::from_secs(2),
        }
    }
}

/// What the running 4x4 answers a launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    /// The requests are queued for the window.
    Ok,
    /// The window takes no more requests, as this 4x4 is quitting; nothing was queued.
    Closing,
    /// The frame could not be read: it is too large, cut short, late, or not one of this version.
    Bad,
}

/// The working directory a frame carries for `cwd`, the working directory of this process: `cwd`
/// itself, or an empty one (unknown) when it is longer than [`MAX_CWD_BYTES`].
fn frame_cwd(cwd: PathBuf) -> PathBuf {
    let bytes = cwd.as_os_str().len();
    if bytes > MAX_CWD_BYTES {
        tracing::debug!(
            bytes,
            "the working directory is too long to hand over; it goes as unknown"
        );
        return PathBuf::new();
    }
    cwd
}

/// Claims the place of the first 4x4 of the user for the app `app_id`, in `runtime_dir` (the
/// user's `$XDG_RUNTIME_DIR`, `None` when there is none), for this process's arguments
/// ([`Frame::of_this_process`]) and with the default [`Timeouts`]. See [`claim_with`].
#[must_use]
pub fn claim(app_id: &str, runtime_dir: Option<&Path>) -> Claim {
    claim_with(
        app_id,
        runtime_dir,
        &Frame::of_this_process(),
        Timeouts::default(),
    )
}

/// Claims the place of the first 4x4 of the user for the app `app_id`, in `runtime_dir`, for the
/// launch `frame`:
/// - with the lock free, this is the first 4x4: [`Claim::Primary`], with the socket bound;
/// - with the lock taken, `frame` goes to the 4x4 that holds it: [`Claim::HandedOff`] once it
///   answers `ok`;
/// - otherwise [`Claim::Standalone`], logged as a warning: no usable runtime directory, a 4x4 that
///   does not answer once the frame is written, or that refuses it, and no 4x4 to take it or lock
///   to take within `timeouts.budget`.
#[must_use]
pub fn claim_with(
    app_id: &str,
    runtime_dir: Option<&Path>,
    frame: &Frame,
    timeouts: Timeouts,
) -> Claim {
    match imp::claim(app_id, runtime_dir, frame, timeouts) {
        Ok(claim) => {
            match &claim {
                Claim::Primary(_) => {
                    tracing::info!(
                        "this is the first 4x4 of the user; later launches hand over to it"
                    );
                }
                Claim::HandedOff => tracing::info!(
                    arguments = frame.args.len(),
                    "the arguments were handed over to the running 4x4"
                ),
                Claim::Standalone(_) => {}
            }
            claim
        }
        Err(reason) => {
            tracing::warn!(%reason, "running on its own; later launches do not hand over to it");
            Claim::Standalone(reason)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_working_directory_too_long_for_a_frame_goes_as_unknown() {
        let longest = PathBuf::from(format!("/{}", "d".repeat(MAX_CWD_BYTES - 1)));
        let longer = PathBuf::from(format!("/{}", "d".repeat(MAX_CWD_BYTES)));

        assert_eq!(frame_cwd(longest.clone()), longest);
        assert_eq!(frame_cwd(longer), PathBuf::new());
        assert_eq!(
            frame_cwd(PathBuf::from("/home/user")),
            Path::new("/home/user")
        );
    }
}
