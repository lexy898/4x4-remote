//! How a [`FileTree`] is grown from the flat list of files: one node per folder and file, the
//! sums of each folder, the padding files in the lists of the folders shown, and every folder's
//! entries in display order.

use std::collections::HashMap;

use x4_rpc::{FilePriority, HashString};

use super::super::names::name_order;
use super::super::{NodeKey, PriorityView, WantedState};
use super::{FileTree, Node};
use crate::details::FileEntry;
use crate::playable::Playable;

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
///
/// The keys of the nodes may take `budget` bytes together. Each key is its node's whole path, so
/// a deep path costs the square of its depth: 20 000 folders one in the other, 40 KB of names,
/// would take some 400 MiB. Once the next key would pass the budget, the builder makes no more
/// nodes and [`finish`](Self::finish) gives a tree that is too large to show.
pub(super) struct Builder<'a> {
    nodes: Vec<Node>,
    top: Vec<usize>,
    /// The folder with this name in this folder (or at the first level).
    folders: HashMap<(Option<usize>, &'a str), usize>,
    by_key: HashMap<NodeKey, usize>,
    /// For each path that was taken, the suffix its next copy tries first.
    suffixes: HashMap<NodeKey, usize>,
    /// The bytes of the keys made so far. A node and `by_key` share the bytes of its key.
    key_bytes: usize,
    budget: usize,
    /// Whether a key did not fit in the budget.
    over_budget: bool,
}

impl<'a> Builder<'a> {
    /// A builder whose keys may take `budget` bytes together.
    pub(super) fn new(budget: usize) -> Self {
        Self {
            nodes: Vec::new(),
            top: Vec::new(),
            folders: HashMap::new(),
            by_key: HashMap::new(),
            suffixes: HashMap::new(),
            key_bytes: 0,
            budget,
            over_budget: false,
        }
    }

    /// Whether a key did not fit in the budget: nothing more is added, and the tree is too large.
    pub(super) fn over_budget(&self) -> bool {
        self.over_budget
    }

    /// Adds a file that is shown, with the folders above it. `parts` are the components of its
    /// path, the last one its name. Nothing is added once the budget is spent.
    pub(super) fn add_file(&mut self, entry: &FileEntry, parts: &[&'a str]) {
        let Some((&name, folder_parts)) = parts.split_last() else {
            return;
        };
        let mut parent = None;
        let mut path = String::new();
        for &part in folder_parts {
            push_component(&mut path, part);
            let folder = match self.folders.get(&(parent, part)) {
                Some(&folder) => folder,
                None => match self.add_node(parent, part, &path, true) {
                    Some(folder) => folder,
                    None => return,
                },
            };
            parent = Some(folder);
        }
        push_component(&mut path, name);
        let Some(leaf) = self.add_node(parent, name, &path, false) else {
            return;
        };

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
    pub(super) fn add_padding(&mut self, index: usize, parts: &[&'a str]) {
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

    /// Adds an empty node under `parent` (or at the first level) and returns its index; `None`,
    /// with nothing added, when its key does not fit in the budget.
    fn add_node(
        &mut self,
        parent: Option<usize>,
        name: &'a str,
        path: &str,
        is_folder: bool,
    ) -> Option<usize> {
        // A key is never shorter than its path, so a path that cannot fit is refused before its
        // key is made; the exact count below takes the suffix of a repeated path into account.
        if self.over_budget || self.budget.saturating_sub(self.key_bytes) < path.len() {
            self.over_budget = true;
            return None;
        }
        let index = self.nodes.len();
        let key = self.unique_key(path);
        match self.key_bytes.checked_add(key.as_str().len()) {
            Some(total) if total <= self.budget => self.key_bytes = total,
            _ => {
                self.over_budget = true;
                return None;
            }
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

    /// Orders every folder's entries and the first level, and sorts the file lists; or, once
    /// the budget is spent, gives an empty tree that says it is too large.
    pub(super) fn finish(mut self, hash: HashString) -> FileTree {
        if self.over_budget {
            return FileTree::too_large(hash);
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
        FileTree {
            hash,
            nodes: self.nodes,
            top: self.top,
            by_key: self.by_key,
            too_large: false,
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
