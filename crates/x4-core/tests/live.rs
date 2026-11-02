//! Live tests of x4-core against a real Transmission daemon, run only by `just test-live`.
//!
//! They use the live test support of x4-rpc (`crates/x4-rpc/tests/live/`) and keep its rules:
//! they act on nothing but a torrent they create in the test directory, change it only through a
//! selection of its hash, never call `session_set`, and check that every other torrent kept its
//! status, sequential mode, file priorities and wanted files. A drop guard removes the test
//! torrent with its data on every path.
//!
//! The engine is driven as the service drives it, on a clock the test moves: each request it
//! hands out is answered by the daemon through the same call the service makes, all of them
//! reads. So the daemon's real answers go through `TorrentRecord` and into the engine's details,
//! built from a full read and merged from the progress reads.

#[path = "../../x4-rpc/tests/live/metainfo.rs"]
mod metainfo;
#[path = "../../x4-rpc/tests/live/support.rs"]
#[expect(
    dead_code,
    reason = "shared with x4-rpc's live tests, which use the rest of it"
)]
mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use support::{
    LiveEnv, MULTI_FILES, MULTI_PIECE_LENGTH, TestTorrent, assert_others_unchanged,
    others_fingerprint, torrent_by_hash,
};
use x4_core::{
    Command, ConnectionState, DetailsRead, Engine, EngineConfig, Event, Request, Response,
    TorrentDetails, Visibility,
};
use x4_rpc::{
    Client, FilePriority, GetIds, HashString, RpcError, TorrentField, TorrentGetResponse,
    TorrentRecord, TorrentSelection, TorrentSet, UreqTransport,
};

/// How many batches of requests the engine may hand out before it has nothing more to ask at one
/// moment; it needs two or three.
const TURNS: usize = 10;

/// The engine, the moment it is told it is, and the daemon that answers it.
struct Driven<'a> {
    engine: Engine,
    now: Instant,
    client: &'a mut Client<UreqTransport>,
    /// The test torrent, the only one the engine may read the details of.
    own: HashString,
    /// The name of its folder, which starts the name of each of its files.
    folder: String,
    /// The details reads the engine handed out, in order.
    reads: Vec<DetailsRead>,
    /// How many times it checked the server: once, unless it lost the connection.
    checks: usize,
}

impl Driven<'_> {
    /// Answers every request due now, and those the answers make due, until none is; returns the
    /// details the engine handed out meanwhile.
    #[expect(
        clippy::panic,
        reason = "a live test stops at the first failure; the guard cleans up"
    )]
    fn settle(&mut self) -> Vec<Arc<TorrentDetails>> {
        let mut details = Vec::new();
        for _ in 0..TURNS {
            let requests = self.engine.due_requests(self.now);
            if requests.is_empty() {
                return details;
            }
            for request in requests {
                if !self.engine.is_current(&request) {
                    continue;
                }
                let answer = self.answer(&request);
                for event in self.engine.handle_response(self.now, &request, Ok(answer)) {
                    if let Event::Details(read) = event {
                        details.push(read);
                    }
                }
            }
        }
        panic!("the engine still had requests after {TURNS} turns");
    }

    /// The daemon's answer to `request`, through the call the service makes for it. Every answer
    /// must come and parse: one that does not is what would make the app reconnect again and
    /// again.
    #[expect(
        clippy::panic,
        reason = "a live test stops at the first failure; the guard cleans up"
    )]
    fn answer(&mut self, request: &Request) -> Response {
        let client = &mut *self.client;
        let answer = match request {
            Request::CheckServer => {
                self.checks += 1;
                client.check_server().map(Response::Server)
            }
            Request::SessionGet => client.session_get().map(Response::Session),
            Request::SessionStats => client.session_stats().map(Response::Stats),
            Request::FreeSpace(path) => client.free_space(path).map(Response::FreeSpace),
            Request::FullList => client
                .torrent_get(&GetIds::All, TorrentField::LIST)
                .map(Response::Torrents),
            Request::RecentlyActive => client
                .torrent_get(&GetIds::RecentlyActive, TorrentField::LIST)
                .map(Response::Torrents),
            Request::Details { hash, kind } => {
                assert_eq!(hash, &self.own, "the engine reads only the test torrent");
                self.reads.push(*kind);
                read_details(client, hash, *kind).map(|answer| {
                    check_record(&answer, *kind, &self.folder);
                    Response::Details(answer)
                })
            }
            Request::Action(..) | Request::MagnetLinks(..) => {
                panic!("this test runs no action through the engine: {request:?}")
            }
        };
        answer.unwrap_or_else(|error| panic!("{request:?} failed: {error}"))
    }

    /// Moves the clock on by `by`.
    fn wait(&mut self, by: Duration) {
        self.now += by;
    }

    /// Hands the engine `command`, which brings no event at once.
    fn command(&mut self, command: Command) {
        let events = self.engine.handle_command(self.now, command);
        assert_eq!(events, [], "the command brought events at once");
    }

    /// Whether the engine is connected, as its latest snapshot says.
    fn connected(&mut self) -> bool {
        self.engine
            .take_snapshot()
            .is_none_or(|snapshot| snapshot.connection == ConnectionState::Connected)
    }
}

