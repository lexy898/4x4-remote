//! The form of the add dialog for one torrent or magnet link, and the check that gates Add.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use x4_rpc::{
    AddSource, DownloadDir, DownloadDirError, HashString, MagnetError, MagnetLink, TorrentAdd,
};

use super::choice::FileChoice;
use crate::files::{FileTree, NodeKey};
use crate::metainfo::{MetainfoError, ParsedTorrent, parse_torrent};
use crate::snapshot::Snapshot;

/// The head of the [`AddQueue`](super::AddQueue) once it is read: the app reads a `.torrent`
/// file, and [`LoadedItem::torrent`] parses it.
#[derive(Debug, Clone)]
pub enum LoadedItem {
    /// A `.torrent` file, read and parsed.
    Torrent(LoadedTorrent),
    /// A magnet link; `None` for the magnet form with an empty link, which the user fills in.
    Magnet(Option<MagnetLink>),
}

impl LoadedItem {
    /// The `.torrent` file read from `path`, whose content is `bytes`, parsed strictly by
    /// [`parse_torrent`], with the tree of its files made for the form.
    ///
    /// No tree is made when the daemon could number the files otherwise, or when the list is too
    /// large to choose from ([`NoChoiceReason`]); the torrent is then added with all its files.
    /// Making the tree takes about [`TREE_BUDGET`](crate::TREE_BUDGET) at most beyond the parse,
    /// the copy of the files it is made from included: each path is read one component at a time,
    /// and one with more folders than the budget can hold is given up as soon as they are counted
    /// ([`FileTree::of_files`]).
    ///
    /// # Errors
    ///
    /// The [`MetainfoError`] of [`parse_torrent`] when the daemon could not add the file.
    pub fn torrent(path: PathBuf, bytes: Arc<[u8]>) -> Result<Self, MetainfoError> {
        let parsed = parse_torrent(&bytes)?;
        let tree = match &parsed.files {
            Some(files) => {
                FileChoice::first_tree(parsed.hash.clone(), files).ok_or(NoChoiceReason::TooLarge)
            }
            None => Err(NoChoiceReason::Ambiguous),
        };
        Ok(Self::Torrent(LoadedTorrent {
            path,
            parsed,
            bytes,
            tree,
        }))
    }
}

/// A `.torrent` file, read and parsed by [`LoadedItem::torrent`], which alone makes one.
///
/// Its fields cannot be set from outside, so the parse is always that of the bytes: the files
/// the user leaves out are counted in the very bytes that are sent. Another crate can neither
/// pair a parse with other bytes nor change the bytes afterwards:
///
/// ```compile_fail,E0451
/// # use std::sync::Arc;
/// # fn pair(parsed: x4_core::ParsedTorrent) -> x4_core::LoadedTorrent {
/// x4_core::LoadedTorrent {
///     path: "/home/user/t.torrent".into(),
///     parsed,
///     bytes: Arc::from(&b"other bytes"[..]),
///     tree: Err(x4_core::NoChoiceReason::Ambiguous),
/// }
/// # }
/// ```
///
/// ```compile_fail,E0616
/// # use std::sync::Arc;
/// # fn swap(torrent: &mut x4_core::LoadedTorrent) {
/// torrent.bytes = Arc::from(&b"other bytes"[..]);
/// # }
/// ```
///
/// Its `Debug` shows it by its path, hash and length: its content names the trackers, whose
/// addresses can carry passkeys.
#[derive(Clone)]
pub struct LoadedTorrent {
    path: PathBuf,
    parsed: ParsedTorrent,
    bytes: Arc<[u8]>,
    /// The tree of `parsed.files` with every file wanted, or why there is none.
    tree: Result<Arc<FileTree>, NoChoiceReason>,
}

impl LoadedTorrent {
    /// The file it was read from.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What the bytes hold, as the daemon will read them.
    #[must_use]
    pub fn parsed(&self) -> &ParsedTorrent {
        &self.parsed
    }

