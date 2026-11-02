//! The texts of the details panel that carry values: what the panel says when it shows no
//! torrent, and the values of the General tab that x4-core's [`GeneralModel`] holds as numbers,
//! sizes, dates and the daemon's own texts.
//!
//! The labels of the General tab and the values without variables ("Yes", "not yet", "—") are
//! plain messages, which the panel fetches itself.
//!
//! [`GeneralModel`]: x4_core::GeneralModel

use std::time::SystemTime;

use gpui_kit::SharedString;
use x4_core::{
    DetailsTarget, ErrorView, HumanSize, PiecesView, TrackerCount, TrackerProblem,
    TrackerProblemKind, TrackerSummary,
};

use super::{date_time_text, has_text, integer, size_string, t};

/// What the panel says instead of a torrent: "Select a torrent" when none is selected, "3 torrents
/// selected" when several are; `None` when it shows one.
pub(crate) fn details_placeholder_text(target: &DetailsTarget) -> Option<SharedString> {
    match target {
        DetailsTarget::None => Some(t!("details-none").into()),
        &DetailsTarget::Many(count) => Some(t!("details-many", count = count).into()),
        DetailsTarget::One(_) => None,
    }
}

/// The size of a torrent: "22.9 GiB", or "22.9 GiB (4.1 GiB wanted)" when only some of its files
/// are wanted, so that the two sizes read differently.
pub(crate) fn size_with_wanted_text(size: HumanSize, wanted: HumanSize) -> SharedString {
    let (size, wanted) = (size_string(size), size_string(wanted));
    if size == wanted {
        return size.into();
    }
    t!("general-size-wanted", size = size, wanted = wanted).into()
}

/// How many pieces a torrent has and how large each is, such as "1,500 pieces of 4.0 MiB".
pub(crate) fn pieces_text(pieces: PiecesView) -> SharedString {
    t!(
        "general-pieces-value",
        count = pieces.count,
        pieces = integer(pieces.count),
        size = size_string(pieces.size)
    )
    .into()
}

/// Who made the metainfo and when, as far as it says: "by mktorrent on 2026-10-29 14:05", "by
/// mktorrent", the date alone, or "—" when it says neither. The creator is drawn as x4-core gives
/// it, which has already left out one that says nothing.
pub(crate) fn created_text(creator: Option<&str>, created: Option<SystemTime>) -> SharedString {
    match (creator, created) {
        (Some(creator), Some(created)) => t!(
            "general-created-by-on",
            creator = creator,
            date = date_time_text(created).to_string()
        )
        .into(),
        (Some(creator), None) => t!("general-created-by", creator = creator).into(),
        (None, Some(created)) => date_time_text(created),
        (None, None) => t!("general-none").into(),
    }
}

/// How the trackers are doing, in the counts x4-core picks: "3 working", "2 working, 1
/// failing", "3 not contacted yet" for a torrent that is paused, or "none" for a torrent without
/// trackers.
pub(crate) fn trackers_text(summary: &TrackerSummary) -> SharedString {
    let counts: Vec<String> = summary
        .counts()
        .into_iter()
        .map(tracker_count_text)
        .collect();
    match counts.as_slice() {
        [] => t!("general-trackers-none"),
        [one] => one.clone(),
        [first, second] => t!(
            "general-trackers-two",
            first = first.as_str(),
            second = second.as_str()
        ),
        [first, second, third, ..] => t!(
            "general-trackers-three",
            first = first.as_str(),
            second = second.as_str(),
            third = third.as_str()
        ),
    }
    .into()
}

/// One count of the trackers, such as "2 working".
fn tracker_count_text(count: TrackerCount) -> String {
    match count {
        TrackerCount::Working(count) => t!("general-trackers-working", count = count),
        TrackerCount::Failing(count) => t!("general-trackers-failing", count = count),
        TrackerCount::NotContacted(count) => t!("general-trackers-not-contacted", count = count),
    }
}

/// The worst problem among the trackers in plain words, such as "Tracker tracker.example.org:
/// connection timed out". The daemon's text is shown as sent.
pub(crate) fn tracker_problem_text(problem: &TrackerProblem) -> SharedString {
    let host = problem.host.as_str();
    let text = || daemon_text(&problem.result);
    match problem.kind {
        TrackerProblemKind::AnnounceFailed => {
            t!(
                "tracker-problem-announce-failed",
                host = host,
                text = text()
            )
        }
        TrackerProblemKind::AnnounceTimedOut => {
            t!("tracker-problem-announce-timed-out", host = host)
        }
        TrackerProblemKind::ScrapeFailed => {
            t!("tracker-problem-scrape-failed", host = host, text = text())
        }
        TrackerProblemKind::ScrapeTimedOut => t!("tracker-problem-scrape-timed-out", host = host),
    }
    .into()
}

/// The label and the value of the problem the daemon reports for a torrent: "Error" and "No space
/// left on device", for one. The text is the daemon's own, shown as sent.
pub(crate) fn error_texts(error: &ErrorView) -> (SharedString, SharedString) {
    let (label, text) = match error {
        ErrorView::LocalError(text) => (t!("general-error-local"), text),
        ErrorView::TrackerError(text) => (t!("general-error-tracker"), text),
        ErrorView::TrackerWarning(text) => (t!("general-warning-tracker"), text),
    };
    (label.into(), daemon_text(text).into())
}

