//! The polling engine: which requests are due, and what their answers change.
//!
//! The engine performs no I/O. [`Engine::due_requests`] says which [`Request`]s to send now,
//! [`Engine::handle_response`] takes their answers, [`Engine::handle_command`] takes what the
//! user asks for and [`Engine::take_snapshot`] hands out the state when it changed. Every method
//! takes the current [`Instant`], so the schedule is tested without waiting.

mod adding;
mod answers;
mod details_reads;
mod folder_space;
mod magnet_links;
mod schedule;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use x4_rpc::{
    AddOutcome, DownloadDir, FreeSpace, HashString, RpcError, ServerVersion, SessionInfo,
    SessionStats, TorrentField, TorrentGetResponse, TransportError, UnsupportedReason,
};

use crate::action::{Action, ActionError, ActionId, AddError, Added};
use crate::details::{DetailsIssue, TorrentDetails};
use crate::snapshot::{ConnectionError, ConnectionState, SessionSummary, Snapshot};
use crate::store::Store;

use answers::{RefusedPolls, SkippedSet};
use details_reads::DetailsReads;
use folder_space::FolderSpace;
use schedule::Timer;

pub use folder_space::FreeSpaceAnswer;

/// How long the engine sleeps when nothing is scheduled.
const IDLE_WAKEUP: Duration = Duration::from_secs(60);

/// How the engine polls the daemon, and how it waits while the daemon does not answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineConfig {
    /// The tick while the window is visible: recently active torrents and the session
    /// statistics.
    pub visible_interval: Duration,
    /// The tick while the window is hidden.
    pub hidden_interval: Duration,
    /// How often the whole torrent list is read.
    pub full_refresh_interval: Duration,
    /// How often the session settings are read.
    pub session_interval: Duration,
    /// How often the free space of the download directory is read.
    pub free_space_interval: Duration,
    /// How often the progress of the selected torrent is read while it downloads or verifies;
    /// otherwise it is read once with each whole list.
    pub details_interval: Duration,
    /// How long a new selection must stay before its details are read, so that moving through
    /// the list with the arrow keys reads only the torrent the selection stops on.
    pub details_settle: Duration,
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
    /// minute, the free space every 30 s, the selected torrent's progress every 3 s once it has
    /// stayed selected for 250 ms, and retries after 1, 2, 4, 8, 16 and then every 30 s, starting
    /// over once the connection has stayed up for 30 s.
    fn default() -> Self {
        Self {
            visible_interval: Duration::from_millis(1500),
            hidden_interval: Duration::from_secs(10),
            full_refresh_interval: Duration::from_secs(60),
            session_interval: Duration::from_secs(60),
            free_space_interval: Duration::from_secs(30),
            details_interval: Duration::from_secs(3),
            details_settle: Duration::from_millis(250),
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
    /// The torrent whose details are shown, if any. They are read in full once the selection
    /// has stayed for [`EngineConfig::details_settle`], then only for their progress, and only
    /// while the window is visible; the UI sends `None` when nothing shows them.
    Select(Option<HashString>),
    /// Carry out an action; [`Event::ActionFinished`] with the same id tells how it went, or
    /// [`Event::AddFinished`] for an [`Action::Add`].
    Run(ActionId, Action),
    /// Read the magnet links of these torrents once; [`Event::MagnetLinks`] with the same id
    /// brings them. Refused like [`Command::Run`]: when it names no torrent, and while the
    /// daemon is not connected.
    ReadMagnetLinks(ActionId, Vec<HashString>),
    /// Read the free space of this folder on the server once, or of its nearest parent that
    /// exists if it does not; [`Event::FreeSpace`] with the same folder brings the answer.
    ///
    /// One call is out at a time and one question makes at most three. A question replaces the
    /// one before it that has not been answered yet, which then gets no answer; asking again
    /// about the folder already asked about changes nothing. While the daemon is not connected
    /// nothing is sent and the answer, [`FreeSpaceAnswer::Unknown`], comes at once; a question
    /// waiting when the connection drops is answered so too.
    FreeSpace(DownloadDir),
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
    /// Read the details of one torrent, whole or only their progress; answered with
    /// [`Response::Details`].
    Details {
        /// The torrent.
        hash: HashString,
        /// What to read.
        kind: DetailsRead,
    },
    /// Carry out an action; answered with [`Response::Done`], or with [`Response::Added`] for an
    /// [`Action::Add`], which waits [`x4_rpc::ADD_TIMEOUT`] for its answer.
    Action(ActionId, Action),
    /// Read the hashes and magnet links of these torrents once; answered with
    /// [`Response::MagnetLinks`].
    MagnetLinks(ActionId, Vec<HashString>),
    /// Read the free space of a folder for a [`Command::FreeSpace`]: the one asked about or one
    /// of its parents; answered with [`Response::FreeSpace`].
    FolderFreeSpace(DownloadDir),
}

/// How much of the selected torrent's details a [`Request::Details`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DetailsRead {
    /// Everything: the metainfo, the files, the trackers and the progress.
    Full,
    /// Only what changes while the torrent downloads: each file's bytes, priority and wanted
    /// switch, the pieces and the totals. It is merged into the last full read.
    Progress,
    /// The progress and the trackers' state. The trackers change slowly, so only the progress read
    /// that goes out with a whole list asks for them, about once a minute, and not the one every
    /// few seconds; the answer's trackers replace the last ones.
    ProgressAndTrackers,
}

