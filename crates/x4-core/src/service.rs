//! The worker thread that runs the [`Engine`] against the daemon, and the channels between it
//! and the UI.
//!
//! [`spawn`] starts the thread `x4-service` and returns a [`ServiceHandle`]. The UI sends
//! [`Command`]s through the handle, which never blocks, and awaits the [`Event`]s of
//! [`ServiceHandle::events`] on its own executor. The thread does all the network I/O: it sends
//! the requests the engine says are due, one at a time, hands each answer back to the engine,
//! publishes the snapshot when it changed, and then sleeps until the engine's next wakeup or the
//! next command, whichever comes first.

use std::any::Any;
use std::ops::ControlFlow;
use std::slice;
use std::thread::{self, JoinHandle};
use std::time::Instant;

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use x4_rpc::{
    AddOutcome, Client, FilePriority, GetIds, HashString, RpcError, SessionSet, TorrentAction,
    TorrentField, TorrentSelection, TorrentSet, Transport,
};

use crate::action::Action;
use crate::engine::{Command, Engine, EngineConfig, Event, Request, Response};

/// The name of the service thread, as debuggers and system monitors show it.
const THREAD_NAME: &str = "x4-service";

/// The UI's end of the service thread: commands go in, events come out.
///
/// Dropping the handle stops the thread and waits for it, as [`ServiceHandle::shutdown`] does.
#[derive(Debug)]
#[must_use = "dropping the handle stops the service"]
pub struct ServiceHandle {
    commands: Sender<Command>,
    events: async_channel::Receiver<Event>,
    /// `None` once the thread has been joined, or when it could not be started.
    thread: Option<JoinHandle<()>>,
}

/// The service thread has stopped, so it takes no more commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the service thread has stopped")]
pub struct ServiceStopped;

impl ServiceHandle {
    /// Hands `command` to the service thread; never blocks.
    ///
    /// # Errors
    ///
    /// [`ServiceStopped`] when the thread has stopped: after a [`Command::Shutdown`], after the
    /// UI closed [`ServiceHandle::events`], or when it could not be started.
    pub fn send(&self, command: Command) -> Result<(), ServiceStopped> {
        self.commands.send(command).map_err(|_| ServiceStopped)
    }

    /// The events of the service, oldest first; the channel closes when the thread stops.
    ///
    /// Clones of the receiver share one queue: each event reaches only one of them. Closing the
    /// channel stops the thread at its next wakeup.
    #[must_use]
    pub fn events(&self) -> &async_channel::Receiver<Event> {
        &self.events
    }

    /// A sender of commands, for code that takes the channel rather than the handle.
    ///
    /// A [`Command::Shutdown`] sent through it stops the thread too, without waiting for it.
    #[must_use]
    pub fn commands(&self) -> Sender<Command> {
        self.commands.clone()
    }

    /// Stops the service thread and waits until it has ended.
    ///
    /// The thread first finishes the request it is sending, if any, which the client's timeout
    /// bounds; actions it has not sent yet are dropped.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        if self.commands.send(Command::Shutdown).is_err() {
            tracing::debug!("the service thread had already stopped");
        }
        if let Err(payload) = thread.join() {
            tracing::error!(
                panic = panic_message(payload.as_ref()),
                "the service thread panicked"
            );
        }
    }
}

impl Drop for ServiceHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Starts the service thread, which drives an [`Engine`] with `config` through `client`.
///
/// The engine starts connecting at once, and the first [`Event::Snapshot`] says so before the
/// daemon has answered.
///
/// If the system cannot start a thread, the failure is logged and the handle comes back already
/// stopped: [`ServiceHandle::send`] fails with [`ServiceStopped`] and [`ServiceHandle::events`]
/// is closed, so the UI sees what it sees when the thread stops.
pub fn spawn<T: Transport + 'static>(client: Client<T>, config: EngineConfig) -> ServiceHandle {
    let (commands, command_rx) = crossbeam_channel::unbounded();
    let (event_tx, events) = async_channel::unbounded();
    let started = thread::Builder::new()
        .name(THREAD_NAME.to_owned())
        .spawn(move || Worker::new(client, config, command_rx, event_tx).run());
    let thread = match started {
        Ok(thread) => Some(thread),
        Err(error) => {
            // The closure is dropped with the error, and with it the thread's ends of both
            // channels, which closes them.
            tracing::error!(%error, "cannot start the service thread");
            None
        }
    };
    ServiceHandle {
        commands,
        events,
        thread,
    }
}

