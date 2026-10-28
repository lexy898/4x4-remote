//! Torrent identifiers and the `ids` request parameter.
//!
//! Omitting `ids` makes actions, `torrent_set` and `torrent_remove` apply to *every* torrent, so
//! the selection type for them, [`TorrentSelection`], cannot be empty. Only `torrent_get` may ask
//! for all torrents, through [`GetIds::All`].

use std::fmt;

use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};

/// The daemon's numeric torrent id.
///
/// Ids are renumbered when the daemon restarts; anything long-lived uses [`HashString`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TorrentId(pub u64);

/// A torrent's info hash as lowercase hex: 40 digits (SHA-1) or 64 digits (SHA-256).
///
/// Stable across daemon restarts. Deserializing validates the value like [`HashString::parse`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct HashString(String);

impl HashString {
    /// Validates a hex info hash; uppercase digits are normalised to lowercase.
    ///
    /// # Errors
    ///
    /// [`IdError::InvalidHash`] when `s` is not 40 or 64 hexadecimal digits.
    pub fn parse(s: &str) -> Result<Self, IdError> {
        let valid_length = s.len() == 40 || s.len() == 64;
        if valid_length && s.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            Ok(Self(s.to_ascii_lowercase()))
        } else {
            Err(IdError::InvalidHash(s.to_owned()))
        }
    }

    /// The hash as lowercase hex.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for HashString {
    type Error = IdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl fmt::Display for HashString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One torrent in an `ids` list: serialized as a number (id) or a string (hash).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(untagged)]
pub enum TorrentRef {
    /// By numeric id; valid only until the daemon restarts.
    Id(TorrentId),
    /// By info hash.
    Hash(HashString),
}

/// A non-empty list of torrents, serialized as the `ids` array.
///
/// The only way to name the torrents an action, `torrent_set` or `torrent_remove` applies to:
/// it can never be empty, so a request can never fall back to "all torrents".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct TorrentSelection(Vec<TorrentRef>);

impl TorrentSelection {
    /// A selection of the given torrents.
    ///
    /// # Errors
    ///
    /// [`IdError::EmptySelection`] when `refs` is empty.
    pub fn new(refs: Vec<TorrentRef>) -> Result<Self, IdError> {
        if refs.is_empty() {
            Err(IdError::EmptySelection)
        } else {
            Ok(Self(refs))
        }
    }

    /// A selection of torrents by info hash.
    ///
    /// # Errors
    ///
    /// [`IdError::EmptySelection`] when `hashes` yields nothing.
    pub fn from_hashes(hashes: impl IntoIterator<Item = HashString>) -> Result<Self, IdError> {
        Self::new(hashes.into_iter().map(TorrentRef::Hash).collect())
    }

    /// The selected torrents; never empty.
    #[must_use]
    pub fn refs(&self) -> &[TorrentRef] {
        &self.0
    }
}

/// Which torrents `torrent_get` reports.
///
/// Serializes as the `ids` *member* of the request parameters (a map with zero or one entry),
/// so a params struct embeds it with `#[serde(flatten)]`: [`GetIds::All`] omits `ids`,
/// [`GetIds::RecentlyActive`] sends `"ids": "recently_active"` and [`GetIds::Selection`] sends
/// the id list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GetIds {
    /// Every torrent (`ids` omitted).
    All,
    /// The given torrents.
    Selection(TorrentSelection),
    /// Torrents changed recently, plus the ids of recently removed ones.
    RecentlyActive,
}

impl Serialize for GetIds {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::All => serializer.serialize_map(Some(0))?.end(),
            Self::Selection(selection) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("ids", selection)?;
                map.end()
            }
            Self::RecentlyActive => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("ids", "recently_active")?;
                map.end()
            }
        }
    }
}

/// An invalid torrent identifier or selection.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdError {
    /// A [`TorrentSelection`] was built from no torrents.
    #[error("a torrent selection must name at least one torrent")]
    EmptySelection,
    /// The value is not a 40- or 64-digit hex info hash.
    #[error("invalid torrent hash {0:?}: expected 40 or 64 hexadecimal digits")]
    InvalidHash(String),
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const HASH: &str = "0123456789abcdef0123456789abcdef01234567";

    #[derive(Serialize)]
    struct Params<'a> {
        #[serde(flatten)]
        ids: &'a GetIds,
        fields: [&'static str; 1],
    }

    fn params(ids: &GetIds) -> serde_json::Value {
        serde_json::to_value(Params {
            ids,
            fields: ["id"],
        })
        .unwrap()
    }

    fn selection() -> TorrentSelection {
        TorrentSelection::new(vec![
            TorrentRef::Id(TorrentId(3)),
            TorrentRef::Hash(HashString::parse(HASH).unwrap()),
        ])
        .unwrap()
    }

    #[test]
    fn selection_serializes_as_mixed_id_list() {
        assert_eq!(serde_json::to_value(selection()).unwrap(), json!([3, HASH]));
    }

    #[test]
    fn get_all_omits_ids_when_flattened() {
        assert_eq!(params(&GetIds::All), json!({ "fields": ["id"] }));
    }

    #[test]
    fn get_recently_active_sends_the_keyword() {
        assert_eq!(
            params(&GetIds::RecentlyActive),
            json!({ "ids": "recently_active", "fields": ["id"] })
        );
    }

    #[test]
    fn get_selection_sends_the_list() {
        assert_eq!(
            params(&GetIds::Selection(selection())),
            json!({ "ids": [3, HASH], "fields": ["id"] })
        );
    }

    #[test]
    fn ids_round_trip_through_json() {
        let id: TorrentId = serde_json::from_value(json!(7)).unwrap();
        assert_eq!(id, TorrentId(7));
        assert_eq!(serde_json::to_value(id).unwrap(), json!(7));

        let hash: HashString = serde_json::from_value(json!(HASH.to_uppercase())).unwrap();
        assert_eq!(hash.as_str(), HASH);
        assert_eq!(serde_json::to_value(&hash).unwrap(), json!(HASH));
    }

    #[test]
    fn deserializing_an_invalid_hash_fails() {
        assert!(serde_json::from_value::<HashString>(json!("xyz")).is_err());
    }

    #[test]
    fn from_hashes_keeps_order() {
        let other = HashString::parse(&"f".repeat(40)).unwrap();
        let hashes = vec![HashString::parse(HASH).unwrap(), other.clone()];

        let selection = TorrentSelection::from_hashes(hashes).unwrap();

        assert_eq!(
            selection.refs(),
            [
                TorrentRef::Hash(HashString::parse(HASH).unwrap()),
                TorrentRef::Hash(other)
            ]
        );
    }
}
