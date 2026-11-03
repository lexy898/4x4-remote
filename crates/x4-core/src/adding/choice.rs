//! The files of a `.torrent` the user chooses to download, before anything is sent.

use std::sync::Arc;

use x4_rpc::{FilePriority, HashString};

use crate::details::FileEntry;
use crate::files::{FileTree, MAX_CHOOSABLE_FILES, NodeKey, TREE_BUDGET};
use crate::metainfo::MetaFile;

/// Which files of a `.torrent` are wanted, and the tree that shows them.
///
/// Each file keeps the index the daemon will give it: its position in the parse. The tree is made
/// again from the files after each change, so its checkboxes always show the choice; a tree handed
/// out before stays as it was, like the details panel's tree of an older read.
pub(super) struct FileChoice {
    /// The files at the daemon's indices (`files[i].index == i`), as the tree is made from:
    /// nothing downloaded, the normal priority, and wanted as chosen.
    files: Vec<FileEntry>,
    tree: Arc<FileTree>,
}

impl FileChoice {
    /// The tree of the files of the torrent `hash` as a form starts: every file wanted. It is made
    /// when the torrent is loaded; `None` when the list is too large to choose from
    /// ([`FileTree::is_too_large`]).
    ///
    /// The tree is made from a copy of the files, which counts against [`TREE_BUDGET`] with it, so
    /// that loading a torrent takes no more than that beyond its parse. A list whose copy alone
    /// would take more is refused before it is made.
    pub(super) fn first_tree(hash: HashString, files: &[MetaFile]) -> Option<Arc<FileTree>> {
        // The tree would give up on them too, but only after their entries were copied: half a
        // million paths, some 40 MiB.
        if files.len() > MAX_CHOOSABLE_FILES {
            return None;
        }
        // 50 000 paths in a folder with a long name take 64 MB, as many as the parse.
        let copy = files.iter().fold(0_usize, |bytes, file| {
            bytes
                .saturating_add(size_of::<FileEntry>())
                .saturating_add(file.path.len())
        });
        let budget = TREE_BUDGET.checked_sub(copy)?;
        let entries: Vec<FileEntry> = files
            .iter()
            .enumerate()
            .map(|(index, file)| entry(index, file.path.clone(), file.length))
            .collect();
        let tree = FileTree::of_files_within(hash, &entries, budget);
        (!tree.is_too_large()).then(|| Arc::new(tree))
    }

    /// The files `files`, every one of them wanted, shown by `tree`, their
    /// [`first_tree`](Self::first_tree).
    pub(super) fn new(files: Vec<MetaFile>, tree: Arc<FileTree>) -> Self {
        let files = files
            .into_iter()
            .enumerate()
            .map(|(index, file)| entry(index, file.path, file.length))
            .collect();
        Self { files, tree }
    }

    /// The tree that shows the files and the choice.
    pub(super) fn tree(&self) -> &Arc<FileTree> {
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
        // The tree costs what the first one did, which fitted with the copy of the files besides.
        self.tree = Arc::new(FileTree::of_files(self.tree.hash().clone(), &self.files));
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

/// The file at `index` of a torrent not added yet: nothing downloaded, the normal priority, and
/// wanted.
fn entry(index: usize, path: String, length: u64) -> FileEntry {
    FileEntry {
        index,
        name: path,
        length,
        bytes_completed: 0,
        wanted: true,
        priority: FilePriority::Normal,
        playable_from_start: 0,
    }
}