/// The daemon's `text` as sent, or the words that stand in when it sent none.
fn daemon_text(text: &str) -> String {
    if has_text(text) {
        text.to_owned()
    } else {
        t!("general-error-no-text")
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use x4_core::human_size;
    use x4_core::test_support::torrent;

    use super::*;

    const MIB: u64 = 1024 * 1024;

    fn problem(kind: TrackerProblemKind, result: &str) -> TrackerProblem {
        TrackerProblem {
            kind,
            host: "tracker.example.org".to_owned(),
            result: result.to_owned(),
        }
    }

    #[test]
    fn the_placeholder_names_none_or_counts_the_torrents() {
        assert_eq!(
            details_placeholder_text(&DetailsTarget::None).as_deref(),
            Some("Select a torrent")
        );
        assert_eq!(
            details_placeholder_text(&DetailsTarget::Many(2)).as_deref(),
            Some("2 torrents selected")
        );
        assert_eq!(
            details_placeholder_text(&DetailsTarget::Many(1500)).as_deref(),
            Some("1500 torrents selected")
        );
        assert_eq!(
            details_placeholder_text(&DetailsTarget::One(torrent("a").hash)),
            None
        );
    }

    #[test]
    fn the_wanted_size_shows_only_when_it_reads_differently() {
        assert_eq!(
            size_with_wanted_text(human_size(3 * 1024 * MIB), human_size(2 * 1024 * MIB)),
            "3.0 GiB (2.0 GiB wanted)"
        );
        assert_eq!(
            size_with_wanted_text(human_size(3 * 1024 * MIB), human_size(3 * 1024 * MIB)),
            "3.0 GiB"
        );
        // A byte less reads the same.
        assert_eq!(
            size_with_wanted_text(human_size(3 * 1024 * MIB), human_size(3 * 1024 * MIB - 1)),
            "3.0 GiB"
        );
    }

    #[test]
    fn pieces_are_counted_with_plurals_and_grouped_digits() {
        let pieces = |count| PiecesView {
            count,
            size: human_size(4 * MIB),
        };
        assert_eq!(pieces_text(pieces(1)), "1 piece of 4.0 MiB");
        assert_eq!(pieces_text(pieces(1500)), "1,500 pieces of 4.0 MiB");
    }

    #[test]
    fn created_says_who_and_when_as_far_as_known() {
        let moment = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let date = date_time_text(moment);

        assert_eq!(
            created_text(Some("mktorrent 1.1"), Some(moment)),
            format!("by mktorrent 1.1 on {date}")
        );
        assert_eq!(
            created_text(Some("mktorrent 1.1"), None),
            "by mktorrent 1.1"
        );
        assert_eq!(created_text(None, Some(moment)), date);
        assert_eq!(created_text(None, None), "—");
        // The creator is drawn as sent: x4-core decides what says nothing.
        assert_eq!(created_text(Some(" mktorrent "), None), "by  mktorrent ");
    }

    #[test]
    fn trackers_are_counted_by_what_was_contacted() {
        let summary = |working, failing, not_contacted| TrackerSummary {
            total: working + failing + not_contacted,
            working,
            failing,
            not_contacted,
            problem: None,
        };
        assert_eq!(trackers_text(&summary(3, 0, 0)), "3 working");
        assert_eq!(trackers_text(&summary(2, 1, 0)), "2 working, 1 failing");
        // A paused or queued torrent has contacted none of them yet.
        assert_eq!(trackers_text(&summary(0, 0, 3)), "3 not contacted yet");
        assert_eq!(
            trackers_text(&summary(1, 1, 1)),
            "1 working, 1 failing, 1 not contacted yet"
        );
        assert_eq!(
            trackers_text(&summary(0, 2, 1)),
            "2 failing, 1 not contacted yet"
        );
        assert_eq!(trackers_text(&summary(0, 0, 0)), "none");
    }

    #[test]
    fn each_kind_of_tracker_problem_has_its_words() {
        use TrackerProblemKind::{AnnounceFailed, AnnounceTimedOut, ScrapeFailed, ScrapeTimedOut};

        assert_eq!(
            tracker_problem_text(&problem(AnnounceFailed, "Unregistered torrent")),
            "Tracker tracker.example.org: Unregistered torrent"
        );
        assert_eq!(
            tracker_problem_text(&problem(AnnounceTimedOut, "Tracker did not respond")),
            "Tracker tracker.example.org: connection timed out"
        );
        assert_eq!(
            tracker_problem_text(&problem(ScrapeFailed, "Could not connect to tracker")),
            "Tracker tracker.example.org sends no peer counts: Could not connect to tracker"
        );
        assert_eq!(
            tracker_problem_text(&problem(ScrapeTimedOut, "")),
            "Tracker tracker.example.org sends no peer counts: connection timed out"
        );
        // Without the daemon's text, the words that stand in for it.
        assert_eq!(
            tracker_problem_text(&problem(AnnounceFailed, " ")),
            "Tracker tracker.example.org: no details from the server"
        );
    }

    #[test]
    fn the_error_of_a_torrent_is_labelled_by_its_kind() {
        let texts = |error: ErrorView| {
            let (label, value) = error_texts(&error);
            (label.to_string(), value.to_string())
        };
        assert_eq!(
            texts(ErrorView::LocalError("No space left on device".to_owned())),
            ("Error".to_owned(), "No space left on device".to_owned())
        );
        assert_eq!(
            texts(ErrorView::TrackerError("down".to_owned())),
            ("Tracker error".to_owned(), "down".to_owned())
        );
        assert_eq!(
            texts(ErrorView::TrackerWarning(String::new())),
            (
                "Tracker warning".to_owned(),
                "no details from the server".to_owned()
            )
        );
    }
}