    /// The content of the file, exactly as parsed; it is sent as it is.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl fmt::Debug for LoadedTorrent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoadedTorrent")
            .field("path", &self.path)
            .field("hash", &self.parsed.hash)
            .field("len", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

/// Why the form of a `.torrent` offers no choice of files, so that the dialog can say so; the
/// torrent is then added with all its files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoChoiceReason {
    /// The daemon could number the files otherwise than the strict reader, so leaving some out
    /// could leave out the wrong ones ([`ParsedTorrent::files`] is `None`).
    Ambiguous,
    /// The list of files is too large to choose from: more than
    /// [`MAX_CHOOSABLE_FILES`](crate::MAX_CHOOSABLE_FILES) files, or a tree that would take
    /// more than [`TREE_BUDGET`](crate::TREE_BUDGET) with the copy of the files it is made from,
    /// as for files nested hundreds of folders deep ([`FileTree::is_too_large`]).
    TooLarge,
}

/// Whether the form can be sent, and what is sent; the app enables Add only for
/// [`Ready`](Self::Ready).
///
/// [`AddForm::check`] looks at the form in this order and gives the first reason it finds:
/// [`NotConnected`](Self::NotConnected), [`AlreadyInList`](Self::AlreadyInList),
/// [`BadLink`](Self::BadLink), [`BadFolder`](Self::BadFolder), then
/// [`NothingWanted`](Self::NothingWanted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddCheck {
    /// Add sends this request.
    Ready(TorrentAdd),
    /// No file that is shown is wanted: the torrent would download nothing.
    NothingWanted,
    /// The folder is not one the daemon can download into.
    BadFolder(DownloadDirError),
    /// The text of the magnet form is not a magnet link the daemon can add.
    BadLink(MagnetError),
    /// The daemon has this torrent already, so nothing is sent; the dialog offers to show it.
    AlreadyInList(HashString),
    /// The daemon is not connected; the form is kept until it is.
    NotConnected,
}

/// The form of the add dialog for one item of the queue.
///
/// For a `.torrent` it holds the parse, the bytes to send and the choice of files; for a magnet
/// link, the link the user typed or the one that came. Both have the folder and "Start
/// immediately". Its `Debug` shows neither the content of the file nor the link.
pub struct AddForm {
    source: Source,
    /// The folder as the user typed or picked it.
    folder: String,
    start: bool,
}

/// What the form adds.
enum Source {
    Torrent {
        path: PathBuf,
        name: String,
        hash: HashString,
        bytes: Arc<[u8]>,
        /// Or why the torrent can be added only with all its files.
        choice: Result<FileChoice, NoChoiceReason>,
    },
    Magnet(Result<MagnetLink, MagnetError>),
}

impl AddForm {
    /// The form for `item`, with the folder `folder` and "Start immediately" set to `start`.
    ///
    /// A `.torrent` starts with every file wanted. One whose files the daemon could number
    /// otherwise, or whose list is too large to choose from, has no choice of files and is added
    /// with all of them ([`choice_unavailable`](Self::choice_unavailable) says why). The magnet
    /// form of `LoadedItem::Magnet(None)` starts with an empty link.
    #[must_use]
    pub fn new(item: LoadedItem, folder: String, start: bool) -> Self {
        let source = match item {
            LoadedItem::Torrent(LoadedTorrent {
                path,
                parsed,
                bytes,
                tree,
            }) => Source::Torrent {
                path,
                name: parsed.name,
                choice: match (tree, parsed.files) {
                    (Ok(tree), Some(files)) => Ok(FileChoice::new(files.len(), tree)),
                    // A tree is made only of files, so this does not happen.
                    (Ok(_), None) => Err(NoChoiceReason::Ambiguous),
                    (Err(reason), _) => Err(reason),
                },
                hash: parsed.hash,
                bytes,
            },
            LoadedItem::Magnet(link) => Source::Magnet(link.ok_or(MagnetError::Empty)),
        };
        Self {
            source,
            folder,
            start,
        }
    }

