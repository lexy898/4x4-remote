//! The polling engine: which requests are due, and what their answers change.
//!
//! The engine performs no I/O. [`Engine::due_requests`] says which [`Request`]s to send now,
//! [`Engine::handle_response`] takes their answers, [`Engine::handle_command`] takes what the
//! user asks for and [`Engine::take_snapshot`] hands out the state when it changed. Every method
//! takes the current [`Instant`], so the schedule is tested without waiting.

mod answers;
mod schedule;
#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use x4_rpc::{
    FreeSpace, HashString, RpcError, ServerVersion, SessionInfo, SessionStats, TorrentGetResponse,
};

use crate::action::{Action, ActionError, ActionId};
use crate::details::{DetailsIssue, TorrentDetails};
use crate::snapshot::{ConnectionError, ConnectionState, SessionSummary, Snapshot};
use crate::store::Store;

use answers::{RefusedPolls, SkippedSet};
use schedule::Timer;

/// How long the engine sleeps when nothing is scheduled.
const IDLE_WAKEUP: Duration = Duration::from_secs(60);

/// How the engine polls the daemon, and how it waits while the daemon does not answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineConfig {
    /// The tick while the window is visible: recently active torrents, the session statistics
    /// and the selected torrent's details.
    pub visible_interval: Duration,
    /// The tick while the window is hidden.
    pub hidden_interval: Duration,
    /// How often the whole torrent list is read.
    pub full_refresh_interval: Duration,
    /// How often the session settings are read.
    pub session_interval: Duration,
    /// How often the free space of the download directory is read.
    pub free_space_interval: Duration,
    /// The wait before each try to reach a daemon that does not answer, the first try first;
    /// the last wait repeats. An empty list waits 30 s.
    pub backoff: Vec<Duration>,
    /// How long the connection must stay up before a failure starts the backoff from its first
    /// step again; until then the steps go on, so a daemon that passes the server check but
    /// fails a poll every time is not asked every second.
    pub stable_after: Duration,
}

impl Default for EngineConfig {
    /// Ticks of 1.5 s while visible and 10 s while hidden, the full list and the session every
    /// minute, the free space every 30 s, and retries after 1, 2, 4, 8, 16 and then every 30 s,
    /// starting over once the connection has stayed up for 30 s.
    fn default() -> Self {
        Self {
            visible_interval: Duration::from_millis(1500),
            hidden_interval: Duration::from_secs(10),
            full_refresh_interval: Duration::from_secs(60),
            session_interval: Duration::from_secs(60),
            free_space_interval: Duration::from_secs(30),
            backoff: [1, 2, 4, 8, 16, 30].map(Duration::from_secs).to_vec(),
            stable_after: Duration::from_secs(30),
        }
    }
}

/// Whether the user can see the window, which sets how often the engine ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    /// The window is shown: tick every [`EngineConfig::visible_interval`].
    Visible,
    /// The window is minimised or hidden: tick every [`EngineConfig::hidden_interval`].
    Hidden,
}

/// What the UI asks the engine to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// The window was shown or hidden.
    SetVisibility(Visibility),
    /// The torrent whose details are shown, if any; its details are read every tick.
    Select(Option<HashString>),
    /// Carry out an action; [`Event::ActionFinished`] with the same id tells how it went.
    Run(ActionId, Action),
    /// Read the session, the whole list, the statistics, the free space and the selected
    /// torrent's details again now; connect again if the daemon is not connected.
    Refresh,
    /// Connect again now, from the first step of the backoff.
    Reconnect,
    /// The service is stopping; the engine does nothing with it.
    Shutdown,
}

/// A call the worker makes to the daemon for the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Check that the server is a Transmission 4x4 supports; answered with [`Response::Server`].
    CheckServer,
    /// Read the session settings; answered with [`Response::Session`].
    SessionGet,
    /// Read the session statistics; answered with [`Response::Stats`].
    SessionStats,
    /// Read the free space of this directory on the server; answered with
    /// [`Response::FreeSpace`].
    FreeSpace(String),
    /// Read every torrent with the list fields; answered with [`Response::Torrents`].
    FullList,
    /// Read the recently active torrents with the list fields, and the ids of those removed
    /// recently; answered with [`Response::Torrents`].
    RecentlyActive,
    /// Read one torrent with the details fields; answered with [`Response::Details`].
    Details(HashString),
    /// Carry out an action; answered with [`Response::Done`].
    Action(ActionId, Action),
}

