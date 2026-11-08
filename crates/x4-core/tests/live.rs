//! Live tests of x4-core against a real Transmission daemon, run only by `just test-live` and
//! `just test-live-legacy`.
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
//!
//! The tabs of the details panel are checked with reads only, on a torrent of the daemon's own
//! that has peers: the engine's reads of the Peers and the Trackers tab, and what their answers
//! become. That test changes nothing and reports counts only, never an address, a name or a hash.
//!
//! The add path is checked against the daemon's own numbering of the files: a torrent of this
//! run, whose files are not in the order their names sort in, with an empty file and a padding
//! file among them, is read by the strict parser, a file past the padding is unchecked in the add
//! form, and the form's request is sent as the add dialog sends it,
//! paused and with no data written. The daemon must list the files in the parse's order, with
//! that file, and only that one, left out.

#[path = "../../x4-rpc/tests/live/metainfo.rs"]
mod metainfo;
#[path = "../../x4-rpc/tests/live/support.rs"]
#[expect(
    dead_code,
    reason = "shared with x4-rpc's live tests, which use the rest of it"
)]
mod support;

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use support::{
    LiveEnv, MULTI_FILES, MULTI_PIECE_LENGTH, OwnTorrent, TestFolder, TestTorrent,
    assert_others_unchanged, others_fingerprint, torrent_by_hash,
};
use x4_core::{
    AddCheck, AddForm, Command, ConnectionState, DetailsRead, DetailsTab, Engine, EngineConfig,
    Event, LoadedItem, NoChoiceReason, NodeKey, PeerList, Request, Response, Snapshot,
    TorrentDetails, Visibility, parse_torrent,
};
use x4_rpc::{
    Client, FilePriority, GetIds, HashString, RpcError, TorrentField, TorrentGetResponse,
    TorrentRecord, TorrentSelection, TorrentSet, UreqTransport,
};

/// How many batches of requests the engine may hand out before it has nothing more to ask at one
/// moment; it needs two or three.
const TURNS: usize = 10;
/// The files of the torrent added with a file left out, in its order, with their sizes: not the
/// order their names sort in, one of them in a folder of the torrent, an empty one, and a padding
/// file (BEP 47) that ends the first piece, so that `alpha.bin` begins the second.
const OUT_OF_ORDER: [(&str, usize); 5] = [
    ("zeta.bin", 9 * 1024),
    (".pad/7168", 7 * 1024),
    ("alpha.bin", 13 * 1024),
    ("empty.bin", 0),
    ("mid/beta.bin", 5 * 1024),
];
/// What a test reads of the files of its torrent: their names and lengths, and which are wanted.
const NAMES_AND_WANTED: &[TorrentField] = &[
    TorrentField::Id,
    TorrentField::HashString,
    TorrentField::Files,
    TorrentField::Wanted,
];

/// The engine, the moment it is told it is, and the daemon that answers it.
struct Driven<'a> {
    engine: Engine,
    now: Instant,
    client: &'a mut Client<UreqTransport>,
    /// The torrent the test selected, the only one the engine may read the details of: the test
    /// torrent, or one of the daemon's own for a test that only reads.
    selected: Option<HashString>,
    /// The name of the test torrent's folder, which starts the name of each of its files; `None`
    /// for a torrent of the daemon's own, whose files the test does not know.
    folder: Option<String>,
    /// The details reads the engine handed out, in order.
    reads: Vec<DetailsRead>,
    /// How many times it checked the server: once, unless it lost the connection.
    checks: usize,
    /// How many calls went to the daemon, each one request (the session handshake aside).
    calls: usize,
    /// How long to wait before each call after the first, in real time: a test that only reads
    /// the daemon's own torrents spares the Raspberry Pi it runs on.
    pace: Duration,
    /// How many entries the last answer of each tab held: peers, and trackers.
    tab_entries: (Option<usize>, Option<usize>),
}

