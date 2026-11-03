//! The tree built from the flat list of files, with the sums of every folder.

mod builder;

use std::collections::HashMap;
use std::sync::Arc;

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
/// name and its list of files: the node, what a read says of it and its wanted state, its place
/// in the map of keys and in the list of its folder, and the allocations of its key and its name.
/// About what one takes.
const NODE_COST: usize = 320;

/// What a folder counts besides: its list of entries and its place in the map of folders.
const FOLDER_COST: usize = 96;

/// What each index a list of files has room for counts.
const INDEX_COST: usize = size_of::<usize>();

/// What each file of the list counts for the node of its own that the tree keeps
/// ([`Shape::file_nodes`]).
const FILE_NODE_COST: usize = size_of::<Option<usize>>();

/// The most files a [`FileTree`] is made for, padding files included: 50 000.
///
/// A tree takes some hundreds of bytes for each file, and is made again whenever a read of the
/// details lists other files, so a torrent of half a million tiny files would take hundreds of
/// MiB. Beyond this
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
/// A tree is a snapshot of one read of the details, and the keys of the nodes stay the same from
/// read to read. It remembers the torrent it was made for, so an action drawn from it names that
/// torrent however the details have changed since. A read that lists the same files keeps the
/// folders and files of the tree before ([`refreshed`](Self::refreshed)): only the bytes done, the
/// priorities, what plays and which files are wanted are taken again, and nothing is sorted. A
/// copy of a tree shares its folders and files and what the read says of them, and has its own
/// wanted states, the one thing that a click in the add dialog changes; so copying costs a byte
/// for each node.
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
    /// The folders and files, which stay as they were made.
    shape: Arc<Shape>,
    /// What the read says of each node, at its index.
    progress: Arc<Vec<Progress>>,
    /// Whether the files of each node are wanted, at the index of the node.
    wanted: Vec<WantedState>,
    /// Whether the tree would have taken more than [`TREE_BUDGET`] or had more than
    /// [`MAX_CHOOSABLE_FILES`] files: it is then empty.
    too_large: bool,
    /// The details the tree was made from, when it was made from details
    /// ([`new`](Self::new)): a later read is compared with them.
    made_from: Option<Arc<TorrentDetails>>,
}

/// The folders and files of a [`FileTree`], what depends only on the list of files: their names,
/// their lengths and their order. Shared by its copies, and by the trees of later reads of the
/// same files.
#[derive(Debug, Default)]
struct Shape {
    nodes: Vec<Node>,
    /// The nodes at the first level, in display order.
    top: Vec<usize>,
    by_key: HashMap<NodeKey, usize>,
    /// The node of each file of the list the tree was made of, at the file's place in the list;
    /// `None` for a padding file, which has none.
    file_nodes: Vec<Option<usize>>,
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
    /// The indices of all files below, padding files included, or the file's own; ascending.
    pub(super) files: Vec<usize>,
}

/// What one read of the files says of a folder or a file.
#[derive(Debug, Clone, Copy)]
pub(super) struct Progress {
    /// The bytes of the shown files below, or of the file, that are downloaded, never more than
    /// the node's size.
    pub(super) done: u64,
    pub(super) priority: PriorityView,
    /// What plays from the start; a file only, and only while it is incomplete and some of its
    /// start has come. It is shown only while the file is wanted.
    pub(super) playable: Option<Playable>,
}

impl Progress {
    /// What a read of `entry` says of its file.
    fn of_file(entry: &FileEntry) -> Self {
        let done = entry.bytes_completed.min(entry.length);
        Self {
            done,
            priority: PriorityView::One(entry.priority),
            // Only a file that is being downloaded has a use for a prefix to play, and only once
            // some of it plays: "0 B" would say nothing. Whether it is wanted is up to the row.
            playable: (done < entry.length && entry.playable_from_start > 0)
                .then(|| Playable::of(entry.playable_from_start, entry.length)),
        }
    }
}

impl FileTree {
    /// The tree of `details.files`. A magnet link without metadata has no files, so its tree is
    /// empty. So is a tree that would be too large to show ([`is_too_large`](Self::is_too_large)).
    ///
    /// The tree keeps `details`, so that the tree of a later read of the same files can keep its
    /// folders and files ([`refreshed`](Self::refreshed)).
    #[must_use]
    pub fn new(details: &Arc<TorrentDetails>) -> Self {
        let mut tree = Self::of_files(details.hash.clone(), &details.files);
        tree.made_from = Some(Arc::clone(details));
        tree
    }

