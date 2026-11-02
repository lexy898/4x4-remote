//! The General tab of the details panel: what is known about the selected torrent, as values.

use std::time::SystemTime;

use x4_rpc::HashString;

use super::trackers::{TrackerSummary, tracker_summary};
use crate::details::TorrentDetails;
use crate::format::{HumanSize, human_size};
use crate::model::{Ratio, Torrent};
use crate::state::Attention;

/// How many pieces a torrent has and how big each one is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PiecesView {
    /// The number of pieces; never 0.
    pub count: u32,
    /// The size of one piece.
    pub size: HumanSize,
}

/// A problem the daemon reports for the torrent. The texts are the daemon's own, shown as sent
/// and not localized; one is empty when the daemon gave none, and the UI words that itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorView {
    /// A local error such as missing data or a full disk; the torrent is stopped.
    LocalError(String),
    /// The tracker's error.
    TrackerError(String),
    /// The tracker's warning.
    TrackerWarning(String),
}

/// Everything the General tab shows, as values. It holds no text: the UI words the labels and
/// formats the sizes, the dates and the ratio through its i18n layer.
#[derive(Debug, Clone, PartialEq)]
pub struct GeneralModel {
    /// Where the data is on the daemon's disk: the download directory and the torrent's name, as
    /// they would be typed in a shell on that machine.
    pub path: String,
    /// The size of all files.
    pub size_total: HumanSize,
    /// The size of the files that are wanted.
    pub size_wanted: HumanSize,
    /// Bytes downloaded over the torrent's life; zero until the details have been read.
    pub downloaded: HumanSize,
    /// Bytes uploaded over the torrent's life; zero until the details have been read.
    pub uploaded: HumanSize,
    /// The share ratio.
    pub ratio: Ratio,
    /// The info hash.
    pub hash: HashString,
    /// When the torrent was added.
    pub added: Option<SystemTime>,
    /// When the torrent finished downloading; `None` while it has not.
    pub done: Option<SystemTime>,
    /// When the metainfo was created; `None` when it does not say, or before the details.
    pub created: Option<SystemTime>,
    /// The comment of the metainfo; `None` when it has none or only blanks, or before the details.
    pub comment: Option<String>,
    /// The program that created the metainfo; `None` when it does not say or says only blanks, or
    /// before the details.
    pub creator: Option<String>,
    /// The source tag of the metainfo, which private trackers set; `None` when it has none or
    /// only blanks, or before the details.
    pub source: Option<String>,
    /// Whether the torrent is private.
    pub private: bool,
    /// The pieces; `None` while a magnet link has no metadata, or before the details.
    pub pieces: Option<PiecesView>,
    /// The problem the daemon reports for the torrent, if any.
    pub error: Option<ErrorView>,
    /// How the trackers are doing.
    pub trackers: TrackerSummary,
    /// Whether the details have been read. Until then the fields that come from them hold
    /// neutral values (zero, empty, no date), which the UI should not present as facts.
    pub details_loaded: bool,
}

/// The General tab for `torrent`, a row of the daemon's list, with what only the full details
/// know: the metainfo (comment, creator, source, creation date, privacy, pieces), the totals of
/// the torrent's life and the trackers. Everything else comes from the list.
///
/// Details that belong to another torrent are not used, so the tab can never show the comment of
/// one torrent under the name of another while the selection changes.
#[must_use]
pub fn general_model(torrent: &Torrent, details: Option<&TorrentDetails>) -> GeneralModel {
    let details = details.filter(|details| details.hash == torrent.hash);
    GeneralModel {
        path: join_path(&torrent.download_dir, &torrent.name),
        size_total: human_size(torrent.total_size),
        size_wanted: human_size(torrent.size_when_done),
        downloaded: human_size(details.map_or(0, |details| details.downloaded_ever)),
        uploaded: human_size(details.map_or(0, |details| details.uploaded_ever)),
        ratio: torrent.ratio,
        hash: torrent.hash.clone(),
        added: torrent.added,
        done: torrent.done,
        created: details.and_then(|details| details.date_created),
        comment: details.and_then(|details| written(&details.comment)),
        creator: details.and_then(|details| written(&details.creator)),
        source: details.and_then(|details| written(details.source.as_deref()?)),
        private: details.is_some_and(|details| details.is_private),
        // A magnet link has no pieces until its metadata comes.
        pieces: details
            .filter(|details| details.piece_count > 0)
            .map(|details| PiecesView {
                count: details.piece_count,
                size: human_size(details.piece_size),
            }),
        error: torrent.attention.as_ref().and_then(error_of),
        trackers: details.map(tracker_summary).unwrap_or_default(),
        details_loaded: details.is_some(),
    }
}

