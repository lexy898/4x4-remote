//! One 4x4 per user, across processes and up to the window: the real `4x4` binary, started as a
//! later launch, hands its arguments to a first 4x4 the test holds and exits; and a launch handed
//! over to a running window opens the add dialog and raises the window.
//!
//! A real `4x4` never becomes the first 4x4 here, as it would open a window and poll the daemon of
//! its config. The test holds the lock and the bound socket in a runtime directory of its own
//! before it starts the binary. The binary also gets a config whose daemon is a dead loopback
//! address, XDG and home directories in the test's temporary directory, no display, and a hard
//! time limit, after which it is killed.
#![cfg(unix)]

use std::fs::{self, File, Permissions};
use std::io;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, TestAppContext, VisualTestContext};
use tempfile::TempDir;
use x4_app::app::{self, DEVEL_APP_ID, ServiceStart, Startup};
use x4_app::instance::{Claim, Frame, Reply, Timeouts, claim_with};

/// How long the binary may take to hand over and exit.
const BINARY_LIMIT: Duration = Duration::from_secs(20);

/// A temporary directory for the files of a test. On macOS it is made in `/tmp`: the user's own
/// temporary directory there, `/var/folders/…/T/`, is so long that the socket of a runtime
/// directory below it would not fit the 104 bytes of a socket address.
fn temp_root() -> io::Result<TempDir> {
    if cfg!(target_os = "macos") {
        tempfile::tempdir_in("/tmp")
    } else {
        tempfile::tempdir()
    }
}

/// The directory `name` in `parent`, of the user's alone: 0700, as logind makes the runtime
/// directory.
#[expect(
    clippy::expect_used,
    reason = "a test helper: a directory that cannot be made fails the test"
)]
fn private_dir(parent: &Path, name: &str) -> PathBuf {
    let dir = parent.join(name);
    fs::create_dir_all(&dir).expect("made");
    fs::set_permissions(&dir, Permissions::from_mode(0o700)).expect("chmod");
    dir
}

/// The first 4x4 of the app `app_id` in `runtime`, held by the test.
#[expect(
    clippy::panic,
    reason = "a test helper: a test that cannot hold the first 4x4 must not go on"
)]
fn hold_primary(app_id: &str, runtime: &Path) -> x4_app::instance::Listener {
    match claim_with(
        app_id,
        Some(runtime),
        &Frame::default(),
        Timeouts::default(),
    ) {
        Claim::Primary(listener) => listener,
        other => panic!("the test holds the first 4x4, not {other:?}"),
    }
}

/// Waits for `child` to exit within `limit`; kills it otherwise.
#[expect(
    clippy::expect_used,
    reason = "a test helper: a child that cannot be waited for fails the test"
)]
fn wait_or_kill(child: &mut Child, limit: Duration) -> Option<ExitStatus> {
    let began = Instant::now();
    while began.elapsed() < limit {
        if let Some(status) = child.try_wait().expect("waited") {
            return Some(status);
        }
        thread::sleep(Duration::from_millis(20));
    }
    child.kill().expect("killed");
    child.wait().expect("reaped");
    None
}

/// A made-up magnet link named "Test".
fn magnet() -> String {
    format!(
        "magnet:?xt=urn:btih:{}&dn=Test",
        "3f9a1c0e5b7d2468ace013579bdf2468ace01357"
    )
}

