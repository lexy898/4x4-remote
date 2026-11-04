//! The peers of a torrent: the records of `peers` and `peers_from` in `torrent_get` (rpc-spec
//! §3.3).
//!
//! Only the keys 4x4 shows are kept (`peer_id` changes between reads and the `bytes_to_*` totals
//! are experimental). Text is as the peers and the daemon wrote it: an address with no port, a
//! client name that a peer chose, a flag string of letters. Every key is read leniently
//! (see the private `lenient` module), so that one odd peer cannot fail the read of the others.

use serde::Deserialize;

use crate::lenient::lenient;

/// One connected peer, an entry of `peers`.
///
/// The booleans keep the wire's names, which point two opposite ways: "client" is this side, so
/// `client_is_choked` means the *peer chokes us*, and `peer_is_choked` means *we choke the peer*.
/// `is_downloading_from` and `is_uploading_to` say that the other side is unchoked and
/// interested, not that bytes are flowing: the rates tell that.
///
/// A key the daemon leaves out, or sends as another type, holds its default.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "a record of the daemon's flags for one peer, as the wire has them"
)]
pub struct PeerRecord {
    /// `address`: the peer's IP in text, without port or brackets. It may also be an IPv4-mapped
    /// IPv6 form (`::ffff:192.0.2.1`) or the daemon's text `Invalid address`.
    #[serde(deserialize_with = "lenient")]
    pub address: String,
    /// `port`: the peer's port; an ephemeral one for an incoming connection.
    #[serde(deserialize_with = "lenient")]
    pub port: u16,
    /// `client_name`: what the peer calls itself, which it controls: `Name/1.2.3`, `Name 1.2.3`,
    /// the first bytes of its peer id with `%XX` for unprintable ones, or empty.
    #[serde(deserialize_with = "lenient")]
    pub client_name: String,
    /// `flag_str`: the letters of the daemon's state flags for the peer (`TODdUuK?EHXI`).
    #[serde(deserialize_with = "lenient")]
    pub flag_str: String,
    /// `progress`: the share of the torrent the peer has, 0.0 to 1.0.
    #[serde(deserialize_with = "lenient")]
    pub progress: f64,
    /// `rate_to_client`: bytes/s from the peer to us, averaged over the last two seconds.
    #[serde(deserialize_with = "lenient")]
    pub rate_to_client: u64,
    /// `rate_to_peer`: bytes/s from us to the peer, averaged over the last two seconds.
    #[serde(deserialize_with = "lenient")]
    pub rate_to_peer: u64,
    /// `is_encrypted`
    #[serde(deserialize_with = "lenient")]
    pub is_encrypted: bool,
    /// `is_utp`: the connection runs over uTP, not TCP.
    #[serde(deserialize_with = "lenient")]
    pub is_utp: bool,
    /// `is_incoming`: the peer connected to us.
    #[serde(deserialize_with = "lenient")]
    pub is_incoming: bool,
    /// `is_downloading_from`: we are interested and the peer has unchoked us.
    #[serde(deserialize_with = "lenient")]
    pub is_downloading_from: bool,
    /// `is_uploading_to`: the peer is interested and we have unchoked it.
    #[serde(deserialize_with = "lenient")]
    pub is_uploading_to: bool,
    /// `client_is_choked`: the peer chokes us.
    #[serde(deserialize_with = "lenient")]
    pub client_is_choked: bool,
    /// `client_is_interested`: we are interested in the peer.
    #[serde(deserialize_with = "lenient")]
    pub client_is_interested: bool,
    /// `peer_is_choked`: we choke the peer.
    #[serde(deserialize_with = "lenient")]
    pub peer_is_choked: bool,
    /// `peer_is_interested`: the peer is interested in us.
    #[serde(deserialize_with = "lenient")]
    pub peer_is_interested: bool,
}

