//! What the window does once the daemon has answered an add: which folder joins the folders used
//! lately, which torrent to select, whether the dialog moves on or keeps the form, and what the
//! notification says.

use x4_rpc::{DownloadDir, HashString};

use crate::action::{AddError, Added};

/// An add sent to the daemon, as the window keeps it until the answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentAdd {
    /// The torrent's info hash, known before the daemon answers.
    pub hash: HashString,
    /// What the dialog called the torrent.
    pub name: String,
    /// The folder the request named; `None` for the session's own folder.
    pub download_dir: Option<DownloadDir>,
}

/// What the add dialog does with the form of a torrent whose add was answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormStep {
    /// The torrent is done with: the dialog shows the next one, or closes.
    Advance,
    /// The add failed: the form stays as it was, to be sent again.
    Keep,
}

/// What a notification says about an answered add.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddNotice {
    /// The daemon had the torrent `name` already, and changed nothing.
    Duplicate {
        /// The torrent's name.
        name: String,
    },
    /// The daemon has not confirmed the add of `name` in time; it may still come.
    Unconfirmed {
        /// The torrent's name.
        name: String,
    },
    /// The add of `name` failed with `error`.
    Failed {
        /// The torrent's name.
        name: String,
        /// Why.
        error: AddError,
    },
}

/// What the window does once the daemon has answered an add.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddOutcome {
    /// The folder to remember among the folders used lately
    /// ([`AddState::remember`](crate::AddState::remember)): the very folder the request named,
    /// and only after a real add.
    pub remember: Option<DownloadDir>,
    /// The torrent to select once it is listed.
    pub select: Option<HashString>,
    /// What the dialog does with the form, if it still shows the torrent.
    pub form: FormStep,
    /// What the notification says, if one is due.
    pub notice: Option<AddNotice>,
}

/// What follows the daemon's answer `result` to `sent`:
/// - added: the folder sent is remembered, the torrent selected, and the dialog moves on;
/// - a torrent the daemon had already: nothing was applied, so no folder is remembered; it is
///   selected, with a notification, and the dialog moves on;
/// - not confirmed in time: the torrent is selected if it shows up, with a notification, and the
///   dialog moves on;
/// - a failure: a notification says why, and the form stays.
#[must_use]
pub fn add_outcome(sent: &SentAdd, result: Result<Added, AddError>) -> AddOutcome {
    match result {
        Ok(added) if added.duplicate => AddOutcome {
            remember: None,
            notice: Some(AddNotice::Duplicate {
                name: name_of(sent, added.name),
            }),
            select: Some(added.hash),
            form: FormStep::Advance,
        },
        Ok(added) => AddOutcome {
            remember: sent.download_dir.clone(),
            select: Some(added.hash),
            form: FormStep::Advance,
            notice: None,
        },
        Err(AddError::Unconfirmed) => AddOutcome {
            remember: None,
            select: Some(sent.hash.clone()),
            form: FormStep::Advance,
            notice: Some(AddNotice::Unconfirmed {
                name: sent.name.clone(),
            }),
        },
        Err(error) => AddOutcome {
            remember: None,
            select: None,
            form: FormStep::Keep,
            notice: Some(AddNotice::Failed {
                name: sent.name.clone(),
                error,
            }),
        },
    }
}

/// The name the daemon gave the torrent, or what the dialog called it when the daemon gave none.
fn name_of(sent: &SentAdd, daemon_name: String) -> String {
    if daemon_name.is_empty() {
        sent.name.clone()
    } else {
        daemon_name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::ConnectionError;
    use crate::test_support::torrent;

    fn sent(download_dir: Option<&str>) -> SentAdd {
        SentAdd {
            hash: torrent("Show").hash,
            name: "Show".to_owned(),
            download_dir: download_dir.map(|dir| DownloadDir::new(dir).unwrap()),
        }
    }

    fn added(name: &str, duplicate: bool) -> Added {
        Added {
            hash: torrent("Show").hash,
            name: name.to_owned(),
            duplicate,
        }
    }

    #[test]
    fn an_add_remembers_the_folder_it_sent_and_moves_on() {
        let sent = sent(Some("/data/shows"));

        assert_eq!(
            add_outcome(&sent, Ok(added("Show", false))),
            AddOutcome {
                remember: Some(DownloadDir::new("/data/shows").unwrap()),
                select: Some(sent.hash.clone()),
                form: FormStep::Advance,
                notice: None,
            }
        );
        assert_eq!(
            add_outcome(&self::sent(None), Ok(added("Show", false))).remember,
            None,
            "the session's own folder is not named, so not remembered"
        );
    }

    #[test]
    fn a_duplicate_remembers_nothing_and_is_named_as_the_daemon_names_it() {
        let sent = sent(Some("/data/shows"));

        assert_eq!(
            add_outcome(&sent, Ok(added("Show (daemon)", true))),
            AddOutcome {
                remember: None,
                select: Some(sent.hash.clone()),
                form: FormStep::Advance,
                notice: Some(AddNotice::Duplicate {
                    name: "Show (daemon)".to_owned(),
                }),
            }
        );
        assert_eq!(
            add_outcome(&sent, Ok(added("", true))).notice,
            Some(AddNotice::Duplicate {
                name: "Show".to_owned(),
            }),
            "a nameless answer keeps the dialog's name"
        );
    }

    #[test]
    fn an_unconfirmed_add_still_waits_for_the_torrent_and_moves_on() {
        let sent = sent(Some("/data/shows"));

        assert_eq!(
            add_outcome(&sent, Err(AddError::Unconfirmed)),
            AddOutcome {
                remember: None,
                select: Some(sent.hash.clone()),
                form: FormStep::Advance,
                notice: Some(AddNotice::Unconfirmed {
                    name: "Show".to_owned(),
                }),
            }
        );
    }

    #[test]
    fn a_failure_keeps_the_form_and_says_why() {
        let sent = sent(Some("/data/shows"));

        for error in [
            AddError::NotConnected,
            AddError::Daemon("unrecognized info".to_owned()),
            AddError::Failed(ConnectionError::Timeout),
        ] {
            assert_eq!(
                add_outcome(&sent, Err(error.clone())),
                AddOutcome {
                    remember: None,
                    select: None,
                    form: FormStep::Keep,
                    notice: Some(AddNotice::Failed {
                        name: "Show".to_owned(),
                        error,
                    }),
                }
            );
        }
    }
}
