//! The tree built from the flat list of files, with the sums of every folder.

mod builder;

use std::collections::HashMap;

use x4_rpc::{FilePriority, HashString};

use super::names::is_padding;
use super::{NodeKey, NonEmptyIndices, PriorityView, WantedState, next_priority};
use crate::action::Action;
use crate::details::{FileEntry, TorrentDetails};
use crate::playable::Playable;
use builder::Builder;

/// The files of one torrent as folders and files: those of its details, or those of a `.torrent`
/// about to be added.
///
/// The torrent's own top folder is the first level. Each node is found by its [`NodeKey`], its
/// path inside the torrent, and each folder carries the sums of the files below it: their size
/// and the bytes done, whether they are wanted and their priority. Padding files (BEP 47) are
/// hidden, but the files an action on a folder reaches ([`indices`](Self::indices)) include them.
/// What the user has done to the tree, such as the folders opened, is kept in a
/// [`FileTreeView`](super::FileTreeView).
///
/// A tree is a snapshot: it is made again from the details after every read, and the keys of the
/// nodes stay the same. It remembers the torrent it was made for, so an action drawn from it
/// names that torrent however the details have changed since.
///
/// **The app builds every file action through [`toggle_wanted_action`](Self::toggle_wanted_action),
/// [`priority_action`](Self::priority_action) and
/// [`step_priority_action`](Self::step_priority_action), on the tree the rows were drawn from.**
/// Each takes the torrent, the files and the current value from this one snapshot. Putting an
/// action together from parts of different reads would change the wrong files of a torrent, and
/// an action with an empty list of files changes all of them.
#[derive(Debug, Clone)]
pub struct FileTree {
    hash: HashString,
    nodes: Vec<Node>,
    /// The nodes at the first level, in display order.
    top: Vec<usize>,
    by_key: HashMap<NodeKey, usize>,
}

/// A folder or a file.
#[derive(Debug, Clone)]
pub(super) struct Node {
    pub(super) key: NodeKey,
    /// The last component of the path.
    pub(super) name: String,
    pub(super) parent: Option<usize>,
    /// The folder's entries in display order; empty for a file.
    pub(super) children: Vec<usize>,
    pub(super) is_folder: bool,
    /// The sum of the lengths of the shown files below, or the file's own length.
    pub(super) size: u64,
    /// The bytes of them that are downloaded, never more than `size`.
    pub(super) done: u64,
    pub(super) wanted: WantedState,
    pub(super) priority: PriorityView,
    /// The indices of all files below, padding files included, or the file's own; ascending.
    pub(super) files: Vec<usize>,
    /// What plays from the start; a file only, and only while it is wanted and incomplete and
    /// some of its start has come.
    pub(super) playable: Option<Playable>,
}

impl FileTree {
    /// The tree of `details.files`. A magnet link without metadata has no files, so its tree is
    /// empty.
    #[must_use]
    pub fn new(details: &TorrentDetails) -> Self {
        Self::of_files(details.hash.clone(), &details.files)
    }

    /// The tree of the files `files` of the torrent `hash`, each at its own
    /// [`index`](FileEntry::index).
    ///
    /// This is how the add dialog shows the files of a `.torrent` that is not added yet: nothing
    /// is downloaded, and `wanted` is what the user chose so far. The details panel's tree,
    /// [`new`](Self::new), is made the same way. A file whose path is empty is shown, without a
    /// name, like any other.
    #[must_use]
    pub fn of_files(hash: HashString, files: &[FileEntry]) -> Self {
        let mut builder = Builder::default();
        let mut padding = Vec::new();
        for entry in files {
            let mut parts: Vec<&str> = entry
                .name
                .split('/')
                .filter(|part| !part.is_empty())
                .collect();
            if parts.is_empty() {
                // A file without a name still is a file the user can see and act on.
                parts.push("");
            }
            if is_padding(&parts) {
                padding.push((entry.index, parts));
            } else {
                builder.add_file(entry, &parts);
            }
        }
        // Padding files join the lists of the folders that are shown, which exist only once all
        // other files have been added.
        for (index, parts) in &padding {
            builder.add_padding(*index, parts);
        }
        builder.finish(hash)
    }

    /// The torrent whose details the tree was made from.
    #[must_use]
    pub fn hash(&self) -> &HashString {
        &self.hash
    }