/// Where the connected peers were first found, the `peers_from` object: each count is of the
/// peers connected now, and together they add up to the number of connected peers.
///
/// A key the daemon leaves out, or sends as another type, holds `0`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct PeersFromRecord {
    /// `from_cache`: from the resume file of an earlier run.
    #[serde(deserialize_with = "lenient")]
    pub from_cache: u32,
    /// `from_dht`
    #[serde(deserialize_with = "lenient")]
    pub from_dht: u32,
    /// `from_incoming`: connected to us on their own.
    #[serde(deserialize_with = "lenient")]
    pub from_incoming: u32,
    /// `from_lpd`: found on the local network.
    #[serde(deserialize_with = "lenient")]
    pub from_lpd: u32,
    /// `from_ltep`: named in a peer's extension handshake.
    #[serde(deserialize_with = "lenient")]
    pub from_ltep: u32,
    /// `from_pex`: named by another peer.
    #[serde(deserialize_with = "lenient")]
    pub from_pex: u32,
    /// `from_tracker`
    #[serde(deserialize_with = "lenient")]
    pub from_tracker: u32,
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    /// A peer entry as the daemon sends it, `peer_id` and `bytes_to_*` included: an idle seeder
    /// reached over uTP.
    fn idle_peer() -> Value {
        json!({
            "address": "192.0.2.7",
            "port": 51413,
            "client_name": "Client/1.2.3",
            "peer_id": "AAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            "flag_str": "TKEX",
            "progress": 1.0,
            "rate_to_client": 0,
            "rate_to_peer": 0,
            "bytes_to_client": 0,
            "bytes_to_peer": 0,
            "is_encrypted": true,
            "is_utp": true,
            "is_incoming": false,
            "is_downloading_from": false,
            "is_uploading_to": false,
            "client_is_choked": false,
            "client_is_interested": false,
            "peer_is_choked": true,
            "peer_is_interested": false,
        })
    }

    fn peer(value: Value) -> PeerRecord {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn an_idle_peer_parses_every_key_read() {
        assert_eq!(
            peer(idle_peer()),
            PeerRecord {
                address: "192.0.2.7".to_owned(),
                port: 51413,
                client_name: "Client/1.2.3".to_owned(),
                flag_str: "TKEX".to_owned(),
                progress: 1.0,
                rate_to_client: 0,
                rate_to_peer: 0,
                is_encrypted: true,
                is_utp: true,
                is_incoming: false,
                is_downloading_from: false,
                is_uploading_to: false,
                client_is_choked: false,
                client_is_interested: false,
                peer_is_choked: true,
                peer_is_interested: false,
            }
        );
    }

    #[test]
    fn an_active_peer_keeps_its_rates_and_flags() {
        let mut entry = idle_peer();
        entry["progress"] = json!(0.37);
        entry["rate_to_client"] = json!(1_500_000);
        entry["rate_to_peer"] = json!(48_000);
        entry["flag_str"] = json!("TODUEI");
        entry["is_incoming"] = json!(true);
        entry["is_downloading_from"] = json!(true);
        entry["is_uploading_to"] = json!(true);
        entry["client_is_interested"] = json!(true);
        entry["peer_is_choked"] = json!(false);
        entry["peer_is_interested"] = json!(true);

        let record = peer(entry);

        assert!((record.progress - 0.37).abs() < f64::EPSILON);
        assert_eq!(
            (record.rate_to_client, record.rate_to_peer),
            (1_500_000, 48_000)
        );
        assert_eq!(record.flag_str, "TODUEI");
        assert!(record.is_incoming && record.is_downloading_from && record.is_uploading_to);
        assert!(record.client_is_interested && record.peer_is_interested);
        assert!(!record.peer_is_choked && !record.client_is_choked);
    }

    #[test]
    fn addresses_are_kept_as_the_daemon_wrote_them() {
        let address = |text: &str| {
            let mut entry = idle_peer();
            entry["address"] = json!(text);
            peer(entry).address
        };

        assert_eq!(address("2001:db8::1"), "2001:db8::1");
        assert_eq!(address("::ffff:192.0.2.7"), "::ffff:192.0.2.7");
        assert_eq!(address("Invalid address"), "Invalid address");
        assert_eq!(address(""), "");
    }

    #[test]
    fn client_names_keep_the_three_shapes_and_the_empty_one() {
        let name = |text: &str| {
            let mut entry = idle_peer();
            entry["client_name"] = json!(text);
            peer(entry).client_name
        };

        assert_eq!(name("Name/1.2.3"), "Name/1.2.3");
        assert_eq!(name("Name 1.2.3"), "Name 1.2.3");
        assert_eq!(name("ab%01%02cd%FE%00"), "ab%01%02cd%FE%00");
        assert_eq!(name(""), "");
    }

    #[test]
    fn an_unknown_flag_letter_is_kept_for_the_core_to_judge() {
        let mut entry = idle_peer();
        entry["flag_str"] = json!("TQ?");

        assert_eq!(peer(entry).flag_str, "TQ?");
    }

    #[test]
    fn the_edges_of_the_ports_and_progress_parse() {
        let mut entry = idle_peer();
        entry["port"] = json!(65535);
        entry["progress"] = json!(0);

        let record = peer(entry);

        assert_eq!(record.port, 65535);
        assert!(record.progress.abs() < f64::EPSILON);
    }

    #[test]
    fn missing_keys_hold_their_defaults() {
        let record = peer(json!({ "address": "192.0.2.7", "port": 6881 }));

        assert_eq!(
            record,
            PeerRecord {
                address: "192.0.2.7".to_owned(),
                port: 6881,
                ..PeerRecord::default()
            }
        );
        assert_eq!(peer(json!({})), PeerRecord::default());
    }

    #[test]
    fn keys_of_another_type_hold_their_defaults_and_the_others_survive() {
        let mut entry = idle_peer();
        entry["port"] = json!("x");
        entry["progress"] = json!("half");
        entry["rate_to_client"] = json!(-5);
        entry["is_utp"] = json!("yes");
        entry["flag_str"] = json!(7);

        let record = peer(entry);

        assert_eq!(record.port, 0);
        assert!(record.progress.abs() < f64::EPSILON);
        assert_eq!(record.rate_to_client, 0);
        assert!(!record.is_utp);
        assert_eq!(record.flag_str, "");
        assert_eq!(record.address, "192.0.2.7");
        assert_eq!(record.client_name, "Client/1.2.3");
        assert!(record.is_encrypted);
    }

    #[test]
    fn peers_from_parses_the_seven_counts() {
        let from: PeersFromRecord = serde_json::from_value(json!({
            "from_cache": 1,
            "from_dht": 2,
            "from_incoming": 3,
            "from_lpd": 4,
            "from_ltep": 5,
            "from_pex": 6,
            "from_tracker": 7,
        }))
        .unwrap();

        assert_eq!(
            from,
            PeersFromRecord {
                from_cache: 1,
                from_dht: 2,
                from_incoming: 3,
                from_lpd: 4,
                from_ltep: 5,
                from_pex: 6,
                from_tracker: 7,
            }
        );
    }

    #[test]
    fn peers_from_with_missing_or_odd_counts_keeps_the_good_ones() {
        let from: PeersFromRecord = serde_json::from_value(json!({
            "from_dht": "x",
            "from_pex": 9,
            "from_tracker": -1,
        }))
        .unwrap();

        assert_eq!(
            from,
            PeersFromRecord {
                from_pex: 9,
                ..PeersFromRecord::default()
            }
        );
    }

    #[test]
    fn a_torrent_record_holds_the_peers_and_their_origins() {
        let record: crate::TorrentRecord = serde_json::from_value(json!({
            "id": 3,
            "peers": [idle_peer(), idle_peer()],
            "peers_from": { "from_dht": 2 },
        }))
        .unwrap();

        assert_eq!(record.peers.as_ref().map(Vec::len), Some(2));
        assert_eq!(record.peers_from.map(|from| from.from_dht), Some(2));
        assert_eq!(
            serde_json::from_value::<crate::TorrentRecord>(json!({ "id": 3 }))
                .unwrap()
                .peers,
            None
        );
    }
}
