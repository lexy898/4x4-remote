//! What the user has done to a tree, and the rows it shows.

use std::collections::HashSet;

use x4_rpc::HashString;

use super::tree::Node;
use super::{FileTree, NodeKey, PriorityView, WantedState};
use crate::format::{HumanSize, human_size};
use crate::playable::Playable;
use crate::view::Edge;
use crate::view::percent::{fraction_of, percent};

/// What a row of the file tree is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// A folder, open or closed.
    Folder {
        /// Whether the folder's entries are listed below it.
        expanded: bool,
    },
    /// A file.
    File,
}

/// One line of the file tree, decided; the view only draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct FileRow {
    /// The node the row shows, which an action is found by.
    pub key: NodeKey,
    /// How many folders are above it; the torrent's top folder is 0.
    pub depth: usize,
    /// Whether it is a folder, and whether that is open.
    pub kind: RowKind,
    /// The name of the file or folder, without the path above it.
    pub name: String,
    /// The size of the file, or of the files below the folder.
    pub size: HumanSize,
    /// Progress in whole percent; 100 only when everything is downloaded.
    pub percent: u8,
    /// Progress limited to 0.0 to 1.0, for the width of the bar.
    pub progress: f64,
    /// Whether it is wanted; a folder can be partly.
    pub wanted: WantedState,
    /// The priority; a folder can have mixed ones.
    pub priority: PriorityView,
    /// What plays from the start of the file; `None` for a folder, a complete file, a file that
    /// is not wanted and a file of which nothing plays yet.
    pub playable: Option<Playable>,
}

/// Which folders of a [`FileTree`] are open and where the cursor is.
///
/// Both are kept by [`NodeKey`], so the state stays valid when the tree is made again; call
/// [`retain`](Self::retain) with the new tree.
///
/// A view belongs to one torrent: the one of the first tree it is given. A tree of another
/// torrent starts it afresh, with every folder closed but the first level and no cursor, so the
/// state of one torrent cannot reach the files of another that has the same paths.
#[derive(Debug, Clone, Default)]
pub struct FileTreeView {
    /// The torrent the state below belongs to; `None` until the first tree, when the state is
    /// still empty.
    hash: Option<HashString>,
    expanded: HashSet<NodeKey>,
    cursor: Option<NodeKey>,
    /// Whether the folders at the first level were opened for the first time. A magnet link has
    /// no files until its metadata arrives, and they open then.
    opened_first_level: bool,
}

impl FileTreeView {
    /// A view of `tree` as it is first opened: the folders at the first level, which is the
    /// torrent's top folder, are open, and the cursor is nowhere.
    #[must_use]
    pub fn new(tree: &FileTree) -> Self {
        let mut view = Self::default();
        view.retain(tree);
        view
    }

    /// The rows to draw, in order: each folder is followed by its entries while it is open, and
    /// the entries of a folder are the folders first, then the files, each by name with numbers
    /// in their natural order.
    ///
    /// For a tree of another torrent than the view's, these are the rows of a view that is new,
    /// which is what the next call that takes the tree makes of this one.
    #[must_use]
    pub fn rows(&self, tree: &FileTree) -> Vec<FileRow> {
        if !self.belongs_to(tree) {
            return Self::new(tree).rows(tree);
        }
        self.shown(tree)
            .into_iter()
            .filter_map(|(index, depth)| Some(self.row(tree.node(index)?, depth)))
            .collect()
    }

    /// The node of `tree` the cursor is on, if any.
    ///
    /// For a tree of another torrent than the view's it is `None`, as in a view that is new: the
    /// same path in another torrent is another file, so the cursor never reaches it.
    #[must_use]
    pub fn cursor(&self, tree: &FileTree) -> Option<&NodeKey> {
        if !self.belongs_to(tree) {
            return None;
        }
        self.cursor.as_ref()
    }

    /// Whether the folder `key` of `tree` is open.
    ///
    /// For a tree of another torrent than the view's it answers as a view that is new, like
    /// [`rows`](Self::rows).
    #[must_use]
    pub fn is_expanded(&self, tree: &FileTree, key: &NodeKey) -> bool {
        if !self.belongs_to(tree) {
            return Self::new(tree).expanded.contains(key);
        }
        self.expanded.contains(key)
    }