/// The daemon's answer to a [`Request`].
#[derive(Debug, Clone, PartialEq)]
pub enum Response {
    /// The server's version, for [`Request::CheckServer`].
    Server(ServerVersion),
    /// The session settings, for [`Request::SessionGet`].
    Session(SessionInfo),
    /// The session statistics, for [`Request::SessionStats`].
    Stats(SessionStats),
    /// The free space, for [`Request::FreeSpace`].
    FreeSpace(FreeSpace),
    /// The torrents, for [`Request::FullList`] and [`Request::RecentlyActive`].
    Torrents(TorrentGetResponse),
    /// The torrent, for [`Request::Details`].
    Details(TorrentGetResponse),
    /// The action was carried out, for [`Request::Action`].
    Done,
}

/// What the UI hears from the engine.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The state changed. The engine hands snapshots out through [`Engine::take_snapshot`];
    /// the service sends them as this event.
    Snapshot(Arc<Snapshot>),
    /// The selected torrent's details changed.
    Details(Arc<TorrentDetails>),
    /// An action finished.
    ActionFinished {
        /// The id it was run with.
        id: ActionId,
        /// How it went.
        result: Result<(), ActionError>,
    },
}

/// The polling schedule, the connection state machine and the state the UI shows.
///
/// It starts [`ConnectionState::Connecting`] and asks only for [`Request::CheckServer`]. A
/// supported server makes it [`ConnectionState::Connected`]: it reads the session, the whole
/// list and the statistics at once, then each on its own interval, and every tick the recently
/// active torrents, the statistics and the selected torrent's details. When the daemon stops
/// answering it becomes [`ConnectionState::Reconnecting`], keeps the last state (marked stale)
/// and checks the server again after each step of [`EngineConfig::backoff`]; the steps go on
/// until the connection has stayed up for [`EngineConfig::stable_after`], so a daemon that passes
/// the check but fails a poll every time is not asked every second. A server older than Transmission 4.1 (RPC version 6) makes it
/// [`ConnectionState::Unsupported`] until [`Command::Reconnect`].
///
/// Each poll is next due one interval after it was handed out, or after its answer came if that
/// is later, so a slow daemon rests a full interval between turns.
///
/// Actions are sent only while connected; otherwise they fail at once with
/// [`ActionError::NotConnected`] instead of waiting.
#[derive(Debug, Clone, PartialEq)]
pub struct Engine {
    config: EngineConfig,
    visibility: Visibility,
    connection: ConnectionState,
    /// When to check the server next; `None` while connected or unsupported.
    check_at: Option<Instant>,
    /// Failed tries in a row to use the daemon. A successful server check does not reset it:
    /// only a connection that stays up for [`EngineConfig::stable_after`] has proved itself.
    attempts: u32,
    /// When the daemon last passed the server check.
    connected_since: Option<Instant>,
    session_timer: Timer,
    full_list_timer: Timer,
    free_space_timer: Timer,
    /// Recently active torrents, the session statistics and the selected torrent's details.
    tick_timer: Timer,
    /// The selected torrent was just chosen, so its details are wanted before the next tick.
    details_now: bool,
    /// Actions accepted while connected and not handed out yet.
    actions: VecDeque<(ActionId, Action)>,
    store: Store,
    session: Option<SessionSummary>,
    /// The directory whose free space is read; known once this connection read the session.
    download_dir: Option<String>,
    free_space_bytes: Option<u64>,
    rate_down: u64,
    rate_up: u64,
    skipped: SkippedSet,
    refused: RefusedPolls,
    selected: Option<HashString>,
    details: Option<Arc<TorrentDetails>>,
    /// What was wrong with the selected torrent's last details answer, logged when it changes.
    details_issues: Vec<DetailsIssue>,
    /// Whether the snapshot changed since [`Engine::take_snapshot`] last handed one out.
    snapshot_changed: bool,
}