#[test]
fn the_real_binary_hands_off_to_a_test_held_primary() {
    let root = temp_root().expect("a temporary directory");
    // The directory as the system names it, with its symbolic links resolved (on macOS, `/tmp` is
    // one): the working directory the binary sends is that one.
    let base = fs::canonicalize(root.path()).expect("a real path");
    let runtime = private_dir(&base, "runtime");
    let home = private_dir(&base, "home");
    let config = private_dir(&base, "config");
    let state = private_dir(&base, "state");
    let data = private_dir(&base, "data");
    let cache = private_dir(&base, "cache");
    let work = private_dir(&base, "work");
    // Should the hand-off ever fail, the binary would run on its own: against a daemon that is
    // not there, and without a display to open its window on.
    fs::create_dir_all(config.join("4x4")).expect("made");
    fs::write(
        config.join("4x4/config.toml"),
        "[connection]\nurl = \"http://127.0.0.1:1/transmission/rpc\"\n",
    )
    .expect("written");
    // The binary is a debug build, or has the development id from X4_DEVEL=1 below.
    let primary = hold_primary(DEVEL_APP_ID, &runtime);
    let (frames, received) = mpsc::channel();
    thread::spawn(move || {
        let reply = primary.answer_next(|frame| frames.send(frame).is_ok());
        assert_eq!(reply.expect("the binary connects"), Reply::Ok);
    });
    let log = base.join("stderr.log");
    let link = magnet();

    let mut child = Command::new(env!("CARGO_BIN_EXE_4x4"))
        .args([link.as_str(), "--", "downloads/Show.torrent"])
        .current_dir(&work)
        .env_clear()
        .env("HOME", &home)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_STATE_HOME", &state)
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CACHE_HOME", &cache)
        .env("X4_DEVEL", "1")
        .env("XDG_ACTIVATION_TOKEN", "token-of-the-test")
        .env("RUST_LOG", "x4_app=debug")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(File::create(&log).expect("a log file"))
        .spawn()
        .expect("the binary starts");
    let status = wait_or_kill(&mut child, BINARY_LIMIT);
    let output = fs::read_to_string(&log).unwrap_or_default();

    assert!(
        status.is_some_and(|status| status.success()),
        "the binary exits with 0, not {status:?}:\n{output}"
    );
    let frame = received
        .recv_timeout(Duration::from_secs(5))
        .expect("the held primary received the frame");
    assert_eq!(
        frame,
        Frame {
            cwd: work,
            activation_token: Some("token-of-the-test".into()),
            desktop_startup_id: None,
            args: vec![link.into(), "--".into(), "downloads/Show.torrent".into()],
        },
        "{output}"
    );
    assert!(
        output.contains("the arguments were handed over to the running 4x4"),
        "{output}"
    );
}

#[gpui_kit::test]
fn a_handed_off_magnet_opens_the_dialog_and_raises_the_window(cx: &mut TestAppContext) {
    const APP: &str = "io.github.lexy898.FourByFour.Test";
    let root = temp_root().expect("a temporary directory");
    let runtime = private_dir(root.path(), "runtime");
    let primary = hold_primary(APP, &runtime);
    let startup = Startup::from_state_file(
        ServiceStart::Failed(io::Error::other("no service in this test")),
        None,
    );
    let adds = startup.adds.sender.clone();
    cx.update(|cx| cx.set_reduce_motion(true));
    let (handle, _view) = cx
        .update(|cx| {
            app::init(cx);
            app::launch(startup, cx)
        })
        .expect("the window opens");
    VisualTestContext::from_window(handle, cx).deactivate_window();
    cx.run_until_parked();
    let link = magnet();
    let second = Frame {
        cwd: "/home/user".into(),
        args: vec![link.clone().into()],
        ..Frame::default()
    };

    // The second launch waits for the answer on a thread of its own. The first 4x4 answers on the
    // test's thread, as its listener's thread would: the test scheduler lets no other thread wake
    // the window.
    let second_launch =
        thread::spawn(move || claim_with(APP, Some(&runtime), &second, Timeouts::default()));
    let reply = primary
        .answer_next_into(&adds)
        .expect("the launch connects");
    let claimed = second_launch.join().expect("no panic");
    assert_eq!(reply, Reply::Ok);
    assert!(matches!(claimed, Claim::HandedOff), "{claimed:?}");
    cx.run_until_parked();

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find("add-title").label(), Some("Add torrent"));
        assert_eq!(window.find("add-name").label(), Some("Test"));
        assert_eq!(window.find("add-link").value(), Some(link.as_str()));
        assert!(window.is_window_active(), "raised");
    })
    .expect("the window is open");
}
