//! The tree built from the flat list of files, with the sums of every folder.

use std::collections::HashMap;

use x4_rpc::{FilePriority, HashString};

use super::names::{is_padding, name_order};
use super::{NodeKey, NonEmptyIndices, PriorityView, WantedState, next_priority};
use crate::action::Action;
use crate::details::{FileEntry, TorrentDetails};
use crate::playable::Playable;

/// The files of one torrent's details as folders and files.
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
        let mut builder = Builder::default();
        let mut padding = Vec::new();
        for entry in &details.files {
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
        builder.finish(details.hash.clone())
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

    /// The action that a click on the checkbox of the node `key` asks for: all its files are
    /// wanted, or none of them when all are wanted now ([`WantedState::toggled`]).
    ///
    /// The torrent, the files and the value all come from this tree, so they cannot belong to
    /// different reads. It is `None` when `key` is not a node of this tree (a padding file is not).
    #[must_use]
    pub fn toggle_wanted_action(&self, key: &NodeKey) -> Option<Action> {
        let (node, files) = self.target(key)?;
        Some(Action::SetFilesWanted {
            hash: self.hash.clone(),
            files: files.into_vec(),
            wanted: node.wanted.toggled(),
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

/// What a shown file adds to every folder above it.
struct Shown {
    index: usize,
    size: u64,
    done: u64,
    wanted: WantedState,
    priority: PriorityView,
}

impl Node {
    fn new(key: NodeKey, name: &str, parent: Option<usize>, is_folder: bool) -> Self {
        Self {
            key,
            name: name.to_owned(),
            parent,
            children: Vec::new(),
            is_folder,
            size: 0,
            done: 0,
            // Replaced by the first file that is added; a node exists only for a shown file.
            wanted: WantedState::All,
            priority: PriorityView::One(FilePriority::Normal),
            files: Vec::new(),
            playable: None,
        }
    }

    /// Adds a shown file to the sums of this folder.
    fn absorb(&mut self, file: &Shown) {
        if self.files.is_empty() {
            self.wanted = file.wanted;
            self.priority = file.priority;
        } else {
            if self.wanted != file.wanted {
                self.wanted = WantedState::Mixed;
            }
            if self.priority != file.priority {
                self.priority = PriorityView::Mixed;
            }
        }
        self.size = self.size.saturating_add(file.size);
        self.done = self.done.saturating_add(file.done);
        self.files.push(file.index);
    }
}

/// Grows the tree one file at a time.
///
/// A folder is made when the first shown file below it is added, so a folder that holds only
/// padding files never exists. The parent of a node always has a smaller index than the node.
#[derive(Default)]
struct Builder<'a> {
    nodes: Vec<Node>,
    top: Vec<usize>,
    /// The folder with this name in this folder (or at the first level).
    folders: HashMap<(Option<usize>, &'a str), usize>,
    by_key: HashMap<NodeKey, usize>,
}

impl<'a> Builder<'a> {
    /// Adds a file that is shown, with the folders above it. `parts` are the components of its
    /// path, the last one its name.
    fn add_file(&mut self, entry: &FileEntry, parts: &[&'a str]) {
        let Some((&name, folder_parts)) = parts.split_last() else {
            return;
        };
        let mut parent = None;
        let mut path = String::new();
        for &part in folder_parts {
            push_component(&mut path, part);
            let folder = match self.folders.get(&(parent, part)) {
                Some(&folder) => folder,
                None => self.add_node(parent, part, &path, true),
            };
            parent = Some(folder);
        }
        push_component(&mut path, name);
        let leaf = self.add_node(parent, name, &path, false);

        let done = entry.bytes_completed.min(entry.length);
        let shown = Shown {
            index: entry.index,
            size: entry.length,
            done,
            wanted: if entry.wanted {
                WantedState::All
            } else {
                WantedState::None
            },
            priority: PriorityView::One(entry.priority),
        };
        if let Some(node) = self.nodes.get_mut(leaf) {
            node.absorb(&shown);
            // Only a file that is being downloaded has a use for a prefix to play, and only once
            // some of it plays: "0 B" would say nothing.
            node.playable = (entry.wanted && done < entry.length && entry.playable_from_start > 0)
                .then(|| Playable::of(entry.playable_from_start, entry.length));
        }
        let mut above = parent;
        while let Some(index) = above {
            let Some(folder) = self.nodes.get_mut(index) else {
                break;
            };
            folder.absorb(&shown);
            above = folder.parent;
        }
    }

    /// Adds the index of a padding file to the folders above it that exist. The ones that do not
    /// are folders of padding only, which are not shown.
    fn add_padding(&mut self, index: usize, parts: &[&'a str]) {
        let Some((_, folder_parts)) = parts.split_last() else {
            return;
        };
        let mut parent = None;
        for &part in folder_parts {
            let Some(&folder) = self.folders.get(&(parent, part)) else {
                return;
            };
            if let Some(node) = self.nodes.get_mut(folder) {
                node.files.push(index);
            }
            parent = Some(folder);
        }
    }

    /// Adds an empty node under `parent` (or at the first level) and returns its index.
    fn add_node(
        &mut self,
        parent: Option<usize>,
        name: &'a str,
        path: &str,
        is_folder: bool,
    ) -> usize {
        let index = self.nodes.len();
        let key = self.unique_key(path);
        self.by_key.insert(key.clone(), index);
        self.nodes.push(Node::new(key, name, parent, is_folder));
        match parent.and_then(|parent| self.nodes.get_mut(parent)) {
            Some(parent) => parent.children.push(index),
            None => self.top.push(index),
        }
        if is_folder {
            self.folders.insert((parent, name), index);
        }
        index
    }

    /// The key for a node at `path`. A path that is taken (the daemon lists a path twice, or a
    /// file has the name of a folder) gets a suffix that no real path can have, so that an action
    /// on a row never reaches the files of another.
    fn unique_key(&self, path: &str) -> NodeKey {
        let mut key = NodeKey::new(path);
        let mut copy = 1_usize;
        while self.by_key.contains_key(&key) {
            key = NodeKey::new(&format!("{path}\0{copy}"));
            copy += 1;
        }
        key
    }

    /// Orders every folder's entries and the first level, and sorts the file lists.
    fn finish(mut self, hash: HashString) -> FileTree {
        for node in &mut self.nodes {
            node.files.sort_unstable();
        }
        for index in 0..self.nodes.len() {
            let Some(node) = self.nodes.get_mut(index) else {
                continue;
            };
            let mut children = std::mem::take(&mut node.children);
            sort_entries(&self.nodes, &mut children);
            if let Some(node) = self.nodes.get_mut(index) {
                node.children = children;
            }
        }
        sort_entries(&self.nodes, &mut self.top);
        FileTree {
            hash,
            nodes: self.nodes,
            top: self.top,
            by_key: self.by_key,
        }
    }
}

/// Appends `part` to `path`, with a `/` between them.
fn push_component(path: &mut String, part: &str) {
    if !path.is_empty() {
        path.push('/');
    }
    path.push_str(part);
}

/// Puts folders before files and each kind in the order of [`name_order`]; entries with the same
/// name stay in the order of the torrent.
fn sort_entries(nodes: &[Node], entries: &mut [usize]) {
    entries.sort_by(|&a, &b| match (nodes.get(a), nodes.get(b)) {
        (Some(a), Some(b)) => b
            .is_folder
            .cmp(&a.is_folder)
            .then_with(|| name_order(&a.name, &b.name)),
        _ => std::cmp::Ordering::Equal,
    });
}
