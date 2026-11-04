//! The texts of the Trackers tab that carry values: what a tracker is doing, how long ago or from
//! now its announces are, its counts, and why the tab shows none.
//!
//! The column headers and the other words without variables ("No peer counts", "—") are plain
//! messages, which the tab fetches itself.

use std::time::SystemTime;

use gpui_kit::SharedString;
use x4_core::{
    NextAnnounce, RelativeTime, TimeDirection, TrackerRow, TrackerStatus, TrackersEmpty,
    relative_time,
};

use super::{duration_string, integer, t};

/// What a tracker is doing, in words: "Working", "Announcing", "Backup", "Not contacted yet",
/// "Announce failed" or "Timed out".
pub(crate) fn tracker_status_words(status: TrackerStatus) -> SharedString {
    match status {
        TrackerStatus::Working => t!("tracker-status-working"),
        TrackerStatus::Announcing => t!("tracker-status-announcing"),
        TrackerStatus::Backup => t!("tracker-status-backup"),
        TrackerStatus::NotContacted => t!("tracker-status-not-contacted"),
        TrackerStatus::Failed => t!("tracker-status-failed"),
        TrackerStatus::TimedOut => t!("tracker-status-timed-out"),
    }
    .into()
}

/// What the line under a tracker's address says: the server's own text for a failed announce, as
/// x4-core cleaned it, or the status in words when the server sent none or the tracker has not
/// failed. The text is passed to the message as it is, never read as part of one.
pub(crate) fn tracker_status_text(row: &TrackerRow) -> SharedString {
    match &row.message {
        Some(message) => message.clone().into(),
        None => tracker_status_words(row.status),
    }
}

/// How many peers the last announce returned, such as "30 peers".
pub(crate) fn tracker_peers_text(count: u32) -> SharedString {
    t!("tracker-peers", count = count).into()
}

/// The tier of a tracker as the tab shows it. The daemon counts from 0 and people from 1.
pub(crate) fn tracker_tier_text(tier: u32) -> SharedString {
    // The tier is a name, not an amount: no language groups its digits.
    tier.saturating_add(1).to_string().into()
}

/// How far a time is from now, in the units of the ETA column: "3 m ago", "in 27 m", "now".
pub(crate) fn relative_time_text(time: RelativeTime) -> SharedString {
    match time {
        RelativeTime::Now => t!("time-now"),
        RelativeTime::Ago(duration) => t!("time-ago", duration = duration_string(&duration)),
        RelativeTime::In(duration) => t!("time-in", duration = duration_string(&duration)),
    }
    .into()
}

/// When the last announce came, as it was at `now`: "3 m ago", or "—" when the server gave no
/// time.
pub(crate) fn tracker_last_text(last: Option<SystemTime>, now: SystemTime) -> SharedString {
    match last {
        Some(at) => relative_time_text(relative_time(at, now, TimeDirection::Past)),
        None => t!("tracker-value-none").into(),
    }
}

/// When the next announce is due, as it is at `now`: "in 27 m", "now" when it is due or late,
/// "queued" while it waits for a free slot, or "—" when nothing is scheduled.
pub(crate) fn tracker_next_text(next: NextAnnounce, now: SystemTime) -> SharedString {
    match next {
        NextAnnounce::At(at) => relative_time_text(relative_time(at, now, TimeDirection::Future)),
        NextAnnounce::Queued => t!("tracker-next-queued").into(),
        NextAnnounce::Idle => t!("tracker-value-none").into(),
    }
}

/// A count the tracker knows, with its digits in groups; "—" when it did not say.
pub(crate) fn tracker_count_text(count: Option<u32>) -> SharedString {
    match count {
        Some(count) => integer(count).into(),
        None => t!("tracker-value-none").into(),
    }
}

/// The name of a tracker's row for screen readers, with its tier: "tracker.example.org, tier 1".
pub(crate) fn tracker_row_label(host: &str, tier: u32) -> SharedString {
    t!(
        "tracker-row-label",
        host = host,
        tier = tracker_tier_text(tier).to_string()
    )
    .into()
}

