//! The rows of the Trackers tab: one per tracker, with its status, its times and its counts.
//!
//! The daemon keeps the announce data of a tier, not of a tracker, and shows it on the tier's
//! current tracker: after an error it makes that tracker a backup, which has no data, and shows
//! the error on the next tracker of the tier. A row therefore shows what its tier is doing, and
//! the rows stay in the daemon's order, tier by tier, which is what makes that readable.

use std::time::SystemTime;

use x4_rpc::{TrackerId, TrackerState};

use super::host_without_port;
use crate::details::{NextAnnounce, TorrentDetails, TrackerOutcome, TrackerStat};
use crate::peers::clean_line;

/// The most characters of a tracker's message that are kept. The daemon's buffer holds 127
/// bytes, so this only matters for a daemon that sends more.
const MAX_MESSAGE_CHARS: usize = 256;

/// The most characters of a tracker's host or site name that are kept; the daemon's buffers hold
/// 72 and 64 bytes.
const MAX_NAME_CHARS: usize = 128;

/// What a tracker is doing, as its row says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrackerStatus {
    /// Another tracker of the tier is in use; this one has no state of its own.
    Backup,
    /// An announce to the tier is under way.
    Announcing,
    /// The torrent has not been announced to the tracker since the daemon started: it has not
    /// run since then, or the tracker is a backup.
    NotContacted,
    /// The last announce succeeded; a warning of the tracker is a success too.
    Working,
    /// The tracker did not answer the last announce in time.
    TimedOut,
    /// The last announce failed: the tracker could not be reached, or refused.
    Failed,
}

/// One tracker as the Trackers tab draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackerRow {
    /// What identifies the row between reads: the daemon's id of the tracker.
    pub id: TrackerId,
    /// The tier, counted from 0; the UI shows it from 1.
    pub tier: u32,
    /// The host, without the port when that is 80 or 443, which the scheme implies.
    pub host: String,
    /// The tracker's site name, when the host does not already say it.
    pub sitename: Option<String>,
    /// What the tracker is doing.
    pub status: TrackerStatus,
    /// The daemon's text for a failed or timed out announce, shown as sent but made to fit a
    /// line (control characters gone, whitespace collapsed); `None` for any other status and
    /// when the daemon said nothing, where the view words the failure itself.
    pub message: Option<String>,
    /// When the last announce came; `None` if none did or the daemon gave no time.
    pub last: Option<SystemTime>,
    /// When the next announce is due.
    pub next: NextAnnounce,
    /// How many peers the last announce returned; `None` unless it succeeded, because the
    /// daemon's count after a failure is a 0 that says nothing.
    pub peers: Option<u32>,
    /// How many seeders the tracker knows of; `None` when it did not say.
    pub seeders: Option<u32>,
    /// How many leechers the tracker knows of; `None` when it did not say.
    pub leechers: Option<u32>,
    /// How many times the tracker saw the torrent completed; `None` when it did not say.
    pub downloads: Option<u32>,
    /// Whether the last scrape failed or timed out and the tracker reported neither seeders nor
    /// leechers. Transmission also takes the counts from the answers to announces, so after a
    /// failed scrape they may be there all the same, and then there is nothing to say. Many
    /// trackers cannot be scraped while their announces work, so the view says it quietly.
    pub scrape_problem: bool,
}

/// What `stat` is doing: a backup first, then an announce under way, then a tracker never
/// announced to, and last how its last announce went.
///
/// A state this version does not know is not an announce under way.
#[must_use]
pub fn tracker_status(stat: &TrackerStat) -> TrackerStatus {
    if stat.is_backup {
        return TrackerStatus::Backup;
    }
    if stat.announce_state == TrackerState::Active {
        return TrackerStatus::Announcing;
    }
    match stat.last_announce.as_ref().map(|answer| answer.outcome) {
        None => TrackerStatus::NotContacted,
        Some(TrackerOutcome::Succeeded) => TrackerStatus::Working,
        Some(TrackerOutcome::TimedOut) => TrackerStatus::TimedOut,
        Some(TrackerOutcome::Failed) => TrackerStatus::Failed,
    }
}

