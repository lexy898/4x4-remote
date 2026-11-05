//! `4x4 --version` and `4x4 -V`, with the real binary: one line on stdout, exit 0, and nothing
//! else. No log directory is made, no lock or socket is taken, and nothing is handed to a 4x4 that
//! runs already, so a user can ask for the version of a build on any terminal, whatever else runs.
//!
//! The binary gets XDG and home directories in the test's temporary directory, a runtime directory
//! of its own, no display, and a hard time limit, after which it is killed: should it ever start
//! the application instead, it opens no window and is not left running.
#![cfg(unix)]

use std::fs::{self, File, Permissions};
use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;
use x4_app::app::DEVEL_APP_ID;
use x4_app::instance::{Claim, Frame, Timeouts, claim_with};

/// How long the binary may take to print the version and exit.
const BINARY_LIMIT: Duration = Duration::from_secs(20);

/// The directories a run of the binary is given, all in one temporary directory.
struct Session {
    _root: TempDir,
    runtime: PathBuf,
    home: PathBuf,
    config: PathBuf,
    state: PathBuf,
    data: PathBuf,
    cache: PathBuf,
}

/// What a run of the binary did.
struct Run {
    /// How it exited; `None` when it was killed for taking longer than [`BINARY_LIMIT`].
    status: Option<ExitStatus>,
    stdout: String,
    stderr: String,
}

impl Session {
    fn new() -> io::Result<Self> {
        let root = tempfile::tempdir()?;
        let dir = |name: &str| -> io::Result<PathBuf> {
            let dir = root.path().join(name);
            fs::create_dir_all(&dir)?;
            // 0700, as logind makes the runtime directory.
            fs::set_permissions(&dir, Permissions::from_mode(0o700))?;
            Ok(dir)
        };
        Ok(Self {
            runtime: dir("runtime")?,
            home: dir("home")?,
            config: dir("config")?,
            state: dir("state")?,
            data: dir("data")?,
            cache: dir("cache")?,
            _root: root,
        })
    }

    /// Runs the binary with `args`, in this session's directories and with nothing else of the
    /// environment, and waits for it.
    fn run(&self, args: &[&str]) -> io::Result<Run> {
        let out = self.home.join("stdout.txt");
        let err = self.home.join("stderr.txt");
        let mut child = Command::new(env!("CARGO_BIN_EXE_4x4"))
            .args(args)
            .current_dir(&self.home)
            .env_clear()
            .env("HOME", &self.home)
            .env("XDG_RUNTIME_DIR", &self.runtime)
            .env("XDG_CONFIG_HOME", &self.config)
            .env("XDG_STATE_HOME", &self.state)
            .env("XDG_DATA_HOME", &self.data)
            .env("XDG_CACHE_HOME", &self.cache)
            // The binary is a debug build, or has the development id from this.
            .env("X4_DEVEL", "1")
            // Were the logging to start, it would write here: the test wants none of it.
            .env("RUST_LOG", "x4_app=debug")
            .stdin(Stdio::null())
            .stdout(File::create(&out)?)
            .stderr(File::create(&err)?)
            .spawn()?;
        let status = wait_or_kill(&mut child, BINARY_LIMIT)?;
        let read = |path: &Path| fs::read(path).map(|bytes| String::from_utf8_lossy(&bytes).into());
        let run = Run {
            status,
            stdout: read(&out)?,
            stderr: read(&err)?,
        };
        // Their files are not the binary's: keep the home directory as the binary left it.
        fs::remove_file(out)?;
        fs::remove_file(err)?;
        Ok(run)
    }
}

/// Waits for `child` to exit within `limit`; kills it otherwise.
fn wait_or_kill(child: &mut Child, limit: Duration) -> io::Result<Option<ExitStatus>> {
    let began = Instant::now();
    while began.elapsed() < limit {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        thread::sleep(Duration::from_millis(20));
    }
    child.kill()?;
    child.wait()?;
    Ok(None)
}

/// The names in `dir`, sorted.
fn names(dir: &Path) -> io::Result<Vec<String>> {
    let mut names = fs::read_dir(dir)?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<io::Result<Vec<_>>>()?;
    names.sort();
    Ok(names)
}

/// The line `--version` prints.
fn version_output() -> String {
    format!("4x4 Remote {}\n", env!("CARGO_PKG_VERSION"))
}

#[test]
fn the_version_is_printed_and_nothing_starts() {
    let session = Session::new().expect("a temporary session");

    let run = session.run(&["--version"]).expect("the binary runs");

    assert!(
        run.status.is_some_and(|status| status.success()),
        "the binary exits with 0, not {:?}: {}",
        run.status,
        run.stderr
    );
    assert_eq!(run.stdout, version_output());
    assert_eq!(run.stderr, "", "nothing is logged");
    assert_eq!(
        names(&session.runtime).expect("listed"),
        Vec::<String>::new(),
        "no lock and no socket"
    );
    assert_eq!(
        names(&session.state).expect("listed"),
        Vec::<String>::new(),
        "no log directory"
    );
}

#[test]
fn the_version_is_printed_while_another_4x4_runs() {
    let session = Session::new().expect("a temporary session");
    // The test holds the lock and the socket of the first 4x4, as `tests/instance.rs` does.
    let primary = match claim_with(
        DEVEL_APP_ID,
        Some(&session.runtime),
        &Frame::default(),
        Timeouts::default(),
    ) {
        Claim::Primary(listener) => listener,
        other => panic!("the test holds the first 4x4, not {other:?}"),
    };
    let held = names(&session.runtime).expect("listed");
    assert_eq!(held.len(), 2, "the lock and the socket: {held:?}");
    let (frames, received) = mpsc::channel();
    let answering = thread::spawn(move || primary.answer_next(|frame| frames.send(frame).is_ok()));

    let run = session.run(&["-V"]).expect("the binary runs");

    assert!(
        run.status.is_some_and(|status| status.success()),
        "the binary exits with 0, not {:?}: {}",
        run.status,
        run.stderr
    );
    assert_eq!(run.stdout, version_output());
    assert_eq!(run.stderr, "", "nothing is logged");
    assert_eq!(
        received.recv_timeout(Duration::from_secs(1)),
        Err(RecvTimeoutError::Timeout),
        "the first 4x4 is handed nothing"
    );
    assert_eq!(
        names(&session.runtime).expect("listed"),
        held,
        "the lock and the socket stay as they were"
    );
    // The first 4x4 does answer: a launch of the test's own, which is the one it waited for.
    let launch = claim_with(
        DEVEL_APP_ID,
        Some(&session.runtime),
        &Frame::default(),
        Timeouts::default(),
    );
    assert!(matches!(launch, Claim::HandedOff), "{launch:?}");
    assert!(
        received.recv_timeout(Duration::from_secs(5)).is_ok(),
        "the first 4x4 gets the launch of the test"
    );
    answering
        .join()
        .expect("no panic")
        .expect("the launch connects");
}
