//! `free_space`, as Transmission 4.1 answers it (`freeSpace`, T/rpcimpl.cc): the room of a folder
//! that exists, and for one that does not, error 5 with the system's words and sizes of -1.
//!
//! The folders that exist are the session's download folder and those of the torrents. Every one
//! of them has the same room: it is one disk.

use serde_json::{Value, json};

use super::refusal::{PATH_NOT_ABSOLUTE, Refusal, SYSTEM_ERROR};
use super::{DOWNLOAD_DIR, INVALID_PARAMS, State};

/// The room on the disk: 800 GiB.
const FREE_BYTES: u64 = 800 << 30;
/// The capacity of the disk: 1 TiB.
const TOTAL_BYTES: u64 = 1 << 40;

/// `path` without the slashes at its end, except for the root's.
fn without_trailing_slashes(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() && path.starts_with('/') {
        "/"
    } else {
        trimmed
    }
}

impl State {
    /// The answer to `free_space` for the `path` in `params`, which comes back as asked.
    ///
    /// # Errors
    ///
    /// -32602 for no path (a path that is not text counts as none), 3 for one that is not
    /// absolute, and 5 for a folder that is none of the daemon's, with sizes of -1 under `data`.
    pub(super) fn free_space(&self, params: Option<&Value>) -> Result<Value, Refusal> {
        let Some(path) = params
            .and_then(|params| params.get("path"))
            .and_then(Value::as_str)
        else {
            return Err(Refusal::daemon(
                INVALID_PARAMS,
                Some("directory path argument is missing"),
                None,
            ));
        };
        if !path.starts_with('/') {
            return Err(Refusal::daemon(
                PATH_NOT_ABSOLUTE,
                Some("directory path is not absolute"),
                None,
            ));
        }
        if self.has_folder(path) {
            return Ok(json!({
                "path": path,
                "size_bytes": FREE_BYTES,
                "total_size": TOTAL_BYTES,
            }));
        }
        Err(Refusal::daemon(
            SYSTEM_ERROR,
            Some("No such file or directory"),
            Some(json!({ "path": path, "size_bytes": -1, "total_size": -1 })),
        ))
    }

    /// Whether `path` is the session's download folder or the folder of a torrent.
    fn has_folder(&self, path: &str) -> bool {
        let folder = without_trailing_slashes(path);
        folder == DOWNLOAD_DIR
            || self
                .torrents
                .iter()
                .any(|torrent| without_trailing_slashes(torrent.download_dir()) == folder)
    }
}
