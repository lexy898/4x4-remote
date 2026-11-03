//! The single instance on Unix: the lock, the socket, the hand-over of a launch and the thread
//! that answers them in the first 4x4.
//!
//! The election follows the lock: `flock` on `<app id>.lock`, which the kernel lets go of when the
//! process ends, and which programs 4x4 starts do not inherit (std opens files close-on-exec). Its
//! holder alone removes a socket left behind and binds `<app id>.sock`, so that no launch ever
//! has to guess whether a socket is stale, and two launches never remove each other's. Both files
//! are the user's alone (0600) and sticky, which keeps them from a clean-up of the runtime
//! directory (XDG base directory spec); the directory itself must be the user's alone (0700).
//! Never an abstract socket: those ignore the runtime directory and its permissions, so a session
//! with a runtime directory of its own would reach the owner's 4x4.

use std::cell::OnceCell;
use std::fs::{self, File, OpenOptions, Permissions, TryLockError};
use std::io::{self, ErrorKind, Read as _, Write as _};
use std::net::Shutdown;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use super::{Claim, Frame, Reply, StandaloneReason, Timeouts, wire};
use crate::adding::{AddRequest, launch_requests};

/// The name of the thread that answers later launches.
const LISTENER_THREAD: &str = "instance-listener";
/// Its stack: it only reads a frame into the heap, tells its arguments apart and sends them on.
const LISTENER_STACK: usize = 64 * 1024;
/// The longest path of a Unix socket's address, without its NUL (unix(7)).
const MAX_SOCKET_PATH: usize = 107;
/// The mode of the lock and the socket: the user's alone, and sticky.
const FILE_MODE: u32 = 0o1600;
/// The pause after the listener could not take a connection, such as with too many files open.
const ACCEPT_PAUSE: Duration = Duration::from_millis(50);
/// How much is read from a socket at once.
const CHUNK: usize = 16 * 1024;

/// The first 4x4 of the user, once it holds the lock and has bound the socket. Later launches wait
/// in the socket's queue until it [serves](Listener::serve) them. Dropping it removes the socket,
/// then lets go of the lock.
#[derive(Debug)]
pub struct Listener {
    /// The bound socket.
    socket: Socket,
    /// The lock, and the path of the socket to remove.
    held: Held,
}

/// The first 4x4 of the user, while its thread answers later launches. Dropping it removes the
/// socket, then lets go of the lock, so that the next launch takes over at once.
#[derive(Debug)]
#[must_use = "dropping it lets the next launch take over"]
pub struct Serving {
    /// The lock, and the path of the socket to remove.
    _held: Held,
}

/// The socket of the first 4x4, and how long it waits for a frame.
#[derive(Debug)]
struct Socket {
    listener: UnixListener,
    read: Duration,
}

/// The lock of the first 4x4 and the path of its socket. Dropping it removes the socket while the
/// lock is still held, then lets go of the lock.
#[derive(Debug)]
struct Held {
    socket_path: PathBuf,
    _lock: File,
}

impl Drop for Held {
    fn drop(&mut self) {
        // The lock goes with the field after this, so the socket removed is still this 4x4's.
        match fs::remove_file(&self.socket_path) {
            Ok(()) => tracing::debug!("the socket of the single instance was removed"),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(%error, "the socket of the single instance could not be removed");
            }
        }
    }
}

impl Listener {
    /// Starts the thread `instance-listener`, which answers every later launch from now on. It
    /// sends the requests of each frame into `adds`: those of its arguments, as
    /// [`launch_requests`] tells them apart, then [`AddRequest::Raise`]. It answers
    /// [`Reply::Ok`] once they are sent, or [`Reply::Closing`] when nobody takes them any more.
    /// Returns the place of the first 4x4, held until dropped; the thread runs until the process
    /// ends.
    ///
    /// Call it once the GPUI platform exists: creating it changes the environment, which is only
    /// safe while no other thread may read it.
    ///
    /// # Errors
    ///
    /// The error of spawning the thread. The socket is then removed and the lock let go of, so
    /// that later launches take over.
    pub fn serve(self, adds: async_channel::Sender<AddRequest>) -> io::Result<Serving> {
        let Self { socket, held } = self;
        thread::Builder::new()
            .name(LISTENER_THREAD.to_owned())
            .stack_size(LISTENER_STACK)
            .spawn(move || socket.serve(&adds))?;
        tracing::debug!("later launches are answered");
        Ok(Serving { _held: held })
    }