impl DetailsRead {
    /// The fields this read asks the daemon for.
    #[must_use]
    pub fn fields(self) -> &'static [TorrentField] {
        match self {
            Self::Full => TorrentField::DETAILS_FULL,
            Self::Progress => TorrentField::DETAILS_PROGRESS,
            Self::ProgressAndTrackers => TorrentField::DETAILS_PROGRESS_TRACKERS,
        }
    }
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
    /// The free space, for [`Request::FreeSpace`] and [`Request::FolderFreeSpace`].
    FreeSpace(FreeSpace),
    /// The torrents, for [`Request::FullList`] and [`Request::RecentlyActive`].
    Torrents(TorrentGetResponse),
    /// The torrent, for [`Request::Details`].
    Details(TorrentGetResponse),
    /// The action was carried out, for [`Request::Action`].
    Done,
    /// What the daemon did with the torrent, for a [`Request::Action`] of an [`Action::Add`].
    Added(AddOutcome),
    /// The torrents with their hashes and magnet links, for [`Request::MagnetLinks`].
    MagnetLinks(TorrentGetResponse),
}

/// What the UI hears from the engine.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The state changed. The engine hands snapshots out through [`Engine::take_snapshot`];
    /// the service sends them as this event.
    Snapshot(Arc<Snapshot>),
    /// The selected torrent's details changed.
    Details(Arc<TorrentDetails>),
    /// An action other than an add finished.
    ActionFinished {
        /// The id it was run with.
        id: ActionId,
        /// How it went.
        result: Result<(), ActionError>,
    },
    /// The magnet links of a [`Command::ReadMagnetLinks`] arrived, or could not be read.
    MagnetLinks {
        /// The id they were asked for with.
        id: ActionId,
        /// Each torrent's hash and magnet link, in the order they were asked for, each torrent
        /// once. A torrent the daemon no longer has, or that has no magnet link, is left out.
        result: Result<Vec<(HashString, String)>, ActionError>,
    },
    /// An [`Action::Add`] finished: the daemon added the torrent or already had it, or it did
    /// not add it, or it is not known whether it did.
    AddFinished {
        /// The id it was run with.
        id: ActionId,
        /// The torrent added or found, or why not.
        result: Result<Added, AddError>,
    },
    /// The answer to a [`Command::FreeSpace`].
    FreeSpace {
        /// The folder asked about.
        path: DownloadDir,
        /// How much room it has, or its nearest parent that exists.
        answer: FreeSpaceAnswer,
    },
}

