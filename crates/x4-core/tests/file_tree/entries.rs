//! The tree made from a bare list of files, as the add dialog makes it from a `.torrent`, and the
//! wanted toggle as data, which the dialog applies to its own choice of files.

use x4_core::test_support::details;
use x4_core::{Action, FileTree, NodeKey, TorrentDetails};
use x4_rpc::FilePriority::{High, Low, Normal};

use super::{Spec, hash, indices, key, open_all, other_hash, tree_of};

/// Files of every kind the tree treats in its own way: folders in an order to sort, a file
/// downloading, mixed states and priorities, padding files of both kinds, a folder of padding
/// only, a path listed twice, a file of a folder's name, and a file without a name.
const ODD_FILES: [Spec<'static>; 11] = [
    ("T/b/E10.mkv", 10, 5, true, High),
    ("T/b/E2.mkv", 20, 20, false, Low),
    ("T/.pad/0", 3, 0, true, Normal),
    ("T/a.txt", 1, 0, true, Normal),
    ("T/b/_____padding_file_1", 2, 0, false, Normal),
    ("T/only_pads/_____padding_file_2", 2, 0, true, Normal),
    ("T/a.txt", 4, 1, false, Normal),
    ("T/x", 5, 0, true, Low),
    ("T/x/y", 6, 6, true, Normal),
    ("", 7, 0, true, Normal),
    ("U/z.bin", 8, 2, true, High),
];

/// Asserts that two trees show the same rows and that every node of them, shown or hidden,
/// leads to the same files, values and actions.
fn assert_same_tree(made: &FileTree, expected: &FileTree, label: &str) {
    assert_eq!(made.hash(), expected.hash(), "{label}");
    assert_eq!(made.is_empty(), expected.is_empty(), "{label}");
    let rows = open_all(expected).rows(expected);
    assert_eq!(open_all(made).rows(made), rows, "{label}");

    let hidden = [
        "T/.pad",
        "T/.pad/0",
        "T/b/_____padding_file_1",
        "T/only_pads",
        "T/missing",
        "T/",
    ];
    let keys = rows
        .iter()
        .map(|row| row.key.clone())
        .chain(hidden.map(NodeKey::new));
    for node in keys {
        let at = format!("{label}: {node:?}");
        assert_eq!(made.contains(&node), expected.contains(&node), "{at}");
        assert_eq!(made.indices(&node), expected.indices(&node), "{at}");
        assert_eq!(
            made.wanted_toggle(&node),
            expected.wanted_toggle(&node),
            "{at}"
        );
        assert_eq!(
            made.toggle_wanted_action(&node),
            expected.toggle_wanted_action(&node),
            "{at}"
        );
        assert_eq!(
            made.priority_action(&node, Low),
            expected.priority_action(&node, Low),
            "{at}"
        );
        for up in [true, false] {
            assert_eq!(
                made.step_priority_action(&node, up),
                expected.step_priority_action(&node, up),
                "{at}"
            );
        }
    }
}

#[test]
fn the_details_tree_is_unchanged_by_the_new_constructor() {
    let mut downloading = details(&hash(), &ODD_FILES);
    if let Some(file) = downloading.files.first_mut() {
        file.playable_from_start = 4;
    }
    // The comparison covers the progress too: the first file has a prefix to play.
    let tree = FileTree::new(&downloading);
    let rows = open_all(&tree).rows(&tree);
    assert!(rows.iter().any(|row| row.playable.is_some()));
    assert!(rows.iter().any(|row| row.percent > 0));

    let cases: [(&str, TorrentDetails); 3] = [
        ("odd files", downloading),
        ("no files yet", details(&other_hash(), &[])),
        (
            "padding only",
            details(&hash(), &[("T/_____padding_file_0", 5, 0, true, Normal)]),
        ),
    ];
    for (label, details) in cases {
        let from_details = FileTree::new(&details);
        let from_files = FileTree::of_files(details.hash.clone(), &details.files);

        assert_same_tree(&from_files, &from_details, label);
    }
}

#[test]
fn the_wanted_toggle_is_what_the_wanted_action_sends() {
    let tree = tree_of(&[
        ("T/a.mkv", 100, 0, true, Normal),
        ("T/_____padding_file_1", 28, 0, true, Normal),
        ("T/sub/b.mkv", 50, 0, false, Normal),
        ("T/sub/_____padding_file_3", 14, 0, false, Normal),
        ("T/on/d.mkv", 50, 0, true, Normal),
    ]);
    let toggle = |path: &str| {
        tree.wanted_toggle(&key(path))
            .map(|(files, wanted)| (files.into_vec(), wanted))
    };

    // The files reached, padding included, and the value a click sets: what is wanted entirely
    // goes off, what is partly or not wanted goes on.
    assert_eq!(toggle("T"), Some((vec![0, 1, 2, 3, 4], true)), "mixed");
    assert_eq!(toggle("T/sub"), Some((vec![2, 3], true)), "none wanted");
    assert_eq!(toggle("T/on"), Some((vec![4], false)), "all wanted");
    assert_eq!(toggle("T/a.mkv"), Some((vec![0], false)), "a file");
    // Nothing for what is not shown.
    for hidden in ["T/_____padding_file_1", "T/sub/_____padding_file_3", "T/x"] {
        assert_eq!(toggle(hidden), None, "{hidden}");
    }

    // The action is that pair for the tree's own torrent.
    let rows = open_all(&tree).rows(&tree);
    for row in &rows {
        let (files, wanted) = tree.wanted_toggle(&row.key).unwrap();
        assert_eq!(
            tree.toggle_wanted_action(&row.key),
            Some(Action::SetFilesWanted {
                hash: hash(),
                files: files.into_vec(),
                wanted,
            }),
            "{:?}",
            row.key
        );
    }
    assert_eq!(indices(&tree, "T/sub"), Some(vec![2, 3]));
}
