//! What the rows of the tree show, and the files an action on each reaches.

use x4_core::{FileTree, FileTreeView, PriorityView, RowKind, WantedState, human_size};
use x4_rpc::FilePriority::{High, Low, Normal};

use super::{Spec, details_of, hash, indices, key, open_all, row, shape, tree_of};

#[test]
fn a_tree_groups_files_by_folder_and_sums_sizes() {
    let tree = tree_of(&[
        ("T/a.mkv", 100, 100, true, Normal),
        ("T/sub/b.mkv", 50, 25, true, Normal),
        ("T/sub/c.mkv", 10, 0, true, Normal),
    ]);
    let mut view = FileTreeView::new(&tree);

    // First open: the top folder is open, one level deep; folders come before files.
    let rows = view.rows(&tree);
    assert_eq!(
        shape(&rows),
        [("T", 0), ("T/sub", 1), ("T/a.mkv", 1)],
        "the top folder is the first level"
    );
    assert_eq!(rows[0].kind, RowKind::Folder { expanded: true });
    assert_eq!(rows[1].kind, RowKind::Folder { expanded: false });
    assert_eq!(rows[2].kind, RowKind::File);
    assert_eq!(rows[0].name, "T");
    assert_eq!(rows[1].name, "sub");
    assert_eq!(rows[2].name, "a.mkv");

    // A folder holds the sums of the files below it.
    assert_eq!(rows[0].size, human_size(160));
    assert_eq!(rows[0].percent, 78, "125 of 160 bytes");
    assert_eq!(rows[1].size, human_size(60));
    assert_eq!(rows[1].percent, 41, "25 of 60 bytes");
    assert!((rows[1].progress - 25.0 / 60.0).abs() < 1e-9);
    assert_eq!(rows[2].size, human_size(100));
    assert_eq!(rows[2].percent, 100);

    view.toggle_expanded(&tree, &key("T/sub"));
    let rows = view.rows(&tree);
    assert_eq!(
        shape(&rows),
        [
            ("T", 0),
            ("T/sub", 1),
            ("T/sub/b.mkv", 2),
            ("T/sub/c.mkv", 2),
            ("T/a.mkv", 1)
        ]
    );
    assert_eq!(row(&rows, "T/sub/b.mkv").percent, 50);
    assert_eq!(row(&rows, "T/sub/c.mkv").percent, 0);

    // An action on a folder reaches every file below it, and on a file only that file.
    assert_eq!(indices(&tree, "T"), Some(vec![0, 1, 2]));
    assert_eq!(indices(&tree, "T/sub"), Some(vec![1, 2]));
    assert_eq!(indices(&tree, "T/sub/c.mkv"), Some(vec![2]));
    assert_eq!(indices(&tree, "T/a.mkv"), Some(vec![0]));
    assert_eq!(tree.hash(), &hash());
}

#[test]
fn indices_follow_the_file_order_of_the_torrent_not_the_display_order() {
    // The display puts the folder first and sorts by name, the torrent lists these the other way.
    let tree = tree_of(&[
        ("T/z.mkv", 1, 0, true, Normal),
        ("T/sub/y.mkv", 1, 0, true, Normal),
        ("T/a.mkv", 1, 0, true, Normal),
        ("T/sub/b.mkv", 1, 0, true, Normal),
    ]);

    assert_eq!(indices(&tree, "T"), Some(vec![0, 1, 2, 3]));
    assert_eq!(indices(&tree, "T/sub"), Some(vec![1, 3]));
    assert_eq!(indices(&tree, "T/sub/b.mkv"), Some(vec![3]));
}

#[test]
fn an_unknown_key_has_no_indices() {
    let tree = tree_of(&[("T/a.mkv", 1, 0, true, Normal)]);

    assert_eq!(indices(&tree, "T/missing"), None);
    assert_eq!(indices(&tree, ""), None);
    assert_eq!(indices(&tree, "T/"), None);
    assert!(tree.contains(&key("T/a.mkv")));
    assert!(!tree.contains(&key("T/missing")));
}

