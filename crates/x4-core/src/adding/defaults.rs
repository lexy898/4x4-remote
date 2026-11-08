//! The folder and "Start immediately" the add dialog gives each torrent of a run: the session's at
//! first, then what the user chose, carried on to the next torrent.

use crate::snapshot::Snapshot;

/// The folder and "Start immediately" of a run of the add dialog, from its first torrent to its
/// last.
///
/// They start as the session's: its download folder and its `start_added_torrents`. A dialog that
/// opens before the session is known takes them once it is, but never over what the user has set
/// meanwhile: a folder typed stays, and so does a click on "Start immediately". The next torrent
/// of the run gets the folder and the start the user chose for the one before.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormDefaults {
    /// The folder as it stands: the session's, or what the user typed.
    folder: String,
    /// Whether the user has edited the folder, which the session's then leaves alone.
    folder_edited: bool,
    /// What the user made of "Start immediately".
    chosen_start: Option<bool>,
    /// The session's `start_added_torrents`, once the session has been taken.
    session_start: Option<bool>,
}

/// What taking the session changed, for the dialog to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TakenDefaults {
    /// The session's folder, for a field the user has not edited; `None` to leave the field.
    pub folder: Option<String>,
    /// Whether "Start immediately" is checked now.
    pub start: bool,
}

impl FormDefaults {
    /// Takes the session's folder and "Start immediately" from `snapshot`, the first time it
    /// knows the session; `None` before that, and every time after.
    pub fn take_session(&mut self, snapshot: &Snapshot) -> Option<TakenDefaults> {
        if self.session_start.is_some() {
            return None;
        }
        let session = snapshot.session.as_ref()?;
        self.session_start = Some(session.start_added_torrents);
        let folder = (!self.folder_edited).then(|| {
            self.folder.clone_from(&session.download_dir);
            session.download_dir.clone()
        });
        Some(TakenDefaults {
            folder,
            start: self.start(),
        })
    }

    /// Takes the folder `text` the user typed or picked, for this torrent and the next ones.
    pub fn set_folder(&mut self, text: String) {
        self.folder = text;
        self.folder_edited = true;
    }

    /// Takes the user's click on "Start immediately", for this torrent and the next ones.
    pub fn choose_start(&mut self, start: bool) {
        self.chosen_start = Some(start);
    }

    /// The folder for the next form.
    #[must_use]
    pub fn folder(&self) -> &str {
        &self.folder
    }

    /// Whether "Start immediately" is checked: as the user chose, or as the session starts added
    /// torrents; before the session is known, as the daemon does by default.
    #[must_use]
    pub fn start(&self) -> bool {
        self.chosen_start.or(self.session_start).unwrap_or(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KbPerSecond;
    use crate::snapshot::{ConnectionState, SessionSummary, SpeedLimit};

    fn snapshot(start_added_torrents: Option<bool>) -> Snapshot {
        let off = SpeedLimit {
            kbps: KbPerSecond(0),
            enabled: false,
        };
        Snapshot {
            torrents: Vec::new(),
            listed: true,
            session: start_added_torrents.map(|start_added_torrents| SessionSummary {
                version: "4.1.4".to_owned(),
                download_dir: "/data/torrents".to_owned(),
                alt_speed_enabled: false,
                speed_limit_down: off,
                speed_limit_up: off,
                alt_speed_down_kbps: KbPerSecond(0),
                alt_speed_up_kbps: KbPerSecond(0),
                start_added_torrents,
                sequential_download_supported: true,
                lists_empty_files: true,
            }),
            free_space_bytes: None,
            rate_down: 0,
            rate_up: 0,
            connection: ConnectionState::Connected,
            stale: false,
        }
    }

    #[test]
    fn the_session_gives_the_folder_and_start_once() {
        let mut defaults = FormDefaults::default();
        assert_eq!(
            (defaults.folder(), defaults.start()),
            ("", true),
            "before the session"
        );
        assert_eq!(
            defaults.take_session(&snapshot(None)),
            None,
            "no session yet"
        );

        assert_eq!(
            defaults.take_session(&snapshot(Some(false))),
            Some(TakenDefaults {
                folder: Some("/data/torrents".to_owned()),
                start: false,
            })
        );
        assert_eq!(
            (defaults.folder(), defaults.start()),
            ("/data/torrents", false)
        );
        assert_eq!(
            defaults.take_session(&snapshot(Some(true))),
            None,
            "only once"
        );
        assert!(!defaults.start());
    }

    #[test]
    fn what_the_user_set_before_the_session_stands() {
        let mut defaults = FormDefaults::default();
        defaults.set_folder("/data/mine".to_owned());
        defaults.choose_start(true);

        assert_eq!(
            defaults.take_session(&snapshot(Some(false))),
            Some(TakenDefaults {
                folder: None,
                start: true,
            })
        );
        assert_eq!((defaults.folder(), defaults.start()), ("/data/mine", true));
    }

    #[test]
    fn the_next_torrent_gets_what_the_user_chose() {
        let mut defaults = FormDefaults::default();
        defaults.take_session(&snapshot(Some(true)));

        defaults.set_folder("/data/shows".to_owned());
        defaults.choose_start(false);

        assert_eq!(
            (defaults.folder(), defaults.start()),
            ("/data/shows", false)
        );
    }
}
