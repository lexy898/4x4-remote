//! The texts of the add dialog that carry values or depend on x4-core's decisions: its title with
//! the place in the queue, the line under the folder field, why there is no choice of files, why
//! Add is disabled, and the notifications of what became of a torrent to add.
//!
//! Every reason comes from a variant, never from an error's `Display` text, which is English and
//! for the log: x4-core's [`AddCheck`] and [`NoChoiceReason`], x4-rpc's [`DownloadDirError`] and
//! [`MagnetError`], a file's [`LoadError`] with x4-core's [`MetainfoError`] inside, the
//! [`AddError`] of an add and the [`RejectKind`] of a launch argument. Only the server's own text
//! of a refusal is shown as it came.

use gpui_kit::SharedString;
use x4_core::launch::{RejectKind, RejectedArg};
use x4_core::{
    AddCheck, AddError, FreeSpaceAnswer, MAX_TORRENT_FILE_BYTES, MetainfoError, NoChoiceReason,
    QUEUE_CAP, human_size,
};
use x4_rpc::{DownloadDirError, MAX_DOWNLOAD_DIR_BYTES, MAX_MAGNET_BYTES, MagnetError};

use super::{connection_reason_text, has_text, size_string, t};
use crate::adding::LoadError;

/// The dialog's title: "Add torrent", or with several in the run, which one it shows at
/// `position`, such as "Add torrent (2 of 5)".
pub(crate) fn add_title_text(position: Option<(usize, usize)>) -> SharedString {
    match position {
        Some((position, count)) if count > 1 => {
            t!("add-title-of", position = position, count = count)
        }
        _ => t!("add-title"),
    }
    .into()
}

/// The line under the folder field for the daemon's `answer` about it: its free space, a new
/// folder with the free space of the parent where it is known, or that it is not known.
pub(crate) fn free_space_line_text(answer: &FreeSpaceAnswer) -> SharedString {
    match answer {
        FreeSpaceAnswer::Free { bytes } => {
            t!("add-free-space", size = size_string(human_size(*bytes)))
        }
        FreeSpaceAnswer::Missing {
            parent: Some(parent),
            free: Some(free),
        } => t!(
            "add-new-folder-free",
            size = size_string(human_size(*free)),
            parent = parent.as_str()
        ),
        FreeSpaceAnswer::Missing { .. } => t!("add-new-folder"),
        FreeSpaceAnswer::Unknown => t!("add-free-space-unknown"),
    }
    .into()
}

/// What the dialog says instead of the list of files of a `.torrent` that has no choice.
pub(crate) fn no_choice_text(reason: NoChoiceReason) -> SharedString {
    match reason {
        NoChoiceReason::Ambiguous => t!("add-no-choice-ambiguous"),
        NoChoiceReason::TooLarge => t!("add-no-choice-too-large"),
    }
    .into()
}

/// Why Add is disabled, as `check` says; `None` when it may send.
pub(crate) fn add_reason_text(check: &AddCheck) -> Option<SharedString> {
    let text = match check {
        AddCheck::Ready(_) => return None,
        AddCheck::NothingWanted => t!("add-reason-nothing-wanted"),
        AddCheck::NotConnected => t!("add-reason-not-connected"),
        AddCheck::AlreadyInList(_) => t!("add-reason-in-list"),
        AddCheck::BadFolder(error) => folder_reason(*error),
        AddCheck::BadLink(error) => link_reason(*error),
    };
    Some(text.into())
}

fn folder_reason(error: DownloadDirError) -> String {
    match error {
        DownloadDirError::Empty => t!("add-reason-folder-empty"),
        DownloadDirError::NotAbsolute => t!("add-reason-folder-not-absolute"),
        DownloadDirError::ControlCharacter { .. } => t!("add-reason-folder-control"),
        DownloadDirError::DotPart => t!("add-reason-folder-dot"),
        DownloadDirError::TooLong { .. } => {
            t!("add-reason-folder-too-long", max = MAX_DOWNLOAD_DIR_BYTES)
        }
    }
}

