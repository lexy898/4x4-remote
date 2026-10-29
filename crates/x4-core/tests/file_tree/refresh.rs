//! What the state of a view keeps when the tree is made again.

use x4_core::{FileTree, FileTreeView, WantedState};
use x4_rpc::FilePriority::{High, Normal};

use super::{Spec, indices, key, open_all, row, shape, tree_of};

#[test]
fn expanded_state_survives_a_progress_update() {
    let files: [Spec<'_>; 3] = [
        ("T/sub/b.mkv", 100, 10, true, Normal),
        ("T/sub/c.mkv", 100, 20, true, Normal),
        ("T/a.mkv", 100, 30, true, Normal),
    ];
    let before = tree_of(&files);
    let mut view = FileTreeView::new(&before);
    view.toggle_expanded(&before, &key("T/sub"));
    view.set_cursor(&before, &key("T/sub/c.mkv"));
    let rows_before = view.rows(&before);

    // The same files after a progress read: other bytes, a changed priority and wanted switch.
    let progressed = tree_of(&[
        ("T/sub/b.mkv", 100, 60, true, High),
        ("T/sub/c.mkv", 100, 70, false, Normal),
        ("T/a.mkv", 100, 100, true, Normal),
    ]);
    view.retain(&progressed);
    let rows_after = view.rows(&progressed);

    assert_eq!(shape(&rows_after), shape(&rows_before));
    assert_eq!(view.cursor(), Some(&key("T/sub/c.mkv")));
    assert!(view.is_expanded(&key("T/sub")));
    assert_eq!(
        row(&rows_after, "T/sub/b.mkv").percent,
        60,
        "the numbers are the new ones"
    );
    assert_eq!(row(&rows_after, "T/sub/c.mkv").wanted, WantedState::None);
    // The same keys find the same files in the new tree.
    assert_eq!(indices(&progressed, "T/sub"), indices(&before, "T/sub"));
}

#[test]
fn retain_drops_what_a_new_set_of_files_no_longer_has() {
    let before = tree_of(&[
        ("T/sub/b.mkv", 1, 0, true, Normal),
        ("T/gone/c.mkv", 1, 0, true, Normal),
        ("T/a.mkv", 1, 0, true, Normal),
    ]);
    let mut view = open_all(&before);
    view.set_cursor(&before, &key("T/gone/c.mkv"));

    let after = tree_of(&[
        ("T/sub/b.mkv", 1, 0, true, Normal),
        ("T/a.mkv", 1, 0, true, Normal),
    ]);
    view.retain(&after);

    assert_eq!(view.cursor(), None);
    assert!(!view.is_expanded(&key("T/gone")));
    assert!(view.is_expanded(&key("T/sub")));
    assert!(view.is_expanded(&key("T")));
    assert_eq!(
        shape(&view.rows(&after)),
        [("T", 0), ("T/sub", 1), ("T/sub/b.mkv", 2), ("T/a.mkv", 1)]
    );

    // A folder that comes back later does not come back open.
    view.retain(&before);
    assert!(!view.is_expanded(&key("T/gone")));
}

#[test]
fn a_folder_the_user_closed_stays_closed_after_a_refresh() {
    let tree = tree_of(&[
        ("T/sub/b.mkv", 1, 0, true, Normal),
        ("T/a.mkv", 1, 0, true, Normal),
    ]);
    let mut view = FileTreeView::new(&tree);
    view.toggle_expanded(&tree, &key("T"));
    assert_eq!(shape(&view.rows(&tree)), [("T", 0)]);

    view.retain(&tree);

    assert_eq!(shape(&view.rows(&tree)), [("T", 0)]);
}

#[test]
fn a_magnet_without_metadata_has_an_empty_tree() {
    let empty = tree_of(&[]);
    let mut view = FileTreeView::new(&empty);

    assert!(empty.is_empty());
    assert_eq!(view.rows(&empty), []);
    assert_eq!(empty.indices(&key("anything")), None);
    // Keys on an empty tree do nothing.
    view.move_cursor(&empty, 1);
    view.move_cursor(&empty, -1);
    view.cursor_to_parent(&empty);
    view.cursor_to_first_child(&empty);
    view.toggle_expanded(&empty, &key("T"));
    assert_eq!(view.cursor(), None);
    assert_eq!(view.rows(&empty), []);

    // When the metadata arrives, the tree appears with its top folder open.
    let arrived = tree_of(&[
        ("T/sub/b.mkv", 1, 0, true, Normal),
        ("T/a.mkv", 1, 0, true, Normal),
    ]);
    view.retain(&arrived);
    assert_eq!(
        shape(&view.rows(&arrived)),
        [("T", 0), ("T/sub", 1), ("T/a.mkv", 1)]
    );
}

#[test]
fn keys_do_not_change_with_progress_or_a_new_read() {
    let files: [Spec<'_>; 3] = [
        ("T/sub/b.mkv", 100, 10, true, Normal),
        ("T/a.mkv", 100, 30, true, Normal),
        ("T/sub/deep/c.mkv", 100, 30, true, Normal),
    ];
    let keys = |tree: &FileTree| -> Vec<String> {
        open_all(tree)
            .rows(tree)
            .into_iter()
            .map(|row| row.key.as_str().to_owned())
            .collect()
    };

    let first = keys(&tree_of(&files));
    let mut moved = files;
    moved[0].2 = 99;
    moved[1].3 = false;

    assert_eq!(keys(&tree_of(&moved)), first);
    assert_eq!(
        first,
        [
            "T",
            "T/sub",
            "T/sub/deep",
            "T/sub/deep/c.mkv",
            "T/sub/b.mkv",
            "T/a.mkv"
        ]
    );
}