/// Sends `request` through `client` and returns the answer in the form the engine takes.
///
/// An action goes out once: nothing is retried here, and the engine decides what a failure means.
///
/// # Errors
///
/// The [`RpcError`] of the call; [`RpcError::Selection`] when an action names no torrent, in
/// which case nothing is sent: the daemon would apply it to every torrent.
pub(crate) fn execute<T: Transport>(
    client: &mut Client<T>,
    request: &Request,
) -> Result<Response, RpcError> {
    match request {
        Request::CheckServer => client.check_server().map(Response::Server),
        Request::SessionGet => client.session_get().map(Response::Session),
        Request::SessionStats => client.session_stats().map(Response::Stats),
        Request::FreeSpace(path) => client.free_space(path).map(Response::FreeSpace),
        Request::FullList => client
            .torrent_get(&GetIds::All, TorrentField::LIST)
            .map(Response::Torrents),
        Request::RecentlyActive => client
            .torrent_get(&GetIds::RecentlyActive, TorrentField::LIST)
            .map(Response::Torrents),
        Request::Details(hash) => {
            let ids = GetIds::Selection(selection(slice::from_ref(hash))?);
            client
                .torrent_get(&ids, TorrentField::DETAILS)
                .map(Response::Details)
        }
        Request::Action(_, action) => perform(client, action).map(|()| Response::Done),
    }
}

/// Carries out `action` with the one call that does it.
fn perform<T: Transport>(client: &mut Client<T>, action: &Action) -> Result<(), RpcError> {
    match action {
        Action::Start(hashes) => client.torrent_action(TorrentAction::Start, &selection(hashes)?),
        Action::StartNow(hashes) => {
            client.torrent_action(TorrentAction::StartNow, &selection(hashes)?)
        }
        Action::Stop(hashes) => client.torrent_action(TorrentAction::Stop, &selection(hashes)?),
        Action::Verify(hashes) => client.torrent_action(TorrentAction::Verify, &selection(hashes)?),
        Action::Reannounce(hashes) => {
            client.torrent_action(TorrentAction::Reannounce, &selection(hashes)?)
        }
        Action::Remove { hashes, data } => client.torrent_remove(&selection(hashes)?, *data),
        Action::SetSequential { hash, enabled } => {
            let set = TorrentSet {
                sequential_download: Some(*enabled),
                ..TorrentSet::default()
            };
            client.torrent_set(&selection(slice::from_ref(hash))?, &set)
        }
        Action::SetFilePriority {
            hash,
            files,
            priority,
        } => client.torrent_set(
            &selection(slice::from_ref(hash))?,
            &priority_set(files, *priority),
        ),
        Action::SetFilesWanted {
            hash,
            files,
            wanted,
        } => client.torrent_set(
            &selection(slice::from_ref(hash))?,
            &wanted_set(files, *wanted),
        ),
        Action::SetAltSpeed(enabled) => client.session_set(&SessionSet {
            alt_speed_enabled: Some(*enabled),
            ..SessionSet::default()
        }),
        Action::SetSpeedLimits { down_kbps, up_kbps } => {
            client.session_set(&speed_limits(*down_kbps, *up_kbps))
        }
        Action::Add(add) => client.torrent_add(add).map(|outcome| {
            if let AddOutcome::Duplicate(_) = outcome {
                tracing::info!("the daemon already had the torrent it was asked to add");
            }
        }),
    }
}

/// The torrents `hashes` names. An empty list is an error and never goes out: the daemon reads
/// a missing `ids` as every torrent.
fn selection(hashes: &[HashString]) -> Result<TorrentSelection, RpcError> {
    TorrentSelection::from_hashes(hashes.iter().cloned()).map_err(RpcError::Selection)
}

