//! The trackers of the synthetic torrents as a running daemon has them: when each tier announces
//! next, counted from now, which are queued or announcing, what a stopped torrent's trackers say,
//! and the torrent that has every status the Trackers tab shows.

use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use x4_core::{NextAnnounce, TrackerOutcome, TrackerStat, TrackerStatus, tracker_status};
use x4_rpc::{Status, TrackerStatRecord, TrackerState};

use super::*;

/// How often a tier announces, in seconds.
const INTERVAL: i64 = 1800;

fn unix_now() -> i64 {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after 1970");
    i64::try_from(since.as_secs()).expect("fits")
}

/// Every torrent's status, peer count and trackers.
fn all(client: &mut Client<SyntheticDaemon>) -> Vec<(Status, u32, Vec<TrackerStatRecord>)> {
    let fields = [
        TorrentField::Status,
        TorrentField::PeersConnected,
        TorrentField::TrackerStats,
    ];
    client
        .torrent_get(&GetIds::All, &fields)
        .expect("the client parses the answer")
        .torrents
        .into_iter()
        .map(|record| {
            (
                record.status.expect("a status"),
                record.peers_connected.expect("a peer count"),
                record.tracker_stats.expect("the trackers"),
            )
        })
        .collect()
}

#[test]
fn a_waiting_tier_announces_next_within_an_interval_from_now() {
    let before = unix_now();
    let torrents = all(&mut client(Activity::Static));
    let after = unix_now();

    let waiting: Vec<&TrackerStatRecord> = torrents
        .iter()
        .flat_map(|(_, _, trackers)| trackers)
        .filter(|tracker| tracker.announce_state == TrackerState::Waiting)
        .collect();
    assert!(waiting.len() > 1000, "{} waiting", waiting.len());
    for tracker in waiting {
        let (last, next) = (tracker.last_announce_time, tracker.next_announce_time);
        assert!(next > before && next <= after + INTERVAL, "{tracker:?}");
        // The last announce came one interval before the next, so it is past.
        assert!(tracker.has_announced, "{tracker:?}");
        assert_eq!(last, next - INTERVAL, "{tracker:?}");
        assert!(last <= after, "{tracker:?}");
    }
}

#[test]
fn some_tiers_are_queued_and_some_announcing_while_their_torrent_runs() {
    let torrents = all(&mut client(Activity::Static));

    let running = torrents
        .iter()
        .filter(|(status, ..)| *status != Status::Stopped)
        .flat_map(|(_, _, trackers)| trackers);
    let mut queued = 0;
    let mut announcing = 0;
    for tracker in running {
        if tracker.is_backup {
            assert_eq!(tracker.announce_state, TrackerState::Inactive);
            continue;
        }
        match tracker.announce_state {
            TrackerState::Queued => queued += 1,
            TrackerState::Active => announcing += 1,
            TrackerState::Waiting => continue,
            other => panic!("a running torrent's tier is {other:?}"),
        }
        assert_eq!(tracker.next_announce_time, 0, "{tracker:?}");
    }
    assert!(queued > 10 && announcing > 10, "{queued} {announcing}");
}

#[test]
fn a_stopped_torrent_announces_nothing_and_keeps_what_its_trackers_last_said() {
    let started = unix_now();
    let torrents = all(&mut client(Activity::Static));

    let stopped: Vec<&TrackerStatRecord> = torrents
        .iter()
        .filter(|(status, ..)| *status == Status::Stopped)
        .flat_map(|(_, _, trackers)| trackers)
        .collect();
    assert!(stopped.len() > 100);
    for tracker in &stopped {
        assert_eq!(
            tracker.announce_state,
            TrackerState::Inactive,
            "{tracker:?}"
        );
        assert_eq!(tracker.next_announce_time, 0, "{tracker:?}");
        if tracker.has_announced {
            assert!(
                tracker.last_announce_time > 0 && tracker.last_announce_time < started,
                "{tracker:?}"
            );
        }
    }
    // Some were not run since the daemon started, so their trackers were never contacted.
    let never = |tracker: &&&TrackerStatRecord| !tracker.is_backup && !tracker.has_announced;
    assert!(stopped.iter().filter(never).count() > 10);
    assert!(stopped.iter().any(|tracker| tracker.has_announced));
}

#[test]
fn a_tracker_can_fail_its_scrape_alone() {
    let torrents = all(&mut client(Activity::Static));

    let scrape_only = torrents
        .iter()
        .flat_map(|(_, _, trackers)| trackers)
        .filter(|tracker| {
            tracker.has_announced
                && tracker.last_announce_succeeded
                && tracker.has_scraped
                && !tracker.last_scrape_succeeded
        })
        .collect::<Vec<_>>();
    assert_ne!(scrape_only, [] as [&TrackerStatRecord; 0]);
    for tracker in scrape_only {
        assert!(!tracker.last_scrape_result.is_empty(), "{tracker:?}");
        // The announce brought the swarm's size; only the scrape knows the downloads.
        assert!(tracker.seeder_count.is_some_and(|count| count >= 0));
        assert!(tracker.leecher_count.is_some_and(|count| count >= 0));
        assert_eq!(tracker.download_count, Some(-1), "{tracker:?}");
    }
}

#[test]
fn the_busy_torrent_has_a_tracker_of_every_status() {
    let torrents = all(&mut client(Activity::Static));
    let (busy, _, records) = torrents
        .iter()
        .find(|(_, peers, _)| *peers > 100)
        .expect("the busy torrent");
    assert_eq!(*busy, Status::Downloading);

    let stats: Vec<TrackerStat> = records.iter().cloned().map(TrackerStat::from).collect();
    let statuses: HashSet<TrackerStatus> = stats.iter().map(tracker_status).collect();
    assert_eq!(
        statuses,
        HashSet::from([
            TrackerStatus::Working,
            TrackerStatus::Announcing,
            TrackerStatus::NotContacted,
            TrackerStatus::TimedOut,
            TrackerStatus::Failed,
            TrackerStatus::Backup,
        ])
    );
    assert!(
        stats.iter().any(|stat| stat.next == NextAnnounce::Queued),
        "a tier waits for a slot"
    );
    assert!(
        stats
            .iter()
            .any(|stat| matches!(stat.next, NextAnnounce::At(_))),
        "a tier waits for its time"
    );
    assert!(
        stats.iter().any(|stat| {
            tracker_status(stat) == TrackerStatus::Working
                && stat
                    .last_scrape
                    .as_ref()
                    .is_some_and(|scrape| scrape.outcome == TrackerOutcome::Failed)
        }),
        "a working tracker whose scrape failed"
    );
    // The tiers in order, the backup in the last one's.
    let tiers: Vec<u32> = stats.iter().map(|stat| stat.tier).collect();
    assert!(tiers.is_sorted(), "{tiers:?}");
}