impl Engine {
    /// An engine that has not reached the daemon yet; the server check is due at `now`.
    #[must_use]
    pub fn new(config: EngineConfig, now: Instant) -> Self {
        Self {
            config,
            visibility: Visibility::Visible,
            connection: ConnectionState::Connecting,
            check_at: Some(now),
            attempts: 0,
            connected_since: None,
            session_timer: Timer::default(),
            full_list_timer: Timer::default(),
            free_space_timer: Timer::default(),
            tick_timer: Timer::default(),
            details_now: false,
            actions: VecDeque::new(),
            store: Store::new(),
            session: None,
            download_dir: None,
            free_space_bytes: None,
            rate_down: 0,
            rate_up: 0,
            skipped: SkippedSet::default(),
            refused: RefusedPolls::default(),
            selected: None,
            details: None,
            details_issues: Vec::new(),
            snapshot_changed: true,
        }
    }

    /// Takes a command from the UI at `now`; returns the events it causes at once, such as an
    /// action refused because the daemon is not connected.
    #[must_use]
    pub fn handle_command(&mut self, now: Instant, command: Command) -> Vec<Event> {
        match command {
            Command::SetVisibility(visibility) => {
                // The tick is measured from the last one, so the new interval applies at once.
                self.visibility = visibility;
                Vec::new()
            }
            Command::Select(hash) => {
                self.select(hash);
                Vec::new()
            }
            Command::Run(id, action) => self.run(id, action),
            Command::Refresh if self.connection == ConnectionState::Connected => {
                self.session_timer.force();
                self.full_list_timer.force();
                self.tick_timer.force();
                // Due only once the download directory is known.
                self.free_space_timer.force();
                Vec::new()
            }
            Command::Refresh | Command::Reconnect => self.restart(now),
            Command::Shutdown => Vec::new(),
        }
    }

    /// The requests to send at `now`: the actions waiting to be sent first, then the polls that
    /// are due, in a fixed order.
    ///
    /// Each poll handed out is next due one interval after `now`, whether or not its answer has
    /// come back, so a request is never handed out twice for the same turn; an answer that comes
    /// later than that moves the next turn to one interval after the answer.
    #[must_use]
    pub fn due_requests(&mut self, now: Instant) -> Vec<Request> {
        let mut due: Vec<Request> = self
            .actions
            .drain(..)
            .map(|(id, action)| Request::Action(id, action))
            .collect();
        match self.connection {
            ConnectionState::Connecting | ConnectionState::Reconnecting { .. } => {
                if self.check_at.is_some_and(|at| at <= now) {
                    due.push(Request::CheckServer);
                    // The answer sets the next check; this one only keeps the engine from
                    // waiting forever if no answer comes.
                    self.check_at = now.checked_add(self.retry_delay(self.next_attempt()));
                }
            }
            ConnectionState::Connected => self.due_polls(now, &mut due),
            ConnectionState::Unsupported { .. } => {}
        }
        due
    }

    /// Whether the answer to `request` still matters: a poll is out of date once the daemon is
    /// no longer connected, and details once their torrent is no longer selected.
    ///
    /// The worker skips a request that is not current instead of sending it. Server checks and
    /// actions are always current: an action handed out is always sent.
    #[must_use]
    pub fn is_current(&self, request: &Request) -> bool {
        let connected = self.connection == ConnectionState::Connected;
        match request {
            Request::CheckServer | Request::Action(..) => true,
            Request::Details(hash) => connected && self.selected.as_ref() == Some(hash),
            Request::SessionGet
            | Request::SessionStats
            | Request::FreeSpace(_)
            | Request::FullList
            | Request::RecentlyActive => connected,
        }
    }