    /// The tree of `details`, a later read of the torrent this tree was made from, when they list
    /// the same files as the details this tree was made from ([`new`](Self::new)): the same
    /// torrent, and the same index, name and length of every file, in the same order. It keeps
    /// the folders and files of this tree, and takes only what the read says of them: the bytes
    /// done, the priorities, what plays and which files are wanted. So it costs a comparison of
    /// the lists and a pass over the nodes, without sorting or comparing a name: a tree of 50 000
    /// files took 20 to 26 ms to make at every read, and crafted names up to 1.8 s.
    ///
    /// It is `None` when the files are not the same, or this tree was not made from details: the
    /// tree of `details` is then made anew with [`new`](Self::new). A list that is the same but
    /// for one name is another list, since the files that an action on a node reaches must be
    /// those the daemon lists under it.
    #[must_use]
    pub fn refreshed(&self, details: &Arc<TorrentDetails>) -> Option<Self> {
        let made_from = self.made_from.as_ref()?;
        if !same_files(made_from, details) {
            return None;
        }
        let (progress, wanted) = if self.too_large {
            (Arc::clone(&self.progress), Vec::new())
        } else {
            read_files(&self.shape, &details.files)
        };
        Some(Self {
            hash: self.hash.clone(),
            shape: Arc::clone(&self.shape),
            progress,
            wanted,
            too_large: self.too_large,
            made_from: Some(Arc::clone(details)),
        })
    }