/// The rows of the trackers of `details`, in the daemon's order: by tier, and within a tier in
/// the order it keeps the trackers. They are not sorted by id.
#[must_use]
pub fn tracker_rows(details: &TorrentDetails) -> Vec<TrackerRow> {
    details.trackers.iter().map(row).collect()
}

/// The row of one tracker.
fn row(stat: &TrackerStat) -> TrackerRow {
    let status = tracker_status(stat);
    let host = display_host(&clean_line(&stat.host, MAX_NAME_CHARS));
    let sitename = Some(clean_line(&stat.sitename, MAX_NAME_CHARS))
        .filter(|sitename| !sitename.is_empty() && !says(&host, sitename));
    let announce = stat.last_announce.as_ref();
    let succeeded = announce.is_some_and(|answer| answer.outcome == TrackerOutcome::Succeeded);
    let message = announce
        .filter(|_| matches!(status, TrackerStatus::TimedOut | TrackerStatus::Failed))
        .map(|answer| clean_line(&answer.result, MAX_MESSAGE_CHARS))
        .filter(|message| !message.is_empty());
    TrackerRow {
        id: stat.id,
        tier: stat.tier,
        host,
        sitename,
        status,
        message,
        last: announce.and_then(|answer| answer.at),
        next: stat.next,
        peers: stat.last_announce_peers.filter(|_| succeeded),
        seeders: stat.seeders,
        leechers: stat.leechers,
        downloads: stat.downloads,
        scrape_problem: stat.seeders.is_none()
            && stat.leechers.is_none()
            && stat.last_scrape.as_ref().is_some_and(|answer| {
                matches!(
                    answer.outcome,
                    TrackerOutcome::Failed | TrackerOutcome::TimedOut
                )
            }),
    }
}

/// `host_and_port` without its port when that is 80 or 443, which the scheme implies; any other
/// port tells trackers of one host apart and stays.
fn display_host(host_and_port: &str) -> String {
    let host = host_without_port(host_and_port);
    match host_and_port.get(host.len()..) {
        Some(":80" | ":443") => host,
        _ => host_and_port,
    }
    .to_owned()
}

