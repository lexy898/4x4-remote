//! A live test of the Connection dialog's Test against a real Transmission daemon, run only by
//! `just test-live` and `just test-live-legacy`.
//!
//! It makes the calls Test makes, through a client of its own: `session_get` of the version
//! fields, and a second one, in the legacy protocol, to a daemon older than Transmission 4.1 (and
//! one HTTP request more, as the daemon first hands out its session id). It only reads, and prints
//! only the version.

use x4_app::connection::check_now;
use x4_core::{CheckOutcome, ConnectionSettings};

#[test]
#[ignore = "live: needs a Transmission daemon at X4_LIVE_URL, run with just test-live"]
fn the_server_check_finds_the_live_daemon() {
    let url = std::env::var("X4_LIVE_URL").expect("X4_LIVE_URL names the daemon's RPC endpoint");
    // The problem never repeats the address, which might hold a password.
    let settings = ConnectionSettings::from_url(&url, None)
        .unwrap_or_else(|problem| panic!("X4_LIVE_URL cannot be used: {problem}"));

    let outcome = check_now(&settings);

    let CheckOutcome::Connected { version } = outcome else {
        panic!(
            "the daemon at {} could not be used: {outcome:?}",
            settings.server_label()
        );
    };
    println!("live test: the daemon is Transmission {version}");
    // 4x4 speaks to Transmission 3.00 and newer.
    assert!(
        version.starts_with("3.") || version.starts_with("4."),
        "{version}"
    );
}
