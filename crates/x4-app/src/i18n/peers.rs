//! The texts of the Peers tab that carry values: the parts of the summary line above the peers,
//! why the tab shows none, and the name screen readers hear for a peer's address.
//!
//! The column headers, the words of the connection's icons and the "—" of a peer without a client
//! name are plain messages, which the tab fetches itself.

use gpui_kit::SharedString;
use x4_core::{PeerSource, PeersEmpty, PeersSummary};

use super::t;

/// The parts of the line above the peers, each shown on its own: how many are connected ("14
/// peers"), how many of them each way found, in the order x4-core gives ("9 from trackers", "3
/// from DHT"), and how many send to us and receive from us now ("4 sending to us", "2 receiving
/// from us"). A part whose count is 0 is left out; the count of peers always comes.
pub(crate) fn peers_summary_texts(summary: &PeersSummary) -> Vec<SharedString> {
    let connected = t!("peers-summary-connected", count = summary.connected);
    let sources = summary
        .sources
        .iter()
        .filter(|&&(_, count)| count > 0)
        .map(|&(source, count)| peer_source_text(source, count));
    let sending = (summary.sending_to_us > 0)
        .then(|| t!("peers-summary-sending", count = summary.sending_to_us));
    let receiving = (summary.receiving_from_us > 0)
        .then(|| t!("peers-summary-receiving", count = summary.receiving_from_us));
    std::iter::once(connected)
        .chain(sources)
        .chain(sending)
        .chain(receiving)
        .map(SharedString::from)
        .collect()
}

/// How many peers `source` found, such as "3 from DHT".
fn peer_source_text(source: PeerSource, count: u32) -> String {
    match source {
        PeerSource::Tracker => t!("peers-source-tracker", count = count),
        PeerSource::Dht => t!("peers-source-dht", count = count),
        PeerSource::Pex => t!("peers-source-pex", count = count),
        PeerSource::Lpd => t!("peers-source-lpd", count = count),
        PeerSource::Incoming => t!("peers-source-incoming", count = count),
        PeerSource::Ltep => t!("peers-source-ltep", count = count),
        PeerSource::Resume => t!("peers-source-resume", count = count),
    }
}

/// What the tab says instead of the peers: "—" until the first read has come, "The torrent is
/// paused", or "No peers connected".
pub(crate) fn peers_empty_text(empty: PeersEmpty) -> SharedString {
    match empty {
        PeersEmpty::Reading => t!("details-pending"),
        PeersEmpty::Paused => t!("peers-paused"),
        PeersEmpty::NoneConnected => t!("peers-none"),
    }
    .into()
}

/// The name of a peer's address for screen readers, with its port, which the row does not show:
/// "192.0.2.7, port 51413".
pub(crate) fn peer_address_label(address: &str, port: u16) -> SharedString {
    // The port is a name, not an amount: it is passed as text, so that no language groups its
    // digits.
    t!(
        "peers-address-label",
        address = address,
        port = port.to_string()
    )
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parts of the summary of `connected` peers found in the ways `sources`, of which
    /// `sending` send to us and `receiving` receive from us.
    fn parts(
        connected: u32,
        sources: &[(PeerSource, u32)],
        sending: u32,
        receiving: u32,
    ) -> Vec<String> {
        peers_summary_texts(&PeersSummary {
            connected,
            sources: sources.to_vec(),
            sending_to_us: sending,
            receiving_from_us: receiving,
        })
        .into_iter()
        .map(|part| part.to_string())
        .collect()
    }

    #[test]
    fn the_summary_counts_the_peers_in_the_plural_of_the_count() {
        assert_eq!(parts(1, &[], 0, 0), ["1 peer"]);
        assert_eq!(parts(5, &[], 0, 0), ["5 peers"]);
    }

    #[test]
    fn each_way_peers_are_found_has_its_words() {
        for (source, count, text) in [
            (PeerSource::Tracker, 1, "1 from a tracker"),
            (PeerSource::Tracker, 9, "9 from trackers"),
            (PeerSource::Dht, 3, "3 from DHT"),
            (PeerSource::Pex, 2, "2 from PEX"),
            (PeerSource::Lpd, 1, "1 from LPD"),
            (PeerSource::Incoming, 4, "4 incoming"),
            (PeerSource::Ltep, 1, "1 from LTEP"),
            (PeerSource::Resume, 6, "6 from the resume file"),
        ] {
            assert_eq!(
                parts(count, &[(source, count)], 0, 0),
                [peer_count(count), text.to_owned()],
                "{source:?}"
            );
        }
    }

    /// "1 peer" or "`count` peers".
    fn peer_count(count: u32) -> String {
        if count == 1 {
            "1 peer".to_owned()
        } else {
            format!("{count} peers")
        }
    }

    #[test]
    fn the_parts_follow_the_count_in_order_and_those_of_none_are_left_out() {
        assert_eq!(
            parts(
                14,
                &[
                    (PeerSource::Tracker, 9),
                    (PeerSource::Dht, 3),
                    (PeerSource::Pex, 2)
                ],
                4,
                2
            ),
            [
                "14 peers",
                "9 from trackers",
                "3 from DHT",
                "2 from PEX",
                "4 sending to us",
                "2 receiving from us"
            ]
        );
        assert_eq!(
            parts(3, &[(PeerSource::Pex, 3)], 0, 1),
            ["3 peers", "3 from PEX", "1 receiving from us"]
        );
        assert_eq!(
            parts(2, &[(PeerSource::Dht, 0)], 1, 0),
            ["2 peers", "1 sending to us"]
        );
    }

    #[test]
    fn each_empty_tab_says_why() {
        assert_eq!(peers_empty_text(PeersEmpty::Reading), "—");
        assert_eq!(
            peers_empty_text(PeersEmpty::Paused),
            "The torrent is paused"
        );
        assert_eq!(
            peers_empty_text(PeersEmpty::NoneConnected),
            "No peers connected"
        );
    }

    #[test]
    fn an_address_is_named_with_its_port() {
        assert_eq!(
            peer_address_label("192.0.2.7", 51413),
            "192.0.2.7, port 51413"
        );
        assert_eq!(
            peer_address_label("2001:db8::1", 6881),
            "2001:db8::1, port 6881"
        );
    }
}