/// What the tab says instead of the trackers: "—" until the details have come, or that the torrent
/// has none.
pub(crate) fn trackers_empty_text(empty: TrackersEmpty) -> SharedString {
    match empty {
        TrackersEmpty::Reading => t!("details-pending"),
        TrackersEmpty::NoTrackers => t!("trackers-none"),
        TrackersEmpty::NoTrackersPrivate => t!("trackers-none-private"),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use x4_core::test_support::tracker;
    use x4_core::{CompactDuration, TorrentDetails, tracker_rows};
    use x4_rpc::HashString;

    use super::*;

    /// A fixed time, so that no text depends on the clock.
    fn now() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_793_784_667)
    }

    fn compact(seconds: u64) -> CompactDuration {
        CompactDuration::from(Duration::from_secs(seconds))
    }

    /// The row of a tracker that failed its announce with `message`.
    fn failed_row(message: Option<&str>) -> TrackerRow {
        let hash = HashString::parse(&"ab".repeat(20)).unwrap();
        let mut row = tracker_rows(&TorrentDetails {
            trackers: vec![tracker("tracker.example.org:6969")],
            ..x4_core::test_support::details(&hash, &[])
        })
        .remove(0);
        row.status = TrackerStatus::Failed;
        row.message = message.map(str::to_owned);
        row
    }

    #[test]
    fn every_status_has_its_words() {
        for (status, words) in [
            (TrackerStatus::Working, "Working"),
            (TrackerStatus::Announcing, "Announcing"),
            (TrackerStatus::Backup, "Backup"),
            (TrackerStatus::NotContacted, "Not contacted yet"),
            (TrackerStatus::Failed, "Announce failed"),
            (TrackerStatus::TimedOut, "Timed out"),
        ] {
            assert_eq!(tracker_status_words(status), words, "{status:?}");
        }
    }

    #[test]
    fn a_failure_says_what_the_server_said_and_words_it_only_when_it_said_nothing() {
        assert_eq!(
            tracker_status_text(&failed_row(Some("Unregistered torrent"))),
            "Unregistered torrent"
        );
        assert_eq!(tracker_status_text(&failed_row(None)), "Announce failed");
        let mut timed_out = failed_row(None);
        timed_out.status = TrackerStatus::TimedOut;
        assert_eq!(tracker_status_text(&timed_out), "Timed out");
        let mut working = failed_row(None);
        working.status = TrackerStatus::Working;
        assert_eq!(tracker_status_text(&working), "Working");
    }

    #[test]
    fn the_servers_text_is_shown_as_sent_whatever_it_looks_like() {
        // Braces and placeholders in the tracker's text are no part of a message.
        let text = "{ $count } peers { -brand } \u{2014} done";
        assert_eq!(tracker_status_text(&failed_row(Some(text))), text);
    }

    #[test]
    fn the_peers_of_an_announce_are_counted_in_the_plural_of_the_count() {
        assert_eq!(tracker_peers_text(0), "0 peers");
        assert_eq!(tracker_peers_text(1), "1 peer");
        assert_eq!(tracker_peers_text(30), "30 peers");
    }

    #[test]
    fn the_tier_is_shown_from_one_without_grouping() {
        assert_eq!(tracker_tier_text(0), "1");
        assert_eq!(tracker_tier_text(2), "3");
        assert_eq!(tracker_tier_text(1_999), "2000");
        assert_eq!(tracker_tier_text(u32::MAX), "4294967295");
    }

    #[test]
    fn a_time_is_worded_in_the_units_of_the_eta_column() {
        assert_eq!(relative_time_text(RelativeTime::Now), "now");
        assert_eq!(
            relative_time_text(RelativeTime::Ago(compact(180))),
            "3 m ago"
        );
        assert_eq!(
            relative_time_text(RelativeTime::Ago(compact(59))),
            "59 s ago"
        );
        assert_eq!(
            relative_time_text(RelativeTime::In(compact(1_620))),
            "in 27 m"
        );
        assert_eq!(
            relative_time_text(RelativeTime::In(compact(7_320))),
            "in 2 h 2 m"
        );
        assert_eq!(
            relative_time_text(RelativeTime::Ago(compact(90_000))),
            "1 d 1 h ago"
        );
    }

    #[test]
    fn the_last_announce_is_counted_back_from_now() {
        let ago = |seconds: u64| {
            tracker_last_text(Some(now() - Duration::from_secs(seconds)), now()).to_string()
        };

        assert_eq!(ago(0), "now");
        assert_eq!(ago(45), "45 s ago");
        // Above a minute the seconds go, so the text does not change every second.
        assert_eq!(ago(24 * 60 + 59), "24 m ago");
        assert_eq!(ago(3 * 3_600 + 2 * 60 + 50), "3 h 2 m ago");
        // A time ahead of ours, as a daemon whose clock runs fast reports, is now.
        assert_eq!(
            tracker_last_text(Some(now() + Duration::from_secs(30)), now()),
            "now"
        );
        assert_eq!(tracker_last_text(None, now()), "—");
    }

    #[test]
    fn the_next_announce_is_counted_forward_from_now_or_says_why_it_is_not() {
        let within = |seconds: u64| {
            tracker_next_text(
                NextAnnounce::At(now() + Duration::from_secs(seconds)),
                now(),
            )
            .to_string()
        };

        assert_eq!(within(27 * 60 + 12), "in 27 m");
        assert_eq!(within(30), "in 30 s");
        assert_eq!(within(0), "now");
        // Due in the past, as a stale read shows it: it is due now.
        assert_eq!(
            tracker_next_text(NextAnnounce::At(now() - Duration::from_secs(90)), now()),
            "now"
        );
        assert_eq!(tracker_next_text(NextAnnounce::Queued, now()), "queued");
        assert_eq!(tracker_next_text(NextAnnounce::Idle, now()), "—");
    }

    #[test]
    fn a_count_has_grouped_digits_and_a_dash_when_the_tracker_did_not_say() {
        assert_eq!(tracker_count_text(Some(0)), "0");
        assert_eq!(tracker_count_text(Some(345)), "345");
        assert_eq!(tracker_count_text(Some(1_234_567)), "1,234,567");
        assert_eq!(tracker_count_text(None), "—");
    }

    #[test]
    fn a_row_is_named_with_its_tracker_and_tier() {
        assert_eq!(
            tracker_row_label("tracker.example.org:6969", 0),
            "tracker.example.org:6969, tier 1"
        );
        assert_eq!(
            tracker_row_label("[2001:db8::1]", 2),
            "[2001:db8::1], tier 3"
        );
    }

    #[test]
    fn each_empty_tab_says_why() {
        assert_eq!(trackers_empty_text(TrackersEmpty::Reading), "—");
        assert_eq!(
            trackers_empty_text(TrackersEmpty::NoTrackers),
            "This torrent has no trackers (DHT and PEX only)"
        );
        assert_eq!(
            trackers_empty_text(TrackersEmpty::NoTrackersPrivate),
            "This torrent has no trackers, and a private torrent cannot use DHT or PEX"
        );
    }
}