    /// Whether this tree was made from `details` themselves ([`new`](Self::new),
    /// [`refreshed`](Self::refreshed)), not from a read of them that only looks the same.
    #[must_use]
    pub fn is_made_from(&self, details: &Arc<TorrentDetails>) -> bool {
        self.made_from
            .as_ref()
            .is_some_and(|made_from| Arc::ptr_eq(made_from, details))
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
        let mut builder = Builder::new(budget, files.len());
        // The padding files themselves, not their parts, which for one deep down are thousands.
        let mut padding = Vec::new();
        for (place, entry) in files.iter().enumerate() {
            if builder.over_budget() {
                break;
            }
            if is_padding(parts(&entry.name)) {
                padding.push(entry);
            } else {
                builder.add_file(place, entry);
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
        let Some(shape) = builder.finish() else {
            return Self::too_large(hash);
        };
        let (progress, wanted) = read_files(&shape, files);
        Self {
            hash,
            shape: Arc::new(shape),
            progress,
            wanted,
            too_large: false,
            made_from: None,
        }
    }

    /// The empty tree of the torrent `hash` that says it is too large to show.
    fn too_large(hash: HashString) -> Self {
        Self {
            hash,
            shape: Arc::default(),
            progress: Arc::default(),
            wanted: Vec::new(),
            too_large: true,
            made_from: None,
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
        self.shape.top.is_empty()
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
        self.shape.top.iter().any(|&index| {
            self.wanted(index)
                .is_some_and(|wanted| wanted != WantedState::None)
        })
    }

    /// Makes the files of the node `key` all wanted, or none of them, as a click on its checkbox
    /// in the add dialog does, and gives `false`, changing nothing, when `key` is not a node of
    /// this tree.
    ///
    /// The node and every node below it take the new state, and each folder above it the state
    /// of its entries, which is the state of the files below it; nothing else changes. So the
    /// tree is as one made again from the files with that choice would be, without being made
    /// again: a click costs no more than the nodes it reaches, and the folders above them.
    pub(crate) fn set_wanted(&mut self, key: &NodeKey, wanted: bool) -> bool {
        let Some(&clicked) = self.shape.by_key.get(key) else {
            return false;
        };
        let state = if wanted {
            WantedState::All
        } else {
            WantedState::None
        };
        let (shape, states) = (&*self.shape, &mut self.wanted);
        let mut below = vec![clicked];
        while let Some(index) = below.pop() {
            if let Some(slot) = states.get_mut(index) {
                *slot = state;
            }
            if let Some(node) = shape.nodes.get(index) {
                below.extend_from_slice(&node.children);
            }
        }
        let mut above = shape.nodes.get(clicked).and_then(|node| node.parent);
        while let Some(index) = above {
            let Some(folder) = shape.nodes.get(index) else {
                break;
            };
            let entries = folder
                .children
                .iter()
                .filter_map(|&child| states.get(child));
            let state = sum_of(entries.copied());
            let Some(slot) = states.get_mut(index) else {
                break;
            };
            // The folders further up have the states of their entries already.
            if *slot == state {
                break;
            }
            *slot = state;
            above = folder.parent;
        }
        true
    }

    /// Whether `key` names a node of this tree.
    #[must_use]
    pub fn contains(&self, key: &NodeKey) -> bool {
        self.shape.by_key.contains_key(key)
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
        self.target(key).map(|(_, _, files)| files)
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
        let (index, _, files) = self.target(key)?;
        Some((files, self.wanted(index)?.toggled()))
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
        let (_, _, files) = self.target(key)?;
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
        let (index, _, files) = self.target(key)?;
        Some(Action::SetFilePriority {
            hash: self.hash.clone(),
            files: files.into_vec(),
            priority: next_priority(self.progress(index)?.priority, up),
        })
    }

    /// The node `key` names, at its index, and the files an action on it reaches; `None` for a
    /// key that is not in this tree or a node without files.
    fn target(&self, key: &NodeKey) -> Option<(usize, &Node, NonEmptyIndices)> {
        let index = *self.shape.by_key.get(key)?;
        let node = self.shape.nodes.get(index)?;
        Some((index, node, NonEmptyIndices::new(node.files.clone())?))
    }

    /// The node at `index`.
    pub(super) fn node(&self, index: usize) -> Option<&Node> {
        self.shape.nodes.get(index)
    }

    /// What the read says of the node at `index`.
    pub(super) fn progress(&self, index: usize) -> Option<&Progress> {
        self.progress.get(index)
    }

    /// Whether the files of the node at `index` are wanted.
    pub(super) fn wanted(&self, index: usize) -> Option<WantedState> {
        self.wanted.get(index).copied()
    }

    /// The index of the node `key` names.
    pub(super) fn position(&self, key: &NodeKey) -> Option<usize> {
        self.shape.by_key.get(key).copied()
    }

    /// The nodes at the first level, in display order.
    pub(super) fn top(&self) -> &[usize] {
        &self.shape.top
    }

    /// Whether this tree and `other` share their folders and files, as a tree does with its
    /// copies and with the trees [`refreshed`](Self::refreshed) from it: whether one was made
    /// without making the other again. For tests (feature `test-support`).
    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn shares_files_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shape, &other.shape)
    }
}

/// Whether the details `before` and `after` list the same files of the same torrent: the same
/// hash, and the same index, name and length of each file, in the same order. The tree of one is
/// then the tree of the other but for what a read changes. The lists are compared in full, not by
/// a sum of them: a tree kept by mistake would send the actions of a row to other files.
fn same_files(before: &TorrentDetails, after: &TorrentDetails) -> bool {
    before.hash == after.hash
        && before.files.len() == after.files.len()
        && before.files.iter().zip(&after.files).all(|(was, is)| {
            was.index == is.index && was.length == is.length && was.name == is.name
        })
}

/// What a read of `files`, the list that `shape` was made of, says of each node, and whether the
/// files of each are wanted, at the index of the node: a file's from its entry, and a folder's
/// from its entries, in one pass from the last node to the first, as a node comes after its
/// folder. A folder sums the bytes done of its entries and takes their priority and their wanted
/// state when all agree, as it does those of the files below it.
fn read_files(shape: &Shape, files: &[FileEntry]) -> (Arc<Vec<Progress>>, Vec<WantedState>) {
    let count = shape.nodes.len();
    let mut progress = vec![Progress::EMPTY; count];
    let mut wanted = vec![WantedState::None; count];
    for (entry, node) in files.iter().zip(&shape.file_nodes) {
        let Some(index) = *node else {
            continue;
        };
        if let (Some(slot), Some(state)) = (progress.get_mut(index), wanted.get_mut(index)) {
            *slot = Progress::of_file(entry);
            *state = if entry.wanted {
                WantedState::All
            } else {
                WantedState::None
            };
        }
    }
    for index in (0..count).rev() {
        let Some(folder) = shape.nodes.get(index).filter(|node| node.is_folder) else {
            continue;
        };
        let mut done: u64 = 0;
        let mut priority = None;
        for entry in folder
            .children
            .iter()
            .filter_map(|&child| progress.get(child))
        {
            done = done.saturating_add(entry.done);
            priority = Some(match priority {
                Some(seen) if seen != entry.priority => PriorityView::Mixed,
                _ => entry.priority,
            });
        }
        let state = sum_of(
            folder
                .children
                .iter()
                .filter_map(|&child| wanted.get(child).copied()),
        );
        if let (Some(slot), Some(slot_state)) = (progress.get_mut(index), wanted.get_mut(index)) {
            *slot = Progress {
                done,
                priority: priority.unwrap_or(Progress::EMPTY.priority),
                playable: None,
            };
            *slot_state = state;
        }
    }
    (Arc::new(progress), wanted)
}

impl Progress {
    /// What a node with nothing below it would show.
    const EMPTY: Self = Self {
        done: 0,
        priority: PriorityView::One(FilePriority::Normal),
        playable: None,
    };
}

/// The state of a folder whose entries are in the states `entries`: all wanted, none, or mixed,
/// as the files below them are.
fn sum_of(mut entries: impl Iterator<Item = WantedState>) -> WantedState {
    let Some(first) = entries.next() else {
        return WantedState::None;
    };
    if entries.all(|entry| entry == first) {
        first
    } else {
        WantedState::Mixed
    }
}

/// The components of `path`, without empty ones, read one by one: a path may have millions, and
/// a list of them would take 16 bytes each before the tree had counted anything.
fn parts(path: &str) -> impl Iterator<Item = &str> {
    path.split('/').filter(|part| !part.is_empty())
}

#[cfg(test)]
mod tests {
    use x4_rpc::FilePriority::{High, Low, Normal};