    /// Takes the answer to `request`, which was handed out by [`Engine::due_requests`], at `now`;
    /// returns the events it causes.
    ///
    /// A failed poll that means the daemon cannot be used (no answer, an HTTP error, an answer
    /// that is not JSON-RPC) starts reconnecting, and an unsupported server stops polling. A
    /// poll the daemon refused is logged and tried again at its next interval. Answers to polls
    /// that are no longer [current](Engine::is_current) are ignored.
    #[must_use]
    pub fn handle_response(
        &mut self,
        now: Instant,
        request: &Request,
        result: Result<Response, RpcError>,
    ) -> Vec<Event> {
        match request {
            Request::CheckServer => self.server_checked(now, result),
            Request::Action(id, action) => self.action_finished(now, *id, action, result),
            poll if !self.is_current(poll) => {
                tracing::debug!(
                    request = poll.name(),
                    "ignored the answer to an outdated poll"
                );
                Vec::new()
            }
            poll => {
                self.answered_at(poll, now);
                match result {
                    Ok(response) => self.poll_answered(now, poll, response),
                    Err(error) => self.poll_failed(now, poll, &error),
                }
            }
        }
    }

    /// The state as the UI shows it, if anything changed since the last call; the first call
    /// always returns one.
    pub fn take_snapshot(&mut self) -> Option<Arc<Snapshot>> {
        if !self.snapshot_changed {
            return None;
        }
        self.snapshot_changed = false;
        Some(Arc::new(Snapshot {
            torrents: self.store.torrents().cloned().collect(),
            session: self.session.clone(),
            free_space_bytes: self.free_space_bytes,
            rate_down: self.rate_down,
            rate_up: self.rate_up,
            connection: self.connection.clone(),
            stale: self.connection != ConnectionState::Connected,
        }))
    }

    /// When [`Engine::due_requests`] next has something to send, at the earliest `now`; a
    /// minute from `now` when nothing is scheduled.
    #[must_use]
    pub fn next_wakeup(&self, now: Instant) -> Instant {
        if !self.actions.is_empty() {
            return now;
        }
        let earliest = match self.connection {
            ConnectionState::Connecting | ConnectionState::Reconnecting { .. } => self.check_at,
            ConnectionState::Connected => self.next_poll(now),
            ConnectionState::Unsupported { .. } => None,
        };
        match earliest {
            Some(at) => at.max(now),
            None => now.checked_add(IDLE_WAKEUP).unwrap_or(now),
        }
    }

    fn select(&mut self, hash: Option<HashString>) {
        if hash == self.selected {
            return;
        }
        // The old torrent's details must not show as the new one's.
        self.details = None;
        self.details_issues.clear();
        self.refused.answered(PollKind::Details);
        self.details_now = hash.is_some();
        self.selected = hash;
    }

    fn run(&mut self, id: ActionId, action: Action) -> Vec<Event> {
        let refused = if action.selects_nothing() {
            // The daemon would apply it to every torrent or every file.
            tracing::warn!(
                action = action.name(),
                "refused an action that names no torrents or no files"
            );
            ActionError::EmptySelection
        } else if self.connection == ConnectionState::Connected {
            self.actions.push_back((id, action));
            return Vec::new();
        } else {
            tracing::debug!(
                action = action.name(),
                "refused an action while not connected"
            );
            ActionError::NotConnected
        };
        vec![Event::ActionFinished {
            id,
            result: Err(refused),
        }]
    }

    /// Starts connecting again now, from the first step of the backoff.
    fn restart(&mut self, now: Instant) -> Vec<Event> {
        tracing::info!("connecting to the daemon");
        self.check_at = Some(now);
        self.attempts = 0;
        self.set_connection(ConnectionState::Connecting)
    }

    /// Moves to `state`; leaving `Connected` fails the actions not handed out yet, which are
    /// never kept for later.
    fn set_connection(&mut self, state: ConnectionState) -> Vec<Event> {
        if state != self.connection {
            self.connection = state;
            self.snapshot_changed = true;
        }
        if self.connection == ConnectionState::Connected {
            return Vec::new();
        }
        self.actions
            .drain(..)
            .map(|(id, _)| Event::ActionFinished {
                id,
                result: Err(ActionError::NotConnected),
            })
            .collect()
    }

