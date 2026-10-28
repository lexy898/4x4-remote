//! Live tests against a real Transmission daemon, run only by `just test-live`.
//!
//! They act on nothing but a torrent they create themselves: a random file written into a
//! dedicated test directory (`X4_LIVE_LOCAL_DIR`, the same directory the daemon sees as
//! `X4_LIVE_REMOTE_DIR`) and added as a trackerless torrent. Every request that changes a
//! torrent names that torrent by the info hash of the metainfo the test built, once
//! `torrent_add` has confirmed it; `session_set` is never called, and each test checks that every
//! other torrent kept its status, sequential mode, file priorities and wanted files. A drop guard
//! removes the test torrent with its data, the local file and the directory it created on every
//! path, including a failed assertion. An interrupted run skips the guards, so each new torrent
//! is named on stderr as soon as it is added.
//!
//! The offline tests of the support code (bencode, piece hashes, accepting the added torrent)
//! run in `just test` too.

#[path = "live/metainfo.rs"]
mod metainfo;
#[path = "live/support.rs"]
mod support;

use std::time::Duration;

use support::{
    LiveEnv, TestTorrent, assert_others_unchanged, file_is_gone, others_fingerprint,
    torrent_by_hash, wait_until,
};
use x4_rpc::{AddOutcome, FilePriority, GetIds, Status, TorrentAction, TorrentField, TorrentSet};

const STOPPED_AFTER_STOP: Duration = Duration::from_secs(10);
const SEEDING_AFTER_START: Duration = Duration::from_secs(30);
const REMOVAL_REPORTED: Duration = Duration::from_secs(5);

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

    let local_file = torrent.local_file().to_owned();
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
