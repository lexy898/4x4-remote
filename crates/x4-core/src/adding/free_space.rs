//! When the add dialog asks the daemon for the free space of the folder in its field, and what it
//! keeps of the answers.
//!
//! The question walks up the folder on the server, a few calls on a small server's disk, so it is
//! asked sparingly: [`FREE_SPACE_DELAY`] after the last edit, only for a folder the daemon could
//! use, and only when nothing is known about it yet. Every answer is kept for its folder while the
//! dialog is open, so going back to a folder asks nothing again; an answer for a folder the field
//! has left is dropped. An unknown answer is forgotten when the daemon comes back, and asked for
//! again at once; nothing else asks before the delay. The session's folder starts with the free
//! space every snapshot carries.

use std::collections::HashMap;
use std::time::Duration;

use x4_rpc::DownloadDir;

use crate::engine::FreeSpaceAnswer;
use crate::snapshot::Snapshot;

/// How long after the last edit of the folder its free space is asked for.
pub const FREE_SPACE_DELAY: Duration = Duration::from_millis(500);

/// What a snapshot changed in a [`FreeSpaceCache`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FreeSpaceFollow {
    /// Whether anything known changed, so that the line under the field may say something else.
    pub changed: bool,
    /// Whether answers that were unknown were forgotten, as when the daemon is back: the folder in
    /// the field is asked about again at once. Nothing else cuts the delay after an edit short.
    pub ask_again: bool,
}

/// What the add dialog knows of the free space of the folders its field has shown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FreeSpaceCache {
    /// What the daemon said about each folder asked about, and what the snapshots say of the
    /// session's folder.
    answers: HashMap<DownloadDir, FreeSpaceAnswer>,
    /// Whether the daemon was connected at the last snapshot.
    connected: bool,
}

impl FreeSpaceCache {
    /// A cache that knows nothing yet, for a daemon `connected` or not.
    #[must_use]
    pub fn new(connected: bool) -> Self {
        Self {
            answers: HashMap::new(),
            connected,
        }
    }

    /// What is known of the free space of `folder`.
    #[must_use]
    pub fn answer(&self, folder: &DownloadDir) -> Option<&FreeSpaceAnswer> {
        self.answers.get(folder)
    }

    /// The folder to ask about for a field that shows `folder` (`None` for a folder the daemon
    /// could not use): the folder itself, unless something is known of it.
    #[must_use]
    pub fn question(&self, folder: Option<&DownloadDir>) -> Option<DownloadDir> {
        folder
            .filter(|folder| !self.answers.contains_key(*folder))
            .cloned()
    }

    /// Keeps `answer` about `folder` if the field shows that folder (`current`); returns whether
    /// it was kept.
    pub fn answered(
        &mut self,
        current: Option<&DownloadDir>,
        folder: DownloadDir,
        answer: FreeSpaceAnswer,
    ) -> bool {
        if current != Some(&folder) {
            return false;
        }
        self.answers.insert(folder, answer);
        true
    }