    fn server_checked(&mut self, now: Instant, result: Result<Response, RpcError>) -> Vec<Event> {
        if !matches!(
            self.connection,
            ConnectionState::Connecting | ConnectionState::Reconnecting { .. }
        ) {
            tracing::debug!("ignored the answer to a server check that is no longer needed");
            return Vec::new();
        }
        match result {
            Ok(Response::Server(version)) if version.is_supported() => {
                self.connected(now, &version)
            }
            Ok(Response::Server(version)) => self.unsupported(version.rpc_semver),
            Ok(other) => self.connect_failed(now, &unexpected(&Request::CheckServer, &other)),
            Err(RpcError::UnsupportedServer { rpc_version }) => self.unsupported(rpc_version),
            // Even an error the daemon reports means there is nothing to poll yet.
            Err(error) => self.connect_failed(now, &error),
        }
    }

    /// The server answered at `now` and is supported: read everything at once.
    fn connected(&mut self, now: Instant, version: &ServerVersion) -> Vec<Event> {
        tracing::info!(
            version = %version.version,
            rpc_version = %version.rpc_semver,
            "connected to the daemon"
        );
        self.check_at = None;
        self.connected_since = Some(now);
        // A timer never sent is due at once. The tick brings the statistics and the details;
        // its recently active torrents come with the full list.
        self.session_timer = Timer::default();
        self.full_list_timer = Timer::default();
        self.free_space_timer = Timer::default();
        self.tick_timer = Timer::default();
        self.details_now = false;
        // The daemon may have restarted with another directory; measure the one it reports.
        self.download_dir = None;
        self.set_connection(ConnectionState::Connected)
    }

    fn unsupported(&mut self, rpc_version: String) -> Vec<Event> {
        tracing::error!(
            rpc_version = %rpc_version,
            "the daemon's RPC version is not supported; 4x4 needs Transmission 4.1 or later"
        );
        self.check_at = None;
        self.set_connection(ConnectionState::Unsupported { rpc_version })
    }

    /// The daemon cannot be used: wait one more step of the backoff, then check it again.
    ///
    /// A connection that stayed up long enough to prove itself starts the backoff over; one
    /// that fails sooner, even after a successful server check, goes on to the next step.
    fn connect_failed(&mut self, now: Instant, error: &RpcError) -> Vec<Event> {
        if self.connection == ConnectionState::Connected && self.was_stable(now) {
            self.attempts = 0;
        }
        let attempt = self.next_attempt();
        self.attempts = attempt;
        let delay = self.retry_delay(attempt);
        tracing::warn!(attempt, retry_in = ?delay, %error, "the daemon cannot be used; retrying");
        self.check_at = now.checked_add(delay);
        self.set_connection(ConnectionState::Reconnecting {
            attempt,
            last_error: ConnectionError::from(error),
        })
    }

    /// Whether the connection has been up without a failure for [`EngineConfig::stable_after`]
    /// at `now`. A connection-class failure ends it at once, so "up" is enough.
    fn was_stable(&self, now: Instant) -> bool {
        self.connected_since
            .and_then(|since| since.checked_add(self.config.stable_after))
            .is_some_and(|stable_at| now >= stable_at)
    }

    /// Reconnects after an error that shows the daemon cannot be used, unless that is already
    /// known: the errors that follow the first one do not count as further attempts.
    fn connection_lost(&mut self, now: Instant, error: &RpcError) -> Vec<Event> {
        if self.connection == ConnectionState::Connected {
            self.connect_failed(now, error)
        } else {
            Vec::new()
        }
    }

    fn action_finished(
        &mut self,
        now: Instant,
        id: ActionId,
        action: &Action,
        result: Result<Response, RpcError>,
    ) -> Vec<Event> {
        let Err(error) = result else {
            tracing::info!(action = action.name(), "action done");
            // Its effect shows in the next tick; bring that tick forward.
            self.tick_timer.force();
            if action.changes_session() {
                self.session_timer.force();
            }
            return vec![Event::ActionFinished { id, result: Ok(()) }];
        };
        tracing::error!(action = action.name(), %error, "action failed");
        let mut events = vec![Event::ActionFinished {
            id,
            result: Err(ActionError::Rpc(ConnectionError::from(&error))),
        }];
        events.extend(match Failure::of(&error) {
            Failure::Unusable => self.connection_lost(now, &error),
            Failure::Unsupported(rpc_version) => self.unsupported(rpc_version.to_owned()),
            Failure::Refused => Vec::new(),
        });
        events
    }