impl Driven<'_> {
    /// Answers every request due now, and those the answers make due, until none is; returns the
    /// details the engine handed out meanwhile.
    fn settle(&mut self) -> Vec<Arc<TorrentDetails>> {
        details_of(&self.settle_events())
    }

    /// Answers every request due now, and those the answers make due, until none is; returns the
    /// events the answers brought.
    #[expect(
        clippy::panic,
        reason = "a live test stops at the first failure; the guard cleans up"
    )]
    fn settle_events(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        for _ in 0..TURNS {
            let requests = self.engine.due_requests(self.now);
            if requests.is_empty() {
                return events;
            }
            for request in requests {
                if !self.engine.is_current(&request) {
                    continue;
                }
                let answer = self.answer(&request);
                events.extend(self.engine.handle_response(self.now, &request, Ok(answer)));
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
        if self.calls > 0 && !self.pace.is_zero() {
            thread::sleep(self.pace);
        }
        let client = &mut *self.client;
        self.calls += 1;
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
                // The hashes are the owner's torrents': the message says only that they differ.
                assert!(
                    Some(hash) == self.selected.as_ref(),
                    "the engine reads only the torrent the test selected"
                );
                self.reads.push(*kind);
                read_details(client, hash, *kind).map(|answer| {
                    let entries = check_record(&answer, *kind, self.folder.as_deref());
                    match kind {
                        DetailsRead::Peers => self.tab_entries.0 = entries,
                        DetailsRead::Trackers => self.tab_entries.1 = entries,
                        DetailsRead::Full
                        | DetailsRead::Progress
                        | DetailsRead::ProgressAndTrackers => {}
                    }
                    Response::Details(answer)
                })
            }
            Request::Action(..)
            | Request::ListOne(_)
            | Request::MagnetLinks(..)
            | Request::FolderFreeSpace(_) => {
                panic!("this test runs no action and asks about no folder: {request:?}")
            }
        };
        // A read of the details names its torrent's hash, which the message leaves out.
        answer.unwrap_or_else(|error| match request {
            Request::Details { kind, .. } => panic!("the {kind:?} read failed: {error}"),
            _ => panic!("{request:?} failed: {error}"),
        })
    }

    /// Moves the clock on by `by`.
    fn wait(&mut self, by: Duration) {
        self.now += by;
    }

    /// Hands the engine `command`, which brings no event at once.
    fn command(&mut self, command: Command) {
        let events = self.engine.handle_command(self.now, command);
        assert!(
            events.is_empty(),
            "the command brought events at once: {:?}",
            kinds(&events)
        );
    }

    /// Hands the engine `command`; returns the events it brings at once.
    fn command_events(&mut self, command: Command) -> Vec<Event> {
        self.engine.handle_command(self.now, command)
    }

    /// Whether the engine is connected, as its latest snapshot says; the test takes none before,
    /// so there is one.
    fn connected(&mut self) -> bool {
        self.engine
            .take_snapshot()
            .is_some_and(|snapshot| snapshot.connection == ConnectionState::Connected)
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

/// The details events of `events`.
fn details_of(events: &[Event]) -> Vec<Arc<TorrentDetails>> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Details(details) => Some(Arc::clone(details)),
            _ => None,
        })
        .collect()
}

/// The peers events of `events`.
fn peers_of(events: &[Event]) -> Vec<Arc<PeerList>> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::Peers(list) => Some(Arc::clone(list)),
            _ => None,
        })
        .collect()
}

/// The kind of each of `events`, for a message: the events hold the owner's torrents, which no
/// message may show.
fn kinds(events: &[Event]) -> Vec<&'static str> {
    events
        .iter()
        .map(|event| match event {
            Event::Snapshot(_) => "Snapshot",
            Event::Details(_) => "Details",
            Event::Peers(_) => "Peers",
            Event::PeersCleared => "PeersCleared",
            Event::ActionFinished { .. } => "ActionFinished",
            Event::MagnetLinks { .. } => "MagnetLinks",
            Event::AddFinished { .. } => "AddFinished",
            Event::FreeSpace { .. } => "FreeSpace",
        })
        .collect()
}

