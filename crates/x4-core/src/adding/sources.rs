//! What the sources of adds inside the window hand over, told apart before anything joins the
//! queue: the files of a drop ([`dropped_files`]), the magnet links on the clipboard
//! ([`clipboard_magnets`], and [`magnet_form_items`] for "Add magnet") and the files a file dialog
//! chose, which it names by their URIs ([`chosen_file`]).
//!
//! Nothing here reads a file: whether a `.torrent` exists, and what it is, is checked when its turn
//! in the queue comes.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use x4_rpc::MagnetLink;

use super::queue::AddItem;
use crate::launch::{LaunchArg, classify};

/// The extension of a `.torrent` file, compared without regard to case.
const TORRENT_EXTENSION: &str = "torrent";

/// The files of a drop, told apart: the `.torrent` files, which join the queue, and how many
/// others there were, which one notification counts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DroppedFiles {
    /// The paths whose name ends in `.torrent`, in any case, in the order they were dropped.
    pub torrents: Vec<PathBuf>,
    /// How many of the other paths were dropped.
    pub others: usize,
}

/// Tells the `.torrent` files of the dropped `paths` from the others, by the extension of their
/// names in any case (`a.torrent`, `B.TORRENT`): a drop never queues an image or a folder by
/// mistake.
#[must_use]
pub fn dropped_files(paths: impl IntoIterator<Item = PathBuf>) -> DroppedFiles {
    let mut dropped = DroppedFiles::default();
    for path in paths {
        if is_torrent_file_name(&path) {
            dropped.torrents.push(path);
        } else {
            dropped.others += 1;
        }
    }
    dropped
}

/// Whether the name of `path` has the extension `torrent`, in any case.
fn is_torrent_file_name(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(TORRENT_EXTENSION))
}

/// The magnet links on the clipboard, whose text is `text`: every line that
/// [`MagnetLink::parse`] takes, trimmed, in the order of the lines. Empty lines and lines that
/// are no magnet link the daemon can add are left out. 4x4's own "Copy magnet link" writes one
/// link per line.
#[must_use]
pub fn clipboard_magnets(text: &str) -> Vec<MagnetLink> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter_map(|line| MagnetLink::parse(line).ok())
        .collect()
}

/// What "Add magnet" queues for the clipboard's text `clipboard` (`None` when the clipboard holds
/// no text): each magnet link on it ([`clipboard_magnets`]), or, without one, the magnet form with
/// an empty link for the user to fill in.
#[must_use]
pub fn magnet_form_items(clipboard: Option<&str>) -> Vec<AddItem> {
    let links = clipboard.map(clipboard_magnets).unwrap_or_default();
    if links.is_empty() {
        return vec![AddItem::Magnet(None)];
    }
    links
        .into_iter()
        .map(|link| AddItem::Magnet(Some(link)))
        .collect()
}

/// The local file that a file dialog's `uri` names: a `file:` URI with no host or `localhost`,
/// and no query or fragment, percent-decoded as a launch argument is ([`classify`]). Anything
/// else, such as a URI of another scheme or a bare path, names no file here.
#[must_use]
pub fn chosen_file(uri: &str) -> Option<PathBuf> {
    let scheme = uri.get(..FILE_SCHEME.len())?;
    if !scheme.eq_ignore_ascii_case(FILE_SCHEME) {
        return None;
    }
    // A `file:` URI is absolute, so no working directory is joined to it.
    match classify(OsStr::new(uri), Path::new("/")) {
        LaunchArg::File(path) => Some(path),
        LaunchArg::Magnet(_) | LaunchArg::Rejected(_) => None,
    }
}

/// The start of a `file:` URI.
const FILE_SCHEME: &str = "file:";

#[cfg(test)]
mod tests {
    use super::*;

    const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn link(hash: &str, name: &str) -> MagnetLink {
        MagnetLink::parse(&format!("magnet:?xt=urn:btih:{hash}&dn={name}")).unwrap()
    }

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names
            .iter()
            .map(|name| PathBuf::from(format!("/home/user/Downloads/{name}")))
            .collect()
    }

    #[test]
    fn only_torrent_files_of_a_drop_join_and_the_others_are_counted() {
        let dropped = dropped_files(paths(&[
            "b.torrent",
            "notes.txt",
            "A.TORRENT",
            "folder",
            "c.Torrent",
            "torrent",
            ".torrent",
            "d.torrent.part",
            "e.torrents",
        ]));

        assert_eq!(
            dropped.torrents,
            paths(&["b.torrent", "A.TORRENT", "c.Torrent"]),
            "in the order they were dropped"
        );
        assert_eq!(dropped.others, 6);
        assert_eq!(dropped_files(Vec::new()), DroppedFiles::default());
    }

    #[test]
    fn every_line_of_the_clipboard_that_is_a_magnet_link_is_taken_in_order() {
        let text = format!(
            "  magnet:?xt=urn:btih:{HASH_B}&dn=Second\r\n\n\
             not a link\n\
             {HASH_A}\n\
             https://example.org/a.torrent\n\
             \tMAGNET:?xt=urn:btih:{HASH_A}&dn=First \n\
             magnet:?xt=urn:btih:{HASH_B}&dn=Second\n\
             magnet:?xt=urn:btih:nothex"
        );

        assert_eq!(
            clipboard_magnets(&text),
            [
                link(HASH_B, "Second"),
                link(HASH_A, "First"),
                link(HASH_B, "Second"),
            ],
            "trimmed, in order, every line that parses"
        );
        assert_eq!(clipboard_magnets(""), []);
        assert_eq!(clipboard_magnets("\n \n"), []);
        assert_eq!(clipboard_magnets("a search text"), []);
    }

    #[test]
    fn add_magnet_takes_the_links_on_the_clipboard_or_opens_an_empty_form() {
        let text = format!(
            "magnet:?xt=urn:btih:{HASH_A}&dn=First\nmagnet:?xt=urn:btih:{HASH_B}&dn=Second"
        );
        assert_eq!(
            magnet_form_items(Some(&text)),
            [
                AddItem::Magnet(Some(link(HASH_A, "First"))),
                AddItem::Magnet(Some(link(HASH_B, "Second"))),
            ]
        );
        for empty in [None, Some(""), Some("no link here")] {
            assert_eq!(
                magnet_form_items(empty),
                [AddItem::Magnet(None)],
                "{empty:?}"
            );
        }
    }

    #[test]
    fn a_file_dialog_names_local_files_by_their_file_uris() {
        assert_eq!(
            chosen_file("file:///home/user/My%20Shows/a.torrent"),
            Some(PathBuf::from("/home/user/My Shows/a.torrent"))
        );
        assert_eq!(
            chosen_file("FILE://localhost/srv/b.torrent"),
            Some(PathBuf::from("/srv/b.torrent"))
        );
        for other in [
            "file://server/share/a.torrent",
            "file:///a.torrent?x=1",
            "sftp://host/a.torrent",
            "https://example.org/a.torrent",
            "/home/user/a.torrent",
            "a.torrent",
            "file",
            "",
            &format!("magnet:?xt=urn:btih:{HASH_A}"),
        ] {
            assert_eq!(chosen_file(other), None, "{other:?}");
        }
    }
}
