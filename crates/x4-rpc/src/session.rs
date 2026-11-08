//! Session wire types: the settings and statistics of the daemon, the free-space report and the
//! server version (rpc-spec §4.1, §4.2, §4.7), and the inputs and outcomes of the session methods
//! on [`Client`](crate::Client).
//!
//! The `*_kbps` fields hold kilobytes per second as the server reports them (`speed_limit_down`,
//! …); turning them into the app's units is `x4-core`'s job.

use std::fmt;

use serde::de::{Unexpected, Visitor};
use serde::{Deserialize, Deserializer};

use crate::client::Protocol;
use crate::error::UnsupportedReason;

/// The `session_get` fields the app reads, in the order it requests them.
pub const SESSION_FIELDS: &[&str] = &[
    "version",
    "rpc_version_semver",
    "download_dir",
    "alt_speed_enabled",
    "alt_speed_down",
    "alt_speed_up",
    "speed_limit_down",
    "speed_limit_down_enabled",
    "speed_limit_up",
    "speed_limit_up_enabled",
    "sequential_download",
    "start_added_torrents",
];

/// The oldest `rpc_version_semver` major 4x4 supports in JSON-RPC: 6 is JSON-RPC 2.0,
/// Transmission 4.1.
const MIN_RPC_MAJOR: u64 = 6;

/// The oldest legacy `rpc-version` 4x4 supports: 16 is Transmission 3.00.
const MIN_LEGACY_RPC_VERSION: u32 = 16;

/// The session settings the app reads (`session_get` with [`SESSION_FIELDS`]).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "a record of the daemon's independent on/off settings, as the wire has them"
)]
pub struct SessionInfo {
    /// `version`: the daemon's long version string, `$version ($revision)`.
    pub version: String,
    /// `rpc_version_semver`: the RPC API version; `None` from a daemon that predates it
    /// (Transmission 3.00).
    #[serde(default)]
    pub rpc_version_semver: Option<String>,
    /// `download_dir`: where new torrents download to by default.
    pub download_dir: String,
    /// `alt_speed_enabled`: whether the alternative speed limits apply.
    pub alt_speed_enabled: bool,
    /// `alt_speed_down`: the alternative download limit in kB/s.
    #[serde(rename = "alt_speed_down", deserialize_with = "kbps")]
    pub alt_speed_down_kbps: u32,
    /// `alt_speed_up`: the alternative upload limit in kB/s.
    #[serde(rename = "alt_speed_up", deserialize_with = "kbps")]
    pub alt_speed_up_kbps: u32,
    /// `speed_limit_down`: the download limit in kB/s, which applies only when enabled.
    #[serde(rename = "speed_limit_down", deserialize_with = "kbps")]
    pub speed_limit_down_kbps: u32,
    /// `speed_limit_down_enabled`: whether the download limit applies.
    pub speed_limit_down_enabled: bool,
    /// `speed_limit_up`: the upload limit in kB/s, which applies only when enabled.
    #[serde(rename = "speed_limit_up", deserialize_with = "kbps")]
    pub speed_limit_up_kbps: u32,
    /// `speed_limit_up_enabled`: whether the upload limit applies.
    pub speed_limit_up_enabled: bool,
    /// `sequential_download`: whether added torrents download sequentially by default; `None`
    /// from a daemon that cannot download in order (before Transmission 4.1.0-beta.2).
    #[serde(default)]
    pub sequential_download: Option<bool>,
    /// `start_added_torrents`: whether an added torrent starts at once when the add leaves out
    /// `paused`.
    pub start_added_torrents: bool,
}