    /// Opens the folder `key` if it is closed and closes it if it is open. A file and a key that
    /// is not in `tree` change nothing. Closing a folder that holds the cursor moves the cursor
    /// to the folder.
    pub fn toggle_expanded(&mut self, tree: &FileTree, key: &NodeKey) {
        self.adopt(tree);
        let Some(folder) = tree.position(key) else {
            return;
        };
        if !tree.node(folder).is_some_and(|node| node.is_folder) {
            return;
        }
        if self.expanded.remove(key) {
            let cursor_inside = self
                .cursor_node(tree)
                .is_some_and(|cursor| is_below(tree, cursor, folder));
            if cursor_inside {
                self.cursor = Some(key.clone());
            }
        } else {
            self.expanded.insert(key.clone());
        }
    }

    /// Puts the cursor on `key`, if it is a row that is shown.
    pub fn set_cursor(&mut self, tree: &FileTree, key: &NodeKey) {
        self.adopt(tree);
        let Some(node) = tree.position(key) else {
            return;
        };
        if self.shown(tree).iter().any(|&(index, _)| index == node) {
            self.cursor = Some(key.clone());
        }
    }

    /// Moves the cursor `step` rows down, or up when `step` is negative, among the rows that are
    /// shown, and stops at the first or the last. Without a cursor any step goes to the first
    /// row, as the keys of the torrent list do with nothing selected; End goes to the last row
    /// through [`FileTreeView::cursor_to_edge`].
    pub fn move_cursor(&mut self, tree: &FileTree, step: isize) {
        self.adopt(tree);
        let shown = self.shown(tree);
        let Some(last) = shown.len().checked_sub(1) else {
            self.cursor = None;
            return;
        };
        let current = self
            .cursor_node(tree)
            .and_then(|cursor| shown.iter().position(|&(index, _)| index == cursor));
        let target = current.map_or(0, |position| position.saturating_add_signed(step).min(last));
        self.cursor = shown
            .get(target)
            .and_then(|&(index, _)| tree.node(index))
            .map(|node| node.key.clone());
    }

    /// Puts the cursor on the first row shown (Home) or the last one (End), with a cursor or
    /// without one.
    pub fn cursor_to_edge(&mut self, tree: &FileTree, edge: Edge) {
        self.adopt(tree);
        let shown = self.shown(tree);
        let end = match edge {
            Edge::First => shown.first(),
            Edge::Last => shown.last(),
        };
        self.cursor = end
            .and_then(|&(index, _)| tree.node(index))
            .map(|node| node.key.clone());
    }

    /// Moves the cursor from a file or folder to the folder around it.
    pub fn cursor_to_parent(&mut self, tree: &FileTree) {
        self.adopt(tree);
        let parent = self
            .cursor_node(tree)
            .and_then(|cursor| tree.node(cursor))
            .and_then(|node| node.parent)
            .and_then(|parent| tree.node(parent));
        if let Some(parent) = parent {
            self.cursor = Some(parent.key.clone());
        }
    }

    /// Moves the cursor from an open folder to its first entry.
    pub fn cursor_to_first_child(&mut self, tree: &FileTree) {
        self.adopt(tree);
        let Some(node) = self.cursor_node(tree).and_then(|cursor| tree.node(cursor)) else {
            return;
        };
        if !node.is_folder || !self.expanded.contains(&node.key) {
            return;
        }
        if let Some(child) = node.children.first().and_then(|&child| tree.node(child)) {
            self.cursor = Some(child.key.clone());
        }
    }

    /// The Left key: closes the folder under the cursor if it is open, and otherwise moves the
    /// cursor to the folder around it.
    pub fn collapse_or_parent(&mut self, tree: &FileTree) {
        self.adopt(tree);
        match self.cursor.clone() {
            Some(key) if self.expanded.contains(&key) => self.toggle_expanded(tree, &key),
            _ => self.cursor_to_parent(tree),
        }
    }

    /// The Right key: opens the folder under the cursor if it is closed, and otherwise moves the
    /// cursor into it.
    pub fn expand_or_first_child(&mut self, tree: &FileTree) {
        self.adopt(tree);
        let Some(key) = self.cursor.clone() else {
            return;
        };
        if self.expanded.contains(&key) {
            self.cursor_to_first_child(tree);
        } else {
            self.toggle_expanded(tree, &key);
        }
    }