#[test]
fn folder_wanted_is_mixed_when_children_differ() {
    let tree = tree_of(&[
        ("T/a.mkv", 1, 0, true, Normal),
        ("T/sub/b.mkv", 1, 0, false, Normal),
        ("T/sub/c.mkv", 1, 0, false, Normal),
        ("T/off/d.mkv", 1, 0, true, Normal),
        ("T/off/e.mkv", 1, 0, true, Normal),
    ]);
    let rows = open_all(&tree).rows(&tree);

    assert_eq!(row(&rows, "T/a.mkv").wanted, WantedState::All);
    assert_eq!(row(&rows, "T/sub/b.mkv").wanted, WantedState::None);
    assert_eq!(row(&rows, "T/sub").wanted, WantedState::None);
    assert_eq!(row(&rows, "T/off").wanted, WantedState::All);
    assert_eq!(
        row(&rows, "T").wanted,
        WantedState::Mixed,
        "a folder with a folder that is not wanted and files that are"
    );

    let one_off = tree_of(&[
        ("T/a.mkv", 1, 0, true, Normal),
        ("T/b.mkv", 1, 0, true, Normal),
        ("T/c.mkv", 1, 0, false, Normal),
    ]);
    assert_eq!(
        row(&FileTreeView::new(&one_off).rows(&one_off), "T").wanted,
        WantedState::Mixed
    );
}

#[test]
fn folder_priority_is_mixed_when_children_differ() {
    let tree = tree_of(&[
        ("T/a.mkv", 1, 0, true, High),
        ("T/sub/b.mkv", 1, 0, true, Normal),
        ("T/sub/c.mkv", 1, 0, true, Normal),
        ("T/low/d.mkv", 1, 0, true, Low),
    ]);
    let rows = open_all(&tree).rows(&tree);

    assert_eq!(row(&rows, "T/a.mkv").priority, PriorityView::One(High));
    assert_eq!(row(&rows, "T/sub").priority, PriorityView::One(Normal));
    assert_eq!(row(&rows, "T/low").priority, PriorityView::One(Low));
    assert_eq!(row(&rows, "T").priority, PriorityView::Mixed);

    let uniform = tree_of(&[
        ("T/a.mkv", 1, 0, true, High),
        ("T/b/c.mkv", 1, 0, true, High),
    ]);
    let rows = FileTreeView::new(&uniform).rows(&uniform);
    assert_eq!(row(&rows, "T").priority, PriorityView::One(High));
}

#[test]
fn pad_files_are_hidden_but_counted_in_folder_indices() {
    let tree = tree_of(&[
        ("T/a.mkv", 100, 100, true, Normal),
        // Not wanted and not the priority of the others: it must not change what the folder says.
        ("T/_____padding_file_1", 28, 0, false, Low),
        ("T/.pad/2", 40, 0, false, Low),
        ("T/sub/b.mkv", 50, 50, true, Normal),
        ("T/sub/_____padding_file_4", 14, 0, false, Low),
    ]);
    let rows = open_all(&tree).rows(&tree);

    assert_eq!(
        shape(&rows),
        [("T", 0), ("T/sub", 1), ("T/sub/b.mkv", 2), ("T/a.mkv", 1)]
    );
    let top = row(&rows, "T");
    assert_eq!(
        top.size,
        human_size(150),
        "the padding is not part of the size"
    );
    assert_eq!(top.percent, 100);
    assert_eq!(top.wanted, WantedState::All);
    assert_eq!(top.priority, PriorityView::One(Normal));
    // An action on a folder reaches the padding files below it too.
    assert_eq!(indices(&tree, "T"), Some(vec![0, 1, 2, 3, 4]));
    assert_eq!(indices(&tree, "T/sub"), Some(vec![3, 4]));
    // The padding files themselves are not nodes.
    assert_eq!(indices(&tree, "T/_____padding_file_1"), None);
    assert_eq!(indices(&tree, "T/sub/_____padding_file_4"), None);
}

#[test]
fn a_folder_with_only_pad_files_offers_no_action() {
    let tree = tree_of(&[
        ("T/a.mkv", 100, 0, true, Normal),
        ("T/pads/_____padding_file_1", 28, 0, true, Normal),
        ("T/pads/deeper/_____padding_file_2", 28, 0, true, Normal),
        ("T/.pad/3", 40, 0, true, Normal),
    ]);
    let rows = open_all(&tree).rows(&tree);

    assert_eq!(shape(&rows), [("T", 0), ("T/a.mkv", 1)]);
    for hidden in ["T/pads", "T/pads/deeper", "T/.pad", "T/.pad/3"] {
        assert!(!tree.contains(&key(hidden)), "{hidden}");
        assert_eq!(tree.indices(&key(hidden)), None, "{hidden}");
    }
    // The top folder still holds all of the torrent's files.
    assert_eq!(indices(&tree, "T"), Some(vec![0, 1, 2, 3]));

    // A torrent of padding only has nothing to show and nothing to act on.
    let only_padding = tree_of(&[("T/_____padding_file_0", 5, 0, true, Normal)]);
    assert!(only_padding.is_empty());
    assert_eq!(only_padding.indices(&key("T")), None);
}

