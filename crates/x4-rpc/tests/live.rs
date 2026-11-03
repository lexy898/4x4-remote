//! Live tests against a real Transmission daemon, run only by `just test-live`.
//!
//! They act on nothing but a torrent they create themselves: a random file, or a folder of a few
//! small random files, written into a dedicated test directory (`X4_LIVE_LOCAL_DIR`, the same
//! directory the daemon sees as `X4_LIVE_REMOTE_DIR`) and added as a trackerless torrent. Some
//! write no data: a paused torrent of data nobody has, or its magnet link; one downloads into a
//! folder of its own directly in the test directory, which does not exist yet and which the
//! daemon makes when the torrent starts. Every request that changes a torrent names that torrent
//! by the info hash of the metainfo the test built, once `torrent_add` has confirmed it;
//! `session_set` is never called, and each test checks that every other torrent kept its status,
//! sequential mode, file priorities and wanted files. A drop guard removes the test torrent with
//! its data, the local files and the directories it created on every path, including a failed
//! assertion. One test removes its torrent keeping the data, checks that the file stayed, and
//! deletes it itself; a guard of the file deletes it on every path too. An interrupted run skips
//! the guards, so each new torrent is named on stderr as soon as it is added.
//!
//! One test only reads: it looks for a few of the daemon's own torrents that have trackers and
//! reads their details as the details panel does (the `tracker_entries` module), so that real
//! tracker entries are parsed. It changes nothing and reports counts and kinds of values only.
//!
//! The offline tests of the support code (bencode, piece hashes, accepting the added torrent,
//! the local data) run in `just test` too. They are in files of their own that only this crate
//! declares: x4-core's live tests share the support, and would run them a second time.

#[path = "live/metainfo.rs"]
mod metainfo;
#[cfg(test)]
#[path = "live/metainfo_tests.rs"]
mod metainfo_tests;
#[path = "live/support.rs"]
mod support;
#[cfg(test)]
#[path = "live/support_tests.rs"]
mod support_tests;
#[path = "live/tracker_entries.rs"]
mod tracker_entries;

use std::sync::Arc;
use std::time::Duration;

use metainfo::random_seed;
use support::{
    FILE_SIZE, LiveEnv, MULTI_FILES, OwnTorrent, TestFolder, TestTorrent, assert_others_unchanged,
    file_is_gone, file_size, folder_exists, others_fingerprint, torrent_by_hash, wait_until,
};
use tracker_entries::{WITH_TRACKERS, listed_hashes, read_and_check, with_trackers};
use x4_rpc::{
    AddOutcome, AddSource, Client, FilePriority, GetIds, RpcError, Status, TorrentAction,
    TorrentAdd, TorrentField, TorrentSelection, TorrentSet, UreqTransport,
};

const STOPPED_AFTER_STOP: Duration = Duration::from_secs(10);
const SEEDING_AFTER_START: Duration = Duration::from_secs(30);
const REMOVAL_REPORTED: Duration = Duration::from_secs(5);
/// How long a file the daemon was told to keep must stay after the removal: a daemon that deletes
/// it all the same does so within moments.
const KEPT_FOR: Duration = Duration::from_secs(2);
/// What the file tests read of their own torrent: the compact per-file arrays the details panel
/// reads.
const FILE_FIELDS: &[TorrentField] = &[
    TorrentField::Id,
    TorrentField::HashString,
    TorrentField::FileCount,
    TorrentField::Wanted,
    TorrentField::Priorities,
];
/// The JSON-RPC error code of a file index past the torrent's last file (`FILE_IDX_OOR`).
const FILE_INDEX_OUT_OF_RANGE: i64 = 6;
/// How long a new paused torrent may take to check its data and settle as stopped.
const CHECKED_AFTER_ADD: Duration = Duration::from_secs(60);
/// How long a torrent that must not start is watched, to see that it does not.
const STAYS_STOPPED_FOR: Duration = Duration::from_secs(3);
/// How long the daemon may take to make the folder of a torrent that starts.
const FOLDER_AFTER_START: Duration = Duration::from_secs(30);
/// How long a file the daemon made may take to show on this machine's mount.
const SHOWS_HERE: Duration = Duration::from_secs(10);
/// The files of the torrent that downloads into a new folder: an empty one, which the daemon
/// makes as soon as the torrent starts, and one of data that nobody has.
const EMPTY_AND_DATA: [(&str, usize); 2] = [("empty.bin", 0), ("data.bin", 16 * 1024)];
/// The one file of the torrent added from a magnet link, of data that nobody has.
const MAGNET_FILES: [(&str, usize); 1] = [("data.bin", 16 * 1024)];