    /// Fits the state to a tree that was made again, as after every read of the details: the
    /// folders that are open stay open and the cursor stays, unless their nodes are gone. The
    /// first time the tree has anything to show, its first level opens.
    ///
    /// A cursor whose node is gone moves to its nearest folder that is still shown, or nowhere.
    /// A tree of another torrent than the view's starts the view afresh.
    pub fn retain(&mut self, tree: &FileTree) {
        if !self.adopt(tree) {
            self.prune(tree);
        }
    }

    /// Whether the state is the state of the torrent `tree` was made for.
    fn belongs_to(&self, tree: &FileTree) -> bool {
        self.hash.as_ref() == Some(tree.hash())
    }

    /// Makes the view belong to the torrent of `tree`. A view that belongs to another torrent, or
    /// to none yet, starts afresh, with the first level of `tree` open. Returns whether it did.
    fn adopt(&mut self, tree: &FileTree) -> bool {
        if self.belongs_to(tree) {
            return false;
        }
        *self = Self {
            hash: Some(tree.hash().clone()),
            ..Self::default()
        };
        self.prune(tree);
        true
    }

    /// Fits the open folders and the cursor to `tree`, which is of the view's torrent.
    fn prune(&mut self, tree: &FileTree) {
        self.expanded.retain(|key| {
            tree.position(key)
                .and_then(|index| tree.node(index))
                .is_some_and(|node| node.is_folder)
        });
        if !self.opened_first_level && !tree.is_empty() {
            self.opened_first_level = true;
            let first_level = tree.top().iter().filter_map(|&index| tree.node(index));
            self.expanded.extend(
                first_level
                    .filter(|node| node.is_folder)
                    .map(|node| node.key.clone()),
            );
        }
        if let Some(cursor) = self.cursor.take() {
            let shown: HashSet<usize> = self
                .shown(tree)
                .into_iter()
                .map(|(index, _)| index)
                .collect();
            self.cursor = nearest_shown(tree, &shown, cursor);
        }
    }

    /// The nodes that are shown, with their depth, in display order.
    fn shown(&self, tree: &FileTree) -> Vec<(usize, usize)> {
        let mut shown = Vec::new();
        // The entries still to visit, the next one last.
        let mut pending: Vec<(usize, usize)> =
            tree.top().iter().rev().map(|&index| (index, 0)).collect();
        while let Some((index, depth)) = pending.pop() {
            let Some(node) = tree.node(index) else {
                continue;
            };
            shown.push((index, depth));
            if node.is_folder && self.expanded.contains(&node.key) {
                pending.extend(node.children.iter().rev().map(|&child| (child, depth + 1)));
            }
        }
        shown
    }

    /// The index in `tree` of the node the cursor is on.
    fn cursor_node(&self, tree: &FileTree) -> Option<usize> {
        tree.position(self.cursor.as_ref()?)
    }

    fn row(&self, node: &Node, depth: usize) -> FileRow {
        let progress = fraction_of(node.done, node.size);
        FileRow {
            key: node.key.clone(),
            depth,
            kind: if node.is_folder {
                RowKind::Folder {
                    expanded: self.expanded.contains(&node.key),
                }
            } else {
                RowKind::File
            },
            name: node.name.clone(),
            size: human_size(node.size),
            percent: percent(progress),
            progress,
            wanted: node.wanted,
            priority: node.priority,
            playable: node.playable,
        }
    }
}

/// The node `key` if it is shown, and otherwise the nearest folder above its path that is shown;
/// `None` when there is none. The folders are looked up by path, because the node itself may be
/// gone from `tree`, with its parents.
fn nearest_shown(tree: &FileTree, shown: &HashSet<usize>, key: NodeKey) -> Option<NodeKey> {
    let is_shown = |key: &NodeKey, folder_only: bool| {
        tree.position(key).is_some_and(|index| {
            shown.contains(&index)
                && (!folder_only || tree.node(index).is_some_and(|n| n.is_folder))
        })
    };
    if is_shown(&key, false) {
        return Some(key);
    }
    let mut path = key.as_str();
    while let Some((above, _)) = path.rsplit_once('/') {
        path = above;
        let candidate = NodeKey::new(path);
        if is_shown(&candidate, true) {
            return Some(candidate);
        }
    }
    None
}

/// Whether the node `node` is somewhere below the folder `folder`.
fn is_below(tree: &FileTree, node: usize, folder: usize) -> bool {
    let mut above = tree.node(node).and_then(|node| node.parent);
    while let Some(index) = above {
        if index == folder {
            return true;
        }
        above = tree.node(index).and_then(|node| node.parent);
    }
    false
}