#[test]
fn a_single_file_torrent_has_no_folders() {
    let tree = tree_of(&[("movie.mkv", 10, 5, true, High)]);
    let view = FileTreeView::new(&tree);

    let rows = view.rows(&tree);

    assert_eq!(shape(&rows), [("movie.mkv", 0)]);
    assert_eq!(rows[0].kind, RowKind::File);
    assert_eq!(rows[0].name, "movie.mkv");
    assert_eq!(rows[0].percent, 50);
    assert_eq!(rows[0].priority, PriorityView::One(High));
    assert_eq!(indices(&tree, "movie.mkv"), Some(vec![0]));
}

#[test]
fn rows_are_folders_first_then_natural_name_order() {
    let tree = tree_of(&[
        ("T/E10.mkv", 1, 0, true, Normal),
        ("T/E2.mkv", 1, 0, true, Normal),
        ("T/e1.mkv", 1, 0, true, Normal),
        ("T/Zeta/x.txt", 1, 0, true, Normal),
        ("T/a.txt", 1, 0, true, Normal),
        ("T/Extras/x.txt", 1, 0, true, Normal),
        ("T/extras 2/x.txt", 1, 0, true, Normal),
        ("T/extras 10/x.txt", 1, 0, true, Normal),
    ]);

    let rows = FileTreeView::new(&tree).rows(&tree);

    let order: Vec<&str> = rows.iter().skip(1).map(|row| row.name.as_str()).collect();
    assert_eq!(
        order,
        [
            "Extras",
            "extras 2",
            "extras 10",
            "Zeta",
            "a.txt",
            "e1.mkv",
            "E2.mkv",
            "E10.mkv"
        ]
    );
}

#[test]
fn the_order_does_not_depend_on_the_order_of_the_files() {
    let files: [Spec<'_>; 4] = [
        ("T/b", 1, 0, true, Normal),
        ("T/B", 1, 0, true, Normal),
        ("T/a1", 1, 0, true, Normal),
        ("T/a01", 1, 0, true, Normal),
    ];
    let reversed: Vec<Spec<'_>> = files.iter().rev().copied().collect();

    let names = |files: &[Spec<'_>]| {
        let tree = tree_of(files);
        FileTreeView::new(&tree)
            .rows(&tree)
            .into_iter()
            .map(|row| row.name)
            .collect::<Vec<_>>()
    };

    assert_eq!(names(&files), ["T", "a01", "a1", "B", "b"]);
    assert_eq!(names(&files), names(&reversed));
}

#[test]
fn playable_is_shown_only_for_wanted_incomplete_files() {
    let mut details = details_of(&[
        ("T/done.mkv", 100, 100, true, Normal),
        ("T/going.mkv", 200, 80, true, Normal),
        ("T/waiting.mkv", 200, 0, true, Normal),
        ("T/unwanted.mkv", 200, 80, false, Normal),
        ("T/empty.txt", 0, 0, true, Normal),
    ]);
    // What the details compute: 100 of 100, 40 of 200, nothing, 40 of 200, nothing.
    for (file, playable) in details.files.iter_mut().zip([100, 40, 0, 40, 0]) {
        file.playable_from_start = playable;
    }
    let tree = FileTree::new(&details);

    let rows = FileTreeView::new(&tree).rows(&tree);

    assert_eq!(
        row(&rows, "T/done.mkv").playable,
        None,
        "a complete file plays whole"
    );
    assert_eq!(row(&rows, "T/unwanted.mkv").playable, None);
    assert_eq!(row(&rows, "T/empty.txt").playable, None);
    assert_eq!(row(&rows, "T").playable, None, "a folder has none");
    let going = row(&rows, "T/going.mkv").playable.unwrap();
    assert_eq!(going.bytes, 40);
    assert_eq!(going.size, human_size(40));
    assert_eq!(going.percent, 20);
    let waiting = row(&rows, "T/waiting.mkv").playable.unwrap();
    assert_eq!((waiting.bytes, waiting.percent), (0, 0));
}
