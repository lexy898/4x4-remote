//! Loopback only: a one-shot HTTP server on 127.0.0.1 stands in for the daemon, and a closed port
//! of this machine for one that is not there.

use std::cell::RefCell;
use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use gpui_kit::TestAppContext;
use x4_core::ConnectionError;
use x4_rpc::Credentials;

use super::*;
use crate::test_log::records_of;

/// A made-up secret, which must never show.
const PASSWORD: &str = "hunter2";

fn settings(url: &str, credentials: Option<Credentials>) -> ConnectionSettings {
    ConnectionSettings::from_url(url, credentials).expect("a plain http address")
}

/// A port on this machine that nothing listens on.
fn closed_port() -> ConnectionSettings {
    settings("http://127.0.0.1:1/transmission/rpc", None)
}

/// How long a test waits for what the server received.
const WAIT: Duration = Duration::from_secs(10);

/// Accepts one connection, reads one request and answers it with a Transmission 4.1 `version`.
/// The receiver gets the request, head and body. A check that never connects leaves the thread
/// waiting, and the test fails on the receiver's timeout instead of hanging.
fn serve_a_version() -> (String, Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let url = format!(
        "http://{}/transmission/rpc",
        listener.local_addr().expect("its address")
    );
    let (received, server) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("the check connects");
        let request = read_request(&mut stream);
        let body = r#"{"jsonrpc":"2.0","result":{"version":"4.1.0 (abc)","rpc_version_semver":"6.0.0"},"id":1}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .expect("the answer is sent");
        received.send(request).expect("the test waits for it");
    });
    (url, server)
}

/// The request on `stream`, read up to the end of its body.
fn read_request(stream: &mut TcpStream) -> String {
    let mut raw = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let read = stream.read(&mut chunk).expect("the request is read");
        assert_ne!(read, 0, "the client closed before it sent a whole request");
        raw.extend_from_slice(chunk.get(..read).expect("within the chunk"));
        let text = String::from_utf8_lossy(&raw).into_owned();
        if let Some(head_end) = text.find("\r\n\r\n") {
            let length = text
                .get(..head_end)
                .expect("the head")
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().expect("a length"))
                })
                .unwrap_or(0);
            if raw.len() >= head_end + 4 + length {
                return text;
            }
        }
    }
}

#[test]
fn a_check_sends_the_credentials_and_reads_the_version() {
    let (url, server) = serve_a_version();
    let settings = settings(
        &url,
        Some(Credentials {
            username: "bob".to_owned(),
            password: PASSWORD.to_owned(),
        }),
    );

    let mut outcome = None;
    let records = records_of(|| outcome = Some(check_now(&settings)));
    let request = server
        .recv_timeout(WAIT)
        .expect("the check reached the server")
        .to_ascii_lowercase();

    assert_eq!(
        outcome,
        Some(CheckOutcome::Connected {
            version: "4.1.0 (abc)".to_owned()
        })
    );
    assert!(request.starts_with("post /transmission/rpc "), "{request}");
    // base64 of `bob:hunter2`.
    assert!(
        request.contains("\r\nauthorization: basic ym9iomh1bnrlcji=\r\n"),
        "{request}"
    );
    assert!(request.contains(r#""method":"session_get""#), "{request}");
    let server_label = settings.server_label();
    assert!(
        records
            .iter()
            .any(|record| record.field("server") == Some(server_label.as_str())),
        "the log names the server: {records:?}"
    );
    let logged = format!("{records:?}");
    assert!(!logged.contains(PASSWORD), "{logged}");
    assert!(!logged.contains("bob"), "no user name either: {logged}");
}

#[test]
fn a_check_of_a_closed_port_finds_it_unreachable() {
    assert_eq!(
        check_now(&closed_port()),
        CheckOutcome::Failed(ConnectionError::Unreachable)
    );
}

#[gpui_kit::test]
fn the_real_check_answers_on_the_window_thread(cx: &mut TestAppContext) {
    let check = real_server_check();
    let answer = Rc::new(RefCell::new(None));
    let slot = Rc::clone(&answer);
    cx.update(|cx| {
        let task = check(&closed_port(), cx);
        cx.spawn(async move |_| {
            slot.replace(Some(task.await));
        })
        .detach();
    });

    // The thread wakes the test's executor from outside it, so real time has to pass.
    cx.executor().allow_parking();
    let deadline = Instant::now() + WAIT;
    while answer.borrow().is_none() && Instant::now() < deadline {
        cx.run_until_parked();
        thread::sleep(Duration::from_millis(5));
    }

    assert_eq!(
        answer.take(),
        Some(CheckOutcome::Failed(ConnectionError::Unreachable))
    );
}

#[test]
fn the_setup_shows_no_password() {
    let setup = ConnectionSetup {
        settings: Some(settings(
            "http://nas:9091/transmission/rpc",
            Some(Credentials {
                username: "bob".to_owned(),
                password: PASSWORD.to_owned(),
            }),
        )),
        connector: Some(real_connector()),
        check: None,
        config_file: Some(PathBuf::from("/x/config.toml")),
    };

    for shown in [format!("{setup:?}"), format!("{setup:#?}")] {
        assert!(!shown.contains(PASSWORD), "{shown}");
        assert!(shown.contains("nas"), "{shown}");
        assert!(shown.contains("connector: true"), "{shown}");
        assert!(shown.contains("check: false"), "{shown}");
    }
    let nothing = format!("{:?}", ConnectionSetup::default());
    assert!(nothing.contains("settings: None"), "{nothing}");
    assert!(nothing.contains("connector: false"), "{nothing}");
}

#[test]
fn disconnected_channels_lead_nowhere() {
    let channels = ServiceChannels::disconnected();

    assert!(channels.events.is_closed(), "no event will come");
    assert!(
        channels.commands.send(Command::Refresh).is_err(),
        "no command arrives"
    );
    assert!(channels.handle.is_none());
}