fn link_reason(error: MagnetError) -> String {
    match error {
        MagnetError::Empty => t!("add-reason-link-empty"),
        MagnetError::TooLong { .. } => t!("add-reason-link-too-long", max = MAX_MAGNET_BYTES),
        MagnetError::ControlCharacter { .. } => t!("add-reason-link-control"),
        MagnetError::BareHash => t!("add-reason-link-bare-hash"),
        MagnetError::NotMagnet => t!("add-reason-link-not-magnet"),
        MagnetError::NoHash => t!("add-reason-link-no-hash"),
        MagnetError::V2Only => t!("add-reason-link-v2-only"),
        MagnetError::InvalidHash => t!("add-reason-link-bad-hash"),
        MagnetError::SeveralHashes { count } => t!("add-reason-link-several", count = count),
    }
}

/// What the notification of a `.torrent` file that cannot be offered says: the file's name
/// `file`, and why, from `error`.
pub(crate) fn load_failure_text(file: &str, error: &LoadError) -> SharedString {
    let reason = match error {
        LoadError::Missing => t!("add-file-missing"),
        LoadError::Denied => t!("add-file-denied"),
        LoadError::Unreadable(_) => t!("add-file-unreadable"),
        LoadError::NotAFile => t!("add-file-not-file"),
        LoadError::TooLarge { .. } | LoadError::Invalid(MetainfoError::TooLarge { .. }) => t!(
            "add-file-too-large",
            max = size_string(human_size(MAX_TORRENT_FILE_BYTES))
        ),
        LoadError::Invalid(MetainfoError::NotBencode { .. } | MetainfoError::TooDeep { .. }) => {
            t!("add-file-not-torrent")
        }
        LoadError::Invalid(MetainfoError::NoInfo) => t!("add-file-no-info"),
        LoadError::Invalid(MetainfoError::V2Only) => t!("add-file-v2-only"),
        LoadError::Invalid(MetainfoError::Invalid(_)) => t!("add-file-invalid"),
    };
    t!("add-file-failed", file = file, reason = reason).into()
}

/// The notification of a torrent the daemon had already, named `name`.
pub(crate) fn add_duplicate_text(name: &str) -> SharedString {
    t!("add-duplicate", name = name).into()
}

/// The notification of an add of `name` that the daemon did not confirm in time.
pub(crate) fn add_unconfirmed_text(name: &str) -> SharedString {
    t!("add-unconfirmed", name = name).into()
}

/// The notification of an add of `name` that failed with `error`. An add that is not confirmed
/// is no failure ([`add_unconfirmed_text`]); it gets the same words here all the same.
pub(crate) fn add_failure_text(name: &str, error: &AddError) -> SharedString {
    let reason: String = match error {
        AddError::Unconfirmed => return add_unconfirmed_text(name),
        AddError::NotConnected => t!("add-failed-not-connected"),
        AddError::Daemon(text) if has_text(text) => text.clone(),
        AddError::Daemon(_) => t!("connection-reason-server"),
        AddError::Failed(error) => connection_reason_text(error).to_string(),
    };
    t!("add-failed", name = name, reason = reason).into()
}

/// The notification of `count` items the queue had no room for.
pub(crate) fn queue_full_text(count: usize) -> SharedString {
    t!("add-queue-full", count = count, max = QUEUE_CAP).into()
}

/// The one notification of the launch arguments `rejected`, which names why the first is refused;
/// `None` when there are none.
pub(crate) fn rejected_text(rejected: &[RejectedArg]) -> Option<SharedString> {
    let first = rejected.first()?;
    let reason = match first.kind {
        RejectKind::Url => t!("add-rejected-url"),
        RejectKind::Option => t!("add-rejected-option"),
        RejectKind::BareHash => t!("add-rejected-bare-hash"),
        RejectKind::Empty => t!("add-rejected-empty"),
        RejectKind::NotUtf8Magnet => t!("add-rejected-not-text"),
        RejectKind::InvalidMagnet(_) => t!("add-rejected-magnet"),
        RejectKind::Other => t!("add-rejected-other"),
    };
    Some(t!("add-rejected", count = rejected.len(), reason = reason).into())
}

#[cfg(test)]
mod tests {
    use x4_core::ConnectionError;
    use x4_rpc::{DownloadDir, HashString};

    use super::*;

