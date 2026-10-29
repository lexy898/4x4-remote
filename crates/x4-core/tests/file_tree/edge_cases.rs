//! Torrents with odd paths, a lot of files, or data that disagrees with itself: the tree must
//! still name the right files and never an empty list.

use x4_core::{FileTree, FileTreeView, RowKind, human_size};
use x4_rpc::FilePriority::Normal;

use super::{details_of, indices, open_all, row, shape, tree_of};

#[test]
fn paths_that_clash_still_get_distinct_keys_and_their_own_files() {
    // The same path twice, and a file that has the name of a folder.
    let tree = tree_of(&[
        ("T/a", 1, 0, true, Normal),
        ("T/a", 2, 0, true, Normal),
        ("T/x", 3, 0, true, Normal),
        ("T/x/y", 4, 0, true, Normal),
    ]);

    let rows = open_all(&tree).rows(&tree);

    let mut seen = std::collections::HashSet::new();
    for row in &rows {
        assert!(seen.insert(row.key.clone()), "{:?} twice", row.key);
    }
    let files_of = |name: &str, kind: RowKind| {
        rows.iter()
            .filter(|row| row.name == name && row.kind == kind)
            .map(|row| tree.indices(&row.key).unwrap().into_vec())
            .collect::<Vec<_>>()
    };
    assert_eq!(files_of("a", RowKind::File), [vec![0], vec![1]]);
    assert_eq!(files_of("x", RowKind::File), [vec![2]]);
    assert_eq!(files_of("x", RowKind::Folder { expanded: true }), [vec![3]]);
    assert_eq!(indices(&tree, "T"), Some(vec![0, 1, 2, 3]));
}

#[test]
fn empty_path_components_are_ignored() {
    let tree = tree_of(&[
        ("T//a.mkv", 1, 0, true, Normal),
        ("/T/b.mkv", 1, 0, true, Normal),
    ]);

    let rows = FileTreeView::new(&tree).rows(&tree);

    assert_eq!(shape(&rows), [("T", 0), ("T/a.mkv", 1), ("T/b.mkv", 1)]);
    assert_eq!(indices(&tree, "T"), Some(vec![0, 1]));
}

#[test]
fn unicode_and_very_long_names_are_kept_whole() {
    let long = "д".repeat(300);
    let path = format!("Сезон/{long}.mkv");
    let tree = tree_of(&[
        (path.as_str(), 1, 0, true, Normal),
        ("Сезон/日本語 ファイル.txt", 1, 0, true, Normal),
    ]);

    let rows = FileTreeView::new(&tree).rows(&tree);

    assert_eq!(rows[0].name, "Сезон");
    assert_eq!(rows[1].name, format!("{long}.mkv"));
    assert_eq!(rows[2].name, "日本語 ファイル.txt");
    assert_eq!(indices(&tree, &path), Some(vec![0]));
}

#[test]
fn a_large_torrent_keeps_every_index_exactly_once_and_in_order() {
    let mut details = details_of(&[]);
    details.files = (0..2_000)
        .map(|index| x4_core::FileEntry {
            index,
            name: format!("T/season {}/deep/e{index}.mkv", index % 17),
            length: 10,
            bytes_completed: 5,
            wanted: true,
            priority: Normal,
            playable_from_start: 0,
        })
        .collect();

    let tree = FileTree::new(&details);
    let rows = open_all(&tree).rows(&tree);

    assert_eq!(indices(&tree, "T"), Some((0..2_000).collect()));
    let season_files: usize = (0..17)
        .map(|season| indices(&tree, &format!("T/season {season}")).unwrap().len())
        .sum();
    assert_eq!(season_files, 2_000);
    assert_eq!(
        rows.iter().filter(|row| row.kind == RowKind::File).count(),
        2_000
    );
    assert_eq!(row(&rows, "T").size, human_size(20_000));
}

#[test]
fn every_row_names_a_non_empty_list_of_files_that_exist() {
    let details = details_of(&[
        ("T/a.mkv", 1, 0, true, Normal),
        ("T/_____padding_file_1", 1, 0, true, Normal),
        ("T/pads/_____padding_file_2", 1, 0, true, Normal),
        ("T/.pad/3", 1, 0, true, Normal),
        ("T/sub/b.mkv", 1, 0, true, Normal),
        ("T/sub/deep/c.mkv", 1, 0, true, Normal),
        ("T/sub/deep/_____padding_file_6", 1, 0, true, Normal),
    ]);
    let tree = FileTree::new(&details);

    for row in open_all(&tree).rows(&tree) {
        let indices = tree
            .indices(&row.key)
            .unwrap_or_else(|| panic!("{:?}", row.key));
        assert!(!indices.as_slice().is_empty(), "{:?}", row.key);
        assert!(
            indices.as_slice().windows(2).all(|pair| pair[0] < pair[1]),
            "{:?}",
            row.key
        );
        assert!(
            indices
                .as_slice()
                .iter()
                .all(|&index| index < details.files.len()),
            "{:?}",
            row.key
        );
    }
}

#[test]
fn a_file_that_reports_more_than_its_length_does_not_inflate_its_folder() {
    let tree = tree_of(&[
        ("T/a.mkv", 100, 150, true, Normal),
        ("T/b.mkv", 100, 0, true, Normal),
    ]);

    let rows = FileTreeView::new(&tree).rows(&tree);

    assert_eq!(row(&rows, "T/a.mkv").percent, 100);
    assert_eq!(row(&rows, "T").percent, 50, "100 of 200 bytes, not 150");
}
