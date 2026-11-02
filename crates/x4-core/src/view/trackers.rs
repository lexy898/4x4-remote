//! What the trackers of a torrent are doing, in one summary.

use crate::details::{TorrentDetails, TrackerOutcome, TrackerStat};

/// What kind of trouble a tracker is in; the UI words it through its i18n layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrackerProblemKind {
    /// The last announce failed: the tracker could not be reached, or it refused. The daemon's
    /// text says which.
    AnnounceFailed,
    /// The tracker did not answer the last announce in time.
    AnnounceTimedOut,
    /// The last scrape failed. Many trackers do not support scrapes, so it is the mildest kind.
    ScrapeFailed,
    /// The tracker did not answer the last scrape in time.
    ScrapeTimedOut,
}

impl TrackerProblemKind {
    /// Whether the kind is mild, for the UI to say it quietly: the problems of a scrape, which
    /// many trackers never answer, while the announce, which brings the peers, works.
    #[must_use]
    pub fn is_mild(self) -> bool {
        match self {
            Self::AnnounceFailed | Self::AnnounceTimedOut => false,
            Self::ScrapeFailed | Self::ScrapeTimedOut => true,
        }
    }

    /// How bad the kind is; the worse a problem is, the greater the number. An announce is what
    /// keeps the torrent supplied with peers, so its problems outrank those of a scrape, and a
    /// refusal, which the user may have to act on, outranks a timeout, which may pass.
    fn severity(self) -> u8 {
        match self {
            Self::AnnounceFailed => 3,
            Self::AnnounceTimedOut => 2,
            Self::ScrapeFailed => 1,
            Self::ScrapeTimedOut => 0,
        }
    }
}

/// The worst problem among the trackers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackerProblem {
    /// What kind of trouble it is.
    pub kind: TrackerProblemKind,
    /// The tracker's host, without the port.
    pub host: String,
    /// The daemon's text for the failure, shown as sent and not localized; it may be empty.
    pub result: String,
}

/// How the trackers of a torrent are doing, and the worst problem among them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrackerSummary {
    /// How many trackers the torrent has.
    pub total: usize,
    /// How many answered their last announce.
    pub working: usize,
    /// How many failed their last announce, or did not answer it in time.
    pub failing: usize,
    /// How many the daemon has not announced to since it started: all of them for a torrent that
    /// has not run since then, and backup trackers. A stop is announced too, so a torrent paused
    /// after it ran keeps its trackers' last answers. These are neither working nor failing.
    pub not_contacted: usize,
    /// The worst current problem, even when other trackers work; `None` when there is none.
    pub problem: Option<TrackerProblem>,
}

/// One count of how the trackers are doing, as the summary says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackerCount {
    /// So many trackers answered their last announce.
    Working(usize),
    /// So many failed their last announce, or did not answer it in time.
    Failing(usize),
    /// So many were not announced to yet.
    NotContacted(usize),
}

impl TrackerCount {
    /// How many trackers it counts.
    fn count(self) -> usize {
        match self {
            Self::Working(count) | Self::Failing(count) | Self::NotContacted(count) => count,
        }
    }
}

impl TrackerSummary {
    /// What the summary says, in this order: how many trackers work, how many fail and how many
    /// were not contacted yet, each only when there are some; nothing for a torrent without
    /// trackers.
    ///
    /// The count of all trackers is not said: a healthy torrent that has not run since the daemon
    /// started has contacted none of them, and "0 of 3 working" would read as broken where "3 not
    /// contacted yet" says what is so.
    #[must_use]
    pub fn counts(&self) -> Vec<TrackerCount> {
        [
            TrackerCount::Working(self.working),
            TrackerCount::Failing(self.failing),
            TrackerCount::NotContacted(self.not_contacted),
        ]
        .into_iter()
        .filter(|count| count.count() > 0)
        .collect()
    }
}