/// The details of `hash` with the fields of `kind`, read as the service reads them: one named
/// torrent.
fn read_details(
    client: &mut Client<UreqTransport>,
    hash: &HashString,
    kind: DetailsRead,
) -> Result<TorrentGetResponse, RpcError> {
    let selection = TorrentSelection::from_hashes([hash.clone()]).map_err(RpcError::Selection)?;
    client.torrent_get(&GetIds::Selection(selection), kind.fields())
}

/// Checks that the daemon's `answer` to a `kind` read holds what that read asks for, file by file
/// for the test torrent of [`MULTI_FILES`] in `folder`, and nothing it does not.
#[expect(
    clippy::panic,
    reason = "a live test stops at the first failure; the guard cleans up"
)]
fn check_record(answer: &TorrentGetResponse, kind: DetailsRead, folder: &str) {
    let [record]: &[TorrentRecord; 1] = answer
        .torrents
        .as_slice()
        .try_into()
        .unwrap_or_else(|_| panic!("{kind:?}: not one torrent: {answer:?}"));
    let count = MULTI_FILES.len();
    let per_file = |values: Option<usize>, what: &str| {
        assert_eq!(values, Some(count), "{kind:?}: one {what} per file");
    };
    assert_eq!(record.file_count, u32::try_from(count).ok(), "{kind:?}");
    per_file(record.priorities.as_ref().map(Vec::len), "priority");
    per_file(record.wanted.as_ref().map(Vec::len), "wanted switch");
    assert!(
        record
            .pieces
            .as_deref()
            .is_some_and(|pieces| !pieces.is_empty()),
        "{kind:?}: no pieces"
    );
    let trackers = record.tracker_stats.as_ref().map(Vec::len);
    match kind {
        DetailsRead::Full => {
            let files: Vec<(String, u64)> = record
                .files
                .iter()
                .flatten()
                .map(|file| (file.name.clone(), file.length))
                .collect();
            let expected: Vec<(String, u64)> = MULTI_FILES
                .iter()
                .map(|(name, size)| {
                    (
                        format!("{folder}/{name}"),
                        u64::try_from(*size).unwrap_or(0),
                    )
                })
                .collect();
            assert_eq!(files, expected, "{kind:?}");
            assert_eq!(record.piece_count, Some(2), "{kind:?}");
            assert_eq!(
                record.piece_size,
                u64::try_from(MULTI_PIECE_LENGTH).ok(),
                "{kind:?}"
            );
            assert_eq!(trackers, Some(0), "{kind:?}: a trackerless torrent");
        }
        DetailsRead::Progress => {
            per_file(record.bytes_completed.as_ref().map(Vec::len), "byte count");
            assert!(record.files.is_none(), "{kind:?}: no file names");
            assert_eq!(trackers, None, "{kind:?}: no trackers");
        }
        DetailsRead::ProgressAndTrackers => {
            per_file(record.bytes_completed.as_ref().map(Vec::len), "byte count");
            assert!(record.files.is_none(), "{kind:?}: no file names");
            assert_eq!(trackers, Some(0), "{kind:?}: the trackers, none");
        }
    }
}

/// Sends `set` for the test torrent `selection` names, which must succeed.
#[expect(
    clippy::panic,
    reason = "a live test stops at the first failure; the guard cleans up"
)]
fn set(client: &mut Client<UreqTransport>, selection: &TorrentSelection, set: &TorrentSet) {
    if let Err(error) = client.torrent_set(selection, set) {
        panic!("torrent_set {set:?} on the test torrent failed: {error}");
    }
}