/// Reads a speed in kB/s. Transmission 4.1.0 (RPC 6.0.0, which is supported) sent the limits as
/// floats and 4.1.1 went back to integers, so both are accepted; fractions are truncated.
fn kbps<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
    struct KbpsVisitor;

    impl Visitor<'_> for KbpsVisitor {
        type Value = u32;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a speed in kB/s: a number from 0 to 4294967295")
        }

        fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<u32, E> {
            u32::try_from(value).map_err(|_| E::invalid_value(Unexpected::Unsigned(value), &self))
        }

        fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<u32, E> {
            u32::try_from(value).map_err(|_| E::invalid_value(Unexpected::Signed(value), &self))
        }

        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "the range check above the cast keeps the value within 0..=u32::MAX"
        )]
        fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<u32, E> {
            // Both bounds also reject NaN; JSON has no infinities.
            if value >= 0.0 && value < f64::from(u32::MAX) + 1.0 {
                Ok(value as u32)
            } else {
                Err(E::invalid_value(Unexpected::Float(value), &self))
            }
        }
    }

    deserializer.deserialize_any(KbpsVisitor)
}

/// Session settings to change (`session_set`); `None` leaves a setting as it is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SessionSet {
    /// `alt_speed_enabled`
    pub alt_speed_enabled: Option<bool>,
    /// `speed_limit_down`, in kB/s.
    pub speed_limit_down_kbps: Option<u32>,
    /// `speed_limit_down_enabled`
    pub speed_limit_down_enabled: Option<bool>,
    /// `speed_limit_up`, in kB/s.
    pub speed_limit_up_kbps: Option<u32>,
    /// `speed_limit_up_enabled`
    pub speed_limit_up_enabled: Option<bool>,
}

/// Transfer totals over one period (rpc-spec §4.2, "stats object").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct StatsBlock {
    /// `downloaded_bytes`
    pub downloaded_bytes: u64,
    /// `uploaded_bytes`
    pub uploaded_bytes: u64,
    /// `files_added`: how many torrents were added.
    pub files_added: u64,
    /// `seconds_active`
    pub seconds_active: u64,
    /// `session_count`: how many times the daemon started.
    pub session_count: u64,
}

/// The daemon's speeds, torrent counts and transfer totals (`session_stats`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct SessionStats {
    /// `download_speed` in bytes/s.
    pub download_speed: u64,
    /// `upload_speed` in bytes/s.
    pub upload_speed: u64,
    /// `active_torrent_count`
    pub active_torrent_count: u32,
    /// `paused_torrent_count`
    pub paused_torrent_count: u32,
    /// `torrent_count`
    pub torrent_count: u32,
    /// `current_stats`: since the daemon started.
    pub current_stats: StatsBlock,
    /// `cumulative_stats`: over every run of the daemon.
    pub cumulative_stats: StatsBlock,
}

/// How much room a directory on the server has (`free_space`).
///
/// The server reports `-1` for a size it could not read, hence the signed sizes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FreeSpace {
    /// `path`: the directory that was asked about.
    pub path: String,
    /// `size_bytes`: free space in bytes.
    pub size_bytes: i64,
    /// `total_size`: capacity in bytes; `None` from a daemon that does not report it
    /// (Transmission 3.00).
    #[serde(default)]
    pub total_size: Option<i64>,
}

/// What a server calls itself, as [`Client::check_server`](crate::Client::check_server) read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerVersion {
    /// `version`: the daemon's long version string, `$version ($revision)`.
    pub version: String,
    /// `rpc_version_semver`: the RPC API version, as the server wrote it; empty when it sends
    /// none (Transmission 3.00).
    pub rpc_semver: String,
    /// `rpc-version`: the legacy RPC version, an integer; read only from a daemon of the legacy
    /// protocol, `None` from any other.
    pub rpc_version: Option<u32>,
    /// The protocol the server speaks, which decides which of the versions above counts.
    pub protocol: Protocol,
}

impl ServerVersion {
    /// The major number of [`rpc_semver`](Self::rpc_semver), or `None` when it does not start
    /// with a number and a dot.
    #[must_use]
    pub fn rpc_major(&self) -> Option<u64> {
        let (major, _) = self.rpc_semver.split_once('.')?;
        // `u64::from_str` would also accept a leading `+`.
        if major.is_empty() || !major.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        major.parse().ok()
    }

