//! The files of the selected torrent as a tree, decided here so the Files tab only draws it, and
//! the files of a `.torrent` about to be added, for the add dialog.
//!
//! [`FileTree::new`] cuts the flat list of [`TorrentDetails`](crate::TorrentDetails) into folders
//! at the `/` of the paths; [`FileTree::of_files`] does the same for a bare list of files. Each
//! node is found by its [`NodeKey`], its path in the torrent, and each folder carries the sums of
//! the files below it: their size and the bytes done, whether they are wanted ([`WantedState`])
//! and at which priority ([`PriorityView`]). The torrent's own top folder is the first level.
//! Padding files (BEP 47), which only align the pieces, are hidden, but a folder's actions reach
//! them with the rest of its files. A tree whose keys would take more than [`TREE_KEY_BUDGET`]
//! is not made ([`FileTree::is_too_large`]), so a crafted list of files cannot take hundreds of
//! MiB.
//!
//! A [`FileTreeView`] is the part that the user changes: which folders are open and where the
//! keyboard cursor is. It lists the visible [`FileRow`]s in order, folders first and then by name
//! with numbers in their natural order, and it survives a refresh of the tree.
//!
//! A click on a row becomes an action through [`FileTree::toggle_wanted_action`],
//! [`FileTree::priority_action`] or [`FileTree::step_priority_action`], so the torrent, the files
//! and the new value all come from the tree that was drawn. The add dialog, which has nothing to
//! send yet, takes the files and the value of a click on a checkbox from
//! [`FileTree::wanted_toggle`]. The list of files is a [`NonEmptyIndices`], which cannot be
//! empty, because the daemon reads an empty list as "all files".

use std::sync::Arc;

use x4_rpc::FilePriority;

mod names;
mod tree;
mod view;

pub use tree::{FileTree, TREE_KEY_BUDGET};
pub use view::{FileRow, FileTreeView, RowKind};

/// The identity of a node of a [`FileTree`]: its path inside the torrent, `/`-separated.
///
/// It stays the same when the progress of the files changes and when the files are read again, so
/// the state of a [`FileTreeView`] can be kept by it. Cloning is cheap.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NodeKey(Arc<str>);

impl NodeKey {
    /// The key of the node at `path` inside the torrent, `/`-separated. The tree finds out whether
    /// there is such a node.
    #[must_use]
    pub fn new(path: &str) -> Self {
        Self(Arc::from(path))
    }

    /// The path, as the daemon names the file or folder inside the torrent.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The indices of the files an action reaches; there is always at least one.
///
/// The daemon reads an empty list of files as all of them, so a list that is empty by mistake
/// would change the whole torrent. The only way to make this type is a [`FileTree`], which gives
/// `None` instead of an empty list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonEmptyIndices(Vec<usize>);

impl NonEmptyIndices {
    /// `indices` as they are, or `None` when there are none.
    pub(crate) fn new(indices: Vec<usize>) -> Option<Self> {
        (!indices.is_empty()).then_some(Self(indices))
    }

    /// The indices, in ascending order. The slice is never empty.
    #[must_use]
    pub fn as_slice(&self) -> &[usize] {
        &self.0
    }

    /// The indices as the `files` of an [`Action`](crate::Action). The list is never empty.
    #[must_use]
    pub fn into_vec(self) -> Vec<usize> {
        self.0
    }
}

/// Whether the files of a node are to be downloaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WantedState {
    /// Every file is wanted.
    All,
    /// No file is wanted.
    None,
    /// Some files are wanted and some are not; a folder only.
    Mixed,
}

impl WantedState {
    /// The value a click on the node's checkbox sets: what is wanted entirely is switched off,
    /// and what is not or only partly wanted is switched on.
    ///
    /// The app does not pair this with files by hand; [`FileTree::toggle_wanted_action`] does.
    #[must_use]
    pub fn toggled(self) -> bool {
        self != Self::All
    }
}

/// The priority of the files of a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PriorityView {
    /// Every file has this priority.
    One(FilePriority),
    /// The files differ in priority; a folder only.
    Mixed,
}

/// The priority that `+` (`up`) or `-` gives to a node: one step higher or lower, and no further
/// than [`FilePriority::High`] or [`FilePriority::Low`]. A node of mixed priority goes to
/// [`FilePriority::Normal`] either way, so a folder becomes the same throughout.
///
/// The app does not pair this with files by hand; [`FileTree::step_priority_action`] does.
#[must_use]
pub fn next_priority(current: PriorityView, up: bool) -> FilePriority {
    let PriorityView::One(current) = current else {
        return FilePriority::Normal;
    };
    match (current, up) {
        (FilePriority::Low, true) | (FilePriority::High, false) => FilePriority::Normal,
        (FilePriority::Normal | FilePriority::High, true) => FilePriority::High,
        (FilePriority::Normal | FilePriority::Low, false) => FilePriority::Low,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_priority_saturates_and_mixed_goes_to_normal() {
        let one = PriorityView::One;
        assert_eq!(
            next_priority(one(FilePriority::Low), true),
            FilePriority::Normal
        );
        assert_eq!(
            next_priority(one(FilePriority::Normal), true),
            FilePriority::High
        );
        assert_eq!(
            next_priority(one(FilePriority::High), true),
            FilePriority::High
        );
        assert_eq!(
            next_priority(one(FilePriority::High), false),
            FilePriority::Normal
        );
        assert_eq!(
            next_priority(one(FilePriority::Normal), false),
            FilePriority::Low
        );
        assert_eq!(
            next_priority(one(FilePriority::Low), false),
            FilePriority::Low
        );
        assert_eq!(
            next_priority(PriorityView::Mixed, true),
            FilePriority::Normal
        );
        assert_eq!(
            next_priority(PriorityView::Mixed, false),
            FilePriority::Normal
        );
    }

    #[test]
    fn a_click_on_the_checkbox_switches_off_only_what_is_wanted_entirely() {
        assert!(!WantedState::All.toggled());
        assert!(WantedState::None.toggled());
        assert!(WantedState::Mixed.toggled());
    }

    #[test]
    fn a_list_of_indices_cannot_be_empty() {
        assert_eq!(NonEmptyIndices::new(Vec::new()), None);

        let indices = NonEmptyIndices::new(vec![2, 5]).unwrap();
        assert_eq!(indices.as_slice(), [2, 5]);
        assert_eq!(indices.into_vec(), [2, 5]);
    }
}
