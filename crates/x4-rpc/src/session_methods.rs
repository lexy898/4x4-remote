//! The session methods of [`Client`] and their request parameters (rpc-spec §4.1, §4.2, §4.7),
//! and the server version check.
//!
//! The parameter structs here are the only place that decides what goes on the wire: they carry
//! the wire names of the `*_kbps` fields and leave out every setting that is `None`.

use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};

use crate::client::Client;
use crate::error::RpcError;
use crate::session::{
    FreeSpace, SESSION_FIELDS, ServerVersion, SessionInfo, SessionSet, SessionStats,
};
use crate::transport::Transport;

/// The only fields `check_server` asks for: it must work on a server too new or too old to
/// answer the rest.
const VERSION_FIELDS: &[&str] = &["version", "rpc_version_semver"];

impl<T: Transport> Client<T> {
    /// Reads the session settings the app uses (`session_get` with [`SESSION_FIELDS`]).
    ///
    /// # Errors
    ///
    /// Any [`RpcError`] of the call; [`RpcError::InvalidResponse`] when a requested field is
    /// missing from the result or has the wrong type.
    pub fn session_get(&mut self) -> Result<SessionInfo, RpcError> {
        self.call(
            "session_get",
            Some(&GetParams {
                fields: SESSION_FIELDS,
            }),
        )
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
    /// [`RpcError::Server`] with code 5 and the system's message as its detail.
    pub fn free_space(&mut self, path: &str) -> Result<FreeSpace, RpcError> {
        self.call("free_space", Some(&FreeSpaceParams { path }))
    }

    /// Reads the server's version and checks that 4x4 supports its RPC version.
    ///
    /// # Errors
    ///
    /// Any [`RpcError`] of the call; [`RpcError::UnsupportedServer`] when the server's
    /// `rpc_version_semver` has a major below 6 (before Transmission 4.1) or is not a version,
    /// or when the server answers in the legacy protocol of Transmission 4.0 and older.
    pub fn check_server(&mut self) -> Result<ServerVersion, RpcError> {
        let result: VersionResult = self.call(
            "session_get",
            Some(&GetParams {
                fields: VERSION_FIELDS,
            }),
        )?;
        let version = ServerVersion {
            version: result.version,
            rpc_semver: result.rpc_version_semver,
        };
        if version.is_supported() {
            Ok(version)
        } else {
            Err(RpcError::UnsupportedServer {
                rpc_version: version.rpc_semver,
            })
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
