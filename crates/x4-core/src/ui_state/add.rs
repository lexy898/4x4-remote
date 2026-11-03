//! What the add dialog remembers: the folders on the server that torrents were added to lately.

use serde::{Deserialize, Serialize};
use x4_rpc::DownloadDir;

/// The most folders the history keeps.
pub const MAX_RECENT_FOLDERS: usize = 10;

/// What the add dialog remembers between runs.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AddState {
    /// The folders on the server that torrents were added to, the newest first, which the folder
    /// field offers after the session's default (see [`folder_picks`](crate::folder_picks)).
    ///
    /// After [`UiState::parse`](super::UiState::parse) each is an absolute folder normalized as
    /// [`DownloadDir::new`] does, so at most 4096 bytes long, and listed once; there are at most
    /// [`MAX_RECENT_FOLDERS`]. Change it through [`remember`](Self::remember), which keeps it so.
    pub recent_folders: Vec<String>,
}

impl AddState {
    /// Puts `folder` first in the history, without listing it twice, and keeps the newest
    /// [`MAX_RECENT_FOLDERS`].
    ///
    /// Call it once a torrent was added to `folder`: a duplicate or a failure does not count. The
    /// session's default folder need not be remembered, since the picks start with it anyway: an
    /// add that names no folder ([`TorrentAdd::download_dir`](x4_rpc::TorrentAdd::download_dir)
    /// is `None`) went there.
    pub fn remember(&mut self, folder: &DownloadDir) {
        self.recent_folders.retain(|kept| kept != folder.as_str());
        self.recent_folders.insert(0, folder.as_str().to_owned());
        self.recent_folders.truncate(MAX_RECENT_FOLDERS);
    }

    /// The history as it can be used: each folder normalized, the ones the daemon could not use
    /// dropped, each folder once (the first counts), and at most [`MAX_RECENT_FOLDERS`].
    #[must_use]
    pub(super) fn normalized(self) -> Self {
        let mut recent_folders: Vec<String> = Vec::with_capacity(MAX_RECENT_FOLDERS);
        let mut unusable = 0_usize;
        for text in self.recent_folders {
            if recent_folders.len() == MAX_RECENT_FOLDERS {
                break;
            }
            match DownloadDir::new(&text) {
                Ok(folder) => {
                    if !recent_folders.iter().any(|kept| kept == folder.as_str()) {
                        recent_folders.push(folder.as_str().to_owned());
                    }
                }
                Err(_) => unusable += 1,
            }
        }
        if unusable > 0 {
            // The folders themselves are paths on the server, which stay out of the log.
            tracing::warn!(
                dropped = unusable,
                "folders the daemon cannot use in the state file's history, dropped"
            );
        }
        Self { recent_folders }
    }
}

#[cfg(test)]
mod tests;
