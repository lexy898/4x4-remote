//! The session methods of [`Client`] and their request parameters (rpc-spec §4.1, §4.2, §4.7),
//! and the server version check.
//!
//! The parameter structs here are the only place that decides what goes on the wire: they carry
//! the wire names of the `*_kbps` fields and leave out every setting that is `None`.

use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};

use crate::client::{Client, JsonRpcAnswer, Protocol};
use crate::error::RpcError;
use crate::legacy::OLDEST_RPC_VERSION;
use crate::session::{
    FreeSpace, MIN_RPC_MAJOR, SESSION_FIELDS, ServerVersion, SessionInfo, SessionSet, SessionStats,
    semver_major,
};
use crate::transport::Transport;

/// The only fields `check_server` asks for: it must work on a server too new or too old to
/// answer the rest.
const VERSION_FIELDS: &[&str] = &["version", "rpc_version_semver"];

/// The fields `check_server` asks a legacy daemon for: Transmission 3.00 has only the integer
/// `rpc-version`, and ignores a field it does not know.
pub(crate) const LEGACY_VERSION_FIELDS: &[&str] = &["version", "rpc_version", "rpc_version_semver"];

impl<T: Transport> Client<T> {
    /// Reads the session settings the app uses (`session_get` with [`SESSION_FIELDS`]).
    ///
    /// In the legacy protocol it also tells whether the daemon still is one of the legacy
    /// protocol. Transmission 4.1 answers legacy requests too, so a daemon upgraded to it since
    /// [`Client::check_server`] would go on being spoken to in the legacy protocol; its session
    /// gives it away, as it reports an `rpc_version_semver` of JSON-RPC (major 6 or later), and
    /// the session is read on every poll of the app.
    ///
    /// # Errors
    ///
    /// Any [`RpcError`] of the call; [`RpcError::InvalidResponse`] when a requested field is
    /// missing from the result or has the wrong type, and in the legacy protocol when the
    /// session reports an RPC version of JSON-RPC: the caller checks the server again, and the
    /// check finds JSON-RPC.
    pub fn session_get(&mut self) -> Result<SessionInfo, RpcError> {
        let session: SessionInfo = self.call(
            "session_get",
            Some(&GetParams {
                fields: SESSION_FIELDS,
            }),
        )?;
        if self.protocol() == Protocol::Legacy
            && let Some(semver) = &session.rpc_version_semver
            && semver_major(semver).is_some_and(|major| major >= MIN_RPC_MAJOR)
        {
            return Err(RpcError::InvalidResponse(format!(
                "session_get: the server reports RPC version {semver}, which is JSON-RPC, since \
                 the check found the legacy protocol"
            )));
        }
        Ok(session)
    }

    /// Changes the session settings that are `Some` in `set` (`session_set`).
    ///
    /// This changes the daemon's own configuration, which outlives the client.
    ///
    /// # Errors
    ///
    /// Any [`RpcError`] of the call.
    pub fn session_set(&mut self, set: &SessionSet) -> Result<(), RpcError> {
        let params = SetParams {
            alt_speed_enabled: set.alt_speed_enabled,
            speed_limit_down: set.speed_limit_down_kbps,
            speed_limit_down_enabled: set.speed_limit_down_enabled,
            speed_limit_up: set.speed_limit_up_kbps,
            speed_limit_up_enabled: set.speed_limit_up_enabled,
        };
        self.call("session_set", Some(&params)).map(|IgnoredAny| ())
    }

    /// Reads the daemon's speeds, torrent counts and transfer totals (`session_stats`).
    ///
    /// # Errors
    ///
    /// Any [`RpcError`] of the call; [`RpcError::InvalidResponse`] when the result does not
    /// decode.
    pub fn session_stats(&mut self) -> Result<SessionStats, RpcError> {
        self.call("session_stats", None::<&()>)
    }

