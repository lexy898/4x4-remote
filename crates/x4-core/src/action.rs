//! What the user asks the daemon to do, and why it may not happen.

use x4_rpc::{FilePriority, HashString, RemoveData, TorrentAdd};

use crate::limits::KbPerSecond;
use crate::snapshot::ConnectionError;

/// Which way data flows; each of the normal speed limits applies to one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Downloading.
    Down,
    /// Uploading.
    Up,
}

/// Names one [`Action`], so that its [`Event::ActionFinished`](crate::Event::ActionFinished) can
/// be matched to it. The caller picks the numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActionId(pub u64);

/// A change the user asks for.
///
/// The torrents are named by hash. An empty list of hashes or of file indices is refused with
/// [`ActionError::EmptySelection`] before anything is sent: the daemon reads an omitted `ids` as
/// every torrent and an empty file list as every file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Start the torrents, or queue them if the download queue is full.
    Start(Vec<HashString>),
    /// Start the torrents now, ignoring the queue.
    StartNow(Vec<HashString>),
    /// Stop the torrents.
    Stop(Vec<HashString>),
    /// Check the torrents' local data against their piece hashes.
    Verify(Vec<HashString>),
    /// Announce the torrents to their trackers now.
    Reannounce(Vec<HashString>),
    /// Remove the torrents from the daemon, and their data too if `data` says so.
    Remove {
        /// The torrents to remove.
        hashes: Vec<HashString>,
        /// Whether the downloaded files stay on the server's disk.
        data: RemoveData,
    },
    /// Switch downloading the pieces of one torrent in order on or off.
    SetSequential {
        /// The torrent.
        hash: HashString,
        /// Whether pieces are downloaded in order.
        enabled: bool,
    },
    /// Give files of one torrent a download priority.
    SetFilePriority {
        /// The torrent.
        hash: HashString,
        /// Indices into the torrent's file list.
        files: Vec<usize>,
        /// The new priority.
        priority: FilePriority,
    },
    /// Choose whether files of one torrent are downloaded.
    SetFilesWanted {
        /// The torrent.
        hash: HashString,
        /// Indices into the torrent's file list.
        files: Vec<usize>,
        /// Whether the files are downloaded.
        wanted: bool,
    },
    /// Switch the alternative speed limits on or off.
    SetAltSpeed(bool),
    /// Set the normal speed limit of one direction, or turn it off.
    ///
    /// Only that direction's settings go out, so a value the UI read a while ago never
    /// overwrites the other direction. Turning a limit off keeps its stored value, which
    /// [`SpeedLimit`](crate::SpeedLimit) shows.
    SetSpeedLimit {
        /// The limit to change.
        direction: Direction,
        /// The limit, switched on; `None` switches it off.
        limit: Option<KbPerSecond>,
    },
    /// Add a torrent.
    Add(TorrentAdd),
}

/// Why an [`Action`] did not happen.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActionError {
    /// The daemon is not connected, so the action was not sent; it is not queued either.
    #[error("the daemon is not connected; the action was not sent")]
    NotConnected,
    /// The action named no torrents or no files, so it was not sent: the daemon would apply it
    /// to all of them.
    #[error("the action names no torrents or no files; it was not sent")]
    EmptySelection,
    /// The daemon did not carry the action out; holds why.
    #[error("the daemon did not carry out the action: {0}")]
    Rpc(ConnectionError),
}

impl Action {
    /// Whether the action names no torrent or no file, which the daemon would read as all.
    ///
    /// No wildcard: a new action has to decide whether it can be empty.
    pub(crate) fn selects_nothing(&self) -> bool {
        match self {
            Self::Start(hashes)
            | Self::StartNow(hashes)
            | Self::Stop(hashes)
            | Self::Verify(hashes)
            | Self::Reannounce(hashes)
            | Self::Remove { hashes, .. } => hashes.is_empty(),
            Self::SetFilePriority { files, .. } | Self::SetFilesWanted { files, .. } => {
                files.is_empty()
            }
            Self::SetSequential { .. }
            | Self::SetAltSpeed(_)
            | Self::SetSpeedLimit { .. }
            | Self::Add(_) => false,
        }
    }

