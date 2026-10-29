//! What the state of a view keeps when the tree is made again.

use x4_core::test_support::details;
use x4_core::{FileTree, FileTreeView, WantedState};
use x4_rpc::FilePriority::{High, Normal};

use super::{Spec, indices, key, open_all, other_hash, row, shape, tree_of};

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

    assert_eq!(
        view.cursor(),
        Some(&key("T")),
        "the nearest folder that is left"
    );
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

#[test]
fn a_lost_cursor_moves_to_its_nearest_surviving_folder() {
    let before = tree_of(&[
        ("T/sub/deep/c.mkv", 1, 0, true, Normal),
        ("T/sub/b.mkv", 1, 0, true, Normal),
        ("T/a.mkv", 1, 0, true, Normal),
    ]);
    let cursor_on = |path: &str| {
        let mut view = open_all(&before);
        view.set_cursor(&before, &key(path));
        assert_eq!(view.cursor(), Some(&key(path)));
        view
    };

    // The deepest folder is gone: the cursor goes up to the folder that is left.
    let mut view = cursor_on("T/sub/deep/c.mkv");
    let no_deep = tree_of(&[
        ("T/sub/b.mkv", 1, 0, true, Normal),
        ("T/a.mkv", 1, 0, true, Normal),
    ]);
    view.retain(&no_deep);
    assert_eq!(view.cursor(), Some(&key("T/sub")));

    // Two levels are gone: it goes up two.
    let mut view = cursor_on("T/sub/deep/c.mkv");
    let only_top = tree_of(&[("T/a.mkv", 1, 0, true, Normal)]);
    view.retain(&only_top);
    assert_eq!(view.cursor(), Some(&key("T")));

    // A file is gone: it goes to the folder around it, not to a neighbour.
    let mut view = cursor_on("T/sub/b.mkv");
    let no_b = tree_of(&[
        ("T/sub/deep/c.mkv", 1, 0, true, Normal),
        ("T/a.mkv", 1, 0, true, Normal),
    ]);
    view.retain(&no_b);
    assert_eq!(view.cursor(), Some(&key("T/sub")));

    // The nearest path above that exists is a file now, so it is not a place for the cursor.
    let mut view = cursor_on("T/sub/b.mkv");
    let sub_is_a_file = tree_of(&[
        ("T/sub", 1, 0, true, Normal),
        ("T/a.mkv", 1, 0, true, Normal),
    ]);
    view.retain(&sub_is_a_file);
    assert_eq!(view.cursor(), Some(&key("T")));

    // Nothing above is left: no cursor.
    let mut view = cursor_on("T/sub/deep/c.mkv");
    let other = tree_of(&[("U/x.mkv", 1, 0, true, Normal)]);
    view.retain(&other);
    assert_eq!(view.cursor(), None);

    // A cursor whose node is still there stays, whatever else is gone.
    let mut view = cursor_on("T/a.mkv");
    view.retain(&no_deep);
    assert_eq!(view.cursor(), Some(&key("T/a.mkv")));
}

#[test]
fn a_view_given_another_torrent_starts_fresh() {
    let files: [Spec<'_>; 3] = [
        ("T/sub/b.mkv", 1, 0, true, Normal),
        ("T/sub/deep/c.mkv", 1, 0, true, Normal),
        ("T/a.mkv", 1, 0, true, Normal),
    ];
    let first = tree_of(&files);
    let second = FileTree::new(&details(&other_hash(), &files));
    let fresh = [("T", 0), ("T/sub", 1), ("T/a.mkv", 1)];

    let mut view = open_all(&first);
    view.set_cursor(&first, &key("T/sub/deep/c.mkv"));
    assert_eq!(view.rows(&first).len(), 6);

    // Without a call that fits the view, the rows are those of a fresh view.
    assert_eq!(shape(&view.rows(&second)), fresh);

    // The same keys exist in the other torrent, but nothing is carried over to it.
    view.retain(&second);
    assert_eq!(view.cursor(), None);
    assert!(!view.is_expanded(&key("T/sub")));
    assert!(view.is_expanded(&key("T")), "the first level opens again");
    assert_eq!(shape(&view.rows(&second)), fresh);

    // A folder closed in one torrent is open again when the view comes back to a torrent, but
    // a refresh of the same torrent keeps what the user did.
    view.toggle_expanded(&second, &key("T"));
    view.retain(&second);
    assert_eq!(shape(&view.rows(&second)), [("T", 0)]);
    view.retain(&first);
    assert_eq!(shape(&view.rows(&first)), fresh);
}

#[test]
fn a_key_press_on_another_torrent_acts_on_a_fresh_view_of_it() {
    let files: [Spec<'_>; 2] = [
        ("T/sub/b.mkv", 1, 0, true, Normal),
        ("T/a.mkv", 1, 0, true, Normal),
    ];
    let first = tree_of(&files);
    let second = FileTree::new(&details(&other_hash(), &files));
    let mut view = open_all(&first);
    view.set_cursor(&first, &key("T/sub/b.mkv"));

    // The cursor of the first torrent does not count: a step down starts at the first row.
    view.move_cursor(&second, 1);

    assert_eq!(view.cursor(), Some(&key("T")));
    assert!(!view.is_expanded(&key("T/sub")));
}
