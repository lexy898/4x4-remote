//! How the window reaches a daemon: the service it starts for a server, and the one-off check
//! of a server that the Connection dialog's Test makes.
//!
//! A [`Connector`] starts the service for a server and hands the window its
//! [`ServiceChannels`]; the window calls it again to switch to another server, without a restart
//! ([`MainView::switch_to`](crate::main_view::MainView::switch_to)). A [`ServerCheck`] asks a
//! server for its version once, through a client of its own, off the UI thread. The app's own
//! are [`real_connector`] and [`real_server_check`]; a demo or a test hands the window others,
//! in its [`ConnectionSetup`].
//!
//! Neither logs the address with credentials (the address 4x4 builds never holds them) nor the
//! password: the log names the server as `host:port`.

use std::fmt;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::{App, Task};
use x4_core::service::{self, ServiceHandle};
use x4_core::{CheckOutcome, Command, ConnectionSettings, EngineConfig, Event};
use x4_rpc::{Client, DEFAULT_TIMEOUT, UreqTransport};

use crate::geoip;
use crate::own_thread::on_a_thread;

/// The name of the thread a check runs on.
const CHECK_THREAD: &str = "x4-check";

/// The stack of that thread: Rust's default, as the service thread has, which makes the same
/// call. Resolving a host name goes through the system's resolver, whose modules may take more
/// stack than 4x4's own code; a stack is mapped, not filled, so only what is used takes memory.
const CHECK_STACK: usize = 2 * 1024 * 1024;

/// The window's end of a service: the events it sends, where commands go, and the handle that
/// keeps its thread running.
#[derive(Debug)]
pub struct ServiceChannels {
    /// The events of the service, oldest first.
    pub events: async_channel::Receiver<Event>,
    /// Where commands for the service go.
    pub commands: crossbeam_channel::Sender<Command>,
    /// The handle of the thread behind the channels; dropping it stops the thread. `None` when
    /// there is no thread, as in tests, whose channels are all there is.
    pub handle: Option<ServiceHandle>,
}

impl ServiceChannels {
    /// Channels with nobody at the other end, for a window with no service: the events are
    /// closed at once and the commands go nowhere.
    pub(crate) fn disconnected() -> Self {
        let (_, events) = async_channel::unbounded();
        let (commands, _) = crossbeam_channel::unbounded();
        Self {
            events,
            commands,
            handle: None,
        }
    }
}

impl From<ServiceHandle> for ServiceChannels {
    /// The channels of the service thread of `handle`, which they keep running.
    fn from(handle: ServiceHandle) -> Self {
        Self {
            events: handle.events().clone(),
            commands: handle.commands(),
            handle: Some(handle),
        }
    }
}

/// Starts the service for a server and gives its channels, or the system's error when no thread
/// can be started. It only starts a thread: the first snapshot says that the service connects.
pub type Connector = Rc<dyn Fn(&ConnectionSettings) -> io::Result<ServiceChannels>>;

/// Asks a server once whether 4x4 can use it, off the UI thread; the task gives the answer.
/// Dropping the task lets go of the answer.
pub type ServerCheck = Rc<dyn Fn(&ConnectionSettings, &App) -> Task<CheckOutcome>>;

/// How the window reaches daemons, beyond the service it starts with: the server it is set to,
/// how it starts a service for another one, how it checks one, and where it saves the choice.
///
/// The default, all `None`, is a window with no server that cannot switch, as the windows of
/// tests are.
#[derive(Clone, Default)]
pub struct ConnectionSetup {
    /// The server the window is set to; `None` when no server is set.
    pub settings: Option<ConnectionSettings>,
    /// Starts the service for another server; `None` when the window cannot switch.
    pub connector: Option<Connector>,
    /// Checks a server for the Connection dialog's Test; `None` when it cannot.
    pub check: Option<ServerCheck>,
    /// The `config.toml` the Connection dialog saves the server to; `None` when it saves nowhere
    /// (no home directory, the demo) and only switches.
    pub config_file: Option<PathBuf>,
}

impl fmt::Debug for ConnectionSetup {
    /// The settings (whose own `Debug` hides the password), the file, and whether there is a
    /// connector and a check.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionSetup")
            .field("settings", &self.settings)
            .field("connector", &self.connector.is_some())
            .field("check", &self.check.is_some())
            .field("config_file", &self.config_file)
            .finish()
    }
}

/// The app's connector: a service thread with a client of its own for each server, over HTTP
/// with the server's credentials, which reads the installed country files for the peers.
#[must_use]
pub fn real_connector() -> Connector {
    Rc::new(|settings| spawn_service(settings).map(ServiceChannels::from))
}

/// Starts the service thread against `settings`, with the app's engine configuration and the
/// installed country files.
///
/// # Errors
///
/// The system's error when no thread can be started.
pub(crate) fn spawn_service(settings: &ConnectionSettings) -> io::Result<ServiceHandle> {
    tracing::info!(server = %settings.server_label(), "connecting");
    // Spawning only starts the thread; the window opens, or switches, at once.
    service::spawn(
        Client::new(transport(settings)),
        EngineConfig::default(),
        Box::new(geoip::open_installed),
    )
}

/// The app's check: [`check_now`] on a thread of its own, named `x4-check`, which ends with it. A
/// thread that cannot be started, or ends without an answer, gives [`CheckOutcome::NotRun`].
#[must_use]
pub fn real_server_check() -> ServerCheck {
    Rc::new(|settings, cx| {
        let settings = settings.clone();
        on_a_thread(
            CHECK_THREAD,
            CHECK_STACK,
            move || check_now(&settings),
            |_| CheckOutcome::NotRun,
            cx,
        )
    })
}

/// Asks the server of `settings` for its version once, through a client of its own, and says
/// whether 4x4 can use it. It blocks for as long as the call takes, up to the client's
/// [`DEFAULT_TIMEOUT`] (twice if the daemon hands out a new session id), so it never runs on the
/// UI thread: [`real_server_check`] runs it on a thread of its own.
#[must_use]
pub fn check_now(settings: &ConnectionSettings) -> CheckOutcome {
    let server = settings.server_label();
    tracing::info!(%server, "testing the connection");
    let outcome = CheckOutcome::from(Client::new(transport(settings)).check_server());
    tracing::info!(%server, ?outcome, "connection tested");
    outcome
}

/// The HTTP transport to the server of `settings`, with its credentials.
fn transport(settings: &ConnectionSettings) -> UreqTransport {
    UreqTransport::new(&settings.url(), DEFAULT_TIMEOUT)
        .with_credentials(settings.credentials().cloned())
}

#[cfg(test)]
mod tests;