/// The last of `details`, which must be the test torrent's.
#[expect(
    clippy::panic,
    reason = "a live test stops at the first failure; the guard cleans up"
)]
fn latest(details: &[Arc<TorrentDetails>], own: &HashString, after: &str) -> TorrentDetails {
    let Some(last) = details.last() else {
        panic!("the engine gave no details after {after}");
    };
    assert_eq!(&last.hash, own, "{after}");
    TorrentDetails::clone(last)
}

/// Checks the details of a full read of the seeding test torrent of [`MULTI_FILES`] in `folder`.
fn check_full(full: &TorrentDetails, folder: &str) {
    let names: Vec<(usize, String, u64)> = full
        .files
        .iter()
        .map(|file| (file.index, file.name.clone(), file.length))
        .collect();
    let expected: Vec<(usize, String, u64)> = MULTI_FILES
        .iter()
        .enumerate()
        .map(|(index, (name, size))| {
            (
                index,
                format!("{folder}/{name}"),
                u64::try_from(*size).unwrap_or(0),
            )
        })
        .collect();
    assert_eq!(names, expected);
    for file in &full.files {
        // Seeding: every piece is there, so all of each file is done and plays from its start.
        assert_eq!(file.bytes_completed, file.length, "{file:?}");
        assert_eq!(file.playable_from_start, file.length, "{file:?}");
    }
    assert_eq!(switches(full), [(true, FilePriority::Normal); 3]);
    assert_eq!((full.piece_count, full.piece_size), (2, 16 * 1024));
    assert_eq!(full.trackers, [], "a trackerless torrent");
    assert_eq!(full.source, None);
    assert!(!full.is_private);
}

/// Each file's wanted switch and priority in `details`.
fn switches(details: &TorrentDetails) -> Vec<(bool, FilePriority)> {
    details
        .files
        .iter()
        .map(|file| (file.wanted, file.priority))
        .collect()
}

#[test]
#[ignore = "live: needs a Transmission daemon, run with just test-live"]
fn details_full_and_progress_fields_parse_on_an_own_torrent() {
    use FilePriority::{High, Normal};
    let env = LiveEnv::from_env();
    let mut client = env.client();
    let before = others_fingerprint(&mut client, None);
    let torrent = TestTorrent::create_multi_file(&mut client, &env);
    let own = torrent.hash().clone();
    let selection = torrent.selection();
    let folder = torrent
        .local_data()
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let start = Instant::now();
    let mut driven = Driven {
        engine: Engine::new(EngineConfig::default(), start),
        now: start,
        client: &mut client,
        own: own.clone(),
        folder: folder.clone(),
        reads: Vec::new(),
        checks: 0,
    };
    driven.command(Command::Select(Some(own.clone())));

    // Connected, the engine reads the details in full once the selection has settled.
    let mut details = driven.settle();
    driven.wait(Duration::from_secs(1));
    details.extend(driven.settle());
    check_full(&latest(&details, &own, "the full read"), &folder);

    // A progress read, as when the window shows again, brings the second file skipped.
    let skip_second = TorrentSet {
        files_unwanted: vec![1],
        ..TorrentSet::default()
    };
    set(driven.client, &selection, &skip_second);
    driven.command(Command::SetVisibility(Visibility::Hidden));
    driven.command(Command::SetVisibility(Visibility::Visible));
    driven.wait(Duration::from_millis(100));
    let progress = latest(&driven.settle(), &own, "the progress read");
    assert_eq!(
        switches(&progress),
        [(true, Normal), (false, Normal), (true, Normal)]
    );
    assert_eq!(progress.files.len(), MULTI_FILES.len());

    // The progress read that goes out with the whole list, a minute on, brings the trackers too.
    let third_high = TorrentSet {
        priority_high: vec![2],
        ..TorrentSet::default()
    };
    set(driven.client, &selection, &third_high);
    driven.wait(Duration::from_secs(61));
    let with_trackers = latest(
        &driven.settle(),
        &own,
        "the progress read with the trackers",
    );
    assert_eq!(
        switches(&with_trackers),
        [(true, Normal), (false, Normal), (true, High)]
    );
    assert_eq!(with_trackers.trackers, []);

    assert_eq!(
        driven.reads,
        [
            DetailsRead::Full,
            DetailsRead::Progress,
            DetailsRead::ProgressAndTrackers
        ]
    );
    assert_eq!(driven.checks, 1, "the engine never lost the connection");
    assert!(driven.connected());
    assert_others_unchanged(&mut client, Some(&own), &before);

    torrent.remove_with_data(&mut client);
    assert_eq!(torrent_by_hash(&mut client, &own), Ok(None));
    assert_others_unchanged(&mut client, None, &before);
}