/// `text` as the metainfo has it, or `None` when it holds only blanks, which says nothing.
fn written(text: &str) -> Option<String> {
    (!text.trim().is_empty()).then(|| text.to_owned())
}

/// The error in `attention`, if it is one: fetching the metadata of a magnet link is not.
fn error_of(attention: &Attention) -> Option<ErrorView> {
    match attention {
        Attention::LocalError(text) => Some(ErrorView::LocalError(text.clone())),
        Attention::TrackerError(text) => Some(ErrorView::TrackerError(text.clone())),
        Attention::TrackerWarning(text) => Some(ErrorView::TrackerWarning(text.clone())),
        Attention::FetchingMetadata => None,
    }
}

/// `dir` and `name` joined with the separator of the daemon's system, which is told from the
/// directory: a Windows path has backslashes and no slash.
fn join_path(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        return name.to_owned();
    }
    if dir.ends_with(['/', '\\']) {
        return format!("{dir}{name}");
    }
    let separator = if dir.contains('\\') && !dir.contains('/') {
        '\\'
    } else {
        '/'
    };
    format!("{dir}{separator}{name}")
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use x4_rpc::FilePriority;

    use super::*;
    use crate::details::{TrackerAnswer, TrackerOutcome, TrackerStat};
    use crate::format::human_size;
    use crate::state::Attention;
    use crate::test_support::{details, torrent};
    use crate::view::trackers::{TrackerProblem, TrackerProblemKind};

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    /// A torrent from the list, and details that are about it.
    fn pair() -> (Torrent, TorrentDetails) {
        let torrent = Torrent {
            download_dir: "/data/torrents".to_owned(),
            total_size: 3_000,
            size_when_done: 2_000,
            ratio: Ratio::Value(1.5),
            added: Some(at(1_000)),
            done: Some(at(2_000)),
            ..torrent("Show")
        };
        let details = TorrentDetails {
            comment: "from the tracker".to_owned(),
            creator: "mktorrent".to_owned(),
            date_created: Some(at(500)),
            source: Some("PT".to_owned()),
            is_private: true,
            piece_count: 100,
            piece_size: 4 * 1024 * 1024,
            downloaded_ever: 5_000,
            uploaded_ever: 7_500,
            trackers: vec![TrackerStat {
                host: "tracker.example.org:443".to_owned(),
                last_announce: Some(TrackerAnswer {
                    outcome: TrackerOutcome::Failed,
                    result: "Unregistered torrent".to_owned(),
                }),
                last_scrape: None,
                seeders: None,
                leechers: None,
            }],
            ..details(
                &torrent.hash,
                &[("Show/a.mkv", 3_000, 3_000, true, FilePriority::Normal)],
            )
        };
        (torrent, details)
    }

    #[test]
    fn general_model_takes_metainfo_from_details_and_the_rest_from_the_list() {
        let (torrent, details) = pair();

        let model = general_model(&torrent, Some(&details));

        // From the list.
        assert_eq!(model.path, "/data/torrents/Show");
        assert_eq!(model.size_total, human_size(3_000));
        assert_eq!(model.size_wanted, human_size(2_000));
        assert_eq!(model.ratio, Ratio::Value(1.5));
        assert_eq!(model.hash, torrent.hash);
        assert_eq!(model.added, Some(at(1_000)));
        assert_eq!(model.done, Some(at(2_000)));
        // From the details.
        assert_eq!(model.downloaded, human_size(5_000));
        assert_eq!(model.uploaded, human_size(7_500));
        assert_eq!(model.created, Some(at(500)));
        assert_eq!(model.comment.as_deref(), Some("from the tracker"));
        assert_eq!(model.creator.as_deref(), Some("mktorrent"));
        assert_eq!(model.source.as_deref(), Some("PT"));
        assert!(model.private);
        assert_eq!(
            model.pieces,
            Some(PiecesView {
                count: 100,
                size: human_size(4 * 1024 * 1024)
            })
        );
        assert_eq!(
            model.trackers.problem,
            Some(TrackerProblem {
                kind: TrackerProblemKind::AnnounceFailed,
                host: "tracker.example.org".to_owned(),
                result: "Unregistered torrent".to_owned()
            })
        );
        assert!(model.details_loaded);
        assert_eq!(model.error, None);
    }

    #[test]
    fn before_the_details_have_come_the_list_fills_what_it_knows() {
        let (torrent, _) = pair();

        let model = general_model(&torrent, None);

        assert_eq!(model.path, "/data/torrents/Show");
        assert_eq!(model.size_total, human_size(3_000));
        assert_eq!(model.added, Some(at(1_000)));
        assert!(!model.details_loaded);
        assert_eq!(model.comment, None);
        assert_eq!(model.creator, None);
        assert_eq!(model.created, None);
        assert_eq!(model.source, None);
        assert!(!model.private);
        assert_eq!(model.pieces, None);
        assert_eq!(model.downloaded, human_size(0));
        assert_eq!(model.trackers, TrackerSummary::default());
    }

    #[test]
    fn a_blank_comment_or_source_is_none() {
        let (torrent, details) = pair();
        let with = |comment: &str, source: Option<&str>| {
            general_model(
                &torrent,
                Some(&TorrentDetails {
                    comment: comment.to_owned(),
                    source: source.map(str::to_owned),
                    ..details.clone()
                }),
            )
        };

        for blank in ["", " ", "\n\t "] {
            let model = with(blank, Some(blank));
            assert_eq!(model.comment, None, "{blank:?}");
            assert_eq!(model.source, None, "{blank:?}");
        }
        assert_eq!(with("", None).source, None);
        // A comment or source with text is kept as the metainfo has it, blanks and all.
        let model = with("  see the site\n", Some(" PT "));
        assert_eq!(model.comment.as_deref(), Some("  see the site\n"));
        assert_eq!(model.source.as_deref(), Some(" PT "));
    }

    #[test]
    fn a_blank_creator_is_none_and_any_other_is_kept_as_sent() {
        let (torrent, details) = pair();
        let creator = |creator: &str| {
            general_model(
                &torrent,
                Some(&TorrentDetails {
                    creator: creator.to_owned(),
                    ..details.clone()
                }),
            )
            .creator
        };

        for blank in ["", " ", "\n\t "] {
            assert_eq!(creator(blank), None, "{blank:?}");
        }
        assert_eq!(
            creator(" mktorrent 1.1\n").as_deref(),
            Some(" mktorrent 1.1\n")
        );
    }

    #[test]
    fn a_magnet_without_metadata_has_no_pieces() {
        let (torrent, details) = pair();
        let magnet = TorrentDetails {
            piece_count: 0,
            piece_size: 0,
            ..details
        };

        let model = general_model(&torrent, Some(&magnet));

        assert!(model.details_loaded);
        assert_eq!(model.pieces, None);
    }

    #[test]
    fn details_of_another_torrent_are_not_used() {
        let (torrent, details) = pair();
        let other = Torrent {
            hash: self::torrent("Other").hash,
            ..torrent
        };

        let model = general_model(&other, Some(&details));

        assert!(!model.details_loaded);
        assert_eq!(model.comment, None);
        assert_eq!(model.creator, None);
        // The other torrent's tracker problem is not this one's either.
        assert_eq!(model.trackers, TrackerSummary::default());
        assert_eq!(model.hash, other.hash);
    }

    #[test]
    fn the_path_joins_the_directory_and_the_name_as_the_daemon_does() {
        let named = |dir: &str, name: &str| {
            general_model(
                &Torrent {
                    download_dir: dir.to_owned(),
                    ..torrent(name)
                },
                None,
            )
            .path
        };

        assert_eq!(named("/data/torrents/", "Show"), "/data/torrents/Show");
        assert_eq!(named("/", "Show"), "/Show");
        assert_eq!(named("", "Show"), "Show");
        assert_eq!(named(r"C:\Downloads", "Show"), r"C:\Downloads\Show");
        assert_eq!(named(r"C:\Downloads\", "Show"), r"C:\Downloads\Show");
    }

    #[test]
    fn the_errors_of_the_list_become_an_error_view() {
        let with = |attention: Option<Attention>| {
            general_model(
                &Torrent {
                    attention,
                    ..torrent("a")
                },
                None,
            )
            .error
        };

        assert_eq!(
            with(Some(Attention::LocalError("No space left".to_owned()))),
            Some(ErrorView::LocalError("No space left".to_owned()))
        );
        assert_eq!(
            with(Some(Attention::TrackerError("down".to_owned()))),
            Some(ErrorView::TrackerError("down".to_owned()))
        );
        assert_eq!(
            with(Some(Attention::TrackerWarning("slow".to_owned()))),
            Some(ErrorView::TrackerWarning("slow".to_owned()))
        );
        // Fetching the metadata is not an error.
        assert_eq!(with(Some(Attention::FetchingMetadata)), None);
        assert_eq!(with(None), None);
    }
}
