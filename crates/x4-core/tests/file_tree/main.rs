//! The file tree of the details panel, through the public API the Files tab uses: what the rows
//! show and which files an action on a row reaches (`structure`, and `edge_cases` for odd torrents),
//! the actions the tree builds itself (`actions`), the tree of a bare list of files and the wanted
//! toggle as data (`entries`), a tree too large to make (`budget`), how the cursor and the open
//! folders are moved (`navigation`), and what survives a refresh (`refresh`).

use std::sync::Arc;

mod actions;
mod budget;
mod edge_cases;
mod entries;
mod navigation;
mod refresh;
mod structure;

use x4_core::test_support::details;
use x4_core::{FileRow, FileTree, FileTreeView, NodeKey, RowKind, TorrentDetails};
use x4_rpc::{FilePriority, HashString};

/// A file as `test_support::details` takes it: path, length, bytes done, wanted, priority.
type Spec<'a> = (&'a str, u64, u64, bool, FilePriority);

#[expect(clippy::unwrap_used, reason = "the hash is a literal of 40 hex digits")]
fn hash() -> HashString {
    HashString::parse(&"ab".repeat(20)).unwrap()
}

#[expect(clippy::unwrap_used, reason = "the hash is a literal of 40 hex digits")]
fn other_hash() -> HashString {
    HashString::parse(&"cd".repeat(20)).unwrap()
}

fn details_of(files: &[Spec<'_>]) -> TorrentDetails {
    details(&hash(), files)
}

fn tree_of(files: &[Spec<'_>]) -> FileTree {
    FileTree::new(&Arc::new(details_of(files)))
}

fn key(path: &str) -> NodeKey {
    NodeKey::new(path)
}

/// A view with every folder open.
fn open_all(tree: &FileTree) -> FileTreeView {
    let mut view = FileTreeView::new(tree);
    loop {
        let closed: Vec<NodeKey> = view
            .rows(tree)
            .into_iter()
            .filter(|row| matches!(row.kind, RowKind::Folder { expanded: false }))
            .map(|row| row.key)
            .collect();
        if closed.is_empty() {
            return view;
        }
        for folder in &closed {
            view.toggle_expanded(tree, folder);
        }
    }
}

/// The path and the depth of every row, in order.
fn shape(rows: &[FileRow]) -> Vec<(&str, usize)> {
    rows.iter()
        .map(|row| (row.key.as_str(), row.depth))
        .collect()
}

#[expect(
    clippy::panic,
    reason = "a test helper: a row that is missing fails the test that asks for it"
)]
fn row<'r>(rows: &'r [FileRow], path: &str) -> &'r FileRow {
    rows.iter()
        .find(|row| row.key.as_str() == path)
        .unwrap_or_else(|| panic!("no row {path} in {:?}", shape(rows)))
}

fn indices(tree: &FileTree, path: &str) -> Option<Vec<usize>> {
    tree.indices(&key(path))
        .map(x4_core::NonEmptyIndices::into_vec)
}