    /// Why 4x4 cannot talk to this server, or `None` when it can: in JSON-RPC when its RPC major
    /// is 6 (Transmission 4.1) or later, in the legacy protocol when its `rpc-version` is 16
    /// (Transmission 3.00) or later.
    #[must_use]
    pub fn unsupported_reason(&self) -> Option<UnsupportedReason> {
        match self.protocol {
            Protocol::JsonRpc => {
                if self.rpc_major().is_some_and(|major| major >= MIN_RPC_MAJOR) {
                    None
                } else {
                    Some(UnsupportedReason::from_rpc_semver(&self.rpc_semver))
                }
            }
            Protocol::Legacy => match self.rpc_version {
                Some(version) if version >= MIN_LEGACY_RPC_VERSION => None,
                Some(version) => Some(UnsupportedReason::Version(version.to_string())),
                None => Some(UnsupportedReason::NoVersion),
            },
        }
    }

    /// Whether 4x4 can talk to this server: it has no
    /// [`unsupported_reason`](Self::unsupported_reason).
    #[must_use]
    pub fn is_supported(&self) -> bool {
        self.unsupported_reason().is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(rpc_semver: &str) -> ServerVersion {
        ServerVersion {
            version: "4.1.3 (838877323f)".to_owned(),
            rpc_semver: rpc_semver.to_owned(),
            rpc_version: None,
            protocol: Protocol::JsonRpc,
        }
    }

    fn legacy(rpc_version: Option<u32>, rpc_semver: &str) -> ServerVersion {
        ServerVersion {
            version: "4.0.5 (a6fe2a64aa)".to_owned(),
            rpc_semver: rpc_semver.to_owned(),
            rpc_version,
            protocol: Protocol::Legacy,
        }
    }

    #[test]
    fn rpc_major_reads_the_number_before_the_first_dot() {
        assert_eq!(version("6.0.1").rpc_major(), Some(6));
        assert_eq!(version("10.2.0").rpc_major(), Some(10));
        assert_eq!(version("6.0.0-beta.1").rpc_major(), Some(6));
    }

    #[test]
    fn rpc_major_is_none_for_text_that_is_not_a_version() {
        for text in [
            "", "abc", ".1", "6", "v6.0.1", "+6.0.1", "-6.0.1", " 6.0.1", "a.6.0",
        ] {
            assert_eq!(version(text).rpc_major(), None, "{text:?}");
        }
    }

    #[test]
    fn rpc_major_is_none_when_the_number_overflows() {
        assert_eq!(version("99999999999999999999999.0.0").rpc_major(), None);
    }

    #[test]
    fn support_starts_at_rpc_major_6() {
        assert!(!version("5.4.0").is_supported());
        assert!(!version("0.0.0").is_supported());
        assert!(version("6.0.0").is_supported());
        assert!(version("7.0.0").is_supported());
        assert!(!version("").is_supported());
    }

    #[test]
    fn a_json_rpc_server_is_judged_by_its_semver_alone() {
        assert_eq!(
            version("5.4.0").unsupported_reason(),
            Some(UnsupportedReason::Version("5.4.0".to_owned()))
        );
        assert_eq!(
            version("").unsupported_reason(),
            Some(UnsupportedReason::NoVersion)
        );
        let with_a_legacy_number = ServerVersion {
            rpc_version: Some(18),
            ..version("5.4.0")
        };
        assert!(!with_a_legacy_number.is_supported());
    }

    #[test]
    fn legacy_support_starts_at_rpc_version_16() {
        assert_eq!(legacy(Some(16), "").unsupported_reason(), None);
        assert_eq!(legacy(Some(18), "5.4.0").unsupported_reason(), None);
        assert_eq!(
            legacy(Some(15), "").unsupported_reason(),
            Some(UnsupportedReason::Version("15".to_owned()))
        );
        assert_eq!(
            legacy(None, "5.3.0").unsupported_reason(),
            Some(UnsupportedReason::NoVersion)
        );
        assert!(legacy(Some(17), "5.3.0").is_supported());
        assert!(!legacy(Some(15), "").is_supported());
    }
}