    /// Reads the free space of the directory `path` on the server (`free_space`).
    ///
    /// # Errors
    ///
    /// Any [`RpcError`] of the call. A directory the server cannot read is an
    /// [`RpcError::Server`] with code 5 and the system's message as its detail. Transmission
    /// 4.1.0-beta reports such a directory as a success with a free space of `-1`; that answer
    /// is the same error, without a detail, so that a missing folder reads alike from every
    /// daemon.
    pub fn free_space(&mut self, path: &str) -> Result<FreeSpace, RpcError> {
        self.call("free_space", Some(&FreeSpaceParams { path }))
    }

    /// Reads the server's version, decides the protocol the calls go in from then on (see
    /// [`Client::protocol`]), and checks that 4x4 supports the server's RPC version.
    ///
    /// It asks in JSON-RPC first, so a Transmission 4.1 or newer costs one request, as ever. A
    /// daemon that answers in the legacy protocol is asked once more, in that protocol, for its
    /// `rpc-version`. Every check starts over in JSON-RPC, so a daemon upgraded or downgraded
    /// behind the same address is found out at the next connect.
    ///
    /// # Errors
    ///
    /// Any [`RpcError`] of the calls; [`RpcError::UnsupportedServer`] with the
    /// [`ServerVersion::unsupported_reason`]: a JSON-RPC daemon whose `rpc_version_semver` has a
    /// major below 6 or is not a version, or a legacy daemon whose `rpc-version` is below 16
    /// (before Transmission 3.00) or missing.
    pub fn check_server(&mut self) -> Result<ServerVersion, RpcError> {
        self.set_protocol(Protocol::JsonRpc);
        self.set_legacy_rpc_version(OLDEST_RPC_VERSION);
        let answer = self.call_json_rpc(
            "session_get",
            Some(&GetParams {
                fields: VERSION_FIELDS,
            }),
            None,
        )?;
        let version = match answer {
            JsonRpcAnswer::Result(VersionResult {
                version,
                rpc_version_semver,
            }) => ServerVersion {
                version,
                rpc_semver: rpc_version_semver,
                rpc_version: None,
                protocol: Protocol::JsonRpc,
            },
            JsonRpcAnswer::Legacy => {
                self.set_protocol(Protocol::Legacy);
                let result: LegacyVersionResult = self.call(
                    "session_get",
                    Some(&GetParams {
                        fields: LEGACY_VERSION_FIELDS,
                    }),
                )?;
                // A daemon that reports none is unsupported; the oldest version asks for least.
                self.set_legacy_rpc_version(result.rpc_version.unwrap_or(OLDEST_RPC_VERSION));
                ServerVersion {
                    version: result.version,
                    rpc_semver: result.rpc_version_semver.unwrap_or_default(),
                    rpc_version: result.rpc_version,
                    protocol: Protocol::Legacy,
                }
            }
        };
        match version.unsupported_reason() {
            None => Ok(version),
            Some(reason) => Err(RpcError::UnsupportedServer { reason }),
        }
    }
}

#[derive(Serialize)]
struct GetParams {
    fields: &'static [&'static str],
}

#[derive(Serialize)]
struct SetParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    alt_speed_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed_limit_down: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed_limit_down_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed_limit_up: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    speed_limit_up_enabled: Option<bool>,
}

#[derive(Serialize)]
struct FreeSpaceParams<'a> {
    path: &'a str,
}

/// The result of `session_get` for [`VERSION_FIELDS`].
#[derive(Deserialize)]
struct VersionResult {
    version: String,
    rpc_version_semver: String,
}

/// The result of a legacy `session_get` for [`LEGACY_VERSION_FIELDS`]. Transmission 3.00 sends no
/// `rpc_version_semver`; a daemon that sends no `rpc_version` either is unsupported for that.
#[derive(Deserialize)]
struct LegacyVersionResult {
    version: String,
    #[serde(default)]
    rpc_version: Option<u32>,
    #[serde(default)]
    rpc_version_semver: Option<String>,
}
