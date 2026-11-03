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

/// The most memory a [`FileTree`] may take, as it is counted while the tree is made: 32 MiB.
///
/// Each folder and file counts some 300 bytes for itself, and the bytes of its key, which is its
/// whole path, and of its name. Each file is in the list of files of its own node and of every
/// folder above it, and a padding file in those of the shown folders above it; each index a list
/// has room for counts 8 bytes. So a deep path costs the square of its depth in keys, and a file
/// deep down its depth in lists: without a bound, a crafted torrent of a few MB would take
/// hundreds of MiB. The count is close to what a
/// tree takes. Real trees count far less: 5000 files four folders deep, with long names, some
/// 3 MiB, and 50 000 files in one folder some 17.
pub const TREE_BUDGET: usize = 32 * 1024 * 1024;

/// What a folder or a file counts against [`TREE_BUDGET`] besides the bytes of its key and its
/// name and its list of files: the node, its place in the map of keys and in the list of its
/// folder, and the allocations of its key and its name. About what one takes.
const NODE_COST: usize = 320;

/// What a folder counts besides: its list of entries and its place in the map of folders.
const FOLDER_COST: usize = 96;

/// What each index a list of files has room for counts.
const INDEX_COST: usize = size_of::<usize>();

/// The most files a [`FileTree`] is made for, padding files included: 50 000.
///
/// A tree takes some hundreds of bytes for each file, and is made again after each click in the
/// add dialog, so a torrent of half a million tiny files would take hundreds of MiB. Beyond this
/// the Files tab says the list is too large to show, and the add dialog adds the torrent with all
/// its files, without a choice.
pub const MAX_CHOOSABLE_FILES: usize = 50_000;

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
    /// Whether the tree would have taken more than [`TREE_BUDGET`] or had more than
    /// [`MAX_CHOOSABLE_FILES`] files: it is then empty.
    too_large: bool,
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
    /// empty. So is a tree that would be too large to show ([`is_too_large`](Self::is_too_large)).
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
    ///
    /// A tree of more than [`MAX_CHOOSABLE_FILES`] files, or that would take more than
    /// [`TREE_BUDGET`], is not made: the tree is empty, and [`is_too_large`](Self::is_too_large)
    /// says so. Too many files are refused before any work. Each path is read one component at a
    /// time, without a list of them, and a path with more folders than the budget can hold is
    /// refused as soon as they are counted; making the tree stops as soon as the budget is spent.
    /// So the memory stays within about the budget however many files there are and however deep
    /// their paths go, and the work done on a path grows with its length.
    #[must_use]
    pub fn of_files(hash: HashString, files: &[FileEntry]) -> Self {
        Self::of_files_within(hash, files, TREE_BUDGET)
    }

    /// [`of_files`](Self::of_files) within a budget of `budget` bytes instead of
    /// [`TREE_BUDGET`].
    pub(crate) fn of_files_within(hash: HashString, files: &[FileEntry], budget: usize) -> Self {
        if files.len() > MAX_CHOOSABLE_FILES {
            return Self::too_large(hash);
        }
        let mut builder = Builder::new(budget);
        // The padding files themselves, not their parts, which for one deep down are thousands.
        let mut padding = Vec::new();
        for entry in files {
            if builder.over_budget() {
                break;
            }
            if is_padding(parts(&entry.name)) {
                padding.push(entry);
            } else {
                builder.add_file(entry);
            }
        }
        // Padding files join the lists of the folders that are shown, which exist only once all
        // other files have been added.
        for entry in padding {
            if builder.over_budget() {
                break;
            }
            builder.add_padding(entry.index, &entry.name);
        }
        builder.finish(hash)
    }

    /// The empty tree of the torrent `hash` that says it is too large to show.
    fn too_large(hash: HashString) -> Self {
        Self {
            hash,
            nodes: Vec::new(),
            top: Vec::new(),
            by_key: HashMap::new(),
            too_large: true,
        }
    }

    /// The torrent whose details the tree was made from.
    #[must_use]
    pub fn hash(&self) -> &HashString {
        &self.hash
    }

    /// Whether there is nothing to show: the torrent has no files yet, or only padding files, or
    /// the tree is [too large](Self::is_too_large) to show.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.top.is_empty()
    }

    /// Whether the tree was not made because it would be too large: it has more than
    /// [`MAX_CHOOSABLE_FILES`] files, or it would have taken more than [`TREE_BUDGET`], as for
    /// folders nested very deeply. Such a tree is empty, and the Files tab says the list is too
    /// large to show.
    #[must_use]
    pub fn is_too_large(&self) -> bool {
        self.too_large
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

/// The components of `path`, without empty ones, read one by one: a path may have millions, and
/// a list of them would take 16 bytes each before the tree had counted anything.
fn parts(path: &str) -> impl Iterator<Item = &str> {
    path.split('/').filter(|part| !part.is_empty())
}

#[cfg(test)]
mod tests {
    use x4_rpc::FilePriority::Normal;

    use super::*;
    use crate::test_support::details;

    fn hash() -> HashString {
        HashString::parse(&"ab".repeat(20)).unwrap()
    }

    /// The bytes a list of `len` indices has room for, grown one index at a time as the tree
    /// grows it.
    fn list_bytes(len: usize) -> usize {
        let mut list = Vec::new();
        for index in 0..len {
            list.reserve(1);
            list.push(index);
        }
        list.capacity() * INDEX_COST
    }

    #[test]
    fn the_budget_counts_every_node_with_its_key_and_name_and_list() {
        assert_eq!(TREE_BUDGET, 32 * 1024 * 1024);
        let files = details(
            &hash(),
            &[
                ("T/a", 1, 0, true, Normal),
                ("T/b", 1, 0, true, Normal),
                ("T/.pad/2", 1, 0, true, Normal),
                ("T/s/c", 1, 0, true, Normal),
                ("T/a", 1, 0, true, Normal),
            ],
        )
        .files;
        // Six nodes, two of them folders; the padding file makes none. Their keys: "T" (1 byte),
        // "T/a" (3), "T/b" (3), "T/s" and "T/s/c" (3 and 5), and last the repeat of "T/a", whose
        // key is "T/a\01" (5, more than its path): 20 bytes. Their names: 6 bytes. Their lists of
        // files: five indices in that of "T", the padding file's last, and one in each other.
        let cost = 6 * NODE_COST + 2 * FOLDER_COST + 20 + 6 + list_bytes(5) + 5 * list_bytes(1);

        let within = FileTree::of_files_within(hash(), &files, cost);
        assert!(!within.is_too_large());
        assert_eq!(
            within
                .indices(&NodeKey::new("T"))
                .map(NonEmptyIndices::into_vec),
            Some(vec![0, 1, 2, 3, 4])
        );

        let over = FileTree::of_files_within(hash(), &files, cost - 1);
        assert!(over.is_too_large());
        assert!(over.is_empty(), "nothing of it is kept");
        assert_eq!(over.hash(), &hash());
        for key in ["T", "T/a", "T/s/c"] {
            assert_eq!(over.indices(&NodeKey::new(key)), None, "{key}");
            assert_eq!(over.wanted_toggle(&NodeKey::new(key)), None, "{key}");
        }
        assert!(!over.wants_a_shown_file());

        // A tree without files is never too large.
        assert!(!FileTree::of_files_within(hash(), &[], 0).is_too_large());
    }

    #[test]
    fn a_path_is_refused_for_its_count_of_folders_only_when_they_cannot_fit() {
        let files = details(&hash(), &[("T/a", 1, 0, true, Normal)]).files;
        // A folder and a file: their nodes, the keys "T" and "T/a", the names, and a list of one
        // index each. The budget holds one folder's cost and no more, and so does the path.
        let cost = 2 * NODE_COST + FOLDER_COST + 4 + 2 + 2 * list_bytes(1);
        assert_eq!(cost / (NODE_COST + FOLDER_COST), 1);

        assert!(!FileTree::of_files_within(hash(), &files, cost).is_too_large());
        assert!(FileTree::of_files_within(hash(), &files, cost - 1).is_too_large());
    }
}
