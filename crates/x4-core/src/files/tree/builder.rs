//! How the folders and files of a [`FileTree`](super::FileTree) are grown from the flat list of
//! files: one node per folder and file, the sizes of each folder, the padding files in the lists
//! of the folders shown, and every folder's entries in display order. What a read says of them is
//! taken afterwards, for this read and for later ones of the same files.

use std::collections::HashMap;

use super::super::NodeKey;
use super::super::names::name_order;
use super::{FILE_NODE_COST, FOLDER_COST, INDEX_COST, NODE_COST, Node, Shape, parts};
use crate::details::FileEntry;

impl Node {
    fn new(key: NodeKey, name: &str, parent: Option<usize>, is_folder: bool) -> Self {
        Self {
            key,
            name: name.to_owned(),
            parent,
            children: Vec::new(),
            is_folder,
            size: 0,
            files: Vec::new(),
        }
    }

    /// Adds a shown file of `size` bytes, at `index`, to this folder or file.
    fn absorb(&mut self, index: usize, size: u64) {
        self.size = self.size.saturating_add(size);
        self.files.push(index);
    }
}

/// Grows the folders and files one file at a time.
///
/// A folder is made when the first shown file below it is added, so a folder that holds only
/// padding files never exists. The parent of a node always has a smaller index than the node.
///
/// The tree may take `budget` bytes, as the builder counts them: [`NODE_COST`] and the bytes of
/// the key and the name for each node, [`FOLDER_COST`] more for a folder, [`INDEX_COST`] for
/// each index a list of files has room for, [`FILE_NODE_COST`] for each file of the list, and the
/// room of the path being walked. Each key is
/// its node's whole path, so a deep path costs the square of its depth: 20 000 folders one in the
/// other, 40 KB of names, would take some 400 MiB. Each file is in the list of every folder above
/// it, so 40 000 files 180 folders deep would take 60 MiB of lists. Once the next cost would pass
/// the budget, the builder adds nothing more and [`finish`](Self::finish) gives a tree that is too
/// large to show.
pub(super) struct Builder<'a> {
    nodes: Vec<Node>,
    top: Vec<usize>,
    /// The node of each file of the list, at its place in it.
    file_nodes: Vec<Option<usize>>,
    /// The folder with this name in this folder (or at the first level).
    folders: HashMap<(Option<usize>, &'a str), usize>,
    by_key: HashMap<NodeKey, usize>,
    /// For each path that was taken, the suffix its next copy tries first.
    suffixes: HashMap<NodeKey, usize>,
    /// The path of the folder or file being made, one buffer for every file. Its room is counted,
    /// once: it grows to the longest path.
    path: String,
    /// The bytes counted so far. A node and `by_key` share the bytes of its key.
    spent: usize,
    budget: usize,
    /// Whether a cost did not fit in the budget.
    over_budget: bool,
}

impl<'a> Builder<'a> {
    /// A builder of a tree of a list of `files` files that may take `budget` bytes.
    pub(super) fn new(budget: usize, files: usize) -> Self {
        let mut builder = Self {
            nodes: Vec::new(),
            top: Vec::new(),
            file_nodes: Vec::new(),
            folders: HashMap::new(),
            by_key: HashMap::new(),
            suffixes: HashMap::new(),
            path: String::new(),
            spent: 0,
            budget,
            over_budget: false,
        };
        if builder.charge(files.saturating_mul(FILE_NODE_COST)) {
            builder.file_nodes = vec![None; files];
        }
        builder
    }

    /// Whether a cost did not fit in the budget: nothing more is added, and the tree is too large.
    pub(super) fn over_budget(&self) -> bool {
        self.over_budget
    }

    /// Adds a file that is shown, the one at `place` in the list, with the folders above it: the
    /// components of its path, the last one its name. A path without any is a file without a
    /// name, which the user still sees and acts on. Nothing is added once the budget is spent.
    ///
    /// Each folder of a path is a node of its own, which takes at least [`NODE_COST`] and
    /// [`FOLDER_COST`]. A path with more folders than the budget holds of those spends the budget
    /// at once, when its components have been counted and before anything is made: 8 million
    /// folders in a path of 16 MB would otherwise be split into a list of parts of 128 MiB.
    pub(super) fn add_file(&mut self, place: usize, entry: &'a FileEntry) {
        let folders = parts(&entry.name).count().saturating_sub(1);
        if folders > self.budget / (NODE_COST + FOLDER_COST) {
            self.over_budget = true;
            return;
        }
        let mut path = std::mem::take(&mut self.path);
        path.clear();
        self.add_file_at(place, entry, &mut path);
        self.path = path;
    }

    /// [`add_file`](Self::add_file), with `path`, empty, as the buffer of the path walked.
    fn add_file_at(&mut self, place: usize, entry: &'a FileEntry, path: &mut String) {
        let mut rest = parts(&entry.name).peekable();
        let mut parent = None;
        let mut name = "";
        while let Some(part) = rest.next() {
            if !self.push_component(path, part) {
                return;
            }
            if rest.peek().is_none() {
                name = part;
                break;
            }
            let folder = match self.folders.get(&(parent, part)) {
                Some(&folder) => folder,
                None => match self.add_node(parent, part, path, true) {
                    Some(folder) => folder,
                    None => return,
                },
            };
            parent = Some(folder);
        }
        let Some(leaf) = self.add_node(parent, name, path, false) else {
            return;
        };
        if let Some(slot) = self.file_nodes.get_mut(place) {
            *slot = Some(leaf);
        }

        let mut at = Some(leaf);
        while let Some(index) = at {
            if !self.room_for_index(index) {
                return;
            }
            let Some(node) = self.nodes.get_mut(index) else {
                break;
            };
            node.absorb(entry.index, entry.length);
            at = node.parent;
        }
    }