    /// The tree of the files to choose from, whose checkboxes show the choice: `None` for a
    /// magnet link, whose files are known only once its metadata arrives, and for a `.torrent`
    /// that can be added only with all its files
    /// ([`choice_unavailable`](Self::choice_unavailable)).
    ///
    /// The tree is shared, so the dialog can keep it for the rows it draws, and a tree kept stays
    /// as it was drawn. A click changes the form's tree where it is, or, while the dialog keeps
    /// it, a copy that shares the folders and files with it and has wanted states of its own.
    #[must_use]
    pub fn tree(&self) -> Option<&Arc<FileTree>> {
        match &self.source {
            Source::Torrent {
                choice: Ok(choice), ..
            } => Some(choice.tree()),
            Source::Torrent { choice: Err(_), .. } | Source::Magnet(_) => None,
        }
    }

    /// Applies a click on the checkbox of the node `key` of the [`tree`](Self::tree): its files
    /// become wanted, or not wanted when all of them are. A folder takes its padding files with
    /// it, as in the details panel. Nothing changes for a key that is not a node of the tree, or
    /// when there is no tree.
    ///
    /// The tree is neither made nor sorted again: a click costs no more than the files and
    /// folders it reaches, and the folders above them.
    pub fn toggle_wanted(&mut self, key: &NodeKey) {
        if let Source::Torrent {
            choice: Ok(choice), ..
        } = &mut self.source
        {
            choice.toggle(key);
        }
    }

    /// Sets the folder to download into, as the user typed or picked it. It is checked by
    /// [`check`](Self::check) and [`folder`](Self::folder).
    pub fn set_folder(&mut self, folder: String) {
        self.folder = folder;
    }

    /// Sets "Start immediately".
    pub fn set_start(&mut self, start: bool) {
        self.start = start;
    }

    /// Sets the text of the magnet form's link field, which [`MagnetLink::parse`] checks. A
    /// `.torrent`'s form has no link, and ignores it.
    pub fn set_link(&mut self, link: &str) {
        if let Source::Magnet(current) = &mut self.source {
            *current = MagnetLink::parse(link);
        }
    }