/// The polling schedule, the connection state machine and the state the UI shows.
///
/// It starts [`ConnectionState::Connecting`] and asks only for [`Request::CheckServer`]. A
/// supported server makes it [`ConnectionState::Connected`]: it reads the session, the whole
/// list and the statistics at once, then each on its own interval, and every tick the recently
/// active torrents and the statistics. The selected torrent's details are read in full once the
/// selection settles and after connecting, then only for their progress: every
/// [`EngineConfig::details_interval`] while it downloads or verifies, otherwise with each whole
/// list, and nothing while the window is hidden. When the daemon stops
/// answering it becomes [`ConnectionState::Reconnecting`], keeps the last state (marked stale)
/// and checks the server again after each step of [`EngineConfig::backoff`]; the steps go on
/// until the connection has stayed up for [`EngineConfig::stable_after`], so a daemon that passes
/// the check but fails a poll every time is not asked every second. A server older than
/// Transmission 4.1 (RPC version 6) makes it [`ConnectionState::Unsupported`] until
/// [`Command::Reconnect`].
///
/// Each poll is next due one interval after it was handed out, or after its answer came if that
/// is later, so a slow daemon rests a full interval between turns.
///
/// Actions and reads of magnet links are sent only while connected; otherwise they fail at once
/// with [`ActionError::NotConnected`] (an add with [`AddError::NotConnected`]) instead of waiting.
/// So are the questions about the free space of a folder, which are answered
/// [`FreeSpaceAnswer::Unknown`]. An add waits longer than any other call for its answer, and
/// getting none in that time does not drop the connection: it is
/// [`AddError::Unconfirmed`].
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
    /// Recently active torrents and the session statistics.
    tick_timer: Timer,
    /// When the selected torrent's details are read next, and how.
    details_reads: DetailsReads,
    /// Actions and reads of magnet links accepted while connected and not handed out yet,
    /// oldest first.
    queued: VecDeque<Queued>,
    /// The newest question about the free space of a folder, and its call out.
    folder_space: FolderSpace,
    /// How many times requests were handed out; numbers the batches.
    batch: u64,
    /// The batch the last tick went out in.
    tick_batch: Option<u64>,
    /// The batch each action that has no answer yet went out in.
    action_batches: HashMap<ActionId, u64>,
    store: Store,
    /// Whether a whole list of torrents has been read since the start; until then an empty
    /// store means nothing has arrived, not that the daemon has no torrents.
    listed: bool,
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
            details_reads: DetailsReads::default(),
            queued: VecDeque::new(),
            folder_space: FolderSpace::default(),
            batch: 0,
            tick_batch: None,
            action_batches: HashMap::new(),
            store: Store::new(),
            listed: false,
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
                self.details_visibility(visibility);
                // The tick is measured from the last one, so the new interval applies at once.
                self.visibility = visibility;
                Vec::new()
            }
            Command::Select(hash) => {
                self.select(now, hash);
                Vec::new()
            }
            Command::Run(id, action) => self.accept(Queued::Action(id, action)),
            Command::ReadMagnetLinks(id, hashes) => self.accept(Queued::MagnetLinks(id, hashes)),
            Command::FreeSpace(path) => self.ask_free_space(path),
            Command::Refresh if self.connection == ConnectionState::Connected => {
                self.session_timer.force();
                self.full_list_timer.force();
                self.tick_timer.force();
                // Due only once the download directory is known.
                self.free_space_timer.force();
                self.details_refresh(now);
                Vec::new()
            }
            Command::Refresh | Command::Reconnect => self.restart(now),
            Command::Shutdown => Vec::new(),
        }
    }

    /// The requests to send at `now`: the actions and reads of magnet links waiting to be sent
    /// first, in the order they came, then the free space of a folder if a question waits for it,
    /// no call for one is out and the last one was long enough ago, then the polls that are due,
    /// in a fixed order.
    ///
    /// Each poll handed out is next due one interval after `now`, whether or not its answer has
    /// come back, so a request is never handed out twice for the same turn; an answer that comes
    /// later than that moves the next turn to one interval after the answer.
    #[must_use]
    pub fn due_requests(&mut self, now: Instant) -> Vec<Request> {
        self.batch = self.batch.wrapping_add(1);
        let mut due = Vec::with_capacity(self.queued.len());
        for queued in self.queued.drain(..) {
            due.push(match queued {
                Queued::Action(id, action) => {
                    self.action_batches.insert(id, self.batch);
                    Request::Action(id, action)
                }
                Queued::MagnetLinks(id, hashes) => Request::MagnetLinks(id, hashes),
            });
        }
        match self.connection {
            ConnectionState::Connecting | ConnectionState::Reconnecting { .. } => {
                if self.check_at.is_some_and(|at| at <= now) {
                    due.push(Request::CheckServer);
                    // The answer sets the next check; this one only keeps the engine from
                    // waiting forever if no answer comes.
                    self.check_at = now.checked_add(self.retry_delay(self.next_attempt()));
                }
            }
            ConnectionState::Connected => {
                if let Some(folder) = self.folder_space.hand_out(now) {
                    due.push(Request::FolderFreeSpace(folder));
                }
                self.due_polls(now, &mut due);
            }
            ConnectionState::Unsupported { .. } => {}
        }
        due
    }

    /// Whether the answer to `request` still matters: a poll or the free space of a folder is out
    /// of date once the daemon is no longer connected, and details once their torrent is no
    /// longer selected.
    ///
    /// The worker skips a request that is not current instead of sending it. Server checks,
    /// actions and reads of magnet links are always current: once handed out, they are always
    /// sent, and their answer always reaches the UI. The free space of a folder is sent once
    /// handed out while connected, even when a newer question has come since, so that only one
    /// call is ever out.
    #[must_use]
    pub fn is_current(&self, request: &Request) -> bool {
        let connected = self.connection == ConnectionState::Connected;
        match request {
            Request::CheckServer | Request::Action(..) | Request::MagnetLinks(..) => true,
            Request::Details { hash, .. } => connected && self.selected.as_ref() == Some(hash),
            Request::SessionGet
            | Request::SessionStats
            | Request::FreeSpace(_)
            | Request::FolderFreeSpace(_)
            | Request::FullList
            | Request::RecentlyActive => connected,
        }
    }

    /// Takes the answer to `request`, which was handed out by [`Engine::due_requests`], at `now`;
    /// returns the events it causes.
    ///
    /// `now` must be the time this answer arrived, not the time its batch was handed out: the
    /// next turn of the poll is measured from it, so a slow daemon gets its rest.
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
            Request::Action(id, Action::Add(_)) => self.add_finished(now, request, *id, result),
            Request::Action(id, action) => self.action_finished(now, request, *id, action, result),
            Request::MagnetLinks(id, hashes) => {
                let result = match result {
                    Ok(Response::MagnetLinks(answer)) => Ok(answer),
                    Ok(other) => Err(unexpected(request, &other)),
                    Err(error) => Err(error),
                };
                self.magnet_links_read(now, request, *id, hashes, result)
            }
            Request::FolderFreeSpace(folder) => {
                self.folder_space_read(now, request, folder, result)
            }
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
            listed: self.listed,
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
        if !self.queued.is_empty() {
            return now;
        }
        let earliest = match self.connection {
            ConnectionState::Connecting | ConnectionState::Reconnecting { .. } => self.check_at,
            ConnectionState::Connected => [self.next_poll(now), self.folder_space.due_at(now)]
                .into_iter()
                .flatten()
                .min(),
            ConnectionState::Unsupported { .. } => None,
        };
        match earliest {
            Some(at) => at.max(now),
            None => now.checked_add(IDLE_WAKEUP).unwrap_or(now),
        }
    }

    /// Queues an action or a read of magnet links, or refuses it at once: when it names
    /// nothing, which the daemon would read as everything, or while the daemon is not connected.
    fn accept(&mut self, queued: Queued) -> Vec<Event> {
        if let Some(refused) = queued.refused_as_empty() {
            tracing::warn!(
                request = queued.name(),
                "refused a request that names no torrents or no files"
            );
            return vec![refused];
        }
        if self.connection == ConnectionState::Connected {
            self.queued.push_back(queued);
            return Vec::new();
        }
        tracing::debug!(
            request = queued.name(),
            "refused a request while not connected"
        );
        vec![queued.not_connected()]
    }

    /// Takes a question about the free space of `path`, or answers it at once while the daemon
    /// is not connected.
    fn ask_free_space(&mut self, path: DownloadDir) -> Vec<Event> {
        if self.connection != ConnectionState::Connected {
            tracing::debug!("answered a free-space question while not connected");
            return vec![Event::FreeSpace {
                path,
                answer: FreeSpaceAnswer::Unknown,
            }];
        }
        self.folder_space.ask(path);
        Vec::new()
    }

    /// Starts connecting again now, from the first step of the backoff.
    fn restart(&mut self, now: Instant) -> Vec<Event> {
        tracing::info!("connecting to the daemon");
        self.check_at = Some(now);
        self.attempts = 0;
        self.set_connection(ConnectionState::Connecting)
    }

    /// Moves to `state`; leaving `Connected` fails the actions and reads of magnet links not
    /// handed out yet, which are never kept for later, and answers the free-space question not
    /// answered yet as unknown.
    fn set_connection(&mut self, state: ConnectionState) -> Vec<Event> {
        if state != self.connection {
            self.connection = state;
            self.snapshot_changed = true;
        }
        if self.connection == ConnectionState::Connected {
            return Vec::new();
        }
        let mut events: Vec<Event> = self.queued.drain(..).map(Queued::not_connected).collect();
        events.extend(self.folder_space.disconnected());
        events
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
            Ok(Response::Server(version)) => {
                self.unsupported(UnsupportedReason::from_rpc_semver(&version.rpc_semver))
            }
            Ok(other) => self.connect_failed(now, &unexpected(&Request::CheckServer, &other)),
            Err(RpcError::UnsupportedServer { reason }) => self.unsupported(reason),
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
        // A timer never sent is due at once. The tick brings the statistics; its recently active
        // torrents come with the full list.
        self.session_timer = Timer::default();
        self.full_list_timer = Timer::default();
        self.free_space_timer = Timer::default();
        self.tick_timer = Timer::default();
        self.details_connected(now);
        // The daemon may have restarted with another directory; measure the one it reports.
        self.download_dir = None;
        self.set_connection(ConnectionState::Connected)
    }

    fn unsupported(&mut self, reason: UnsupportedReason) -> Vec<Event> {
        tracing::error!(
            %reason,
            "the daemon's RPC version is not supported; 4x4 needs Transmission 4.1 or later"
        );
        self.check_at = None;
        self.set_connection(ConnectionState::Unsupported { reason })
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
        request: &Request,
        id: ActionId,
        action: &Action,
        result: Result<Response, RpcError>,
    ) -> Vec<Event> {
        let sent_in = self.action_batches.remove(&id);
        let Err(error) = result else {
            tracing::info!(action = action.name(), "action done");
            self.pull_tick_forward(sent_in);
            if action.changes_session() {
                self.session_timer.force();
            }
            self.details_after_action(action, sent_in);
            return vec![Event::ActionFinished { id, result: Ok(()) }];
        };
        tracing::error!(action = action.name(), %error, "action failed");
        let mut events = vec![Event::ActionFinished {
            id,
            result: Err(ActionError::Rpc(ConnectionError::from(&error))),
        }];
        events.extend(self.call_failed(now, request, &error));
        events
    }

    /// The effect of an action sent in batch `sent_in` shows in the next tick: brings that tick
    /// forward, unless a tick already went out with the action or after it (polls follow actions
    /// in a batch).
    fn pull_tick_forward(&mut self, sent_in: Option<u64>) {
        let seen = sent_in
            .zip(self.tick_batch)
            .is_some_and(|(action_batch, tick_batch)| tick_batch >= action_batch);
        if !seen {
            self.tick_timer.force();
        }
    }

    /// What a failed call says about the connection: a daemon that cannot be used is reconnected
    /// to, an unsupported one is given up on, and a refusal, or an add that got no answer in
    /// time, leaves the connection as it is. Nothing is retried.
    fn call_failed(&mut self, now: Instant, request: &Request, error: &RpcError) -> Vec<Event> {
        match Failure::of(request, error) {
            Failure::Unusable => self.connection_lost(now, error),
            Failure::Unsupported(reason) => self.unsupported(reason.clone()),
            Failure::Refused | Failure::Unanswered => Vec::new(),
        }
    }

    /// As [`Engine::call_failed`]; a refused poll is also logged, as a warning only the first
    /// time in a row.
    fn poll_failed(&mut self, now: Instant, poll: &Request, error: &RpcError) -> Vec<Event> {
        if Failure::of(poll, error) != Failure::Refused {
            return self.call_failed(now, poll, error);
        }
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
            tracing::debug!(
                request = poll.name(),
                %error,
                "the daemon refused a poll again"
            );
        }
        Vec::new()
    }
}