    /// Adds the index of a padding file, at `path`, to the folders above it that exist. The ones
    /// that do not are folders of padding only, which are not shown. Nothing more is added once
    /// the budget is spent.
    pub(super) fn add_padding(&mut self, index: usize, path: &str) {
        let mut rest = parts(path).peekable();
        let mut parent = None;
        while let Some(part) = rest.next() {
            // The last component is the name of the padding file itself.
            if rest.peek().is_none() {
                return;
            }
            let Some(&folder) = self.folders.get(&(parent, part)) else {
                return;
            };
            if !self.room_for_index(folder) {
                return;
            }
            if let Some(node) = self.nodes.get_mut(folder) {
                node.files.push(index);
            }
            parent = Some(folder);
        }
    }

    /// Appends `part` to `path`, with a `/` between them, and counts the room the path grows by
    /// before it grows: `false`, with nothing appended, when that does not fit in the budget. The
    /// room doubles, as a `String`'s does, so that a long path is not copied at every part.
    fn push_component(&mut self, path: &mut String, part: &str) -> bool {
        let separator = if path.is_empty() { "" } else { "/" };
        let len = path
            .len()
            .saturating_add(separator.len())
            .saturating_add(part.len());
        let room = path.capacity();
        if len > room {
            let grown = len.max(room.saturating_mul(2));
            if !self.charge(grown.saturating_sub(room)) {
                return false;
            }
            path.reserve_exact(grown.saturating_sub(path.len()));
            // In case the allocator gave more room than was asked for.
            if !self.charge(path.capacity().saturating_sub(grown)) {
                return false;
            }
        }
        path.push_str(separator);
        path.push_str(part);
        true
    }

    /// Counts `cost` bytes; `false`, with nothing counted and the budget marked spent, when they
    /// do not fit in it.
    fn charge(&mut self, cost: usize) -> bool {
        match self.spent.checked_add(cost) {
            Some(spent) if !self.over_budget && spent <= self.budget => {
                self.spent = spent;
                true
            }
            _ => {
                self.over_budget = true;
                false
            }
        }
    }

    /// Makes room for one more index in the list of files of the node `index`, and counts the
    /// room the list grew by: `false` when that did not fit in the budget, or there is no such
    /// node. A list grows by doubling, so its room is counted, not its indices: 2 000 lists of 725
    /// indices, in which a padding file deep down is, take the room of 1 024 indices each.
    fn room_for_index(&mut self, index: usize) -> bool {
        let Some(node) = self.nodes.get_mut(index) else {
            return false;
        };
        let before = node.files.capacity();
        node.files.reserve(1);
        let grown = node.files.capacity().saturating_sub(before);
        self.charge(grown.saturating_mul(INDEX_COST))
    }

    /// Adds an empty node under `parent` (or at the first level) and returns its index; `None`,
    /// with nothing added, when it does not fit in the budget.
    fn add_node(
        &mut self,
        parent: Option<usize>,
        name: &'a str,
        path: &str,
        is_folder: bool,
    ) -> Option<usize> {
        // A key is never shorter than its path, so a node that cannot fit is refused before its
        // key is made; the suffix of a repeated path is counted once the key is.
        let own = if is_folder {
            NODE_COST.saturating_add(FOLDER_COST)
        } else {
            NODE_COST
        };
        if !self.charge(own.saturating_add(path.len()).saturating_add(name.len())) {
            return None;
        }
        let index = self.nodes.len();
        let key = self.unique_key(path);
        if !self.charge(key.as_str().len().saturating_sub(path.len())) {
            return None;
        }
        self.by_key.insert(key.clone(), index);
        self.nodes.push(Node::new(key, name, parent, is_folder));
        match parent.and_then(|parent| self.nodes.get_mut(parent)) {
            Some(parent) => parent.children.push(index),
            None => self.top.push(index),
        }
        if is_folder {
            self.folders.insert((parent, name), index);
        }
        Some(index)
    }

    /// The key for a node at `path`. A path that is taken (the daemon lists a path twice, or a
    /// file has the name of a folder) gets a suffix that no real path can have, so that an action
    /// on a row never reaches the files of another.
    ///
    /// The copies of a path are numbered from 1 in the order they come. Each path remembers the
    /// number its next copy takes, so that 50 000 copies of one path do not try every number
    /// before theirs: that took minutes.
    fn unique_key(&mut self, path: &str) -> NodeKey {
        let key = NodeKey::new(path);
        let Some((taken, _)) = self.by_key.get_key_value(&key) else {
            return key;
        };
        let next = self.suffixes.entry(taken.clone()).or_insert(1);
        loop {
            let copy = NodeKey::new(&format!("{path}\0{next}"));
            *next += 1;
            // A number is skipped only for a path that holds the suffix itself, as a crafted
            // name may: each such node makes one number be skipped, once.
            if !self.by_key.contains_key(&copy) {
                return copy;
            }
        }
    }

    /// Orders every folder's entries and the first level, and sorts the file lists; `None` once
    /// the budget is spent, for a tree that is too large.
    pub(super) fn finish(mut self) -> Option<Shape> {
        if self.over_budget {
            return None;
        }
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
        Some(Shape {
            nodes: self.nodes,
            top: self.top,
            by_key: self.by_key,
            file_nodes: self.file_nodes,
        })
    }
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