/// Whether `host` already contains `sitename`, in any case, so that showing it adds nothing.
fn says(host: &str, sitename: &str) -> bool {
    host.to_lowercase().contains(&sitename.to_lowercase())
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use x4_rpc::testing::fixtures::TORRENT_GET_TRACKERS;
    use x4_rpc::{HashString, TorrentGetResponse, TrackerId, TrackerStatRecord, TrackerState};

    use super::*;
    use crate::details::{NextAnnounce, TrackerAnswer, TrackerOutcome, TrackerStat};
    use crate::test_support::{details, fixture_result, tracker};

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    fn answer(outcome: TrackerOutcome, result: &str, seconds: u64) -> TrackerAnswer {
        TrackerAnswer {
            outcome,
            result: result.to_owned(),
            at: Some(at(seconds)),
        }
    }

    /// A tracker whose last announce ended as `outcome` with this text.
    fn announced(host: &str, outcome: TrackerOutcome, result: &str) -> TrackerStat {
        TrackerStat {
            last_announce: Some(answer(outcome, result, 1_000)),
            ..tracker(host)
        }
    }

    fn rows_of(trackers: Vec<TrackerStat>) -> Vec<TrackerRow> {
        let hash = HashString::parse(&"cd".repeat(20)).unwrap();
        tracker_rows(&TorrentDetails {
            trackers,
            ..details(&hash, &[])
        })
    }

    fn row_of(stat: TrackerStat) -> TrackerRow {
        rows_of(vec![stat]).remove(0)
    }

    #[test]
    fn every_kind_of_tracker_has_its_status() {
        let status = |stat: &TrackerStat| tracker_status(stat);
        let backup = TrackerStat {
            is_backup: true,
            ..tracker("b.example.org:80")
        };
        // A backup holds no state of its own; even a stray state does not make it work.
        let backup_with_state = TrackerStat {
            announce_state: TrackerState::Active,
            last_announce: Some(answer(TrackerOutcome::Succeeded, "Success", 1_000)),
            ..backup.clone()
        };
        let announcing = TrackerStat {
            announce_state: TrackerState::Active,
            ..announced("a.example.org:80", TrackerOutcome::Failed, "refused")
        };
        let announcing_first_time = TrackerStat {
            announce_state: TrackerState::Active,
            ..tracker("a.example.org:80")
        };
        let unknown_state = TrackerStat {
            announce_state: TrackerState::Other(9),
            ..announced("a.example.org:80", TrackerOutcome::Succeeded, "Success")
        };
        let never = tracker("n.example.org:80");
        // A tracker's warning is a success that came with text.
        let warning = announced(
            "w.example.org:80",
            TrackerOutcome::Succeeded,
            "Tracker warning: slow down",
        );

        assert_eq!(status(&backup), TrackerStatus::Backup);
        assert_eq!(status(&backup_with_state), TrackerStatus::Backup);
        assert_eq!(status(&announcing), TrackerStatus::Announcing);
        assert_eq!(status(&announcing_first_time), TrackerStatus::Announcing);
        assert_eq!(status(&unknown_state), TrackerStatus::Working);
        assert_eq!(status(&never), TrackerStatus::NotContacted);
        assert_eq!(
            status(&announced(
                "s.example.org:80",
                TrackerOutcome::Succeeded,
                "Success"
            )),
            TrackerStatus::Working
        );
        assert_eq!(status(&warning), TrackerStatus::Working);
        assert_eq!(
            status(&announced("t.example.org:80", TrackerOutcome::TimedOut, "")),
            TrackerStatus::TimedOut
        );
        assert_eq!(
            status(&announced("f.example.org:80", TrackerOutcome::Failed, "")),
            TrackerStatus::Failed
        );
    }

    #[test]
    fn rows_keep_the_daemons_order_and_show_the_error_on_the_tiers_current_tracker() {
        // The first tracker of tier 0 failed, so the daemon made it a backup and shows its error
        // on the next tracker of the tier. The ids are not in order and the rows must not be.
        let failed_and_replaced = TrackerStat {
            id: TrackerId(9),
            tier: 0,
            is_backup: true,
            ..tracker("first.example.org:80")
        };
        let current = TrackerStat {
            id: TrackerId(4),
            tier: 0,
            announce_state: TrackerState::Waiting,
            next: NextAnnounce::At(at(5_000)),
            last_announce: Some(answer(
                TrackerOutcome::Failed,
                "Unregistered torrent",
                1_000,
            )),
            ..tracker("second.example.org:80")
        };
        let other_tier = TrackerStat {
            id: TrackerId(2),
            tier: 1,
            last_announce: Some(answer(TrackerOutcome::Succeeded, "Success", 900)),
            last_announce_peers: Some(30),
            ..tracker("third.example.org:80")
        };

        let rows = rows_of(vec![failed_and_replaced, current, other_tier]);

        let order: Vec<(TrackerId, u32, TrackerStatus)> = rows
            .iter()
            .map(|row| (row.id, row.tier, row.status))
            .collect();
        assert_eq!(
            order,
            [
                (TrackerId(9), 0, TrackerStatus::Backup),
                (TrackerId(4), 0, TrackerStatus::Failed),
                (TrackerId(2), 1, TrackerStatus::Working),
            ]
        );
        assert_eq!(rows[0].message, None);
        assert_eq!(rows[0].last, None);
        assert_eq!(rows[0].next, NextAnnounce::Idle);
        assert_eq!(rows[1].message.as_deref(), Some("Unregistered torrent"));
        assert_eq!(rows[1].last, Some(at(1_000)));
        assert_eq!(rows[1].next, NextAnnounce::At(at(5_000)));
        assert_eq!(rows[2].host, "third.example.org");
    }

    #[test]
    fn a_torrent_without_trackers_has_no_rows() {
        assert_eq!(rows_of(Vec::new()), []);
    }

    #[test]
    fn the_port_is_dropped_only_for_the_web_ports() {
        let host = |host: &str| row_of(tracker(host)).host;

        assert_eq!(host("tracker.example.org:80"), "tracker.example.org");
        assert_eq!(host("tracker.example.org:443"), "tracker.example.org");
        assert_eq!(host("tracker.example.org:6969"), "tracker.example.org:6969");
        assert_eq!(host("tracker.example.org:8080"), "tracker.example.org:8080");
        assert_eq!(host("tracker.example.org:1337"), "tracker.example.org:1337");
        assert_eq!(host("tracker.example.org"), "tracker.example.org");
        assert_eq!(host("203.0.113.7:80"), "203.0.113.7");
        assert_eq!(host("[2001:db8::1]:443"), "[2001:db8::1]");
        assert_eq!(host("[2001:db8::1]:6969"), "[2001:db8::1]:6969");
        // Not a port, so nothing to drop.
        assert_eq!(host("2001:db8::80"), "2001:db8::80");
        assert_eq!(host(""), "");
    }

    #[test]
    fn the_sitename_is_shown_only_when_the_host_does_not_say_it() {
        let sitename = |host: &str, sitename: &str| {
            row_of(TrackerStat {
                sitename: sitename.to_owned(),
                ..tracker(host)
            })
            .sitename
        };

        // The host already says it, in whatever case.
        assert_eq!(sitename("tracker.example.org:80", "example"), None);
        assert_eq!(sitename("Tracker.Example.org:80", "example"), None);
        assert_eq!(sitename("tracker.example.org:80", "EXAMPLE"), None);
        // Nothing to show.
        assert_eq!(sitename("tracker.example.org:80", ""), None);
        assert_eq!(sitename("tracker.example.org:80", " \t "), None);
        // The host is an address, or says something else.
        assert_eq!(
            sitename("203.0.113.7:6969", "example"),
            Some("example".to_owned())
        );
        assert_eq!(
            sitename("t1.cdn.test:443", "example"),
            Some("example".to_owned())
        );
        // It is cleaned like the host's other text.
        assert_eq!(
            sitename("203.0.113.7:6969", "\u{202E}exam\u{7}ple\n"),
            Some("example".to_owned())
        );
    }

    #[test]
    fn the_message_is_the_daemons_text_for_a_failure_or_a_timeout_made_to_fit_a_line() {
        let message = |outcome: TrackerOutcome, text: &str| {
            row_of(announced("a.example.org:80", outcome, text)).message
        };

        assert_eq!(
            message(TrackerOutcome::Failed, "Unregistered torrent"),
            Some("Unregistered torrent".to_owned())
        );
        assert_eq!(
            message(TrackerOutcome::TimedOut, "Tracker did not respond"),
            Some("Tracker did not respond".to_owned())
        );
        // Control and direction characters are dropped and the whitespace is collapsed.
        assert_eq!(
            message(
                TrackerOutcome::Failed,
                "\u{202E}Torrent\r\n  not\tregistered\u{7}\u{1b}[0m  "
            ),
            Some("Torrent not registered[0m".to_owned())
        );
        // Without text there is no message: the view words the failure itself.
        assert_eq!(message(TrackerOutcome::Failed, ""), None);
        assert_eq!(message(TrackerOutcome::TimedOut, " \n\t"), None);
        assert_eq!(message(TrackerOutcome::Failed, "\u{7}\u{202E}"), None);
        // A success has nothing to say, a warning included.
        assert_eq!(message(TrackerOutcome::Succeeded, "Success"), None);
        assert_eq!(
            message(TrackerOutcome::Succeeded, "Warning: slow down"),
            None
        );
    }

    #[test]
    fn only_the_failures_of_a_tracker_that_is_in_use_have_a_message() {
        let text = Some(answer(TrackerOutcome::Failed, "refused", 1_000));
        let with_state = |is_backup: bool, announce_state: TrackerState| {
            row_of(TrackerStat {
                is_backup,
                announce_state,
                last_announce: text.clone(),
                ..tracker("a.example.org:80")
            })
        };

        // A tier that is announcing again has a new answer coming; the old one is not shown.
        assert_eq!(with_state(false, TrackerState::Active).message, None);
        assert_eq!(with_state(true, TrackerState::Inactive).message, None);
        assert_eq!(
            with_state(false, TrackerState::Waiting).message.as_deref(),
            Some("refused")
        );
    }

    #[test]
    fn a_message_is_cut_where_a_hostile_daemon_could_make_it_unbounded() {
        let endless = "ab ".repeat(10_000);

        let message = row_of(announced(
            "a.example.org:80",
            TrackerOutcome::Failed,
            &endless,
        ))
        .message
        .unwrap();

        assert_eq!(message.chars().count(), MAX_MESSAGE_CHARS);
        assert!(message.starts_with("ab ab ab"), "{message:?}");
        assert!(!message.ends_with(' '));
    }

    #[test]
    fn the_last_announce_and_its_peers_are_shown_for_an_answer_that_came() {
        let row = row_of(TrackerStat {
            announce_state: TrackerState::Waiting,
            next: NextAnnounce::At(at(2_000)),
            last_announce: Some(answer(TrackerOutcome::Succeeded, "Success", 1_000)),
            last_announce_peers: Some(30),
            ..tracker("a.example.org:80")
        });

        assert_eq!(row.status, TrackerStatus::Working);
        assert_eq!(row.last, Some(at(1_000)));
        assert_eq!(row.next, NextAnnounce::At(at(2_000)));
        assert_eq!(row.peers, Some(30));
    }

    #[test]
    fn a_failed_announce_returned_no_peers_and_says_none() {
        // The daemon reports 0 after a failure; that is no answer of "no peers".
        let failed = row_of(TrackerStat {
            last_announce_peers: Some(0),
            ..announced("a.example.org:80", TrackerOutcome::Failed, "refused")
        });
        let timed_out = row_of(TrackerStat {
            last_announce_peers: Some(0),
            ..announced("a.example.org:80", TrackerOutcome::TimedOut, "")
        });
        // A success that returned none says so.
        let empty = row_of(TrackerStat {
            last_announce_peers: Some(0),
            ..announced("a.example.org:80", TrackerOutcome::Succeeded, "Success")
        });

        assert_eq!(failed.peers, None);
        assert_eq!(timed_out.peers, None);
        assert_eq!(empty.peers, Some(0));
        assert_eq!(failed.last, Some(at(1_000)));
    }

    #[test]
    fn a_tracker_that_gave_no_time_has_no_last_announce() {
        let row = row_of(TrackerStat {
            last_announce: Some(TrackerAnswer {
                outcome: TrackerOutcome::Failed,
                result: "refused".to_owned(),
                at: None,
            }),
            ..tracker("a.example.org:80")
        });

        assert_eq!(row.status, TrackerStatus::Failed);
        assert_eq!(row.last, None);
    }

    #[test]
    fn counts_the_tracker_did_not_report_are_none() {
        // The daemon sends -1 for each, as it does for every tracker of the owner's.
        let unknown = TrackerStatRecord {
            has_announced: true,
            last_announce_succeeded: true,
            seeder_count: Some(-1),
            leecher_count: Some(-1),
            download_count: Some(-1),
            ..TrackerStatRecord::default()
        };
        let known = TrackerStatRecord {
            seeder_count: Some(12),
            leecher_count: Some(0),
            download_count: Some(345),
            ..unknown.clone()
        };

        let rows = rows_of(vec![unknown.into(), known.into()]);

        let counts: Vec<_> = rows
            .iter()
            .map(|row| (row.seeders, row.leechers, row.downloads))
            .collect();
        assert_eq!(counts, [(None, None, None), (Some(12), Some(0), Some(345))]);
    }

    #[test]
    fn a_scrape_that_failed_or_timed_out_is_flagged_without_changing_the_status() {
        // The tracker reported no seeders and no leechers: the note says why they are missing.
        let scrape = |last_scrape: Option<TrackerAnswer>| {
            row_of(TrackerStat {
                last_scrape,
                seeders: None,
                leechers: None,
                ..announced("a.example.org:80", TrackerOutcome::Succeeded, "Success")
            })
        };

        let failed = scrape(Some(answer(TrackerOutcome::Failed, "HTTP 404", 900)));
        let timed_out = scrape(Some(answer(TrackerOutcome::TimedOut, "", 900)));

        assert!(failed.scrape_problem);
        assert!(timed_out.scrape_problem);
        assert_eq!(failed.status, TrackerStatus::Working);
        assert!(!scrape(Some(answer(TrackerOutcome::Succeeded, "Success", 900))).scrape_problem);
        // Many trackers have no scrape at all.
        assert!(!scrape(None).scrape_problem);
    }

    #[test]
    fn a_failed_scrape_is_not_flagged_when_the_tracker_gave_counts_all_the_same() {
        // Transmission also takes the counts from the answers to announces, so a failed scrape
        // does not mean that the counts are missing.
        let with_counts = |outcome: TrackerOutcome, seeders, leechers| {
            row_of(TrackerStat {
                last_scrape: Some(answer(outcome, "HTTP 404", 900)),
                seeders,
                leechers,
                ..announced("a.example.org:80", TrackerOutcome::Succeeded, "Success")
            })
        };

        for outcome in [TrackerOutcome::Failed, TrackerOutcome::TimedOut] {
            for (seeders, leechers) in [(Some(5), None), (None, Some(0)), (Some(5), Some(12))] {
                let row = with_counts(outcome, seeders, leechers);
                assert!(!row.scrape_problem, "{outcome:?} {seeders:?} {leechers:?}");
                assert_eq!((row.seeders, row.leechers), (seeders, leechers));
            }
            // The downloads alone do not say who is on the torrent.
            assert!(
                with_counts(outcome, None, None).scrape_problem,
                "{outcome:?}"
            );
        }
    }

    #[test]
    fn the_trackers_fixture_gives_a_working_and_a_failing_row() {
        let response: TorrentGetResponse =
            serde_json::from_value(fixture_result(TORRENT_GET_TRACKERS)).unwrap();
        let record = response.torrents.into_iter().next().unwrap();

        let rows = rows_of(
            record
                .tracker_stats
                .unwrap()
                .into_iter()
                .map(TrackerStat::from)
                .collect(),
        );

        assert_eq!(rows.len(), 2);
        let (first, second) = (&rows[0], &rows[1]);
        assert_eq!((first.id, first.tier), (TrackerId(6), 0));
        assert_eq!(first.host, "tracker2.example.org");
        assert_eq!(first.sitename, None);
        assert_eq!(first.status, TrackerStatus::Working);
        assert_eq!(first.last, Some(at(1_793_784_667)));
        assert_eq!(first.next, NextAnnounce::At(at(1_793_787_926)));
        assert_eq!(first.peers, Some(30));
        assert_eq!(
            (first.seeders, first.leechers, first.downloads),
            (None, None, None)
        );
        assert!(!first.scrape_problem);
        assert_eq!((second.id, second.tier), (TrackerId(7), 1));
        assert_eq!(second.status, TrackerStatus::Failed);
        assert_eq!(
            second.message.as_deref(),
            Some("Could not connect to tracker")
        );
        assert_eq!(second.next, NextAnnounce::At(at(1_793_791_162)));
        assert_eq!(second.peers, None);
        assert!(second.scrape_problem);
    }
}
