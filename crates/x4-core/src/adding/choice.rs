//! The files of a `.torrent` the user chooses to download, before anything is sent.

use x4_rpc::{FilePriority, HashString};

use crate::details::FileEntry;
use crate::files::{FileTree, NodeKey};
use crate::metainfo::MetaFile;

/// Which files of a `.torrent` are wanted, and the tree that shows them.
///
/// Each file keeps the index the daemon will give it: its position in the parse. The tree is made
/// again from the files after each change, so its checkboxes always show the choice.
pub(super) struct FileChoice {
    /// The files at the daemon's indices (`files[i].index == i`), as the tree is made from:
    /// nothing downloaded, the normal priority, and wanted as chosen.
    files: Vec<FileEntry>,
    tree: FileTree,
}

impl FileChoice {
    /// The files of the torrent `hash`, every one of them wanted.
    pub(super) fn new(hash: HashString, files: Vec<MetaFile>) -> Self {
        let files: Vec<FileEntry> = files
            .into_iter()
            .enumerate()
            .map(|(index, file)| FileEntry {
                index,
                name: file.path,
                length: file.length,
                bytes_completed: 0,
                wanted: true,
                priority: FilePriority::Normal,
                playable_from_start: 0,
            })
            .collect();
        let tree = FileTree::of_files(hash, &files);
        Self { files, tree }
    }

    /// The tree that shows the files and the choice.
    pub(super) fn tree(&self) -> &FileTree {
        &self.tree
    }

    /// Applies a click on the checkbox of the node `key`: the files it reaches, padding files
    /// included ([`FileTree::wanted_toggle`]), all become wanted, or not wanted when all of them
    /// are. A key that is not a node of the tree changes nothing.
    pub(super) fn toggle(&mut self, key: &NodeKey) {
        let Some((indices, wanted)) = self.tree.wanted_toggle(key) else {
            return;
        };
        for &index in indices.as_slice() {
            if let Some(file) = self.files.get_mut(index) {
                file.wanted = wanted;
            }
        }
        self.tree = FileTree::of_files(self.tree.hash().clone(), &self.files);
    }

    /// The indices of the files not wanted, in ascending order, each once and each less than the
    /// number of files.
    pub(super) fn unwanted(&self) -> Vec<usize> {
        self.files
            .iter()
            .filter(|file| !file.wanted)
            .map(|file| file.index)
            .collect()
    }

    /// Whether there is nothing to download: no file that is shown is wanted (the padding files
    /// that may still be do not count), or no file at all is. A torrent of padding files only
    /// shows nothing, so it is never ready either.
    pub(super) fn nothing_wanted(&self) -> bool {
        // The second test follows from the first while the tree is made from these files. It
        // stays as a guard, so that a request never leaves out every file.
        !self.tree.wants_a_shown_file() || self.files.iter().all(|file| !file.wanted)
    }
}
