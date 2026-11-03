//! Folders on the server to download into, validated and normalized (see [`DownloadDir::new`]).
//!
//! The daemon runs on Linux and keeps `download_dir` as it was sent (T/torrent.cc:910-917), so a
//! folder is normalized here once: `/a/` and `/a` are then one folder in the list and in the
//! history, and an empty or relative path, which the daemon refuses, is never sent.

use thiserror::Error;

/// The longest folder accepted, in bytes: 4096, Linux's `PATH_MAX`.
pub const MAX_DOWNLOAD_DIR_BYTES: usize = 4096;

/// An absolute folder on the server, normalized: no repeated `/`, and no trailing `/` except for
/// the root `/` itself.
///
/// Built only by [`DownloadDir::new`]. It is compared case-sensitively, as the server's file
/// system does.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DownloadDir(String);

impl DownloadDir {
    /// Validates and normalizes a folder on the server.
    ///
    /// Control characters at either end, such as the line break of a pasted line, are trimmed;
    /// spaces are kept, since they can be part of a name. Runs of `/` become one, and a trailing
    /// `/` is dropped. `~` is not expanded: the daemon does not expand it either.
    ///
    /// # Errors
    ///
    /// A [`DownloadDirError`] when the path is empty, is not absolute, holds a control character
    /// or a `.` or `..` part, which the daemon does not resolve, or is longer than
    /// [`MAX_DOWNLOAD_DIR_BYTES`] once normalized.
    pub fn new(path: &str) -> Result<Self, DownloadDirError> {
        let path = path.trim_matches(|character: char| character.is_ascii_control());
        if path.is_empty() {
            return Err(DownloadDirError::Empty);
        }
        if !path.starts_with('/') {
            return Err(DownloadDirError::NotAbsolute);
        }
        if let Some(offset) = path.find(char::is_control) {
            return Err(DownloadDirError::ControlCharacter { offset });
        }
        let mut normalized = String::with_capacity(path.len());
        for part in path.split('/').filter(|part| !part.is_empty()) {
            if part == "." || part == ".." {
                return Err(DownloadDirError::DotPart);
            }
            normalized.push('/');
            normalized.push_str(part);
        }
        if normalized.is_empty() {
            normalized.push('/');
        }
        if normalized.len() > MAX_DOWNLOAD_DIR_BYTES {
            return Err(DownloadDirError::TooLong {
                len: normalized.len(),
            });
        }
        Ok(Self(normalized))
    }

    /// The folder as it is sent.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The folder that holds this one; `None` for the root `/`.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        if self.0 == "/" {
            return None;
        }
        // Normalized: it starts with `/` and does not end with one.
        let (parent, _) = self.0.rsplit_once('/')?;
        Some(Self(if parent.is_empty() {
            "/".to_owned()
        } else {
            parent.to_owned()
        }))
    }
}

/// Why a text is not a folder to download into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DownloadDirError {
    /// Nothing but control characters.
    #[error("the folder is empty")]
    Empty,
    /// The path does not start with `/`.
    #[error("the folder is not an absolute path")]
    NotAbsolute,
    /// A control character inside the path.
    #[error("the folder has a control character at byte {offset}")]
    ControlCharacter {
        /// The offset of the character in the trimmed path.
        offset: usize,
    },
    /// A `.` or `..` part, which the daemon would keep as it is.
    #[error("the folder has a . or .. part")]
    DotPart,
    /// Longer than [`MAX_DOWNLOAD_DIR_BYTES`] once normalized.
    #[error(
        "the folder has {len} bytes, more than the {max} accepted",
        max = MAX_DOWNLOAD_DIR_BYTES
    )]
    TooLong {
        /// The length of the normalized path in bytes.
        len: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(path: &str) -> Result<String, DownloadDirError> {
        DownloadDir::new(path).map(|dir| dir.as_str().to_owned())
    }

    #[test]
    fn download_dir_must_be_absolute_and_is_normalized() {
        for (path, normalized) in [
            ("/data/torrents", "/data/torrents"),
            ("/data/torrents/", "/data/torrents"),
            ("//data///torrents//", "/data/torrents"),
            ("/", "/"),
            ("///", "/"),
            ("/data/my films ", "/data/my films "),
            ("/data/Фильмы", "/data/Фильмы"),
            ("/data/x\r\n", "/data/x"),
            ("\t/data/x\n", "/data/x"),
            ("/data/~x/.hidden/..x", "/data/~x/.hidden/..x"),
        ] {
            assert_eq!(dir(path), Ok(normalized.to_owned()), "{path:?}");
        }

        for (path, error) in [
            ("", DownloadDirError::Empty),
            ("\n", DownloadDirError::Empty),
            ("data/torrents", DownloadDirError::NotAbsolute),
            ("~/torrents", DownloadDirError::NotAbsolute),
            (" /data", DownloadDirError::NotAbsolute),
            ("/data/./x", DownloadDirError::DotPart),
            ("/data/../etc", DownloadDirError::DotPart),
            ("/data/..", DownloadDirError::DotPart),
            (
                "/data/a\0b",
                DownloadDirError::ControlCharacter { offset: 7 },
            ),
            (
                "/data/a\nb",
                DownloadDirError::ControlCharacter { offset: 7 },
            ),
            (
                "/data/a\u{9b}b",
                DownloadDirError::ControlCharacter { offset: 7 },
            ),
        ] {
            assert_eq!(dir(path), Err(error), "{path:?}");
        }

        let longest = format!("/{}", "a".repeat(MAX_DOWNLOAD_DIR_BYTES - 1));
        assert_eq!(dir(&format!("{longest}//")), Ok(longest.clone()));
        assert_eq!(
            dir(&format!("{longest}a")),
            Err(DownloadDirError::TooLong {
                len: MAX_DOWNLOAD_DIR_BYTES + 1
            })
        );
    }

    #[test]
    fn the_parent_of_a_folder_is_the_folder_that_holds_it() {
        let parent = |path: &str| {
            DownloadDir::new(path)
                .unwrap()
                .parent()
                .map(|dir| dir.as_str().to_owned())
        };

        assert_eq!(
            parent("/data/torrents/video"),
            Some("/data/torrents".to_owned())
        );
        assert_eq!(
            parent("/data/my films /x"),
            Some("/data/my films ".to_owned())
        );
        assert_eq!(parent("/data"), Some("/".to_owned()));
        assert_eq!(parent("/"), None);
    }
}