/// Checks that the daemon's `answer` to a `kind` read holds what that read asks for, file by file
/// for the test torrent of [`MULTI_FILES`] in `folder`, and nothing it does not; with no `folder`,
/// a torrent of the daemon's own, the files are not checked. Returns how many entries a read of a
/// tab brought: peers or trackers.
#[expect(
    clippy::panic,
    reason = "a live test stops at the first failure; the guard cleans up"
)]
fn check_record(
    answer: &TorrentGetResponse,
    kind: DetailsRead,
    folder: Option<&str>,
) -> Option<usize> {
    let [record]: &[TorrentRecord; 1] = answer
        .torrents
        .as_slice()
        .try_into()
        .unwrap_or_else(|_| panic!("{kind:?}: not one torrent"));
    let trackers = record.tracker_stats.as_ref().map(Vec::len);
    match kind {
        DetailsRead::Peers => {
            let peers = record.peers.as_ref().map(Vec::len);
            let from = record
                .peers_from
                .unwrap_or_else(|| panic!("{kind:?}: no peers_from"));
            let sources = [
                from.from_cache,
                from.from_dht,
                from.from_incoming,
                from.from_lpd,
                from.from_ltep,
                from.from_pex,
                from.from_tracker,
            ];
            // One answer of a single-threaded daemon: each peer is counted by one source.
            assert_eq!(
                peers.and_then(|peers| u32::try_from(peers).ok()),
                Some(sources.iter().sum()),
                "{kind:?}: the peers and their sources"
            );
            assert!(record.files.is_none() && trackers.is_none(), "{kind:?}");
            return peers;
        }
        DetailsRead::Trackers => {
            assert!(trackers.is_some(), "{kind:?}: no tracker_stats");
            assert!(record.files.is_none() && record.peers.is_none(), "{kind:?}");
            return trackers;
        }
        DetailsRead::Full | DetailsRead::Progress | DetailsRead::ProgressAndTrackers => {}
    }
    let folder = folder?;
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
        DetailsRead::Peers | DetailsRead::Trackers => {}
    }
    None
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
    assert!(&last.hash == own, "{after}: the details of another torrent");
    TorrentDetails::clone(last)
}

/// Each file's index, name and length in `details`.
fn names(details: &TorrentDetails) -> Vec<(usize, String, u64)> {
    details
        .files
        .iter()
        .map(|file| (file.index, file.name.clone(), file.length))
        .collect()
}

/// The index, name and length of each file of the test torrent of [`MULTI_FILES`] in `folder`.
fn expected_names(folder: &str) -> Vec<(usize, String, u64)> {
    MULTI_FILES
        .iter()
        .enumerate()
        .map(|(index, (name, size))| {
            (
                index,
                format!("{folder}/{name}"),
                u64::try_from(*size).unwrap_or(0),
            )
        })
        .collect()
}

