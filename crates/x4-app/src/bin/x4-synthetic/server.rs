//! The demo's server, for the Connection dialog: the made-up server the window is set to, a
//! connector that starts the synthetic daemon's service again whatever server the dialog saves,
//! and a check that answers with the synthetic daemon's version. Test and Save can be tried, and
//! a switch replaces the service as in `4x4`, without a network.
//!
//! There is no config path: Save switches and writes no file.

use std::io;
use std::rc::Rc;

use anyhow::Context as _;
use gpui_kit::Task;
use x4_app::connection::{ConnectionSetup, Connector, ServerCheck, ServiceChannels};
use x4_core::service::{self, ServiceHandle};
use x4_core::{CheckOutcome, ConnectionSettings, EngineConfig};
use x4_rpc::Client;

use crate::countries;
use crate::daemon::{self, Recipe};

/// The server the window says it is set to: a name that never resolves (`.invalid` is reserved
/// for that, RFC 2606), on Transmission's own port and path. Nothing is ever sent to it.
const SERVER: &str = "http://synthetic.invalid:9091/transmission/rpc";

/// How the demo's window reaches daemons: set to [`SERVER`], switching to the synthetic daemon
/// again whatever server is saved, testing a server with the synthetic daemon's answer, and saving
/// nowhere. The daemon is the one `recipe` makes, after a switch as well.
///
/// # Errors
///
/// None in practice: an error says that [`SERVER`] is not an address 4x4 takes.
pub(crate) fn setup(recipe: Recipe) -> anyhow::Result<ConnectionSetup> {
    let settings =
        ConnectionSettings::from_url(SERVER, None).context("the demo's made-up server")?;
    Ok(ConnectionSetup {
        settings: Some(settings),
        connector: Some(connector(recipe)),
        check: Some(check()),
        config_file: None,
    })
}

/// Starts the service over a new synthetic daemon made by `recipe`, with the demo's own country
/// databases.
///
/// # Errors
///
/// The system's error when no thread can be started.
pub(crate) fn spawn(recipe: Recipe) -> io::Result<ServiceHandle> {
    let client = Client::new(recipe.daemon());
    service::spawn(client, EngineConfig::default(), Box::new(countries))
}

/// Starts the service over a new synthetic daemon for any server, as `4x4` starts one for the
/// server saved: a switch drops the old daemon with its service, and the new one starts afresh, made
/// by the same `recipe`, so that a showcase stays one.
fn connector(recipe: Recipe) -> Connector {
    Rc::new(move |settings| {
        tracing::info!(
            server = %settings.server_label(),
            "connecting; the synthetic daemon answers for any server"
        );
        spawn(recipe).map(ServiceChannels::from)
    })
}

/// Answers a test of any server at once, with the synthetic daemon's version.
fn check() -> ServerCheck {
    Rc::new(|settings, _| {
        tracing::info!(
            server = %settings.server_label(),
            "testing the connection; the synthetic daemon answers for any server"
        );
        Task::ready(CheckOutcome::Connected {
            version: daemon::VERSION.to_owned(),
        })
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::thread;
    use std::time::{Duration, Instant};

    use gpui_kit::TestAppContext;
    use x4_core::Event;

    use super::*;
    use crate::daemon::{Activity, SHOWCASE_NAMES, TORRENTS};

    /// How long a test waits for the service's first list.
    const WAIT: Duration = Duration::from_secs(10);

    /// The default demo, with idle torrents.
    const STATIC: Recipe = Recipe {
        activity: Activity::Static,
        showcase: false,
    };

    /// A server the dialog could save, other than the demo's own.
    fn elsewhere() -> ConnectionSettings {
        ConnectionSettings::from_url("http://192.0.2.7:9092/rpc", None).expect("an address")
    }

    #[test]
    fn the_demo_saves_its_server_nowhere() {
        let setup = setup(STATIC).expect("the made-up server is an address");

        assert_eq!(setup.config_file, None, "the demo writes no config.toml");
        assert_eq!(
            setup
                .settings
                .as_ref()
                .map(ConnectionSettings::server_label),
            Some("synthetic.invalid:9091".to_owned())
        );
        assert!(setup.connector.is_some(), "the demo switches");
        assert!(setup.check.is_some(), "the demo tests");
    }

    /// How many torrents the first list of the service the connector of `recipe` starts holds, for
    /// a server the dialog could save.
    fn torrents_after_a_switch(recipe: Recipe) -> usize {
        let setup = setup(recipe).expect("the made-up server is an address");
        let connector = setup.connector.expect("the demo switches");

        let channels = connector(&elsewhere()).expect("the service thread starts");

        assert!(
            channels.handle.is_some(),
            "a thread runs behind the channels"
        );
        let deadline = Instant::now() + WAIT;
        loop {
            match channels.events.try_recv() {
                Ok(Event::Snapshot(snapshot)) if snapshot.listed => break snapshot.torrents.len(),
                Ok(_) => {}
                Err(async_channel::TryRecvError::Empty) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("no list came: {error}"),
            }
        }
    }

    #[test]
    fn the_connector_starts_the_synthetic_daemon_whatever_the_server() {
        let listed = torrents_after_a_switch(Recipe {
            activity: Activity::Static,
            showcase: false,
        });

        assert_eq!(listed, usize::try_from(TORRENTS).expect("a count"));
    }

    #[test]
    fn a_restarted_showcase_service_is_a_showcase_again() {
        let listed = torrents_after_a_switch(Recipe {
            activity: Activity::Static,
            showcase: true,
        });

        assert_eq!(listed, SHOWCASE_NAMES.len());
    }

    #[gpui_kit::test]
    fn the_check_answers_with_the_synthetic_version(cx: &mut TestAppContext) {
        let setup = setup(STATIC).expect("the made-up server is an address");
        let check = setup.check.expect("the demo tests");
        let answer = Rc::new(RefCell::new(None));
        let slot = Rc::clone(&answer);

        cx.update(|cx| {
            let task = check(&elsewhere(), cx);
            cx.spawn(async move |_| {
                slot.replace(Some(task.await));
            })
            .detach();
        });
        cx.run_until_parked();

        assert_eq!(
            answer.take(),
            Some(CheckOutcome::Connected {
                version: daemon::VERSION.to_owned()
            })
        );
    }
}