fn priority_set(files: &[usize], priority: FilePriority) -> TorrentSet {
    let files = files.to_vec();
    match priority {
        FilePriority::High => TorrentSet {
            priority_high: files,
            ..TorrentSet::default()
        },
        FilePriority::Normal => TorrentSet {
            priority_normal: files,
            ..TorrentSet::default()
        },
        FilePriority::Low => TorrentSet {
            priority_low: files,
            ..TorrentSet::default()
        },
    }
}

fn wanted_set(files: &[usize], wanted: bool) -> TorrentSet {
    let files = files.to_vec();
    if wanted {
        TorrentSet {
            files_wanted: files,
            ..TorrentSet::default()
        }
    } else {
        TorrentSet {
            files_unwanted: files,
            ..TorrentSet::default()
        }
    }
}

/// The normal speed limits in kB/s; a missing one is switched off and keeps its stored value.
fn speed_limits(down_kbps: Option<u32>, up_kbps: Option<u32>) -> SessionSet {
    SessionSet {
        speed_limit_down_kbps: down_kbps,
        speed_limit_down_enabled: Some(down_kbps.is_some()),
        speed_limit_up_kbps: up_kbps,
        speed_limit_up_enabled: Some(up_kbps.is_some()),
        ..SessionSet::default()
    }
}

fn panic_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("a panic without a message")
}

/// What the service thread owns: the engine, the client it drives and the thread's ends of the
/// two channels.
struct Worker<T: Transport> {
    client: Client<T>,
    engine: Engine,
    commands: Receiver<Command>,
    events: async_channel::Sender<Event>,
}

impl<T: Transport> Worker<T> {
    fn new(
        client: Client<T>,
        config: EngineConfig,
        commands: Receiver<Command>,
        events: async_channel::Sender<Event>,
    ) -> Self {
        Self {
            client,
            engine: Engine::new(config, Instant::now()),
            commands,
            events,
        }
    }

    fn run(mut self) {
        tracing::info!("service started");
        // The UI hears that the service is connecting before the first answer, which can take as
        // long as the client's timeout.
        let mut flow = self.publish_snapshot();
        while flow.is_continue() {
            flow = self.step();
        }
        tracing::info!("service stopped");
    }

    /// One turn of the loop: take the commands that are waiting, so that actions lead the batch;
    /// send the requests that are due; publish the snapshot if it changed; then wait for the
    /// next wakeup or command.
    fn step(&mut self) -> ControlFlow<()> {
        if self.events.is_closed() {
            tracing::debug!("the UI closed its events");
            return ControlFlow::Break(());
        }
        self.take_waiting_commands()?;
        self.send_due_requests()?;
        self.publish_snapshot()?;
        self.wait()
    }

    /// Publishes the state if it changed since the last snapshot.
    fn publish_snapshot(&mut self) -> ControlFlow<()> {
        match self.engine.take_snapshot() {
            Some(snapshot) => self.publish(Event::Snapshot(snapshot)),
            None => ControlFlow::Continue(()),
        }
    }

    fn send_due_requests(&mut self) -> ControlFlow<()> {
        for request in self.engine.due_requests(Instant::now()) {
            // A command taken between two requests can make the rest of the batch out of date,
            // and a shutdown should not wait for the whole batch.
            self.take_waiting_commands()?;
            if !self.engine.is_current(&request) {
                tracing::debug!(
                    request = request.name(),
                    "skipped a request that is no longer current"
                );
                continue;
            }
            let sent = Instant::now();
            let result = execute(&mut self.client, &request);
            // The engine measures the next turn of a poll from the moment its answer arrived.
            let arrived = Instant::now();
            tracing::debug!(
                request = request.name(),
                took = ?arrived.duration_since(sent),
                ok = result.is_ok(),
                "request answered"
            );
            let events = self.engine.handle_response(arrived, &request, result);
            self.publish_all(events)?;
        }
        ControlFlow::Continue(())
    }