/// Whether `status` is that of a torrent that runs or waits to run, which one added paused must
/// never show.
fn is_started(status: Status) -> bool {
    matches!(
        status,
        Status::DownloadWait | Status::Downloading | Status::SeedWait | Status::Seeding
    )
}

/// Whether each file of `torrent` is wanted, and its priority, as the daemon has them.
fn files_of(
    torrent: &TestTorrent,
    client: &mut Client<UreqTransport>,
) -> (Vec<bool>, Vec<FilePriority>) {
    let record = torrent.read(client, FILE_FIELDS);
    assert_eq!(record.file_count, u32::try_from(MULTI_FILES.len()).ok());
    (
        record.wanted.unwrap_or_default(),
        record.priorities.unwrap_or_default(),
    )
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

#[test]
#[ignore = "live: needs a Transmission daemon, run with just test-live"]
fn live_server_is_supported() {
    let env = LiveEnv::from_env();
    let mut client = env.client();
    let before = others_fingerprint(&mut client, None);

    let version = client.check_server().expect("check_server failed");

    assert!(version.is_supported(), "unsupported server: {version:?}");
    assert_others_unchanged(&mut client, None, &before);
}

#[test]
#[ignore = "live: needs a Transmission daemon, run with just test-live"]
fn live_lifecycle_on_own_torrent() {
    let env = LiveEnv::from_env();
    let mut client = env.client();
    let before = others_fingerprint(&mut client, None);

    let torrent = TestTorrent::create(&mut client, &env);
    let hash = torrent.hash().clone();
    let own = Some(&hash);
    let selection = torrent.selection();

    let listed = torrent.record(&mut client);
    assert_eq!(listed.status, Some(Status::Seeding), "{listed:?}");

    client
        .torrent_action(TorrentAction::Stop, &selection)
        .expect("torrent_stop failed");
    torrent.wait_for(&mut client, Status::Stopped, STOPPED_AFTER_STOP);
    assert_others_unchanged(&mut client, own, &before);

    client
        .torrent_action(TorrentAction::Start, &selection)
        .expect("torrent_start failed");
    torrent.wait_for(&mut client, Status::Seeding, SEEDING_AFTER_START);
    assert_others_unchanged(&mut client, own, &before);

    let sequential = TorrentSet {
        sequential_download: Some(true),
        ..TorrentSet::default()
    };
    client
        .torrent_set(&selection, &sequential)
        .expect("torrent_set sequential_download failed");
    assert_eq!(torrent.record(&mut client).sequential_download, Some(true));

    let high = TorrentSet {
        priority_high: vec![0],
        ..TorrentSet::default()
    };
    client
        .torrent_set(&selection, &high)
        .expect("torrent_set priority_high failed");
    let priorities = torrent
        .record(&mut client)
        .file_stats
        .map(|stats| stats.iter().map(|stat| stat.priority).collect::<Vec<_>>());
    assert_eq!(priorities, Some(vec![FilePriority::High]));
    assert_others_unchanged(&mut client, own, &before);

    client
        .torrent_action(TorrentAction::Verify, &selection)
        .expect("torrent_verify failed");
    torrent.wait_for(&mut client, Status::Seeding, SEEDING_AFTER_START);
    client
        .torrent_action(TorrentAction::Reannounce, &selection)
        .expect("torrent_reannounce failed");
    assert_others_unchanged(&mut client, own, &before);

    let local_file = torrent.local_data().to_owned();
    torrent.remove_with_data(&mut client);

    assert_eq!(torrent_by_hash(&mut client, &hash), Ok(None));
    assert!(
        file_is_gone(&local_file),
        "{} is still there",
        local_file.display()
    );
    assert_others_unchanged(&mut client, None, &before);
}

#[test]
#[ignore = "live: needs a Transmission daemon, run with just test-live"]
fn live_recently_active_reports_removal() {
    let env = LiveEnv::from_env();
    let mut client = env.client();
    let before = others_fingerprint(&mut client, None);
    let torrent = TestTorrent::create(&mut client, &env);
    let id = torrent.id();

    torrent.remove_with_data(&mut client);
    let reported = wait_until(REMOVAL_REPORTED, || {
        client
            .torrent_get(&GetIds::RecentlyActive, &[TorrentField::Id])
            .expect("torrent_get recently_active failed")
            .removed
            .contains(&id)
    });

    assert!(
        reported,
        "removed did not list {id:?} within {REMOVAL_REPORTED:?}"
    );
    assert_others_unchanged(&mut client, None, &before);
}

#[test]
#[ignore = "live: needs a Transmission daemon, run with just test-live"]
fn live_add_duplicate_is_reported() {
    let env = LiveEnv::from_env();
    let mut client = env.client();
    let before = others_fingerprint(&mut client, None);
    let torrent = TestTorrent::create(&mut client, &env);
    let hash = torrent.hash().clone();

    let again = client
        .torrent_add(torrent.add_request())
        .expect("second torrent_add failed");

    match again {
        AddOutcome::Duplicate(duplicate) => {
            assert_eq!(duplicate.hash, hash);
            assert_eq!(duplicate.id, torrent.id());
        }
        AddOutcome::Added(added) => panic!("the same metainfo was added twice: {added:?}"),
    }
    assert_others_unchanged(&mut client, Some(&hash), &before);

    // One torrent on the daemon, so one removal cleans up.
    torrent.remove_with_data(&mut client);

    assert_eq!(torrent_by_hash(&mut client, &hash), Ok(None));
    assert_others_unchanged(&mut client, None, &before);
}

#[test]
#[ignore = "live: needs a Transmission daemon, run with just test-live"]
fn start_now_starts_a_stopped_own_torrent() {
    let env = LiveEnv::from_env();
    let mut client = env.client();
    let before = others_fingerprint(&mut client, None);
    let torrent = TestTorrent::create(&mut client, &env);
    let hash = torrent.hash().clone();
    let own = Some(&hash);
    let selection = torrent.selection();
    client
        .torrent_action(TorrentAction::Stop, &selection)
        .expect("torrent_stop failed");
    torrent.wait_for(&mut client, Status::Stopped, STOPPED_AFTER_STOP);
    assert_others_unchanged(&mut client, own, &before);

    client
        .torrent_action(TorrentAction::StartNow, &selection)
        .expect("torrent_start_now failed");

    torrent.wait_for(&mut client, Status::Seeding, SEEDING_AFTER_START);
    assert_others_unchanged(&mut client, own, &before);
    torrent.remove_with_data(&mut client);
    assert_eq!(torrent_by_hash(&mut client, &hash), Ok(None));
    assert_others_unchanged(&mut client, None, &before);
}

#[test]
#[ignore = "live: needs a Transmission daemon, run with just test-live"]
fn remove_keeping_data_leaves_the_file() {
    let env = LiveEnv::from_env();
    let mut client = env.client();
    let before = others_fingerprint(&mut client, None);
    let torrent = TestTorrent::create(&mut client, &env);
    let hash = torrent.hash().clone();

    let kept = torrent.remove_keeping_data(&mut client);

    assert_eq!(torrent_by_hash(&mut client, &hash), Ok(None));
    // `kept` is the file this run wrote into X4_LIVE_LOCAL_DIR (`LocalFile::write` builds its
    // path there, and deletes only what this run created), so it needs no check of its place.
    assert!(
        !wait_until(KEPT_FOR, || file_is_gone(kept.path())),
        "the daemon deleted {} although it was told to keep it",
        kept.path().display()
    );
    assert_eq!(
        file_size(kept.path()).ok(),
        u64::try_from(FILE_SIZE).ok(),
        "{} is not the whole file",
        kept.path().display()
    );
    assert_others_unchanged(&mut client, None, &before);

    // The daemon no longer knows the file, so the test deletes it.
    let path = kept.path().to_owned();
    kept.delete();
    assert!(file_is_gone(&path), "{} is still there", path.display());
}

#[test]
#[ignore = "live: needs a Transmission daemon, run with just test-live"]
fn file_wanted_and_priority_on_an_own_multi_file_torrent() {
    use FilePriority::{High, Low, Normal};
    let env = LiveEnv::from_env();
    let mut client = env.client();
    let before = others_fingerprint(&mut client, None);
    let torrent = TestTorrent::create_multi_file(&mut client, &env);
    let hash = torrent.hash().clone();
    let own = Some(&hash);
    let selection = torrent.selection();
    assert_eq!(
        files_of(&torrent, &mut client),
        (vec![true; 3], vec![Normal; 3])
    );

    let skip_second = TorrentSet {
        files_unwanted: vec![1],
        ..TorrentSet::default()
    };
    set(&mut client, &selection, &skip_second);
    assert_eq!(
        files_of(&torrent, &mut client).0,
        [true, false, true],
        "only the second file is skipped"
    );
    assert_others_unchanged(&mut client, own, &before);

    let want_second = TorrentSet {
        files_wanted: vec![1],
        ..TorrentSet::default()
    };
    set(&mut client, &selection, &want_second);
    assert_eq!(files_of(&torrent, &mut client).0, [true; 3]);
    assert_others_unchanged(&mut client, own, &before);

    let low_and_high = TorrentSet {
        priority_low: vec![0],
        priority_high: vec![2],
        ..TorrentSet::default()
    };
    set(&mut client, &selection, &low_and_high);
    assert_eq!(files_of(&torrent, &mut client).1, [Low, Normal, High]);
    assert_others_unchanged(&mut client, own, &before);

    let normal_again = TorrentSet {
        priority_normal: vec![0, 2],
        ..TorrentSet::default()
    };
    set(&mut client, &selection, &normal_again);
    assert_eq!(
        files_of(&torrent, &mut client),
        (vec![true; 3], vec![Normal; 3])
    );
    assert_others_unchanged(&mut client, own, &before);

    let local_data = torrent.local_data().to_owned();
    torrent.remove_with_data(&mut client);

    assert_eq!(torrent_by_hash(&mut client, &hash), Ok(None));
    assert!(
        file_is_gone(&local_data),
        "{} is still there",
        local_data.display()
    );
    assert_others_unchanged(&mut client, None, &before);
}

#[test]
#[ignore = "live: needs a Transmission daemon, run with just test-live"]
fn an_out_of_range_file_index_is_refused() {
    let env = LiveEnv::from_env();
    let mut client = env.client();
    let before = others_fingerprint(&mut client, None);
    let torrent = TestTorrent::create_multi_file(&mut client, &env);
    let hash = torrent.hash().clone();
    let files = files_of(&torrent, &mut client);

    // One past the last file; never an empty list, which the daemon reads as every file.
    let past_the_end = TorrentSet {
        files_wanted: vec![MULTI_FILES.len()],
        ..TorrentSet::default()
    };
    let result = client.torrent_set(&torrent.selection(), &past_the_end);

    assert!(
        matches!(
            &result,
            Err(RpcError::Server { code, .. }) if *code == FILE_INDEX_OUT_OF_RANGE
        ),
        "{result:?}"
    );
    assert_eq!(
        files_of(&torrent, &mut client),
        files,
        "the refused call changed nothing"
    );
    assert_others_unchanged(&mut client, Some(&hash), &before);
    torrent.remove_with_data(&mut client);
    assert_eq!(torrent_by_hash(&mut client, &hash), Ok(None));
    assert_others_unchanged(&mut client, None, &before);
}

#[test]
#[ignore = "live: needs a Transmission daemon, run with just test-live"]
fn sequential_on_and_off_on_an_own_torrent() {
    let env = LiveEnv::from_env();
    let mut client = env.client();
    let before = others_fingerprint(&mut client, None);
    let torrent = TestTorrent::create_multi_file(&mut client, &env);
    let hash = torrent.hash().clone();
    let own = Some(&hash);
    let selection = torrent.selection();
    assert_eq!(torrent.record(&mut client).sequential_download, Some(false));

    for on in [true, false] {
        let sequential = TorrentSet {
            sequential_download: Some(on),
            ..TorrentSet::default()
        };
        set(&mut client, &selection, &sequential);
        assert_eq!(
            torrent.record(&mut client).sequential_download,
            Some(on),
            "sequential {on}"
        );
        assert_others_unchanged(&mut client, own, &before);
    }

    torrent.remove_with_data(&mut client);
    assert_eq!(torrent_by_hash(&mut client, &hash), Ok(None));
    assert_others_unchanged(&mut client, None, &before);
}

#[test]
#[ignore = "live: needs a Transmission daemon, run with just test-live"]
fn real_tracker_stats_parse_on_the_daemons_torrents() {
    let env = LiveEnv::from_env();
    let mut client = env.client();
    let hashes = listed_hashes(&mut client);
    let (tracked, looked_at) = with_trackers(&mut client, &hashes);
    eprintln!(
        "tracker entries: {} torrents listed, {looked_at} looked at, {} of at most \
         {WITH_TRACKERS} with trackers chosen",
        hashes.len(),
        tracked.len()
    );
    assert!(
        !tracked.is_empty(),
        "none of the {looked_at} torrents looked at has trackers"
    );

    // The read of the progress that goes out with the whole list, then the full read on select.
    for (what, fields) in [
        (
            "progress and trackers",
            TorrentField::DETAILS_PROGRESS_TRACKERS,
        ),
        ("full", TorrentField::DETAILS_FULL),
    ] {
        let tally = read_and_check(&mut client, &tracked, fields, what);
        eprintln!("{what}: {tally}");
        assert!(tally.entries > 0, "{what}: no tracker entry was parsed");
    }
}

#[test]
#[ignore = "live: needs a Transmission daemon, run with just test-live"]
fn a_paused_add_stays_stopped() {
    let env = LiveEnv::from_env();
    let mut client = env.client();
    let before = others_fingerprint(&mut client, None);
    // All of its data is there, so a torrent that started would seed: only `paused` stops it.
    let own = OwnTorrent::multi_file(&mut client, &env);
    let request = own.metainfo_request(true, Vec::new());
    let torrent = TestTorrent::add(&mut client, &env, own, &request);
    let hash = torrent.hash().clone();

    // The daemon checks a new torrent's data, or takes files older than the add for complete:
    // either way it then knows that all of it is there.
    let (checked, mut seen) = torrent.wait_until(&mut client, CHECKED_AFTER_ADD, |record| {
        record.status == Some(Status::Stopped)
            && record.percent_done.is_some_and(|done| done >= 1.0)
    });
    seen.extend(torrent.statuses_for(&mut client, STAYS_STOPPED_FOR));

    assert!(
        !seen.iter().copied().any(is_started),
        "the paused torrent ran: {seen:?}; checked: {checked:?}"
    );
    assert_eq!(seen.last(), Some(&Status::Stopped), "{seen:?}");
    assert_others_unchanged(&mut client, Some(&hash), &before);
    torrent.remove_with_data(&mut client);
    assert_eq!(torrent_by_hash(&mut client, &hash), Ok(None));
    assert_others_unchanged(&mut client, None, &before);
}

#[test]
#[ignore = "live: needs a Transmission daemon, run with just test-live"]
fn adding_the_same_torrent_again_is_a_duplicate_and_changes_nothing() {
    let env = LiveEnv::from_env();
    let mut client = env.client();
    let before = others_fingerprint(&mut client, None);
    let own = OwnTorrent::unwritten(
        &mut client,
        &env,
        TestFolder::test_dir(&env.remote_dir),
        &MULTI_FILES,
    );
    let first = own.metainfo_request(true, vec![1]);
    // The same content again, as when the same file is opened twice.
    let content: Arc<[u8]> = Arc::from(own.bytes());
    let torrent = TestTorrent::add(&mut client, &env, own, &first);
    let hash = torrent.hash().clone();
    let own = Some(&hash);
    let added = torrent.wait_for(&mut client, Status::Stopped, CHECKED_AFTER_ADD);
    let files = files_of(&torrent, &mut client);
    assert_eq!(files.0, [true, false, true]);

    // The same metainfo, into another folder of this run, started, with another file left out:
    // the daemon takes none of it.
    let elsewhere = TestFolder::new_subfolder(&env.remote_dir, random_seed());
    let again = TorrentAdd {
        source: AddSource::Metainfo(content),
        download_dir: Some(elsewhere.remote().clone()),
        paused: false,
        files_unwanted: vec![0],
        priority_high: Vec::new(),
        priority_low: Vec::new(),
        sequential_download: None,
    };
    let outcome = torrent
        .add_again(&mut client, &again, &elsewhere)
        .expect("the second torrent_add failed");

    match outcome {
        AddOutcome::Duplicate(duplicate) => {
            assert_eq!(duplicate.hash, hash);
            assert_eq!(duplicate.id, torrent.id());
        }
        AddOutcome::Added(added) => panic!("the same metainfo was added twice: {added:?}"),
    }
    let seen = torrent.statuses_for(&mut client, STAYS_STOPPED_FOR);
    assert!(
        !seen.iter().copied().any(is_started),
        "the duplicate started the torrent: {seen:?}"
    );
    assert_eq!(
        torrent.record(&mut client).download_dir,
        added.download_dir,
        "the duplicate moved the torrent"
    );
    assert_eq!(
        files_of(&torrent, &mut client),
        files,
        "the duplicate changed the files"
    );
    assert_others_unchanged(&mut client, own, &before);

    torrent.remove_with_data(&mut client);
    assert_eq!(torrent_by_hash(&mut client, &hash), Ok(None));
    assert_others_unchanged(&mut client, None, &before);
}

#[test]
#[ignore = "live: needs a Transmission daemon, run with just test-live"]
fn a_missing_subfolder_of_the_test_dir_is_accepted_and_created_on_start() {
    let env = LiveEnv::from_env();
    let mut client = env.client();
    let before = others_fingerprint(&mut client, None);
    let folder = TestFolder::new_subfolder(&env.remote_dir, random_seed());
    // `unwritten` asks the daemon first that the folder is not there.
    let own = OwnTorrent::unwritten(&mut client, &env, folder.clone(), &EMPTY_AND_DATA);
    let request = own.metainfo_request(true, Vec::new());
    let torrent = TestTorrent::add(&mut client, &env, own, &request);
    let hash = torrent.hash().clone();
    let own = Some(&hash);

    // The folder is kept as it was sent, and a paused torrent makes nothing.
    let added = torrent.wait_for(&mut client, Status::Stopped, CHECKED_AFTER_ADD);
    assert_eq!(
        added.download_dir.as_deref(),
        Some(folder.remote().as_str())
    );
    // Its check ends first: a start during the check waits for it, and then for the queue.
    let seen = torrent.statuses_for(&mut client, STAYS_STOPPED_FOR);
    assert!(!seen.iter().copied().any(is_started), "{seen:?}");
    assert_eq!(
        folder_exists(&mut client, &folder),
        Ok(false),
        "a paused add made the folder"
    );

    client
        .torrent_action(TorrentAction::StartNow, &torrent.selection())
        .expect("torrent_start_now failed");

    // A torrent that starts makes its wanted empty files, and the folders they are in.
    let mut last = None;
    let made = wait_until(FOLDER_AFTER_START, || {
        let exists = folder_exists(&mut client, &folder);
        let made = exists == Ok(true);
        last = Some(exists);
        made
    });
    assert!(
        made,
        "the daemon did not make {:?} within {FOLDER_AFTER_START:?} of the start (last answer: \
         {last:?}); the torrent: {:?}",
        folder.remote(),
        torrent.record(&mut client)
    );
    let empty = torrent.local_data().join("empty.bin");
    assert!(
        wait_until(SHOWS_HERE, || file_size(&empty).is_ok_and(|size| size == 0)),
        "{} is not here",
        empty.display()
    );
    assert_others_unchanged(&mut client, own, &before);

    // The guard accepts exactly this run's folder, and removes it once it is empty.
    torrent.remove_with_data(&mut client);
    assert_eq!(torrent_by_hash(&mut client, &hash), Ok(None));
    assert!(file_is_gone(&empty), "{} is still there", empty.display());
    assert_eq!(
        folder_exists(&mut client, &folder),
        Ok(false),
        "the test's folder is still there"
    );
    assert_others_unchanged(&mut client, None, &before);
}

#[test]
#[ignore = "live: needs a Transmission daemon, run with just test-live"]
fn a_magnet_add_of_an_own_torrent_is_accepted() {
    let env = LiveEnv::from_env();
    let mut client = env.client();
    let before = others_fingerprint(&mut client, None);
    // A torrent of this run, of data nobody has, which the link names by its hash and its name.
    let own = OwnTorrent::unwritten(
        &mut client,
        &env,
        TestFolder::test_dir(&env.remote_dir),
        &MAGNET_FILES,
    );
    let name = own.name().to_owned();
    // Paused, so the daemon does not look for the metadata, which nobody could send.
    let request = own.magnet_request(true);
    let torrent = TestTorrent::add(&mut client, &env, own, &request);
    let hash = torrent.hash().clone();

    let listed = torrent.record(&mut client);

    assert_eq!(listed.name.as_deref(), Some(name.as_str()), "{listed:?}");
    assert_eq!(
        listed.download_dir.as_deref(),
        Some(env.remote_dir.as_str()),
        "{listed:?}"
    );
    assert_eq!(listed.status, Some(Status::Stopped), "{listed:?}");
    assert!(
        listed
            .metadata_percent_complete
            .is_some_and(|done| done < 1.0),
        "{listed:?}"
    );
    assert_others_unchanged(&mut client, Some(&hash), &before);
    torrent.remove_with_data(&mut client);
    assert_eq!(torrent_by_hash(&mut client, &hash), Ok(None));
    assert_others_unchanged(&mut client, None, &before);
}
