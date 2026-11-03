//! What the sources of adds inside the window hand over, told apart before anything joins the
//! queue: the files of a drop ([`dropped_files`]), the magnet links on the clipboard
//! ([`clipboard_magnets`], and [`magnet_form_items`] for "Add magnet") and the files a file dialog
//! chose, which it names by their URIs ([`chosen_files`]).
//!
//! Nothing here reads a file: whether a `.torrent` exists, and what it is, is checked when its turn
//! in the queue comes. The clipboard may hold any amount of text, so no more magnet links are
//! parsed than the queue can hold ([`QUEUE_CAP`]), and no more lines than twice that; the lines
//! after them are only counted.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use x4_rpc::MagnetLink;

use super::queue::{AddItem, QUEUE_CAP};
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

/// What the clipboard gives to add: the items, at most [`QUEUE_CAP`] of them, in the order of its
/// lines, and how many more lines the queue would have no room for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardItems<T> {
    /// What the lines give, each once, in their order.
    pub items: Vec<T>,
    /// How many lines after the first [`QUEUE_CAP`] items, or after twice that many lines were
    /// parsed, look like magnet links (they start with `magnet:?`, in any case). They are left out
    /// unparsed, so a line among them that is a duplicate, or no link the daemon can add, counts
    /// too.
    pub left_out: usize,
}

impl<T> Default for ClipboardItems<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            left_out: 0,
        }
    }
}

/// The magnet links on the clipboard, whose text is `text`: every line that
/// [`MagnetLink::parse`] takes, trimmed, once, in the order of the lines, up to [`QUEUE_CAP`]
/// links. Empty lines and lines that are no magnet link the daemon can add are left out. 4x4's
/// own "Copy magnet link" writes one link per line.
///
/// Only lines that look like magnet links are parsed, and no more of them than twice
/// [`QUEUE_CAP`]: once [`QUEUE_CAP`] links are taken, or twice that many lines were tried, the
/// other lines that look like magnet links are only counted ([`ClipboardItems::left_out`]). So a
/// clipboard of millions of lines costs one pass over its text, even when its lines are copies of
/// one link, which never fill the queue.
#[must_use]
pub fn clipboard_magnets(text: &str) -> ClipboardItems<MagnetLink> {
    let mut magnets = ClipboardItems::default();
    let mut parsed = 0;
    for line in text.lines().map(str::trim) {
        // `MagnetLink::parse` takes no other line.
        if !looks_like_magnet_link(line) {
            continue;
        }
        if magnets.items.len() == QUEUE_CAP || parsed == PARSED_LINES {
            magnets.left_out += 1;
            continue;
        }
        parsed += 1;
        if let Ok(link) = MagnetLink::parse(line)
            && !magnets.items.contains(&link)
        {
            magnets.items.push(link);
        }
    }
    magnets
}

/// The most lines of the clipboard that are parsed as magnet links: twice what the queue holds,
/// room for duplicates and for lines that are no link the daemon can add.
const PARSED_LINES: usize = 2 * QUEUE_CAP;

/// Whether `line` starts as a magnet link does, `magnet:?` in any case; nothing more is checked.
fn looks_like_magnet_link(line: &str) -> bool {
    line.get(..MAGNET_START.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(MAGNET_START))
}

/// The start of a magnet link.
const MAGNET_START: &str = "magnet:?";

/// What "Add magnet" queues for the clipboard's text `clipboard` (`None` when the clipboard holds
/// no text): each magnet link on it ([`clipboard_magnets`]), or, without one, the magnet form with
/// an empty link for the user to fill in.
#[must_use]
pub fn magnet_form_items(clipboard: Option<&str>) -> ClipboardItems<AddItem> {
    let ClipboardItems { items, left_out } = clipboard.map(clipboard_magnets).unwrap_or_default();
    if items.is_empty() {
        return ClipboardItems {
            items: vec![AddItem::Magnet(None)],
            left_out,
        };
    }
    ClipboardItems {
        items: items
            .into_iter()
            .map(|link| AddItem::Magnet(Some(link)))
            .collect(),
        left_out,
    }
}

/// The files a file dialog chose, told apart: those on this computer, which join the queue, and
/// how many others there were, which one notification counts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChosenFiles {
    /// The local files, in the order the dialog gave them.
    pub paths: Vec<PathBuf>,
    /// How many of the files chosen are not on this computer, such as those of a network place
    /// the dialog names by an `sftp:` or `smb:` URI.
    pub not_local: usize,
}

/// The files a file dialog chose, which it names by their `uris`: the local file of each URI that
/// names one, in order, and how many name none. A local file is named by a `file:` URI with no
/// host or `localhost`, and no query or fragment, percent-decoded as a launch argument is
/// ([`classify`]); a URI of another scheme, or a bare path, names no file here.
#[must_use]
pub fn chosen_files<'a>(uris: impl IntoIterator<Item = &'a str>) -> ChosenFiles {
    let mut chosen = ChosenFiles::default();
    for uri in uris {
        match chosen_file(uri) {
            Some(path) => chosen.paths.push(path),
            None => chosen.not_local += 1,
        }
    }
    chosen
}