/// The summary of the trackers in `details`.
///
/// The worst problem is the one of the highest [severity](TrackerProblemKind): a failed announce,
/// then an announce that timed out, then a failed scrape, then a scrape that timed out. Among
/// problems of the same kind the first tracker in the list wins. A tracker is held to its
/// announce first: a scrape problem of a tracker whose announce failed is not reported on its own.
#[must_use]
pub fn tracker_summary(details: &TorrentDetails) -> TrackerSummary {
    let mut summary = TrackerSummary {
        total: details.trackers.len(),
        ..TrackerSummary::default()
    };
    let mut worst: Option<(TrackerProblemKind, &str, &str)> = None;
    for tracker in &details.trackers {
        match tracker.last_announce.as_ref().map(|answer| answer.outcome) {
            Some(TrackerOutcome::Succeeded) => summary.working += 1,
            Some(TrackerOutcome::Failed | TrackerOutcome::TimedOut) => summary.failing += 1,
            None => summary.not_contacted += 1,
        }
        if let Some((kind, result)) = problem_of(tracker)
            && worst.is_none_or(|(found, ..)| kind.severity() > found.severity())
        {
            worst = Some((kind, &tracker.host, result));
        }
    }
    summary.problem = worst.map(|(kind, host, result)| TrackerProblem {
        kind,
        host: host_without_port(host).to_owned(),
        result: result.to_owned(),
    });
    summary
}

/// The problem a tracker is in, with the daemon's text for it; the announce before the scrape.
fn problem_of(tracker: &TrackerStat) -> Option<(TrackerProblemKind, &str)> {
    let announce = tracker.last_announce.as_ref().and_then(|answer| {
        match answer.outcome {
            TrackerOutcome::Succeeded => None,
            TrackerOutcome::Failed => Some(TrackerProblemKind::AnnounceFailed),
            TrackerOutcome::TimedOut => Some(TrackerProblemKind::AnnounceTimedOut),
        }
        .map(|kind| (kind, answer.result.as_str()))
    });
    let scrape = || {
        tracker.last_scrape.as_ref().and_then(|answer| {
            match answer.outcome {
                TrackerOutcome::Succeeded => None,
                TrackerOutcome::Failed => Some(TrackerProblemKind::ScrapeFailed),
                TrackerOutcome::TimedOut => Some(TrackerProblemKind::ScrapeTimedOut),
            }
            .map(|kind| (kind, answer.result.as_str()))
        })
    };
    announce.or_else(scrape)
}

/// The host of `host_and_port` as the daemon reports a tracker (`tracker.example.org:443`),
/// without the port.
fn host_without_port(host_and_port: &str) -> &str {
    if host_and_port.starts_with('[') {
        // An IPv6 address in brackets, which has colons of its own: `[2001:db8::1]:6969`.
        return host_and_port
            .find(']')
            .and_then(|end| host_and_port.get(..=end))
            .unwrap_or(host_and_port);
    }
    match host_and_port.rsplit_once(':') {
        // A second colon means a bare IPv6 address, whose last group is not a port.
        Some((host, port))
            if !host.contains(':')
                && !port.is_empty()
                && port.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            host
        }
        _ => host_and_port,
    }
}

#[cfg(test)]
mod tests {
    use x4_rpc::HashString;

    use super::*;
    use crate::details::TrackerAnswer;
    use crate::test_support::details;

    fn answer(outcome: TrackerOutcome, result: &str) -> TrackerAnswer {
        TrackerAnswer {
            outcome,
            result: result.to_owned(),
        }
    }

    fn tracker(
        host: &str,
        last_announce: Option<TrackerAnswer>,
        last_scrape: Option<TrackerAnswer>,
    ) -> TrackerStat {
        TrackerStat {
            host: host.to_owned(),
            last_announce,
            last_scrape,
            seeders: None,
            leechers: None,
        }
    }

    fn working(host: &str) -> TrackerStat {
        tracker(
            host,
            Some(answer(TrackerOutcome::Succeeded, "Success")),
            None,
        )
    }