    /// Sleeps until the engine's next wakeup, or until a command comes in.
    fn wait(&mut self) -> ControlFlow<()> {
        let wakeup = self.engine.next_wakeup(Instant::now());
        match self.commands.recv_deadline(wakeup) {
            Ok(command) => self.take(command),
            Err(RecvTimeoutError::Timeout) => ControlFlow::Continue(()),
            Err(RecvTimeoutError::Disconnected) => Self::no_senders(),
        }
    }

    fn take_waiting_commands(&mut self) -> ControlFlow<()> {
        loop {
            match self.commands.try_recv() {
                Ok(command) => self.take(command)?,
                Err(TryRecvError::Empty) => return ControlFlow::Continue(()),
                Err(TryRecvError::Disconnected) => return Self::no_senders(),
            }
        }
    }

    fn take(&mut self, command: Command) -> ControlFlow<()> {
        match command {
            Command::Shutdown => {
                tracing::debug!("the service was asked to stop");
                ControlFlow::Break(())
            }
            command => {
                let events = self.engine.handle_command(Instant::now(), command);
                self.publish_all(events)
            }
        }
    }

    /// The handle sends a shutdown before it drops its sender, so this is only a safety net.
    fn no_senders() -> ControlFlow<()> {
        tracing::debug!("no one can send commands any more");
        ControlFlow::Break(())
    }

    fn publish_all(&self, events: Vec<Event>) -> ControlFlow<()> {
        events.into_iter().try_for_each(|event| self.publish(event))
    }