/// Checks the details of a full read of the seeding test torrent of [`MULTI_FILES`] in `folder`.
fn check_full(full: &TorrentDetails, folder: &str) {
    assert_eq!(names(full), expected_names(folder));
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
        selected: Some(own.clone()),
        folder: Some(folder.clone()),
        reads: Vec::new(),
        checks: 0,
        calls: 0,
        pace: Duration::ZERO,
        tab_entries: (None, None),
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
    assert_others_unchanged(driven.client, Some(&own), &before);
    driven.command(Command::SetVisibility(Visibility::Hidden));
    driven.command(Command::SetVisibility(Visibility::Visible));
    driven.wait(Duration::from_millis(100));
    let progress = latest(&driven.settle(), &own, "the progress read");
    assert_eq!(
        switches(&progress),
        [(true, Normal), (false, Normal), (true, Normal)]
    );
    // The progress read brings no names: the merge keeps those of the full read.
    assert_eq!(
        names(&progress),
        expected_names(&folder),
        "after a progress read"
    );

    // The progress read that goes out with the whole list, a minute on, brings the trackers too.
    let third_high = TorrentSet {
        priority_high: vec![2],
        ..TorrentSet::default()
    };
    set(driven.client, &selection, &third_high);
    assert_others_unchanged(driven.client, Some(&own), &before);
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
    assert_eq!(
        names(&with_trackers),
        expected_names(&folder),
        "after a progress read with the trackers"
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

#[test]
#[ignore = "live: needs a Transmission daemon, run with just test-live"]
fn the_peers_and_trackers_tabs_read_a_torrent_with_peers() {
    let env = LiveEnv::from_env();
    let mut client = env.client();
    let start = Instant::now();
    let mut driven = Driven {
        engine: Engine::new(EngineConfig::default(), start),
        now: start,
        client: &mut client,
        selected: None,
        folder: None,
        reads: Vec::new(),
        checks: 0,
        calls: 0,
        pace: Duration::from_secs(1),
        tab_entries: (None, None),
    };

    // Connected, the engine reads the whole list, which says how many peers each torrent has.
    driven.settle();
    let snapshot = driven
        .engine
        .take_snapshot()
        .expect("the first snapshot of the engine");
    assert_eq!(snapshot.connection, ConnectionState::Connected);
    let busiest = snapshot
        .torrents
        .iter()
        .max_by_key(|torrent| torrent.peers_connected)
        .filter(|torrent| torrent.peers_connected > 0);
    let Some(busiest) = busiest else {
        eprintln!(
            "tabs: skipped: none of the {} torrents has peers now ({} calls)",
            snapshot.torrents.len(),
            driven.calls
        );
        return;
    };
    let (hash, listed_peers) = (busiest.hash.clone(), busiest.peers_connected);

    // The Peers tab of the torrent: the full read once the selection settles, then the peers.
    driven.selected = Some(hash.clone());
    driven.command(Command::ShowTab(DetailsTab::Peers));
    driven.command(Command::Select(Some(hash.clone())));
    driven.wait(Duration::from_millis(250));
    let events = driven.settle_events();
    let full = latest(&details_of(&events), &hash, "the full read");
    let [peers] = <[Arc<PeerList>; 1]>::try_from(peers_of(&events))
        .unwrap_or_else(|events| panic!("not one list of peers: {} lists", events.len()));
    // No value of the owner's torrent goes into a message: only counts.
    assert!(peers.hash == hash, "the peers of another torrent");
    assert_eq!(
        Some(peers.peers.len()),
        driven.tab_entries.0,
        "a row for each peer of the answer"
    );
    assert!(peers.peers.iter().all(|peer| peer.country.is_none()));
    let addressed = peers.peers.iter().filter(|peer| peer.ip.is_some()).count();

    // The Trackers tab: the peers go, and the trackers are read and merged into the details.
    let left = driven.command_events(Command::ShowTab(DetailsTab::Trackers));
    assert!(
        matches!(left.as_slice(), [Event::PeersCleared]),
        "leaving the Peers tab sends only PeersCleared, not {:?}",
        kinds(&left)
    );
    let events = driven.settle_events();
    assert_eq!(peers_of(&events).len(), 0, "no peers on the Trackers tab");
    let merged = details_of(&events)
        .last()
        .map_or_else(|| full.clone(), |last| TorrentDetails::clone(last));
    assert!(merged.hash == hash, "the details of another torrent");
    assert_eq!(
        Some(merged.trackers.len()),
        driven.tab_entries.1,
        "the trackers of the answer replace the details' trackers"
    );
    assert!(merged.files == full.files, "the trackers keep the files");
    assert!(
        TorrentDetails {
            trackers: full.trackers.clone(),
            ..merged.clone()
        } == full,
        "nothing but the trackers changed"
    );

    assert_eq!(
        driven.reads,
        [DetailsRead::Full, DetailsRead::Peers, DetailsRead::Trackers]
    );
    assert_eq!(driven.checks, 1, "the engine never lost the connection");
    eprintln!(
        "tabs: {} torrents listed; the busiest has {listed_peers} peers in the list and {} in \
         its peers read ({addressed} with an IP address), {} trackers; {} calls to the daemon",
        snapshot.torrents.len(),
        peers.peers.len(),
        merged.trackers.len(),
        driven.calls
    );
}

/// A snapshot without the daemon's settings, in which the add form names its folder whatever it
/// is.
fn without_session() -> Snapshot {
    Snapshot {
        torrents: Vec::new(),
        listed: true,
        session: None,
        free_space_bytes: None,
        rate_down: 0,
        rate_up: 0,
        connection: ConnectionState::Connected,
        stale: false,
    }
}

#[test]
#[ignore = "live: needs a Transmission daemon, run with just test-live"]
fn adding_with_unwanted_files_matches_the_daemons_file_order() {
    let env = LiveEnv::from_env();
    let (mut client, server) = env.checked_client();
    let before = others_fingerprint(&mut client, None);
    let own = OwnTorrent::unwritten(
        &mut client,
        &env,
        TestFolder::test_dir(&env.remote_dir),
        &OUT_OF_ORDER,
    );
    let name = own.name().to_owned();
    let parsed = parse_torrent(own.bytes()).expect("the test torrent does not parse");
    let files = parsed
        .files
        .expect("the strict parse leaves the test torrent no choice of files");
    let paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
    let in_order: Vec<String> = OUT_OF_ORDER
        .iter()
        .map(|(file, _)| format!("{name}/{file}"))
        .collect();
    assert_eq!(
        paths, in_order,
        "the parse keeps every file at its place, the padding and the empty one too"
    );
    let mut sorted = paths.clone();
    sorted.sort_unstable();
    assert_ne!(
        paths, sorted,
        "the test needs files out of the order of their names"
    );
    // The add form, with `alpha.bin` unchecked, makes the request the add dialog sends. The
    // padding file is hidden in the form, but keeps its index, so the one left out is past it.
    let loaded = LoadedItem::torrent(
        PathBuf::from(format!("{name}.torrent")),
        Arc::from(own.bytes()),
    )
    .expect("the add form cannot read the test torrent");
    let lists_empty_files = server.lists_empty_files();
    let mut form = AddForm::new(
        loaded,
        env.remote_dir.as_str().to_owned(),
        false,
        lists_empty_files,
    );
    if !lists_empty_files {
        // Transmission 4.0 leaves the empty file out and numbers the others without it, so the
        // form offers no choice, and the torrent would be added with all its files.
        eprintln!("file order: skipped: the daemon leaves empty files out");
        assert_eq!(form.choice_unavailable(), Some(NoChoiceReason::Ambiguous));
        assert!(form.tree().is_none(), "no choice of files");
        return;
    }
    form.toggle_wanted(&NodeKey::new(&format!("{name}/alpha.bin")));
    let request = match form.check(&without_session(), true) {
        AddCheck::Ready(request) => request,
        other => panic!("the add form would send nothing: {other:?}"),
    };
    assert_eq!(request.files_unwanted, [2]);
    assert!(request.paused);

    let torrent = TestTorrent::add(&mut client, &env, own, &request);
    let hash = torrent.hash().clone();
    let record = torrent.read(&mut client, NAMES_AND_WANTED);

    let listed: Vec<(String, u64)> = record
        .files
        .iter()
        .flatten()
        .map(|file| (file.name.clone(), file.length))
        .collect();
    let parsed: Vec<(String, u64)> = files
        .iter()
        .map(|file| (file.path.clone(), file.length))
        .collect();
    assert_eq!(
        listed, parsed,
        "the daemon numbers the files as the parse does"
    );
    assert_eq!(
        record.wanted.as_deref(),
        Some(&[true, true, false, true, true][..])
    );
    assert_others_unchanged(&mut client, Some(&hash), &before);

    torrent.remove_with_data(&mut client);
    assert_eq!(torrent_by_hash(&mut client, &hash), Ok(None));
    assert_others_unchanged(&mut client, None, &before);
}