    /// Whether the action changes the session settings, which are polled only once a minute.
    pub(crate) fn changes_session(&self) -> bool {
        match self {
            Self::SetAltSpeed(_) | Self::SetSpeedLimit { .. } => true,
            Self::Start(_)
            | Self::StartNow(_)
            | Self::Stop(_)
            | Self::Verify(_)
            | Self::Reannounce(_)
            | Self::Remove { .. }
            | Self::SetSequential { .. }
            | Self::SetFilePriority { .. }
            | Self::SetFilesWanted { .. }
            | Self::Add(_) => false,
        }
    }

    /// A short name for logs; it carries no torrent names or paths.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Self::Start(_) => "start",
            Self::StartNow(_) => "start now",
            Self::Stop(_) => "stop",
            Self::Verify(_) => "verify",
            Self::Reannounce(_) => "reannounce",
            Self::Remove { .. } => "remove",
            Self::SetSequential { .. } => "set sequential",
            Self::SetFilePriority { .. } => "set file priority",
            Self::SetFilesWanted { .. } => "set files wanted",
            Self::SetAltSpeed(_) => "set alternative speed",
            Self::SetSpeedLimit { .. } => "set speed limit",
            Self::Add(_) => "add",
        }
    }
}

#[cfg(test)]
mod tests {
    use x4_rpc::AddSource;

    use super::*;

    use crate::test_support::torrent;

    fn add() -> TorrentAdd {
        TorrentAdd {
            source: AddSource::Magnet("magnet:?xt=urn:btih:example".to_owned()),
            download_dir: None,
            paused: false,
            files_unwanted: Vec::new(),
            priority_high: Vec::new(),
            priority_low: Vec::new(),
            sequential_download: None,
        }
    }

    #[test]
    fn an_empty_hash_list_selects_nothing() {
        let hash = || vec![torrent("a").hash];
        let lists: [fn(Vec<HashString>) -> Action; 5] = [
            Action::Start,
            Action::StartNow,
            Action::Stop,
            Action::Verify,
            Action::Reannounce,
        ];

        for action in lists {
            assert!(action(Vec::new()).selects_nothing());
            assert!(!action(hash()).selects_nothing());
        }
        let remove = |hashes| Action::Remove {
            hashes,
            data: RemoveData::Delete,
        };
        assert!(remove(Vec::new()).selects_nothing());
        assert!(!remove(hash()).selects_nothing());
    }

    #[test]
    fn an_empty_file_list_selects_nothing() {
        let hash = torrent("a").hash;
        let priority = |files| Action::SetFilePriority {
            hash: hash.clone(),
            files,
            priority: FilePriority::High,
        };
        let wanted = |files| Action::SetFilesWanted {
            hash: hash.clone(),
            files,
            wanted: false,
        };

        assert!(priority(Vec::new()).selects_nothing());
        assert!(wanted(Vec::new()).selects_nothing());
        assert!(!priority(vec![0]).selects_nothing());
        assert!(!wanted(vec![3]).selects_nothing());
    }

    #[test]
    fn actions_without_a_list_always_select_something() {
        for action in [
            Action::SetSequential {
                hash: torrent("a").hash,
                enabled: true,
            },
            Action::SetAltSpeed(false),
            Action::SetSpeedLimit {
                direction: Direction::Down,
                limit: None,
            },
            Action::SetSpeedLimit {
                direction: Direction::Up,
                limit: Some(KbPerSecond(100)),
            },
            Action::Add(add()),
        ] {
            assert!(!action.selects_nothing(), "{action:?}");
        }
    }

    #[test]
    fn only_speed_actions_change_the_session() {
        assert!(Action::SetAltSpeed(true).changes_session());
        for direction in [Direction::Down, Direction::Up] {
            for limit in [Some(KbPerSecond(100)), None] {
                let action = Action::SetSpeedLimit { direction, limit };
                assert!(action.changes_session(), "{action:?}");
            }
        }
        assert!(!Action::Stop(vec![torrent("a").hash]).changes_session());
        assert!(!Action::Add(add()).changes_session());
    }
}
