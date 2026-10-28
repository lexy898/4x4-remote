//! Builders shared by the unit tests of this crate.

use x4_rpc::{HashString, TorrentId};

use crate::model::{Eta, Ratio, Torrent};
use crate::state::DisplayState;

/// The `result` of a recorded JSON-RPC response from `x4_rpc::testing::fixtures`; deserialize it
/// into the type the method returns.
pub(crate) fn fixture_result(envelope: &str) -> serde_json::Value {
    let mut envelope: serde_json::Value =
        serde_json::from_str(envelope).expect("a fixture is valid JSON");
    envelope
        .get_mut("result")
        .map(serde_json::Value::take)
        .expect("a fixture is a success response with a result")
}

/// A complete, idle, seeding torrent called `name`, with every other field neutral.
///
/// Override fields with struct-update syntax: `Torrent { rate_up: 5, ..torrent("a") }`. The hash
/// is derived from the name: equal names give equal hashes and different names (in practice)
/// different ones, so a test that depends on the order of hashes sets `hash` itself.
pub(crate) fn torrent(name: &str) -> Torrent {
    Torrent {
        id: TorrentId(0),
        hash: hash_for(name),
        name: name.to_owned(),
        state: DisplayState::Seeding,
        attention: None,
        active: false,
        progress: 1.0,
        metadata_progress: 1.0,
        total_size: 0,
        size_when_done: 0,
        left_until_done: 0,
        rate_down: 0,
        rate_up: 0,
        eta: Eta::NotAvailable,
        ratio: Ratio::NotAvailable,
        peers_connected: 0,
        peers_getting_from_us: 0,
        peers_sending_to_us: 0,
        download_dir: String::new(),
        added: None,
        done: None,
        queue_position: 0,
        sequential: false,
        recheck_progress: 0.0,
        labels: Vec::new(),
    }
}

/// A valid 40-digit hash made of two FNV-1a digests of the name and its length.
///
/// FNV is spelled out here, rather than taken from `DefaultHasher`, because the standard hasher
/// does not promise the same output across Rust versions.
fn hash_for(name: &str) -> HashString {
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    const BASIS: u64 = 0xcbf2_9ce4_8422_2325;

    let digest = |basis: u64| {
        name.bytes().fold(basis, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(PRIME)
        })
    };
    let length = u32::try_from(name.len()).unwrap_or(u32::MAX);
    let hex = format!("{:016x}{:016x}{length:08x}", digest(BASIS), digest(!BASIS));
    HashString::parse(&hex).expect("sixteen plus sixteen plus eight hex digits is a valid hash")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hash_follows_the_name() {
        assert_eq!(torrent("a").hash, torrent("a").hash);
        assert_ne!(torrent("a").hash, torrent("b").hash);
        assert_ne!(torrent("a").hash, torrent("A").hash);
        assert_eq!(torrent("Тед Лассо").hash.as_str().len(), 40);
    }
}