    fn poll_failed(&mut self, now: Instant, poll: &Request, error: &RpcError) -> Vec<Event> {
        match Failure::of(error) {
            Failure::Unusable => self.connection_lost(now, error),
            Failure::Unsupported(rpc_version) => self.unsupported(rpc_version.to_owned()),
            Failure::Refused => {
                let news = poll
                    .poll_kind()
                    .is_some_and(|kind| self.refused.failed(kind));
                if news {
                    tracing::warn!(
                        request = poll.name(),
                        %error,
                        "the daemon refused a poll; it is tried again at its next interval"
                    );
                } else {
                    tracing::debug!(request = poll.name(), %error, "the daemon refused a poll again");
                }
                Vec::new()
            }
        }
    }
}

/// A kind of poll, without its arguments: [`Request::SessionGet`] and so on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum PollKind {
    SessionGet,
    SessionStats,
    FreeSpace,
    FullList,
    RecentlyActive,
    Details,
}

impl Request {
    /// The kind of poll this is; `None` for the server check and actions, which are not polls.
    fn poll_kind(&self) -> Option<PollKind> {
        match self {
            Self::SessionGet => Some(PollKind::SessionGet),
            Self::SessionStats => Some(PollKind::SessionStats),
            Self::FreeSpace(_) => Some(PollKind::FreeSpace),
            Self::FullList => Some(PollKind::FullList),
            Self::RecentlyActive => Some(PollKind::RecentlyActive),
            Self::Details(_) => Some(PollKind::Details),
            Self::CheckServer | Self::Action(..) => None,
        }
    }

    /// A short name for logs; it carries no torrent names, hashes or paths.
    fn name(&self) -> &'static str {
        match self {
            Self::CheckServer => "check server",
            Self::SessionGet => "session get",
            Self::SessionStats => "session stats",
            Self::FreeSpace(_) => "free space",
            Self::FullList => "full list",
            Self::RecentlyActive => "recently active",
            Self::Details(_) => "details",
            Self::Action(..) => "action",
        }
    }
}

impl Response {
    /// A short name for logs.
    fn name(&self) -> &'static str {
        match self {
            Self::Server(_) => "server",
            Self::Session(_) => "session",
            Self::Stats(_) => "stats",
            Self::FreeSpace(_) => "free space",
            Self::Torrents(_) => "torrents",
            Self::Details(_) => "details",
            Self::Done => "done",
        }
    }
}

/// The error for an answer of the wrong kind, which only a broken worker sends.
fn unexpected(request: &Request, response: &Response) -> RpcError {
    RpcError::InvalidResponse(format!(
        "a {} request was answered with {}",
        request.name(),
        response.name()
    ))
}

/// What a failed call says about the daemon.
enum Failure<'a> {
    /// The daemon cannot be used: it does not answer, refuses this host or does not speak
    /// JSON-RPC. Reconnect.
    Unusable,
    /// The daemon speaks an RPC version 4x4 does not support; holds the version.
    Unsupported(&'a str),
    /// The daemon answered and refused this one call. The connection is fine.
    Refused,
}

impl<'a> Failure<'a> {
    /// No wildcard: a new error has to decide what it says about the daemon.
    fn of(error: &'a RpcError) -> Self {
        match error {
            RpcError::Transport(_)
            | RpcError::Unauthorized
            | RpcError::Forbidden
            | RpcError::HttpStatus(_)
            | RpcError::SessionIdMissing
            | RpcError::SessionIdRejected
            | RpcError::InvalidResponse(_) => Self::Unusable,
            RpcError::UnsupportedServer { rpc_version } => Self::Unsupported(rpc_version),
            RpcError::Server { .. } | RpcError::InvalidRequest(_) | RpcError::Selection(_) => {
                Self::Refused
            }
        }
    }
}