    fn summary(trackers: Vec<TrackerStat>) -> TrackerSummary {
        let hash = HashString::parse(&"cd".repeat(20)).unwrap();
        tracker_summary(&TorrentDetails {
            trackers,
            ..details(&hash, &[])
        })
    }

    fn problem(kind: TrackerProblemKind, host: &str, result: &str) -> TrackerProblem {
        TrackerProblem {
            kind,
            host: host.to_owned(),
            result: result.to_owned(),
        }
    }

    #[test]
    fn tracker_summary_reports_the_failing_tracker_even_with_several_trackers() {
        let summary = summary(vec![
            working("good.example.org:443"),
            tracker(
                "bad.example.org:6969",
                Some(answer(TrackerOutcome::Failed, "Unregistered torrent")),
                None,
            ),
            tracker("backup.example.org:80", None, None),
            working("udp.example.org:1337"),
        ]);

        assert_eq!(summary.total, 4);
        assert_eq!(summary.working, 2);
        assert_eq!(summary.failing, 1);
        assert_eq!(summary.not_contacted, 1);
        assert_eq!(
            summary.problem,
            Some(problem(
                TrackerProblemKind::AnnounceFailed,
                "bad.example.org",
                "Unregistered torrent"
            ))
        );
    }

    #[test]
    fn a_failed_announce_outranks_a_timeout_and_a_timeout_outranks_a_scrape() {
        let scrape_failed = tracker(
            "scrape.example.org:80",
            Some(answer(TrackerOutcome::Succeeded, "Success")),
            Some(answer(TrackerOutcome::Failed, "HTTP 404")),
        );
        let timed_out = tracker(
            "slow.example.org:80",
            Some(answer(TrackerOutcome::TimedOut, "Tracker did not respond")),
            None,
        );
        let refused = tracker(
            "refuse.example.org:80",
            Some(answer(
                TrackerOutcome::Failed,
                "Could not connect to tracker",
            )),
            None,
        );

        let kind = |trackers: Vec<TrackerStat>| summary(trackers).problem.map(|p| (p.kind, p.host));

        assert_eq!(
            kind(vec![
                scrape_failed.clone(),
                timed_out.clone(),
                refused.clone()
            ]),
            Some((
                TrackerProblemKind::AnnounceFailed,
                "refuse.example.org".to_owned()
            ))
        );
        assert_eq!(
            kind(vec![scrape_failed.clone(), timed_out.clone()]),
            Some((
                TrackerProblemKind::AnnounceTimedOut,
                "slow.example.org".to_owned()
            ))
        );
        assert_eq!(
            kind(vec![working("ok.example.org:80"), scrape_failed]),
            Some((
                TrackerProblemKind::ScrapeFailed,
                "scrape.example.org".to_owned()
            ))
        );
    }

    #[test]
    fn a_scrape_that_timed_out_is_the_mildest_problem() {
        let summary = summary(vec![tracker(
            "a.example.org:80",
            Some(answer(TrackerOutcome::Succeeded, "Success")),
            Some(answer(TrackerOutcome::TimedOut, "Tracker did not respond")),
        )]);

        assert_eq!((summary.working, summary.failing), (1, 0));
        assert_eq!(
            summary.problem,
            Some(problem(
                TrackerProblemKind::ScrapeTimedOut,
                "a.example.org",
                "Tracker did not respond"
            ))
        );
    }

    #[test]
    fn only_the_problems_of_a_scrape_are_mild() {
        assert!(!TrackerProblemKind::AnnounceFailed.is_mild());
        assert!(!TrackerProblemKind::AnnounceTimedOut.is_mild());
        assert!(TrackerProblemKind::ScrapeFailed.is_mild());
        assert!(TrackerProblemKind::ScrapeTimedOut.is_mild());
    }

