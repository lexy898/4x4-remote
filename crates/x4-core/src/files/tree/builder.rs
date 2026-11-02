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
#[derive(Default)]
pub(super) struct Builder<'a> {
    nodes: Vec<Node>,
    top: Vec<usize>,
    /// The folder with this name in this folder (or at the first level).
    folders: HashMap<(Option<usize>, &'a str), usize>,
    by_key: HashMap<NodeKey, usize>,
}

impl<'a> Builder<'a> {
    /// Adds a file that is shown, with the folders above it. `parts` are the components of its
    /// path, the last one its name.
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
    pub(super) fn finish(mut self, hash: HashString) -> FileTree {
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