/// The local file that a file dialog's `uri` names: a `file:` URI with no host or `localhost`,
/// and no query or fragment, percent-decoded as a launch argument is ([`classify`]). Anything
/// else, such as a URI of another scheme or a bare path, names no file here.
fn chosen_file(uri: &str) -> Option<PathBuf> {
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
            ClipboardItems {
                items: vec![link(HASH_B, "Second"), link(HASH_A, "First")],
                left_out: 0,
            },
            "trimmed, in order, once, every line that parses"
        );
        for none in ["", "\n \n", "a search text"] {
            assert_eq!(
                clipboard_magnets(none),
                ClipboardItems::default(),
                "{none:?}"
            );
        }
    }

    /// A magnet link of its own for each `n`.
    fn numbered_link(n: usize) -> String {
        format!("magnet:?xt=urn:btih:{n:040x}")
    }

    #[test]
    fn no_more_magnet_links_are_taken_than_the_queue_holds_and_the_rest_are_counted() {
        let mut lines: Vec<String> = (0..QUEUE_CAP + 3).map(numbered_link).collect();
        // A duplicate takes no room; among the lines after the cap, only what looks like a magnet
        // link counts, unparsed.
        lines.insert(1, numbered_link(0));
        lines.extend([
            "  MAGNET:?xt=urn:btih:nothex".to_owned(),
            "not a link".to_owned(),
            String::new(),
            numbered_link(0),
        ]);

        let magnets = clipboard_magnets(&lines.join("\n"));
        let first: Vec<_> = (0..QUEUE_CAP)
            .map(|n| MagnetLink::parse(&numbered_link(n)).unwrap())
            .collect();
        assert_eq!(
            magnets.items.len(),
            QUEUE_CAP,
            "no more than the queue holds"
        );
        assert_eq!(magnets.items, first, "the first links, in order");
        assert_eq!(
            magnets.left_out, 5,
            "3 more links, a bad one and a duplicate"
        );

        let items = magnet_form_items(Some(&lines.join("\n")));
        assert_eq!(items.items.len(), QUEUE_CAP);
        assert_eq!(items.left_out, 5);
    }

    #[test]
    fn no_more_lines_are_parsed_than_twice_what_the_queue_holds() {
        // A million copies of one link: each was parsed, as a duplicate never fills the queue,
        // and that took 0.4 s of the window's thread. Past the lines tried, those that look like
        // magnet links are counted unparsed, so the count tells how many were parsed.
        let copies = format!("{}\n", numbered_link(0)).repeat(1_000_000);

        let magnets = clipboard_magnets(&copies);

        assert_eq!(magnets.items.len(), 1);
        assert_eq!(magnets.left_out, 1_000_000 - 2 * QUEUE_CAP);

        // Lines that look like links and are none are tried too; text that does not look like a
        // link is not, and neither counts.
        let mut lines = vec!["magnet:?xt=urn:btih:nothex".to_owned(); 2 * QUEUE_CAP - 1];
        lines.extend(std::iter::repeat_n("not a link".to_owned(), 50));
        lines.extend([numbered_link(1), numbered_link(2)]);
        let magnets = clipboard_magnets(&lines.join("\n"));
        assert_eq!(
            magnets.items,
            [MagnetLink::parse(&numbered_link(1)).unwrap()]
        );
        assert_eq!(magnets.left_out, 1, "the second link, past the lines tried");
    }

    #[test]
    fn add_magnet_takes_the_links_on_the_clipboard_or_opens_an_empty_form() {
        let text = format!(
            "magnet:?xt=urn:btih:{HASH_A}&dn=First\nmagnet:?xt=urn:btih:{HASH_B}&dn=Second"
        );
        assert_eq!(
            magnet_form_items(Some(&text)).items,
            [
                AddItem::Magnet(Some(link(HASH_A, "First"))),
                AddItem::Magnet(Some(link(HASH_B, "Second"))),
            ]
        );
        for empty in [None, Some(""), Some("no link here")] {
            assert_eq!(
                magnet_form_items(empty),
                ClipboardItems {
                    items: vec![AddItem::Magnet(None)],
                    left_out: 0,
                },
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

    #[test]
    fn the_files_a_file_dialog_chose_off_this_computer_are_counted() {
        assert_eq!(
            chosen_files([
                "file:///home/user/b.torrent",
                "sftp://host/a.torrent",
                "file:///home/user/My%20Shows/a.torrent",
                "smb://server/share/c.torrent",
            ]),
            ChosenFiles {
                paths: vec![
                    PathBuf::from("/home/user/b.torrent"),
                    PathBuf::from("/home/user/My Shows/a.torrent"),
                ],
                not_local: 2,
            },
            "the local files in order"
        );
        assert_eq!(chosen_files([]), ChosenFiles::default());
    }
}
