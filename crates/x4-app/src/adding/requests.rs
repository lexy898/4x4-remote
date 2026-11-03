//! What the sources of adds ask the window for ([`AddRequest`]), and the reading of a `.torrent`
//! file when its turn in the queue comes ([`read_torrent`]).
//!
//! A `.torrent` file is untrusted: it may be a FIFO that never ends, a device, a directory, or a
//! file of gigabytes. It is read only once `stat` says it is a regular file of at most
//! [`MAX_TORRENT_FILE_BYTES`], and never more than that is read, even from a file that grows
//! meanwhile. Reading and parsing take a while for a large or hostile file, so the main view runs
//! them on the background executor, never on the UI thread.

use std::fs::File;
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use x4_core::launch::RejectedArg;
use x4_core::{AddItem, LoadedItem, MAX_TORRENT_FILE_BYTES, MetainfoError};

/// What a source of adds asks the window for: the toolbar, the keys, the file picker, the
/// clipboard, a drop, the launch arguments and a second launch all send these, through the one
/// channel that [`MainView::listen_for_adds`](crate::main_view::MainView::listen_for_adds) takes.
///
/// Nothing is added by a request: the items wait in the add dialog's queue for the user's Add.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddRequest {
    /// `.torrent` files and magnet links to offer in the add dialog, in this order.
    Items(Vec<AddItem>),
    /// Launch arguments 4x4 does not open; the window says so in one notification.
    Rejected(Vec<RejectedArg>),
    /// Bring the window to the front, as for a second launch that handed its arguments over.
    Raise,
}

/// Why a `.torrent` file cannot be offered in the add dialog.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum LoadError {
    /// It is not there.
    #[error("the file does not exist")]
    Missing,
    /// 4x4 may not read it.
    #[error("the file may not be read")]
    Denied,
    /// It could not be read for another reason.
    #[error("the file could not be read: {0}")]
    Unreadable(io::ErrorKind),
    /// It is a directory, a FIFO, a socket or a device.
    #[error("not a regular file")]
    NotAFile,
    /// It is larger than [`MAX_TORRENT_FILE_BYTES`].
    #[error("the file has {size} bytes, more than the {MAX_TORRENT_FILE_BYTES} that are read")]
    TooLarge {
        /// Its size in bytes, as far as it is known.
        size: u64,
    },
    /// It is not a torrent the daemon can add.
    #[error(transparent)]
    Invalid(MetainfoError),
}

impl From<io::Error> for LoadError {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::NotFound => Self::Missing,
            io::ErrorKind::PermissionDenied => Self::Denied,
            kind => Self::Unreadable(kind),
        }
    }
}

/// The item `item` as the add dialog shows it: a magnet link as it is, a `.torrent` file read and
/// parsed by [`read_torrent`]. It blocks while a file is read, so it runs off the UI thread.
///
/// # Errors
///
/// The [`LoadError`] of [`read_torrent`] for a file that cannot be offered.
pub(crate) fn load(item: AddItem) -> Result<LoadedItem, LoadError> {
    match item {
        AddItem::File(path) => read_torrent(path),
        AddItem::Magnet(link) => Ok(LoadedItem::Magnet(link)),
    }
}

/// Reads the `.torrent` file at `path` and parses it, once `stat` says it is a regular file of at
/// most [`MAX_TORRENT_FILE_BYTES`]. The file is checked again once it is open, and no more than one
/// byte past the limit is ever read from it.
///
/// # Errors
///
/// [`LoadError::NotAFile`] for anything but a regular file, [`LoadError::TooLarge`] for a file
/// larger than [`MAX_TORRENT_FILE_BYTES`], the [`LoadError`] of an I/O error for a file that
/// cannot be read, and [`LoadError::Invalid`] for one that is not a torrent the daemon can add.
pub(crate) fn read_torrent(path: PathBuf) -> Result<LoadedItem, LoadError> {
    // A FIFO would block the open itself, so it is turned away before.
    check_metadata(&std::fs::metadata(&path)?)?;
    let bytes = read_at_most(&path)?;
    LoadedItem::torrent(path, bytes).map_err(LoadError::Invalid)
}