    /// Answers the next launch that connects, on this thread: reads its frame, waiting no longer
    /// than [`Timeouts::read`] for it, and hands it to `deliver`. Answers [`Reply::Ok`] when
    /// `deliver` returns `true`, [`Reply::Closing`] when it returns `false`, and [`Reply::Bad`],
    /// without calling `deliver`, for a frame that is too large, cut short, late or not one of
    /// this version. Returns the answer.
    ///
    /// # Errors
    ///
    /// The error of accepting the connection.
    pub fn answer_next(&self, deliver: impl FnOnce(Frame) -> bool) -> io::Result<Reply> {
        self.socket.answer_next(deliver)
    }

    /// Answers the next launch that connects as the thread of [`Listener::serve`] does, but on
    /// this thread: sends its requests into `adds`.
    ///
    /// # Errors
    ///
    /// The error of accepting the connection.
    pub fn answer_next_into(&self, adds: &async_channel::Sender<AddRequest>) -> io::Result<Reply> {
        self.socket.answer_next_into(adds)
    }
}

impl Socket {
    /// Answers launches one at a time, as long as the process runs. Each holds it at most for the
    /// read timeout and the answer's write.
    fn serve(&self, adds: &async_channel::Sender<AddRequest>) {
        loop {
            if let Err(error) = self.answer_next_into(adds) {
                tracing::warn!(%error, "the connection of another launch could not be taken");
                thread::sleep(ACCEPT_PAUSE);
            }
        }
    }

    /// [`Listener::answer_next_into`].
    fn answer_next_into(&self, adds: &async_channel::Sender<AddRequest>) -> io::Result<Reply> {
        self.answer_next(|frame| forward(&frame, adds))
    }

    /// [`Listener::answer_next`].
    fn answer_next(&self, deliver: impl FnOnce(Frame) -> bool) -> io::Result<Reply> {
        let (mut stream, _) = self.listener.accept()?;
        let reply = match read_until_closed(&mut stream, wire::MAX_FRAME_BYTES, self.read) {
            Ok(bytes) => match wire::decode(&bytes) {
                Ok(frame) => {
                    if deliver(frame) {
                        Reply::Ok
                    } else {
                        Reply::Closing
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "another launch sent arguments that cannot be read");
                    Reply::Bad
                }
            },
            Err(error) => {
                tracing::warn!(%error, "another launch did not send its whole arguments");
                Reply::Bad
            }
        };
        // A few bytes, which the socket's buffer takes at once; the timeout is for a launch that
        // went away meanwhile.
        let answered = stream
            .set_write_timeout(Some(self.read))
            .and_then(|()| stream.write_all(wire::reply_bytes(reply)));
        if let Err(error) = answered {
            tracing::debug!(%error, "another launch did not wait for the answer");
        }
        Ok(reply)
    }
}

/// Sends the requests of `frame` into `adds`: whether they all went.
fn forward(frame: &Frame, adds: &async_channel::Sender<AddRequest>) -> bool {
    // The tokens of the launch: gpui-pre 0.3.10 cannot raise an open window with a token from
    // another process, so they are only kept for a later version.
    tracing::debug!(
        activation_token = ?frame.activation_token,
        desktop_startup_id = ?frame.desktop_startup_id,
        "the activation tokens of another launch"
    );
    requests_of(frame)
        .into_iter()
        .all(|request| adds.send_blocking(request).is_ok())
}

/// The requests of a frame handed over: those of its arguments ([`launch_requests`]), then
/// [`AddRequest::Raise`]. A frame without arguments only raises the window.
fn requests_of(frame: &Frame) -> Vec<AddRequest> {
    tracing::info!(
        arguments = frame.args.len(),
        "another launch handed its arguments over"
    );
    let mut requests = launch_requests(&frame.args, &frame.cwd);
    requests.push(AddRequest::Raise);
    requests
}