    /// Takes `snapshot`, with the daemon `connected` or not: keeps the free space it has for the
    /// session's folder, and forgets the unknown answers when the daemon is back, so that they
    /// are asked for again. Says what changed.
    pub fn follow(&mut self, snapshot: &Snapshot, connected: bool) -> FreeSpaceFollow {
        let mut followed = FreeSpaceFollow::default();
        if connected && !self.connected {
            let before = self.answers.len();
            self.answers
                .retain(|_, answer| *answer != FreeSpaceAnswer::Unknown);
            followed.ask_again = self.answers.len() != before;
            followed.changed = followed.ask_again;
        }
        self.connected = connected;
        if let (Some(session), Some(bytes)) = (&snapshot.session, snapshot.free_space_bytes)
            && let Ok(folder) = DownloadDir::new(&session.download_dir)
        {
            let answer = FreeSpaceAnswer::Free { bytes };
            followed.changed |= self.answers.get(&folder) != Some(&answer);
            self.answers.insert(folder, answer);
        }
        followed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KbPerSecond;
    use crate::snapshot::{ConnectionState, SessionSummary, SpeedLimit};

    fn dir(path: &str) -> DownloadDir {
        DownloadDir::new(path).unwrap()
    }

    fn snapshot(free: Option<u64>) -> Snapshot {
        let off = SpeedLimit {
            kbps: KbPerSecond(0),
            enabled: false,
        };
        Snapshot {
            torrents: Vec::new(),
            listed: true,
            session: Some(SessionSummary {
                version: "4.1.4".to_owned(),
                download_dir: "/data/torrents/".to_owned(),
                alt_speed_enabled: false,
                speed_limit_down: off,
                speed_limit_up: off,
                alt_speed_down_kbps: KbPerSecond(0),
                alt_speed_up_kbps: KbPerSecond(0),
                start_added_torrents: true,
                sequential_download_supported: true,
            }),
            free_space_bytes: free,
            rate_down: 0,
            rate_up: 0,
            connection: ConnectionState::Connected,
            stale: false,
        }
    }

    #[test]
    fn a_folder_is_asked_about_once_and_only_while_nothing_is_known() {
        let mut cache = FreeSpaceCache::new(true);
        let new = dir("/data/new");

        assert_eq!(
            cache.question(None),
            None,
            "not a folder the daemon could use"
        );
        assert_eq!(cache.question(Some(&new)), Some(new.clone()));
        assert!(cache.answered(Some(&new), new.clone(), FreeSpaceAnswer::Unknown));
        assert_eq!(
            cache.question(Some(&new)),
            None,
            "an unknown answer is an answer"
        );
        assert_eq!(cache.answer(&new), Some(&FreeSpaceAnswer::Unknown));
    }

    #[test]
    fn an_answer_for_a_folder_the_field_has_left_is_dropped() {
        let mut cache = FreeSpaceCache::new(true);
        let old = dir("/data/old");
        let new = dir("/data/new");

        assert!(!cache.answered(Some(&new), old.clone(), FreeSpaceAnswer::Free { bytes: 1 }));
        assert!(!cache.answered(None, old.clone(), FreeSpaceAnswer::Free { bytes: 1 }));
        assert_eq!(cache.answer(&old), None);
        assert_eq!(cache.question(Some(&old)), Some(old));
    }

    #[test]
    fn the_session_folder_is_known_from_the_snapshots() {
        let mut cache = FreeSpaceCache::new(true);
        let session = dir("/data/torrents");

        let shown = FreeSpaceFollow {
            changed: true,
            ask_again: false,
        };
        assert_eq!(cache.follow(&snapshot(Some(800)), true), shown);
        assert_eq!(
            cache.answer(&session),
            Some(&FreeSpaceAnswer::Free { bytes: 800 })
        );
        assert_eq!(cache.question(Some(&session)), None);
        assert_eq!(
            cache.follow(&snapshot(Some(800)), true),
            FreeSpaceFollow::default(),
            "nothing new"
        );
        assert_eq!(
            cache.follow(&snapshot(Some(700)), true),
            shown,
            "fresher, and nothing to ask again"
        );
        assert_eq!(
            cache.answer(&session),
            Some(&FreeSpaceAnswer::Free { bytes: 700 })
        );
        assert_eq!(
            cache.follow(&snapshot(None), true),
            FreeSpaceFollow::default(),
            "a snapshot that does not know keeps it"
        );
    }

    #[test]
    fn unknown_answers_are_asked_again_once_the_daemon_is_back() {
        let mut cache = FreeSpaceCache::new(true);
        let srv = dir("/srv");
        let new = dir("/data/new");
        let missing = FreeSpaceAnswer::Missing {
            parent: Some(dir("/data")),
            free: Some(3),
        };
        assert!(cache.answered(Some(&srv), srv.clone(), FreeSpaceAnswer::Unknown));
        assert!(cache.answered(Some(&new), new.clone(), missing.clone()));

        assert_eq!(
            cache.follow(&snapshot(None), false),
            FreeSpaceFollow::default(),
            "away: nothing forgotten"
        );
        assert_eq!(cache.question(Some(&srv)), None);
        assert_eq!(
            cache.follow(&snapshot(None), true),
            FreeSpaceFollow {
                changed: true,
                ask_again: true,
            },
            "back"
        );

        assert_eq!(cache.question(Some(&srv)), Some(srv));
        assert_eq!(cache.answer(&new), Some(&missing), "what is known stays");
        assert_eq!(
            cache.follow(&snapshot(None), true),
            FreeSpaceFollow::default(),
            "only once"
        );
    }

    #[test]
    fn a_daemon_back_with_nothing_unknown_asks_nothing_again() {
        let mut cache = FreeSpaceCache::new(true);
        let new = dir("/data/new");
        assert!(cache.answered(Some(&new), new.clone(), FreeSpaceAnswer::Free { bytes: 1 }));
        cache.follow(&snapshot(Some(800)), true);
        cache.follow(&snapshot(Some(800)), false);

        let back = cache.follow(&snapshot(Some(700)), true);

        assert_eq!(
            back,
            FreeSpaceFollow {
                changed: true,
                ask_again: false,
            },
            "the session's folder has news, and nothing was forgotten"
        );
    }
}