    use super::super::FileTreeView;
    use super::super::names::work;
    use super::*;
    use crate::test_support::details;

    /// A file as `test_support::details` takes it: path, length, bytes done, wanted, priority.
    type Spec<'a> = (&'a str, u64, u64, bool, FilePriority);

    /// A season of a show and its extras, two folders deep, with a padding file.
    const SEASON: [Spec<'static>; 6] = [
        ("Show/Season 1/E02.mkv", 100, 0, true, Normal),
        ("Show/Season 1/E10.mkv", 100, 0, true, Normal),
        ("Show/Season 1/E01.mkv", 100, 0, true, Normal),
        ("Show/.pad/7", 3, 0, true, Normal),
        ("Show/Extras/Making of.mkv", 50, 0, true, Normal),
        ("Show/info.nfo", 1, 0, true, Normal),
    ];

    /// A read of the details of `files`, a new one each time, as the service sends.
    fn read(files: &[Spec<'_>]) -> Arc<TorrentDetails> {
        Arc::new(details(&hash(), files))
    }

    /// Every row of `tree`, with every folder open.
    fn all_rows(tree: &FileTree) -> Vec<super::super::FileRow> {
        let mut view = FileTreeView::new(tree);
        for key in ["Show/Season 1", "Show/Extras"] {
            view.toggle_expanded(tree, &NodeKey::new(key));
        }
        view.rows(tree)
    }

    #[test]
    fn a_read_of_the_same_files_keeps_the_tree_and_compares_no_name() {
        let first = FileTree::new(&read(&SEASON));
        let mut later = SEASON;
        later[0] = ("Show/Season 1/E02.mkv", 100, 60, true, High);
        later[1] = ("Show/Season 1/E10.mkv", 100, 100, false, Normal);
        later[4] = ("Show/Extras/Making of.mkv", 50, 10, true, Low);
        let mut progressed = details(&hash(), &later);
        if let Some(file) = progressed.files.first_mut() {
            file.playable_from_start = 40;
        }
        let progressed = Arc::new(progressed);

        let before = work::so_far();
        let refreshed = first.refreshed(&progressed);
        let after = work::so_far();

        let refreshed = refreshed.expect("the same files");
        assert_eq!(after.comparisons, before.comparisons, "nothing is sorted");
        assert!(refreshed.shares_files_with(&first));
        assert!(refreshed.is_made_from(&progressed));
        // What the read changed shows as in a tree made of it, which does compare names.
        let fresh = FileTree::new(&progressed);
        assert!(work::so_far().comparisons > after.comparisons);
        assert!(!fresh.shares_files_with(&first));
        assert_eq!(all_rows(&refreshed), all_rows(&fresh));
        assert_ne!(all_rows(&refreshed), all_rows(&first));
        for key in [
            "Show",
            "Show/Season 1",
            "Show/Season 1/E02.mkv",
            "Show/Extras",
        ] {
            let key = NodeKey::new(key);
            assert_eq!(
                refreshed.toggle_wanted_action(&key),
                fresh.toggle_wanted_action(&key),
                "{key:?}"
            );
            assert_eq!(
                refreshed.step_priority_action(&key, true),
                fresh.step_priority_action(&key, true),
                "{key:?}"
            );
        }
    }

    #[test]
    fn a_changed_name_length_index_count_or_torrent_makes_a_new_tree() {
        let first = FileTree::new(&read(&SEASON));
        let changed = |change: &dyn Fn(&mut TorrentDetails)| {
            let mut details = details(&hash(), &SEASON);
            change(&mut details);
            first.refreshed(&Arc::new(details)).is_none()
        };

        assert!(changed(&|details| {
            details.files[1].name = "Show/Season 1/E11.mkv".to_owned();
        }));
        assert!(changed(&|details| details.files[4].length = 51));
        assert!(changed(&|details| details.files[2].index = 9));
        assert!(changed(&|details| {
            details.files.pop();
        }));
        assert!(changed(&|details| {
            let extra = details.files[5].clone();
            details.files.push(extra);
        }));
        assert!(changed(&|details| details.files.swap(0, 2)));
        assert!(changed(&|details| {
            details.hash = HashString::parse(&"cd".repeat(20)).unwrap();
        }));
        // The same files, read again, keep it.
        assert!(!changed(&|_| {}));

        // A tree of a bare list of files, as the add dialog makes, is not made from details.
        let bare = FileTree::of_files(hash(), &read(&SEASON).files);
        assert!(bare.refreshed(&read(&SEASON)).is_none());
    }

    #[test]
    fn a_tree_too_large_to_show_stays_so_for_the_same_files() {
        let paths: Vec<String> = (0..=MAX_CHOOSABLE_FILES)
            .map(|n| format!("T/{n}"))
            .collect();
        let files: Vec<Spec<'_>> = paths
            .iter()
            .map(|path| (path.as_str(), 1, 0, true, Normal))
            .collect();
        let first = FileTree::new(&read(&files));
        assert!(first.is_too_large());

        let refreshed = first.refreshed(&read(&files)).expect("the same files");

        assert!(refreshed.is_too_large());
        assert!(refreshed.is_empty());
    }

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
    fn the_budget_counts_every_node_with_its_key_and_name_and_list_and_the_path_walked() {
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
        // files: five indices in that of "T", the padding file's last, and one in each other. The
        // path walked, one buffer for all files: room for 1 byte, then 3 for "T/a", then 6 for
        // "T/s/c", twice the 3 it had. And the node of each of the five files of the list.
        let cost = 6 * NODE_COST
            + 2 * FOLDER_COST
            + 20
            + 6
            + list_bytes(5)
            + 5 * list_bytes(1)
            + 6
            + 5 * FILE_NODE_COST;

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
        // A folder and a file: their nodes, the keys "T" and "T/a", the names, a list of one
        // index each, the 3 bytes of the path walked, and the node of the one file of the list.
        // The budget holds one folder's cost and no more, and so does the path.
        let cost = 2 * NODE_COST + FOLDER_COST + 4 + 2 + 2 * list_bytes(1) + 3 + FILE_NODE_COST;
        assert_eq!(cost / (NODE_COST + FOLDER_COST), 1);

        assert!(!FileTree::of_files_within(hash(), &files, cost).is_too_large());
        assert!(FileTree::of_files_within(hash(), &files, cost - 1).is_too_large());
    }
}