    /// Whether there is nothing to show: the torrent has no files yet, or only padding files.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.top.is_empty()
    }

    /// Whether a file that is shown is wanted; padding files do not count. `false` for an empty
    /// tree.
    pub(crate) fn wants_a_shown_file(&self) -> bool {
        self.top
            .iter()
            .filter_map(|&index| self.nodes.get(index))
            .any(|node| node.wanted != WantedState::None)
    }

    /// Whether `key` names a node of this tree.
    #[must_use]
    pub fn contains(&self, key: &NodeKey) -> bool {
        self.by_key.contains_key(key)
    }

    /// The indices of the files an action on the node reaches: the file itself, or every file
    /// below the folder, padding files included.
    ///
    /// It is `None` for a key that is not in the tree, which includes a padding file and a folder
    /// that holds nothing but padding files: those are not shown, so there is nothing to act on.
    /// It never gives an empty list.
    ///
    /// This is for looking at what a node reaches. To change the files, build the action with
    /// [`toggle_wanted_action`](Self::toggle_wanted_action),
    /// [`priority_action`](Self::priority_action) or
    /// [`step_priority_action`](Self::step_priority_action), which keep the files, the torrent
    /// and the value from one tree.
    #[must_use]
    pub fn indices(&self, key: &NodeKey) -> Option<NonEmptyIndices> {
        self.target(key).map(|(_, files)| files)
    }

    /// What a click on the checkbox of the node `key` asks for: the files it reaches, padding
    /// files included, and whether they are to be wanted. All of them are wanted, or none of them
    /// when all are wanted now ([`WantedState::toggled`]).
    ///
    /// Both come from this tree. The details panel sends them to the daemon through
    /// [`toggle_wanted_action`](Self::toggle_wanted_action); the add dialog applies them to its
    /// own choice of files, which nothing has been sent for yet. It is `None` when `key` is not a
    /// node of this tree (a padding file is not).
    #[must_use]
    pub fn wanted_toggle(&self, key: &NodeKey) -> Option<(NonEmptyIndices, bool)> {
        let (node, files) = self.target(key)?;
        Some((files, node.wanted.toggled()))
    }

    /// The action that a click on the checkbox of the node `key` asks for: the
    /// [`wanted_toggle`](Self::wanted_toggle) of the node, for this tree's torrent.
    ///
    /// The torrent, the files and the value all come from this tree, so they cannot belong to
    /// different reads. It is `None` when `key` is not a node of this tree (a padding file is not).
    #[must_use]
    pub fn toggle_wanted_action(&self, key: &NodeKey) -> Option<Action> {
        let (files, wanted) = self.wanted_toggle(key)?;
        Some(Action::SetFilesWanted {
            hash: self.hash.clone(),
            files: files.into_vec(),
            wanted,
        })
    }

    /// The action that gives the files of the node `key` the priority `priority`, as a choice
    /// from the dropdown does.
    ///
    /// The torrent and the files come from this tree. It is `None` when `key` is not a node of
    /// this tree.
    #[must_use]
    pub fn priority_action(&self, key: &NodeKey, priority: FilePriority) -> Option<Action> {
        let (_, files) = self.target(key)?;
        Some(Action::SetFilePriority {
            hash: self.hash.clone(),
            files: files.into_vec(),
            priority,
        })
    }

    /// The action that `+` (`up`) or `-` asks for: the files of the node `key` get the priority
    /// one step higher or lower than the node has now ([`next_priority`]).
    ///
    /// The torrent, the files and the current priority all come from this tree. It is `None` when
    /// `key` is not a node of this tree.
    #[must_use]
    pub fn step_priority_action(&self, key: &NodeKey, up: bool) -> Option<Action> {
        let (node, files) = self.target(key)?;
        Some(Action::SetFilePriority {
            hash: self.hash.clone(),
            files: files.into_vec(),
            priority: next_priority(node.priority, up),
        })
    }

    /// The node `key` names and the files an action on it reaches; `None` for a key that is not
    /// in this tree or a node without files.
    fn target(&self, key: &NodeKey) -> Option<(&Node, NonEmptyIndices)> {
        let node = self.nodes.get(*self.by_key.get(key)?)?;
        Some((node, NonEmptyIndices::new(node.files.clone())?))
    }

    /// The node at `index`.
    pub(super) fn node(&self, index: usize) -> Option<&Node> {
        self.nodes.get(index)
    }

    /// The index of the node `key` names.
    pub(super) fn position(&self, key: &NodeKey) -> Option<usize> {
        self.by_key.get(key).copied()
    }

    /// The nodes at the first level, in display order.
    pub(super) fn top(&self) -> &[usize] {
        &self.top
    }
}
