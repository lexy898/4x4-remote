//! Why the Trackers tab has no rows.

use crate::details::TorrentDetails;

/// Why the Trackers tab shows no trackers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackersEmpty {
    /// The details have not come: nothing is known yet.
    Reading,
    /// The torrent has no trackers, so it finds peers through DHT and peer exchange alone.
    NoTrackers,
    /// A private torrent has no trackers. It may not use DHT or peer exchange either, so nothing
    /// finds it peers.
    NoTrackersPrivate,
}

/// Why the Trackers tab shows no trackers for a torrent whose details were last read as
/// `details`; `None` when there are trackers to show.
///
/// Before the details have come nothing is known, so no torrent is said to have no trackers yet:
/// the same care as for [`peers_empty`](crate::peers_empty).
#[must_use]
pub fn trackers_empty(details: Option<&TorrentDetails>) -> Option<TrackersEmpty> {
    let Some(details) = details else {
        return Some(TrackersEmpty::Reading);
    };
    if !details.trackers.is_empty() {
        return None;
    }
    Some(if details.is_private {
        TrackersEmpty::NoTrackersPrivate
    } else {
        TrackersEmpty::NoTrackers
    })
}

#[cfg(test)]
mod tests {
    use x4_rpc::HashString;

    use super::*;
    use crate::test_support::{details, tracker};

    fn details_of(trackers: &[&str], is_private: bool) -> TorrentDetails {
        let hash = HashString::parse(&"ab".repeat(20)).unwrap();
        TorrentDetails {
            trackers: trackers.iter().map(|host| tracker(host)).collect(),
            is_private,
            ..details(&hash, &[])
        }
    }

    #[test]
    fn before_the_details_come_nothing_is_known() {
        assert_eq!(trackers_empty(None), Some(TrackersEmpty::Reading));
    }

    #[test]
    fn a_torrent_with_trackers_has_rows_to_show() {
        assert_eq!(
            trackers_empty(Some(&details_of(&["a.example.org:80"], false))),
            None
        );
        assert_eq!(
            trackers_empty(Some(&details_of(&["a.example.org:80"], true))),
            None
        );
    }

    #[test]
    fn a_torrent_without_trackers_says_so() {
        assert_eq!(
            trackers_empty(Some(&details_of(&[], false))),
            Some(TrackersEmpty::NoTrackers)
        );
    }

    #[test]
    fn a_private_torrent_without_trackers_is_not_told_it_has_dht_and_pex() {
        assert_eq!(
            trackers_empty(Some(&details_of(&[], true))),
            Some(TrackersEmpty::NoTrackersPrivate)
        );
    }
}