/// [`claim_with`](super::claim_with) on Unix. Never [`Claim::Standalone`]: the reason comes as the
/// error.
pub(super) fn claim(
    app_id: &str,
    runtime_dir: Option<&Path>,
    frame: &Frame,
    timeouts: Timeouts,
) -> Result<Claim, StandaloneReason> {
    let dir = runtime_dir
        .filter(|dir| dir.is_absolute())
        .ok_or(StandaloneReason::NoRuntimeDir)?;
    check_private(dir)?;
    let lock_path = dir.join(format!("{app_id}.lock"));
    let socket_path = dir.join(format!("{app_id}.sock"));
    let bytes = socket_path.as_os_str().len();
    if bytes > MAX_SOCKET_PATH {
        return Err(StandaloneReason::PathTooLong { bytes });
    }
    tracing::debug!(dir = %dir.display(), app_id, "claiming the single instance");
    let started = Instant::now();
    let encoded = OnceCell::new();
    // A frame went to a 4x4 that closed the connection without an answer. It is never sent
    // again; this launch takes the place of that 4x4 if it has gone.
    let mut unanswered = false;
    loop {
        let lock = open_lock(&lock_path).map_err(io_failed("open the lock file"))?;
        match lock.try_lock() {
            Ok(()) => return become_primary(lock, &lock_path, socket_path, timeouts.read),
            Err(TryLockError::WouldBlock) => {}
            Err(TryLockError::Error(source)) => {
                return Err(StandaloneReason::Io {
                    what: "lock the lock file",
                    source,
                });
            }
        }
        drop(lock);
        if unanswered {
            return Err(StandaloneReason::NoAnswer);
        }
        let frame_bytes = encoded
            .get_or_init(|| wire::encode(frame))
            .as_ref()
            .map_err(|error| {
                tracing::debug!(%error, "the arguments do not fit a frame");
                StandaloneReason::Unencodable
            })?;
        match UnixStream::connect(&socket_path) {
            Ok(stream) => match hand_over(stream, frame_bytes, timeouts.reply) {
                Ok(Reply::Ok) => return Ok(Claim::HandedOff),
                // It queued nothing, so the frame may go again, to it or to the next 4x4.
                Ok(Reply::Closing) => tracing::debug!("the running 4x4 is quitting"),
                Ok(Reply::Bad) => return Err(StandaloneReason::Refused),
                Err(error) if is_timeout(&error) => {
                    tracing::debug!(%error, "the running 4x4 did not answer");
                    return Err(StandaloneReason::NoAnswer);
                }
                Err(error) => {
                    tracing::debug!(%error, "the running 4x4 closed the connection without an answer");
                    unanswered = true;
                }
            },
            // Between taking the lock and binding the socket, or quitting.
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::NotFound | ErrorKind::ConnectionRefused
                ) =>
            {
                tracing::debug!(%error, "the running 4x4 does not listen yet, or any more");
            }
            Err(source) => {
                return Err(StandaloneReason::Io {
                    what: "connect to the running 4x4",
                    source,
                });
            }
        }
        if started.elapsed() >= timeouts.budget {
            return Err(StandaloneReason::Timeout(timeouts.budget));
        }
        thread::sleep(timeouts.retry);
    }
}

/// Checks that `dir` is a directory of the user's alone, as the XDG base directory spec requires
/// of the runtime directory: a symbolic link is not taken.
fn check_private(dir: &Path) -> Result<(), StandaloneReason> {
    let metadata = fs::symlink_metadata(dir).map_err(io_failed("look at the runtime directory"))?;
    let mode = metadata.mode() & 0o7777;
    if !metadata.is_dir() || mode & 0o077 != 0 {
        return Err(StandaloneReason::UnsafeDir { mode });
    }
    Ok(())
}

/// Opens the lock file at `path`, made the user's alone if it is new.
fn open_lock(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(FILE_MODE & 0o777)
        .open(path)
}

/// Takes the place of the first 4x4 with `lock`, held: removes a socket left at `socket_path`,
/// which no 4x4 can listen on any more, and binds a new one.
fn become_primary(
    lock: File,
    lock_path: &Path,
    socket_path: PathBuf,
    read: Duration,
) -> Result<Claim, StandaloneReason> {
    keep_private(lock_path);
    match fs::remove_file(&socket_path) {
        Ok(()) => tracing::debug!("the socket an earlier 4x4 left was removed"),
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(source) => {
            return Err(StandaloneReason::Io {
                what: "remove the socket an earlier 4x4 left",
                source,
            });
        }
    }
    let listener = UnixListener::bind(&socket_path).map_err(io_failed("bind the socket"))?;
    keep_private(&socket_path);
    Ok(Claim::Primary(Listener {
        socket: Socket { listener, read },
        held: Held {
            socket_path,
            _lock: lock,
        },
    }))
}