    #[test]
    fn the_first_tracker_wins_among_problems_of_the_same_kind() {
        let failed = |host: &str, text: &str| {
            tracker(host, Some(answer(TrackerOutcome::Failed, text)), None)
        };

        let summary = summary(vec![
            failed("first.example.org:80", "one"),
            failed("second.example.org:80", "two"),
        ]);

        assert_eq!(summary.failing, 2);
        assert_eq!(
            summary.problem,
            Some(problem(
                TrackerProblemKind::AnnounceFailed,
                "first.example.org",
                "one"
            ))
        );
    }

    #[test]
    fn a_failed_announce_is_reported_before_the_scrape_of_the_same_tracker() {
        let summary = summary(vec![tracker(
            "a.example.org:80",
            Some(answer(TrackerOutcome::Failed, "refused")),
            Some(answer(TrackerOutcome::Failed, "also refused")),
        )]);

        assert_eq!(
            summary.problem,
            Some(problem(
                TrackerProblemKind::AnnounceFailed,
                "a.example.org",
                "refused"
            ))
        );
    }

    #[test]
    fn trackers_that_were_never_announced_to_count_as_neither_working_nor_failing() {
        let summary = summary(vec![
            tracker("a.example.org:80", None, None),
            tracker("b.example.org:80", None, None),
        ]);

        assert_eq!(
            summary,
            TrackerSummary {
                total: 2,
                working: 0,
                failing: 0,
                not_contacted: 2,
                problem: None
            }
        );
    }

    #[test]
    fn the_counts_say_what_was_contacted_and_leave_out_those_of_none() {
        let failed = || {
            tracker(
                "bad.example.org:80",
                Some(answer(TrackerOutcome::Failed, "refused")),
                None,
            )
        };
        let waiting = || tracker("backup.example.org:80", None, None);

        // A torrent that has not run since the daemon started has contacted none of its trackers:
        // that is not a failure.
        assert_eq!(
            summary(vec![waiting(), waiting(), waiting()]).counts(),
            [TrackerCount::NotContacted(3)]
        );
        assert_eq!(
            summary(vec![
                working("a.example.org:80"),
                working("b.example.org:80")
            ])
            .counts(),
            [TrackerCount::Working(2)]
        );
        assert_eq!(
            summary(vec![waiting(), failed(), working("a.example.org:80")]).counts(),
            [
                TrackerCount::Working(1),
                TrackerCount::Failing(1),
                TrackerCount::NotContacted(1)
            ]
        );
        assert_eq!(
            summary(vec![failed(), failed()]).counts(),
            [TrackerCount::Failing(2)]
        );
        assert_eq!(summary(Vec::new()).counts(), []);
    }

    #[test]
    fn healthy_trackers_and_no_trackers_have_no_problem() {
        let healthy = summary(vec![
            working("a.example.org:80"),
            working("b.example.org:80"),
        ]);
        assert_eq!((healthy.total, healthy.working, healthy.failing), (2, 2, 0));
        assert_eq!(healthy.problem, None);

        assert_eq!(summary(Vec::new()), TrackerSummary::default());
    }

    #[test]
    fn the_port_is_left_off_the_host() {
        assert_eq!(
            host_without_port("tracker.example.org:443"),
            "tracker.example.org"
        );
        assert_eq!(
            host_without_port("tracker.example.org"),
            "tracker.example.org"
        );
        assert_eq!(host_without_port("203.0.113.7:6969"), "203.0.113.7");
        assert_eq!(host_without_port("[2001:db8::1]:6969"), "[2001:db8::1]");
        assert_eq!(host_without_port("[2001:db8::1]"), "[2001:db8::1]");
        // A bare IPv6 address has colons of its own, and the last group could be taken for a port.
        assert_eq!(host_without_port("2001:db8::1"), "2001:db8::1");
        assert_eq!(
            host_without_port("tracker.example.org:http"),
            "tracker.example.org:http"
        );
        assert_eq!(host_without_port(""), "");
    }
}