    /// Sends `event` to the UI; the channel is unbounded, so this never blocks. Stops the loop
    /// when the UI has closed the channel.
    fn publish(&self, event: Event) -> ControlFlow<()> {
        if self.events.send_blocking(event).is_err() {
            tracing::debug!("the UI closed its events");
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use serde_json::{Value, json};
    use x4_rpc::testing::fixtures::{
        FREE_SPACE, SESSION_GET, SESSION_STATS, TORRENT_GET_DETAILS, TORRENT_GET_LIST,
        TORRENT_GET_RECENT,
    };
    use x4_rpc::testing::{FakeTransport, RecordedRequest, ok};
    use x4_rpc::{
        AddSource, FilePriority, HashString, HttpResponse, IdError, RemoveData, SESSION_FIELDS,
        TorrentAdd, TorrentField,
    };

    use super::*;
    use crate::action::{Action, ActionId};
    use crate::test_support::{fixture_result, torrent};

    /// Answers like the recorded daemon: each read with its fixture, an add with a new torrent
    /// and every other action with success.
    fn daemon(method: &str, params: &Value) -> HttpResponse {
        let fixture = match method {
            "session_get" => SESSION_GET,
            "session_stats" => SESSION_STATS,
            "free_space" => FREE_SPACE,
            "torrent_get" => match params.get("ids") {
                None => TORRENT_GET_LIST,
                Some(Value::String(_)) => TORRENT_GET_RECENT,
                Some(_) => TORRENT_GET_DETAILS,
            },
            "torrent_add" => {
                let added = json!({ "id": 9, "name": "Torrent 9", "hash_string": hash("a") });
                return ok(json!({ "torrent_added": added }));
            }
            _ => return ok(json!({})),
        };
        ok(fixture_result(fixture))
    }

    fn hash(name: &str) -> HashString {
        torrent(name).hash
    }

    /// Executes `request` against [`daemon`]; returns the answer and what went over the wire.
    fn execute_one(request: &Request) -> (Result<Response, RpcError>, Vec<RecordedRequest>) {
        let fake = FakeTransport::routed(|method, params| Ok(daemon(method, params)));
        let mut client = Client::new(fake.clone());
        let result = execute(&mut client, request);
        (result, fake.requests())
    }

    /// The one call `action` makes, as method and params; the action must succeed.
    fn sent_for(action: Action) -> (String, Value) {
        let (result, mut sent) = execute_one(&Request::Action(ActionId(1), action));
        assert_eq!(result, Ok(Response::Done));
        assert_eq!(sent.len(), 1, "{sent:?}");
        let request = sent.remove(0);
        (request.method, request.params)
    }

    fn fields(fields: &[TorrentField]) -> Value {
        serde_json::to_value(fields).unwrap()
    }

    #[test]
    fn each_poll_reads_what_it_names() {
        let selected = hash("a");
        let cases = [
            (
                Request::CheckServer,
                "session_get",
                json!({ "fields": ["version", "rpc_version_semver"] }),
            ),
            (
                Request::SessionGet,
                "session_get",
                json!({ "fields": SESSION_FIELDS }),
            ),
            (Request::SessionStats, "session_stats", Value::Null),
            (
                Request::FreeSpace("/data/torrents/video".to_owned()),
                "free_space",
                json!({ "path": "/data/torrents/video" }),
            ),
            (
                Request::FullList,
                "torrent_get",
                json!({ "fields": fields(TorrentField::LIST) }),
            ),
            (
                Request::RecentlyActive,
                "torrent_get",
                json!({ "ids": "recently_active", "fields": fields(TorrentField::LIST) }),
            ),
            (
                Request::Details(selected.clone()),
                "torrent_get",
                json!({ "ids": [selected.as_str()], "fields": fields(TorrentField::DETAILS) }),
            ),
        ];

        for (request, method, params) in cases {
            let (result, sent) = execute_one(&request);

            assert!(result.is_ok(), "{request:?}: {result:?}");
            assert_eq!(sent.len(), 1, "{request:?}");
            assert_eq!(sent[0].method, method, "{request:?}");
            assert_eq!(sent[0].params, params, "{request:?}");
        }
    }

    #[test]
    fn each_poll_is_answered_with_its_response() {
        let answer = |request| execute_one(&request).0.unwrap();

        assert!(matches!(answer(Request::CheckServer), Response::Server(v) if v.is_supported()));
        assert!(matches!(answer(Request::SessionGet), Response::Session(_)));
        assert!(matches!(answer(Request::SessionStats), Response::Stats(_)));
        assert!(matches!(
            answer(Request::FreeSpace("/data/torrents/video".to_owned())),
            Response::FreeSpace(_)
        ));
        assert!(matches!(answer(Request::FullList), Response::Torrents(_)));
        assert!(matches!(
            answer(Request::RecentlyActive),
            Response::Torrents(_)
        ));
        assert!(matches!(
            answer(Request::Details(hash("a"))),
            Response::Details(_)
        ));
    }

    #[test]
    fn torrent_actions_name_their_torrents_by_hash() {
        let hashes = || vec![hash("a"), hash("b")];
        let ids = json!({ "ids": [hash("a").as_str(), hash("b").as_str()] });
        let cases = [
            (Action::Start(hashes()), "torrent_start"),
            (Action::StartNow(hashes()), "torrent_start_now"),
            (Action::Stop(hashes()), "torrent_stop"),
            (Action::Verify(hashes()), "torrent_verify"),
            (Action::Reannounce(hashes()), "torrent_reannounce"),
        ];

        for (action, method) in cases {
            assert_eq!(sent_for(action), (method.to_owned(), ids.clone()));
        }
    }

    #[test]
    fn remove_passes_the_data_choice_through() {
        let remove = |data| Action::Remove {
            hashes: vec![hash("a")],
            data,
        };
        let params =
            |delete: bool| json!({ "ids": [hash("a").as_str()], "delete_local_data": delete });

        assert_eq!(
            sent_for(remove(RemoveData::Keep)),
            ("torrent_remove".to_owned(), params(false))
        );
        assert_eq!(
            sent_for(remove(RemoveData::Delete)),
            ("torrent_remove".to_owned(), params(true))
        );
    }

    #[test]
    fn torrent_settings_fill_only_their_own_field() {
        let one = |extra: Value| {
            let mut params = json!({ "ids": [hash("a").as_str()] });
            params
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            ("torrent_set".to_owned(), params)
        };
        let priority = |priority| Action::SetFilePriority {
            hash: hash("a"),
            files: vec![0, 2],
            priority,
        };
        let wanted = |wanted| Action::SetFilesWanted {
            hash: hash("a"),
            files: vec![1, 3],
            wanted,
        };

        assert_eq!(
            sent_for(Action::SetSequential {
                hash: hash("a"),
                enabled: true,
            }),
            one(json!({ "sequential_download": true }))
        );
        assert_eq!(
            sent_for(priority(FilePriority::High)),
            one(json!({ "priority_high": [0, 2] }))
        );
        assert_eq!(
            sent_for(priority(FilePriority::Normal)),
            one(json!({ "priority_normal": [0, 2] }))
        );
        assert_eq!(
            sent_for(priority(FilePriority::Low)),
            one(json!({ "priority_low": [0, 2] }))
        );
        assert_eq!(
            sent_for(wanted(true)),
            one(json!({ "files_wanted": [1, 3] }))
        );
        assert_eq!(
            sent_for(wanted(false)),
            one(json!({ "files_unwanted": [1, 3] }))
        );
    }

    #[test]
    fn a_missing_speed_limit_turns_that_limit_off() {
        assert_eq!(
            sent_for(Action::SetSpeedLimits {
                down_kbps: None,
                up_kbps: Some(250),
            }),
            (
                "session_set".to_owned(),
                json!({
                    "speed_limit_down_enabled": false,
                    "speed_limit_up": 250,
                    "speed_limit_up_enabled": true,
                })
            )
        );
        assert_eq!(
            sent_for(Action::SetSpeedLimits {
                down_kbps: Some(100),
                up_kbps: None,
            }),
            (
                "session_set".to_owned(),
                json!({
                    "speed_limit_down": 100,
                    "speed_limit_down_enabled": true,
                    "speed_limit_up_enabled": false,
                })
            )
        );
    }

    #[test]
    fn the_alternative_speed_switch_sets_only_itself() {
        assert_eq!(
            sent_for(Action::SetAltSpeed(true)),
            (
                "session_set".to_owned(),
                json!({ "alt_speed_enabled": true })
            )
        );
    }

    #[test]
    fn add_sends_the_torrent() {
        let add = TorrentAdd {
            source: AddSource::Magnet("magnet:?xt=urn:btih:example".to_owned()),
            download_dir: None,
            paused: true,
            files_unwanted: Vec::new(),
            priority_high: Vec::new(),
            priority_low: Vec::new(),
            sequential_download: None,
        };

        assert_eq!(
            sent_for(Action::Add(add)),
            (
                "torrent_add".to_owned(),
                json!({ "filename": "magnet:?xt=urn:btih:example", "paused": true })
            )
        );
    }

    #[test]
    fn an_empty_selection_never_reaches_the_wire() {
        let empty = [
            Action::Stop(Vec::new()),
            Action::Remove {
                hashes: Vec::new(),
                data: RemoveData::Delete,
            },
        ];

        for action in empty {
            let (result, sent) = execute_one(&Request::Action(ActionId(1), action));

            assert_eq!(result, Err(RpcError::Selection(IdError::EmptySelection)));
            assert!(sent.is_empty(), "{sent:?}");
        }
    }

    #[test]
    fn the_loop_sleeps_until_the_next_wakeup() {
        let (_commands_tx, commands) = crossbeam_channel::unbounded();
        let (events, _events_rx) = async_channel::unbounded();
        let config = EngineConfig {
            visible_interval: Duration::from_millis(20),
            full_refresh_interval: Duration::from_secs(60),
            session_interval: Duration::from_secs(60),
            free_space_interval: Duration::from_secs(60),
            ..EngineConfig::default()
        };
        let mut worker = Worker::new(
            Client::new(FakeTransport::routed(|method, params| {
                Ok(daemon(method, params))
            })),
            config,
            commands,
            events,
        );

        let started = Instant::now();
        let mut steps = 0_u32;
        while started.elapsed() < Duration::from_millis(300) {
            assert!(worker.step().is_continue());
            steps += 1;
        }

        // One step per 20 ms tick plus the few that connect; a loop that did not wait would
        // turn thousands of times.
        assert!(steps <= 40, "{steps} steps in 300 ms");
    }
}