    #[test]
    fn the_title_shows_the_place_in_a_run_of_several() {
        assert_eq!(add_title_text(None), "Add torrent");
        assert_eq!(add_title_text(Some((1, 1))), "Add torrent");
        assert_eq!(add_title_text(Some((2, 5))), "Add torrent (2 of 5)");
    }

    #[test]
    fn the_folder_line_says_the_room_or_that_the_folder_is_new() {
        let data = DownloadDir::new("/data").unwrap();
        assert_eq!(
            free_space_line_text(&FreeSpaceAnswer::Free { bytes: 3 << 30 }),
            "3.0 GiB free"
        );
        assert_eq!(
            free_space_line_text(&FreeSpaceAnswer::Missing {
                parent: Some(data.clone()),
                free: Some(512 << 20),
            }),
            "New folder; 512 MiB free in /data"
        );
        for missing in [
            FreeSpaceAnswer::Missing {
                parent: Some(data),
                free: None,
            },
            FreeSpaceAnswer::Missing {
                parent: None,
                free: None,
            },
        ] {
            assert_eq!(free_space_line_text(&missing), "New folder");
        }
        assert_eq!(
            free_space_line_text(&FreeSpaceAnswer::Unknown),
            "Free space unknown"
        );
    }

    #[test]
    fn each_reason_without_a_choice_has_its_own_line() {
        assert_eq!(
            no_choice_text(NoChoiceReason::Ambiguous),
            "All files are added: the list of files could be read in more than one way"
        );
        assert_eq!(
            no_choice_text(NoChoiceReason::TooLarge),
            "All files are added: there are too many to choose from"
        );
    }

    #[test]
    fn each_check_that_holds_add_back_has_a_reason() {
        let hash = HashString::parse(&"a".repeat(40)).unwrap();
        for (check, reason) in [
            (AddCheck::NothingWanted, "Choose at least one file"),
            (AddCheck::NotConnected, "Not connected to the server"),
            (
                AddCheck::AlreadyInList(hash),
                "This torrent is in the list already",
            ),
            (
                AddCheck::BadFolder(DownloadDirError::Empty),
                "Name a folder on the server",
            ),
            (
                AddCheck::BadFolder(DownloadDirError::NotAbsolute),
                "The folder must be a full path on the server, from /",
            ),
            (
                AddCheck::BadFolder(DownloadDirError::ControlCharacter { offset: 3 }),
                "The folder has a line break or another control character",
            ),
            (
                AddCheck::BadFolder(DownloadDirError::DotPart),
                "The folder cannot have . or .. in its path",
            ),
            (
                AddCheck::BadFolder(DownloadDirError::TooLong { len: 5000 }),
                "The folder is longer than 4096 bytes",
            ),
            (AddCheck::BadLink(MagnetError::Empty), "Paste a magnet link"),
            (
                AddCheck::BadLink(MagnetError::TooLong { len: 70_000 }),
                "The magnet link is longer than 65536 bytes",
            ),
            (
                AddCheck::BadLink(MagnetError::ControlCharacter { offset: 1 }),
                "The magnet link has a line break or another control character",
            ),
            (
                AddCheck::BadLink(MagnetError::BareHash),
                "A bare info hash is no link: write it as magnet:?xt=urn:btih:…",
            ),
            (
                AddCheck::BadLink(MagnetError::NotMagnet),
                "This is not a magnet link: it must start with magnet:?",
            ),
            (
                AddCheck::BadLink(MagnetError::NoHash),
                "The magnet link has no BitTorrent info hash",
            ),
            (
                AddCheck::BadLink(MagnetError::V2Only),
                "The magnet link has only a BitTorrent v2 info hash, which the server cannot add",
            ),
            (
                AddCheck::BadLink(MagnetError::InvalidHash),
                "The info hash of the magnet link is not valid",
            ),
            (
                AddCheck::BadLink(MagnetError::SeveralHashes { count: 2 }),
                "The magnet link has 2 info hashes, and may have only one",
            ),
        ] {
            assert_eq!(
                add_reason_text(&check).as_deref(),
                Some(reason),
                "{check:?}"
            );
        }
    }