/// An action or a read of magnet links that the UI asked for and that waits for the next batch,
/// in which it goes out before the polls.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Queued {
    /// [`Command::Run`].
    Action(ActionId, Action),
    /// [`Command::ReadMagnetLinks`].
    MagnetLinks(ActionId, Vec<HashString>),
}

impl Queued {
    /// The event that refuses it for naming no torrent or no file, which the daemon would read as
    /// all of them; `None` when it names some. An add names no torrents or files, so it is never
    /// refused for this.
    fn refused_as_empty(&self) -> Option<Event> {
        match self {
            Self::Action(_, Action::Add(_)) => None,
            Self::Action(id, action) => action.selects_nothing().then_some(Event::ActionFinished {
                id: *id,
                result: Err(ActionError::EmptySelection),
            }),
            Self::MagnetLinks(id, hashes) => hashes.is_empty().then_some(Event::MagnetLinks {
                id: *id,
                result: Err(ActionError::EmptySelection),
            }),
        }
    }

    /// A short name for logs; it carries no torrent names, hashes or paths.
    fn name(&self) -> &'static str {
        match self {
            Self::Action(_, action) => action.name(),
            Self::MagnetLinks(..) => "read magnet links",
        }
    }

    /// The event that tells the UI it was not sent because the daemon is not connected.
    fn not_connected(self) -> Event {
        match self {
            Self::Action(id, Action::Add(_)) => Event::AddFinished {
                id,
                result: Err(AddError::NotConnected),
            },
            Self::Action(id, _) => Event::ActionFinished {
                id,
                result: Err(ActionError::NotConnected),
            },
            Self::MagnetLinks(id, _) => Event::MagnetLinks {
                id,
                result: Err(ActionError::NotConnected),
            },
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
    /// The kind of poll this is; `None` for the server check, actions, reads of magnet links
    /// and the free space of a folder, which are not polls.
    fn poll_kind(&self) -> Option<PollKind> {
        match self {
            Self::SessionGet => Some(PollKind::SessionGet),
            Self::SessionStats => Some(PollKind::SessionStats),
            Self::FreeSpace(_) => Some(PollKind::FreeSpace),
            Self::FullList => Some(PollKind::FullList),
            Self::RecentlyActive => Some(PollKind::RecentlyActive),
            Self::Details { .. } => Some(PollKind::Details),
            Self::CheckServer
            | Self::Action(..)
            | Self::MagnetLinks(..)
            | Self::FolderFreeSpace(_) => None,
        }
    }

    /// Whether the worker waits [`x4_rpc::ADD_TIMEOUT`] for the answer rather than the client's
    /// usual timeout: only for an add ([`x4_rpc::Client::torrent_add`]).
    ///
    /// No wildcard: a new request has to decide.
    fn waits_long(&self) -> bool {
        match self {
            Self::Action(_, Action::Add(_)) => true,
            Self::CheckServer
            | Self::SessionGet
            | Self::SessionStats
            | Self::FreeSpace(_)
            | Self::FolderFreeSpace(_)
            | Self::FullList
            | Self::RecentlyActive
            | Self::Details { .. }
            | Self::Action(..)
            | Self::MagnetLinks(..) => false,
        }
    }

    /// A short name for logs; it carries no torrent names, hashes or paths.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::CheckServer => "check server",
            Self::SessionGet => "session get",
            Self::SessionStats => "session stats",
            Self::FreeSpace(_) => "free space",
            Self::FullList => "full list",
            Self::RecentlyActive => "recently active",
            Self::Details {
                kind: DetailsRead::Full,
                ..
            } => "full details",
            Self::Details {
                kind: DetailsRead::Progress,
                ..
            } => "details progress",
            Self::Details {
                kind: DetailsRead::ProgressAndTrackers,
                ..
            } => "details progress and trackers",
            Self::Action(..) => "action",
            Self::MagnetLinks(..) => "magnet links",
            Self::FolderFreeSpace(_) => "folder free space",
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
            Self::Added(_) => "added",
            Self::MagnetLinks(_) => "magnet links",
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Failure<'a> {
    /// The daemon cannot be used: it does not answer, refuses this host or does not speak
    /// JSON-RPC. Reconnect.
    Unusable,
    /// The daemon speaks an RPC version 4x4 does not support; holds why.
    Unsupported(&'a UnsupportedReason),
    /// The daemon answered and refused this one call. The connection is fine.
    Refused,
    /// An add got no answer within its own long timeout ([`x4_rpc::ADD_TIMEOUT`]). Whether the
    /// torrent was added is not known, and the daemon may only be busy reading it, so the
    /// connection stays as it is: the next poll tells whether the daemon answers.
    Unanswered,
}

impl<'a> Failure<'a> {
    /// What `error` says about the daemon when `request` failed with it.
    ///
    /// The one place that decides it, the timeout of an add too: that timeout alone is no sign
    /// of a lost connection, while the timeout of any other call is. No wildcard: a new error
    /// has to decide what it says about the daemon.
    fn of(request: &Request, error: &'a RpcError) -> Self {
        match error {
            RpcError::Transport(TransportError::Timeout) if request.waits_long() => {
                Self::Unanswered
            }
            RpcError::Transport(_)
            | RpcError::Unauthorized
            | RpcError::Forbidden
            | RpcError::HttpStatus(_)
            | RpcError::SessionIdMissing
            | RpcError::SessionIdRejected
            | RpcError::InvalidResponse(_) => Self::Unusable,
            RpcError::UnsupportedServer { reason } => Self::Unsupported(reason),
            RpcError::Server { .. }
            | RpcError::InvalidRequest(_)
            | RpcError::MagnetWithFileChoices
            | RpcError::Selection(_) => Self::Refused,
        }
    }
}