    /// The torrent's name as the daemon will show it: the `.torrent`'s name, or the magnet link's
    /// display name. `None` when it has none, or the link is not valid.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        match &self.source {
            Source::Torrent { name, .. } => Some(name.as_str()).filter(|name| !name.is_empty()),
            Source::Magnet(link) => link.as_ref().ok().and_then(MagnetLink::name),
        }
    }

    /// The `.torrent` file the form was read from; `None` for a magnet link.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        match &self.source {
            Source::Torrent { path, .. } => Some(path),
            Source::Magnet(_) => None,
        }
    }

    /// Why a `.torrent`'s form has no choice of files and adds it with all of them; `None` when
    /// it has a [`tree`](Self::tree) to choose from. A magnet link has no choice either, for
    /// another reason that [`is_magnet`](Self::is_magnet) gives: its files come with its
    /// metadata, after it is added. So it is `None` for a magnet link too.
    #[must_use]
    pub fn choice_unavailable(&self) -> Option<NoChoiceReason> {
        match &self.source {
            Source::Torrent {
                choice: Err(reason),
                ..
            } => Some(*reason),
            Source::Torrent { choice: Ok(_), .. } | Source::Magnet(_) => None,
        }
    }

    /// Whether this is the magnet form, which has a link field and no choice of files.
    #[must_use]
    pub fn is_magnet(&self) -> bool {
        matches!(self.source, Source::Magnet(_))
    }

    /// The folder to download into, checked and normalized; the app asks for its free space.
    ///
    /// # Errors
    ///
    /// The [`DownloadDirError`] of [`DownloadDir::new`] when the daemon could not use the folder.
    pub fn folder(&self) -> Result<DownloadDir, DownloadDirError> {
        DownloadDir::new(&self.folder)
    }

    /// Whether "Start immediately" is set.
    #[must_use]
    pub fn start(&self) -> bool {
        self.start
    }

    /// Whether Add may send the form, and the request it sends; `connected` tells whether the
    /// daemon answers now.
    ///
    /// The checks come in this order, and the first that fails is the answer:
    /// 1. [`AddCheck::NotConnected`], whatever the form holds.
    /// 2. [`AddCheck::AlreadyInList`] when a torrent of `snapshot` has the info hash of the form,
    ///    whatever else is wrong with it: nothing is sent for it.
    /// 3. [`AddCheck::BadLink`], then [`AddCheck::BadFolder`]. A link that is not valid names no
    ///    torrent, so it cannot be in the list.
    /// 4. [`AddCheck::NothingWanted`] when no file shown is wanted.
    ///
    /// The request of [`AddCheck::Ready`]:
    /// - sends a `.torrent` as the very bytes that were parsed, with `files_unwanted` from that
    ///   parse: in ascending order, each index once and less than the number of files, and never
    ///   all of them;
    /// - sends a magnet link with no file choices at all, and a `.torrent` without a choice of
    ///   files ([`tree`](Self::tree) is `None`) with all its files;
    /// - names the folder, normalized, only when it is not the session's default folder. The
    ///   daemon then uses its own default, as it does for every add without a folder. While the
    ///   session is not known, the folder is always named;
    /// - adds the torrent paused unless "Start immediately" is set, and sets no priorities.
    ///
    /// A `.torrent`'s content is shared with the form, not copied, so the check is cheap.
    #[must_use]
    pub fn check(&self, snapshot: &Snapshot, connected: bool) -> AddCheck {
        if !connected {
            return AddCheck::NotConnected;
        }
        let hash = match &self.source {
            Source::Torrent { hash, .. } => hash,
            Source::Magnet(Ok(link)) => link.hash(),
            Source::Magnet(Err(error)) => return AddCheck::BadLink(*error),
        };
        if snapshot
            .torrents
            .iter()
            .any(|torrent| torrent.hash == *hash)
        {
            return AddCheck::AlreadyInList(hash.clone());
        }
        let folder = match DownloadDir::new(&self.folder) {
            Ok(folder) => folder,
            Err(error) => return AddCheck::BadFolder(error),
        };
        let (source, files_unwanted) = match &self.source {
            Source::Torrent { bytes, choice, .. } => {
                let files_unwanted = match choice {
                    Ok(choice) if choice.nothing_wanted() => return AddCheck::NothingWanted,
                    Ok(choice) => choice.unwanted(),
                    Err(_) => Vec::new(),
                };
                (AddSource::Metainfo(Arc::clone(bytes)), files_unwanted)
            }
            Source::Magnet(Ok(link)) => (AddSource::Magnet(link.clone()), Vec::new()),
            Source::Magnet(Err(error)) => return AddCheck::BadLink(*error),
        };
        let default = snapshot
            .session
            .as_ref()
            .and_then(|session| DownloadDir::new(&session.download_dir).ok());
        let download_dir = (default.as_ref() != Some(&folder)).then_some(folder);
        AddCheck::Ready(TorrentAdd {
            source,
            download_dir,
            paused: !self.start,
            files_unwanted,
            priority_high: Vec::new(),
            priority_low: Vec::new(),
            sequential_download: None,
        })
    }
}

impl fmt::Debug for AddForm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut form = f.debug_struct("AddForm");
        match &self.source {
            Source::Torrent { hash, bytes, .. } => {
                form.field("torrent", hash).field("len", &bytes.len())
            }
            Source::Magnet(link) => form.field("magnet", link),
        };
        form.field("start", &self.start).finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