/// Makes the file at `path` the user's alone, and sticky. Only the directory's mode keeps others
/// out of it, so a failure is no reason to run on its own.
fn keep_private(path: &Path) {
    if let Err(error) = fs::set_permissions(path, Permissions::from_mode(FILE_MODE)) {
        tracing::debug!(%error, "the mode of a file of the single instance could not be set");
    }
}

/// Sends `frame` over `stream` and reads the answer, waiting `wait` for each.
fn hand_over(mut stream: UnixStream, frame: &[u8], wait: Duration) -> io::Result<Reply> {
    stream.set_write_timeout(Some(wait))?;
    stream.write_all(frame)?;
    stream.shutdown(Shutdown::Write)?;
    let answer = read_until_closed(&mut stream, wire::MAX_REPLY_BYTES, wait)?;
    wire::parse_reply(&answer).ok_or_else(|| {
        io::Error::new(
            ErrorKind::InvalidData,
            "the running 4x4 answered something else",
        )
    })
}

/// Reads from `stream` until the other side closes it: at most `limit` bytes, within `wait` in
/// all, so that neither a large frame nor a slow one holds the reader. Never reads more than one
/// byte past `limit`.
fn read_until_closed(stream: &mut UnixStream, limit: usize, wait: Duration) -> io::Result<Vec<u8>> {
    let started = Instant::now();
    let mut bytes = Vec::new();
    let mut chunk = vec![0_u8; limit.saturating_add(1).min(CHUNK)];
    loop {
        let left = wait.saturating_sub(started.elapsed());
        if left.is_zero() {
            return Err(ErrorKind::TimedOut.into());
        }
        stream.set_read_timeout(Some(left))?;
        let room = limit.saturating_add(1).saturating_sub(bytes.len());
        let into = chunk.get_mut(..room.min(CHUNK)).unwrap_or_default();
        match stream.read(into) {
            Ok(0) => return Ok(bytes),
            Ok(read) => {
                bytes.extend_from_slice(into.get(..read).unwrap_or_default());
                if bytes.len() > limit {
                    return Err(io::Error::new(
                        ErrorKind::FileTooLarge,
                        format!("more than {limit} bytes"),
                    ));
                }
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

/// Whether `error` says that the other side took too long.
fn is_timeout(error: &io::Error) -> bool {
    matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
}

/// The [`StandaloneReason::Io`] of an error of `what`.
fn io_failed(what: &'static str) -> impl FnOnce(io::Error) -> StandaloneReason {
    move |source| StandaloneReason::Io { what, source }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::FileTypeExt as _;
    use std::sync::{Arc, Barrier, mpsc};

    use x4_core::AddItem;
    use x4_core::launch::{RejectKind, RejectedArg};
    use x4_rpc::MagnetLink;

    use super::super::{claim_with, wire};
    use super::*;
    use crate::adding::AddRequests;
    use crate::app::{APP_ID, DEVEL_APP_ID};

    const APP: &str = "io.github.lexy898.FourByFour.Test";

    /// A runtime directory of the user's alone, 0700 as logind makes it: tempfile's follow the
    /// umask.
    fn runtime_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("a temporary directory");
        fs::set_permissions(dir.path(), Permissions::from_mode(0o700)).expect("chmod");
        dir
    }

    /// Short waits, so that the tests that wait for them are quick.
    fn quick() -> Timeouts {
        Timeouts {
            budget: Duration::from_millis(600),
            reply: Duration::from_millis(300),
            retry: Duration::from_millis(10),
            read: Duration::from_millis(300),
        }
    }

    /// The frame of a launch in `/home/user` with `args`.
    fn launch(args: &[&str]) -> Frame {
        Frame {
            cwd: "/home/user".into(),
            activation_token: Some("token".into()),
            desktop_startup_id: None,
            args: args.iter().map(Into::into).collect(),
        }
    }

    fn claim_in(dir: &Path, frame: &Frame, timeouts: Timeouts) -> Claim {
        claim_with(APP, Some(dir), frame, timeouts)
    }

    fn primary_in(dir: &Path) -> Listener {
        match claim_in(dir, &launch(&[]), quick()) {
            Claim::Primary(listener) => listener,
            other => panic!("the first claim is primary, not {other:?}"),
        }
    }

    /// Answers one launch on a thread of its own, and sends what it was handed into the receiver.
    fn answer_one(listener: Listener) -> mpsc::Receiver<(Reply, Option<Frame>)> {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut handed = None;
            let reply = listener
                .answer_next(|frame| {
                    handed = Some(frame);
                    true
                })
                .expect("a launch connects");
            sender.send((reply, handed)).expect("the test waits");
        });
        receiver
    }

    /// The bytes a raw client gets back for `bytes`, sent as a launch sends a frame.
    fn exchange(path: &Path, bytes: &[u8]) -> Vec<u8> {
        let mut stream = UnixStream::connect(path).expect("connected");
        stream.write_all(bytes).expect("written");
        stream.shutdown(Shutdown::Write).expect("half-closed");
        let mut answer = Vec::new();
        stream.read_to_end(&mut answer).expect("read");
        answer
    }

    #[test]
    fn the_first_claim_is_primary_and_the_second_hands_off() {
        let dir = runtime_dir();
        let listener = primary_in(dir.path());
        for name in [format!("{APP}.lock"), format!("{APP}.sock")] {
            let mode = fs::metadata(dir.path().join(&name))
                .expect("there")
                .permissions()
                .mode();
            assert_eq!(
                mode & 0o7777,
                0o1600,
                "{name}: the user's alone, and sticky"
            );
        }
        let answered = answer_one(listener);
        let second = launch(&["magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567"]);

        let claimed = claim_in(dir.path(), &second, Timeouts::default());

        assert!(matches!(claimed, Claim::HandedOff), "{claimed:?}");
        let (reply, handed) = answered
            .recv_timeout(Duration::from_secs(5))
            .expect("answered");
        assert_eq!(reply, Reply::Ok);
        assert_eq!(handed, Some(second), "the whole frame");
    }

    #[test]
    fn a_stale_socket_is_replaced_by_the_lock_holder() {
        let dir = runtime_dir();
        let path = dir.path().join(format!("{APP}.sock"));
        // Bound by a 4x4 that crashed: the file stays, and nobody listens.
        drop(UnixListener::bind(&path).expect("bound"));
        assert!(fs::metadata(&path).expect("left").file_type().is_socket());
        assert_eq!(
            UnixStream::connect(&path)
                .map_err(|error| error.kind())
                .err(),
            Some(ErrorKind::ConnectionRefused)
        );

        let listener = primary_in(dir.path());
        let answered = answer_one(listener);

        let claimed = claim_in(dir.path(), &launch(&["a.torrent"]), quick());
        assert!(matches!(claimed, Claim::HandedOff), "{claimed:?}");
        let (reply, _) = answered
            .recv_timeout(Duration::from_secs(5))
            .expect("answered");
        assert_eq!(reply, Reply::Ok);
    }

    #[test]
    fn ten_racing_claims_make_exactly_one_primary() {
        const LAUNCHES: usize = 10;
        let dir = runtime_dir();
        let start = Arc::new(Barrier::new(LAUNCHES));
        let launches: Vec<_> = (0..LAUNCHES)
            .map(|n| {
                let dir = dir.path().to_path_buf();
                let start = Arc::clone(&start);
                thread::spawn(move || {
                    let frame = launch(&[&format!("{n}.torrent")]);
                    let timeouts = Timeouts {
                        budget: Duration::from_secs(5),
                        ..Timeouts::default()
                    };
                    start.wait();
                    match claim_in(&dir, &frame, timeouts) {
                        Claim::Primary(listener) => {
                            let mut handed = Vec::new();
                            for _ in 1..LAUNCHES {
                                let reply = listener
                                    .answer_next(|frame| {
                                        handed.push(frame);
                                        true
                                    })
                                    .expect("a launch connects");
                                assert_eq!(reply, Reply::Ok);
                            }
                            Some(handed)
                        }
                        Claim::HandedOff => None,
                        Claim::Standalone(reason) => panic!("standalone: {reason}"),
                    }
                })
            })
            .collect();

        let outcomes: Vec<_> = launches
            .into_iter()
            .map(|launch| launch.join().expect("no launch panicked"))
            .collect();

        let primaries: Vec<_> = outcomes.into_iter().flatten().collect();
        assert_eq!(primaries.len(), 1, "exactly one primary");
        let mut handed: Vec<_> = primaries
            .into_iter()
            .flatten()
            .flat_map(|frame| frame.args)
            .collect();
        handed.sort();
        handed.dedup();
        assert_eq!(handed.len(), LAUNCHES - 1, "each of the others once");
    }

    #[test]
    fn oversized_or_garbage_frames_are_refused_without_blocking() {
        let dir = runtime_dir();
        let listener = primary_in(dir.path());
        let path = dir.path().join(format!("{APP}.sock"));
        let mut too_large = wire::encode(&launch(&[])).expect("a frame");
        too_large.resize(wire::MAX_FRAME_BYTES + 1, b'a');
        let garbage: [&[u8]; 3] = [
            b"GET / HTTP/1.1\r\n\r\n",
            b"4x4-open\x002\x00cwd=/\x00",
            &too_large,
        ];

        let good = wire::encode(&launch(&["a.torrent"])).expect("a frame");

        thread::scope(|scope| {
            for bytes in garbage {
                let client = scope.spawn(|| exchange(&path, bytes));
                let reply = listener
                    .answer_next(|_| panic!("never delivered"))
                    .expect("connected");
                assert_eq!(reply, Reply::Bad);
                assert_eq!(client.join().expect("no panic"), b"bad\n");
            }

            // A launch that connects and never finishes its frame holds the listener no longer
            // than the read timeout.
            let silent = UnixStream::connect(&path).expect("connected");
            let began = Instant::now();
            let reply = listener
                .answer_next(|_| panic!("never delivered"))
                .expect("connected");
            assert_eq!(reply, Reply::Bad);
            assert!(
                began.elapsed() < Duration::from_secs(2),
                "{:?}",
                began.elapsed()
            );
            drop(silent);

            // The next launch is answered as usual.
            let client = scope.spawn(|| exchange(&path, &good));
            let reply = listener.answer_next(|_| true).expect("connected");
            assert_eq!(reply, Reply::Ok);
            assert_eq!(client.join().expect("no panic"), b"ok\n");
        });
    }

    #[test]
    fn a_stalled_primary_makes_the_second_launch_standalone_without_a_resend() {
        let dir = runtime_dir();
        // It holds the lock and the socket, and never answers: stopped, or hung.
        let listener = primary_in(dir.path());
        let second = launch(&["a.torrent"]);
        let began = Instant::now();

        let claimed = claim_in(dir.path(), &second, quick());

        assert!(
            matches!(claimed, Claim::Standalone(StandaloneReason::NoAnswer)),
            "{claimed:?}"
        );
        assert!(
            began.elapsed() < quick().budget,
            "no retries after the frame: {:?}",
            began.elapsed()
        );
        // Exactly one connection waits, with the whole frame in it.
        listener
            .socket
            .listener
            .set_nonblocking(true)
            .expect("nonblocking");
        let (mut stream, _) = listener.socket.listener.accept().expect("one connection");
        stream.set_nonblocking(false).expect("blocking");
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).expect("the frame");
        assert_eq!(wire::decode(&bytes), Ok(second));
        assert_eq!(
            listener
                .socket
                .listener
                .accept()
                .map_err(|error| error.kind())
                .err(),
            Some(ErrorKind::WouldBlock),
            "never sent again"
        );
    }

    #[test]
    fn a_closing_primary_hands_over_to_the_next_launch_once_it_is_gone() {
        let dir = runtime_dir();
        let listener = primary_in(dir.path());
        let (sender, receiver) = mpsc::channel();
        let quitting = thread::spawn(move || {
            let reply = listener.answer_next(|frame| {
                sender.send(frame).expect("the test waits");
                false
            });
            // Then it quits, and lets go of the lock.
            drop(listener);
            reply
        });

        let claimed = claim_in(dir.path(), &launch(&["a.torrent"]), quick());

        assert_eq!(
            quitting.join().expect("no panic").expect("connected"),
            Reply::Closing
        );
        assert!(
            receiver.recv().is_ok(),
            "the frame came, and was not queued"
        );
        let Claim::Primary(_next) = claimed else {
            panic!("the next launch takes over, not {claimed:?}");
        };
    }

    /// Takes the next connection of `listener` without answering: reads the whole frame.
    fn take_frame(listener: &Listener) -> (UnixStream, Frame) {
        let (mut stream, _) = listener.socket.listener.accept().expect("a connection");
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).expect("the frame");
        (stream, wire::decode(&bytes).expect("a frame"))
    }

    #[test]
    fn a_launch_whose_4x4_ends_without_an_answer_takes_its_place_without_a_resend() {
        let dir = runtime_dir();
        let listener = primary_in(dir.path());
        let dying = thread::spawn(move || {
            let (stream, frame) = take_frame(&listener);
            // It ends after reading: the lock goes, then the connection, unanswered.
            drop(listener);
            drop(stream);
            frame
        });
        let second = launch(&["a.torrent"]);

        let claimed = claim_in(dir.path(), &second, quick());

        assert_eq!(dying.join().expect("no panic"), second);
        let Claim::Primary(next) = claimed else {
            panic!("the launch takes the place of the 4x4 that ended, not {claimed:?}");
        };
        next.socket
            .listener
            .set_nonblocking(true)
            .expect("nonblocking");
        assert_eq!(
            next.socket
                .listener
                .accept()
                .map_err(|error| error.kind())
                .err(),
            Some(ErrorKind::WouldBlock),
            "nobody connects to it"
        );
    }

    #[test]
    fn a_4x4_that_closes_without_an_answer_and_runs_on_makes_the_launch_standalone() {
        let dir = runtime_dir();
        let listener = primary_in(dir.path());
        let second = launch(&["a.torrent"]);

        let claimed = thread::scope(|scope| {
            let launch = scope.spawn(|| claim_in(dir.path(), &second, quick()));
            let (stream, frame) = take_frame(&listener);
            assert_eq!(frame, second);
            drop(stream);
            launch.join().expect("no panic")
        });

        assert!(
            matches!(claimed, Claim::Standalone(StandaloneReason::NoAnswer)),
            "{claimed:?}"
        );
        listener
            .socket
            .listener
            .set_nonblocking(true)
            .expect("nonblocking");
        assert_eq!(
            listener
                .socket
                .listener
                .accept()
                .map_err(|error| error.kind())
                .err(),
            Some(ErrorKind::WouldBlock),
            "never sent again"
        );
    }

    #[test]
    fn no_runtime_dir_is_standalone() {
        let frame = launch(&[]);

        for dir in [None, Some(Path::new("run/user/1000"))] {
            let claimed = claim_with(APP, dir, &frame, quick());
            assert!(
                matches!(claimed, Claim::Standalone(StandaloneReason::NoRuntimeDir)),
                "{dir:?}: {claimed:?}"
            );
        }
        let gone = runtime_dir().path().join("gone");
        assert!(matches!(
            claim_in(&gone, &frame, quick()),
            Claim::Standalone(StandaloneReason::Io { .. })
        ));
    }

    #[test]
    fn a_runtime_dir_others_may_use_is_standalone() {
        let dir = runtime_dir();
        fs::set_permissions(dir.path(), Permissions::from_mode(0o755)).expect("chmod");

        let claimed = claim_in(dir.path(), &launch(&[]), quick());

        assert!(
            matches!(
                claimed,
                Claim::Standalone(StandaloneReason::UnsafeDir { mode: 0o755 })
            ),
            "{claimed:?}"
        );
        assert_eq!(fs::read_dir(dir.path()).expect("listed").count(), 0);

        let file = runtime_dir();
        let not_a_dir = file.path().join("file");
        fs::write(&not_a_dir, b"").expect("written");
        fs::set_permissions(&not_a_dir, Permissions::from_mode(0o600)).expect("chmod");
        assert!(matches!(
            claim_in(&not_a_dir, &launch(&[]), quick()),
            Claim::Standalone(StandaloneReason::UnsafeDir { .. })
        ));
    }

    #[test]
    fn a_socket_path_too_long_for_an_address_is_standalone() {
        let dir = runtime_dir();
        let deep = dir.path().join("d".repeat(100));
        fs::create_dir(&deep).expect("made");
        fs::set_permissions(&deep, Permissions::from_mode(0o700)).expect("chmod");

        let claimed = claim_in(&deep, &launch(&[]), quick());

        assert!(
            matches!(
                claimed,
                Claim::Standalone(StandaloneReason::PathTooLong { bytes }) if bytes > 107
            ),
            "{claimed:?}"
        );
    }

    #[test]
    fn debug_and_release_names_never_meet() {
        let dir = runtime_dir();
        let frame = launch(&[]);

        let release = claim_with(APP_ID, Some(dir.path()), &frame, quick());
        let debug = claim_with(DEVEL_APP_ID, Some(dir.path()), &frame, quick());

        assert!(matches!(release, Claim::Primary(_)), "{release:?}");
        assert!(matches!(debug, Claim::Primary(_)), "{debug:?}");
        let mut names: Vec<_> = fs::read_dir(dir.path())
            .expect("listed")
            .map(|entry| entry.expect("an entry").file_name())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "io.github.lexy898.FourByFour.Devel.lock",
                "io.github.lexy898.FourByFour.Devel.sock",
                "io.github.lexy898.FourByFour.lock",
                "io.github.lexy898.FourByFour.sock",
            ]
        );
    }

    #[test]
    fn the_first_4x4_removes_its_socket_when_it_stops_and_keeps_the_lock_file() {
        let dir = runtime_dir();
        let socket = dir.path().join(format!("{APP}.sock"));
        let lock = dir.path().join(format!("{APP}.lock"));
        let serving = primary_in(dir.path())
            .serve(AddRequests::new().sender)
            .expect("serving");
        assert!(socket.exists());

        drop(serving);

        assert!(!socket.exists(), "removed");
        assert!(lock.exists(), "kept");
        assert!(matches!(
            claim_in(dir.path(), &launch(&[]), quick()),
            Claim::Primary(_)
        ));
    }

    #[test]
    fn a_handed_over_frame_asks_for_its_items_then_a_raise() {
        let dir = runtime_dir();
        let adds = AddRequests::new();
        let _serving = primary_in(dir.path())
            .serve(adds.sender.clone())
            .expect("serving");
        let link = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567";

        // Told apart on the listener's own small stack, the URL parser of a file URI included.
        let claimed = claim_in(
            dir.path(),
            &launch(&[
                link,
                "Show.torrent",
                "https://example.org/a.torrent",
                "file:///srv/Film%20%C3%BC.torrent",
            ]),
            quick(),
        );

        assert!(matches!(claimed, Claim::HandedOff), "{claimed:?}");
        let requests: Vec<_> = std::iter::from_fn(|| adds.receiver.try_recv().ok()).collect();
        assert_eq!(
            requests,
            [
                AddRequest::Items(vec![
                    AddItem::Magnet(Some(MagnetLink::parse(link).expect("a link"))),
                    AddItem::File("/home/user/Show.torrent".into()),
                    AddItem::File("/srv/Film ü.torrent".into()),
                ]),
                AddRequest::Rejected(vec![RejectedArg {
                    kind: RejectKind::Url
                }]),
                AddRequest::Raise,
            ]
        );
    }

    #[test]
    fn a_launch_without_arguments_only_raises_the_window() {
        let dir = runtime_dir();
        let adds = AddRequests::new();
        let _serving = primary_in(dir.path())
            .serve(adds.sender.clone())
            .expect("serving");

        let claimed = claim_in(dir.path(), &launch(&[]), quick());

        assert!(matches!(claimed, Claim::HandedOff), "{claimed:?}");
        assert_eq!(adds.receiver.try_recv(), Ok(AddRequest::Raise));
        assert!(adds.receiver.is_empty());
    }

    #[test]
    fn a_window_that_stopped_listening_makes_the_listener_say_closing() {
        let dir = runtime_dir();
        let adds = AddRequests::new();
        let AddRequests { sender, receiver } = adds;
        drop(receiver);
        let _serving = primary_in(dir.path()).serve(sender).expect("serving");
        let began = Instant::now();

        let claimed = claim_in(dir.path(), &launch(&["a.torrent"]), quick());

        // It tries again until the budget is spent, as the lock is still held.
        assert!(
            matches!(claimed, Claim::Standalone(StandaloneReason::Timeout(_))),
            "{claimed:?}"
        );
        assert!(began.elapsed() >= quick().budget);
    }
}