/// Whether the file of `metadata` may be read: a regular file of at most
/// [`MAX_TORRENT_FILE_BYTES`].
fn check_metadata(metadata: &std::fs::Metadata) -> Result<(), LoadError> {
    if !metadata.is_file() {
        return Err(LoadError::NotAFile);
    }
    if metadata.len() > MAX_TORRENT_FILE_BYTES {
        return Err(LoadError::TooLarge {
            size: metadata.len(),
        });
    }
    Ok(())
}

/// The content of the regular file at `path`, of at most [`MAX_TORRENT_FILE_BYTES`].
fn read_at_most(path: &Path) -> Result<Arc<[u8]>, LoadError> {
    let file = File::open(path)?;
    // It may have been replaced since the first look.
    check_metadata(&file.metadata()?)?;
    let mut bytes = Vec::new();
    file.take(MAX_TORRENT_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)?;
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if size > MAX_TORRENT_FILE_BYTES {
        return Err(LoadError::TooLarge { size });
    }
    Ok(Arc::from(bytes))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    /// "a": one file of 16 KiB, in one piece.
    const ONE_FILE: &[u8] =
        b"d4:infod6:lengthi16384e4:name1:a12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaaee";

    #[test]
    fn a_regular_torrent_file_is_read_and_parsed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.torrent");
        fs::write(&path, ONE_FILE).unwrap();

        let Ok(LoadedItem::Torrent(torrent)) = read_torrent(path.clone()) else {
            panic!("a torrent");
        };
        assert_eq!(torrent.path(), path);
        assert_eq!(torrent.bytes(), ONE_FILE);
        assert_eq!(torrent.parsed().name, "a");
    }

    #[test]
    fn only_a_regular_file_within_the_limit_is_read() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            read_torrent(dir.path().join("missing.torrent")).err(),
            Some(LoadError::Missing)
        );
        assert_eq!(
            read_torrent(dir.path().to_path_buf()).err(),
            Some(LoadError::NotAFile)
        );
        let huge = dir.path().join("huge.torrent");
        File::create(&huge)
            .unwrap()
            .set_len(MAX_TORRENT_FILE_BYTES + 1)
            .unwrap();
        assert_eq!(
            read_torrent(huge).err(),
            Some(LoadError::TooLarge {
                size: MAX_TORRENT_FILE_BYTES + 1
            })
        );
        let at_the_limit = dir.path().join("limit.torrent");
        File::create(&at_the_limit)
            .unwrap()
            .set_len(MAX_TORRENT_FILE_BYTES)
            .unwrap();
        assert!(
            matches!(read_torrent(at_the_limit), Err(LoadError::Invalid(_))),
            "a file at the limit is read, and parsed"
        );
        let text = dir.path().join("text.torrent");
        fs::write(&text, b"not bencode").unwrap();
        assert!(matches!(
            read_torrent(text),
            Err(LoadError::Invalid(MetainfoError::NotBencode { .. }))
        ));
    }

    #[test]
    fn a_fifo_is_refused_without_being_opened() {
        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("fifo.torrent");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());

        // Opening it would wait for a writer forever.
        assert_eq!(read_torrent(fifo).err(), Some(LoadError::NotAFile));
    }

    #[test]
    fn a_file_that_may_not_be_read_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("closed.torrent");
        fs::write(&path, ONE_FILE).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
        // Root reads it anyway; the test means something only for a user.
        if File::open(&path).is_err() {
            assert_eq!(read_torrent(path).err(), Some(LoadError::Denied));
        }
    }

    #[test]
    fn a_magnet_link_is_offered_as_it_is() {
        assert!(matches!(
            load(AddItem::Magnet(None)),
            Ok(LoadedItem::Magnet(None))
        ));
    }
}