    #[test]
    fn a_file_that_cannot_be_offered_is_named_with_its_reason() {
        for (error, reason) in [
            (LoadError::Missing, "it does not exist"),
            (LoadError::Denied, "4x4 may not read it"),
            (
                LoadError::Unreadable(std::io::ErrorKind::Interrupted),
                "it could not be read",
            ),
            (LoadError::NotAFile, "it is not a file"),
            (
                LoadError::TooLarge { size: 1 << 30 },
                "it is larger than 16.0 MiB",
            ),
            (
                LoadError::Invalid(MetainfoError::TooLarge { size: 1 << 30 }),
                "it is larger than 16.0 MiB",
            ),
            (
                LoadError::Invalid(MetainfoError::NotBencode { offset: 0 }),
                "it is not a torrent file",
            ),
            (
                LoadError::Invalid(MetainfoError::TooDeep { offset: 40 }),
                "it is not a torrent file",
            ),
            (
                LoadError::Invalid(MetainfoError::NoInfo),
                "it holds no torrent",
            ),
            (
                LoadError::Invalid(MetainfoError::V2Only),
                "it is a BitTorrent v2 torrent, which the server cannot add",
            ),
            (
                LoadError::Invalid(MetainfoError::Invalid("the torrent has no piece hashes")),
                "the server would refuse it as broken",
            ),
        ] {
            assert_eq!(
                load_failure_text("a.torrent", &error),
                format!("Could not open a.torrent: {reason}"),
                "{error:?}"
            );
        }
    }

    #[test]
    fn what_became_of_an_add_names_the_torrent() {
        assert_eq!(add_duplicate_text("Show"), "Show is in the list already");
        assert_eq!(
            add_unconfirmed_text("Show"),
            "The server has not confirmed Show yet; it shows in the list once it is added"
        );
        assert_eq!(
            add_failure_text("Show", &AddError::Unconfirmed),
            add_unconfirmed_text("Show")
        );
        assert_eq!(
            add_failure_text("Show", &AddError::NotConnected),
            "Could not add Show: the server is not connected, so nothing was sent"
        );
        assert_eq!(
            add_failure_text("Show", &AddError::Daemon("unrecognized info".to_owned())),
            "Could not add Show: unrecognized info"
        );
        assert_eq!(
            add_failure_text("Show", &AddError::Daemon(" ".to_owned())),
            format!(
                "Could not add Show: {}",
                connection_reason_text(&ConnectionError::Server(String::new()))
            )
        );
        assert_eq!(
            add_failure_text("Show", &AddError::Failed(ConnectionError::Timeout)),
            format!(
                "Could not add Show: {}",
                connection_reason_text(&ConnectionError::Timeout)
            )
        );
    }

    #[test]
    fn a_full_queue_and_refused_arguments_are_counted() {
        assert_eq!(
            queue_full_text(1),
            "1 more torrent was left out: at most 100 wait to be added"
        );
        assert_eq!(
            queue_full_text(5),
            "5 more torrents were left out: at most 100 wait to be added"
        );
        assert_eq!(rejected_text(&[]), None);
        let rejected = |kind| RejectedArg { kind };
        for (kind, reason) in [
            (
                RejectKind::Url,
                "a web address, and only magnet links and .torrent files can be added",
            ),
            (RejectKind::Option, "an option, and 4x4 has none"),
            (
                RejectKind::BareHash,
                "a bare info hash, which must be given as a magnet link",
            ),
            (RejectKind::Empty, "empty"),
            (RejectKind::NotUtf8Magnet, "a magnet link that is not text"),
            (
                RejectKind::InvalidMagnet(MagnetError::NoHash),
                "a magnet link the server cannot add",
            ),
            (RejectKind::Other, "no name of a file"),
        ] {
            assert_eq!(
                rejected_text(&[rejected(kind)]).as_deref(),
                Some(format!("Could not open the item 4x4 was given: it is {reason}").as_str())
            );
        }
        assert_eq!(
            rejected_text(&[rejected(RejectKind::Option), rejected(RejectKind::Url)]).as_deref(),
            Some(
                "Could not open 2 of the items 4x4 was given; the first is an option, and 4x4 has none"
            )
        );
    }
}
