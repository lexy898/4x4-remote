//! Torrents with odd paths, a lot of files, or data that disagrees with itself: the tree must
//! still name the right files and never an empty list.

use x4_core::{Action, FileTree, FileTreeView, PriorityView, RowKind, WantedState, human_size};
use x4_rpc::FilePriority::{High, Low, Normal};

use super::{details_of, hash, indices, key, open_all, row, shape, tree_of};

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
fn a_path_that_holds_a_suffix_itself_still_gets_a_key_of_its_own() {
    // A repeated path gets the suffix "\0" and a number, which a crafted name may hold already.
    let tree = tree_of(&[
        ("T/a", 1, 0, true, Normal),
        ("T/a\u{0}1", 2, 0, true, Normal),
        ("T/a", 3, 0, true, Normal),
        ("T/a", 4, 0, true, Normal),
    ]);

    let rows = open_all(&tree).rows(&tree);

    assert_eq!(
        shape(&rows),
        [
            ("T", 0),
            ("T/a", 1),
            ("T/a\u{0}2", 1),
            ("T/a\u{0}3", 1),
            ("T/a\u{0}1", 1)
        ]
    );
    assert_eq!(indices(&tree, "T/a"), Some(vec![0]));
    assert_eq!(indices(&tree, "T/a\u{0}1"), Some(vec![1]));
    assert_eq!(indices(&tree, "T/a\u{0}2"), Some(vec![2]));
    assert_eq!(indices(&tree, "T/a\u{0}3"), Some(vec![3]));
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

#[test]
fn a_pad_file_listed_first_in_its_folder_is_reached_but_does_not_change_the_folder() {
    // The padding file comes before every shown file of T, not wanted and at another priority.
    let tree = tree_of(&[
        ("T/_____padding_file_0", 28, 0, false, Low),
        ("T/a.mkv", 100, 100, true, Normal),
        ("T/b.mkv", 100, 50, true, Normal),
    ]);

    let rows = FileTreeView::new(&tree).rows(&tree);

    assert_eq!(indices(&tree, "T"), Some(vec![0, 1, 2]));
    assert_eq!(shape(&rows), [("T", 0), ("T/a.mkv", 1), ("T/b.mkv", 1)]);
    let top = row(&rows, "T");
    assert_eq!(top.wanted, WantedState::All, "from the shown files only");
    assert_eq!(top.priority, PriorityView::One(Normal));
    assert_eq!(top.size, human_size(200));
    assert_eq!(top.percent, 75, "150 of 200 bytes");
    assert_eq!(
        tree.toggle_wanted_action(&key("T")),
        Some(Action::SetFilesWanted {
            hash: hash(),
            files: vec![0, 1, 2],
            wanted: false,
        })
    );
}

#[test]
fn indices_and_actions_carry_the_index_of_the_entry_not_its_position() {
    let mut details = details_of(&[
        ("T/a.mkv", 1, 0, true, Normal),
        ("T/sub/b.mkv", 1, 0, false, Normal),
        ("T/sub/c.mkv", 1, 0, false, Normal),
        ("T/sub/_____padding_file_x", 1, 0, false, Normal),
    ]);
    for (file, index) in details.files.iter_mut().zip([10, 20, 30, 25]) {
        file.index = index;
    }
    let tree = FileTree::new(&details);

    assert_eq!(indices(&tree, "T"), Some(vec![10, 20, 25, 30]));
    assert_eq!(indices(&tree, "T/sub"), Some(vec![20, 25, 30]));
    assert_eq!(indices(&tree, "T/a.mkv"), Some(vec![10]));
    assert_eq!(indices(&tree, "T/sub/c.mkv"), Some(vec![30]));
    assert_eq!(
        tree.toggle_wanted_action(&key("T/sub")),
        Some(Action::SetFilesWanted {
            hash: hash(),
            files: vec![20, 25, 30],
            wanted: true,
        })
    );
    assert_eq!(
        tree.priority_action(&key("T/sub/b.mkv"), High),
        Some(Action::SetFilePriority {
            hash: hash(),
            files: vec![20],
            priority: High,
        })
    );
    assert_eq!(
        tree.step_priority_action(&key("T/a.mkv"), true),
        Some(Action::SetFilePriority {
            hash: hash(),
            files: vec![10],
            priority: High,
        })
    );
}

/// `names` in a shuffled order, the same every time.
fn shuffled(mut names: Vec<String>) -> Vec<String> {
    let mut seed: u64 = 42;
    for at in (1..names.len()).rev() {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let other = usize::try_from(seed >> 33).unwrap_or(0) % (at + 1);
        names.swap(at, other);
    }
    names
}

/// 14 000 names alike in their first 600 bytes, `a1` 300 times, and each with its own number
/// after them, in a shuffled order: 8.9 MB of names, within the budget.
pub(super) fn names_crafted_to_compare_slowly() -> Vec<String> {
    shuffled(
        (0..14_000)
            .map(|n| format!("{}{n}", "a1".repeat(300)))
            .collect(),
    )
}

/// The tree of the files `T/<name>` for each of `names`, and how long it took to make.
fn timed_tree_of(names: &[String]) -> (FileTree, std::time::Duration) {
    let paths: Vec<String> = names.iter().map(|name| format!("T/{name}")).collect();
    let files: Vec<super::Spec<'_>> = paths
        .iter()
        .map(|path| (path.as_str(), 1, 0, true, Normal))
        .collect();
    let crafted = details_of(&files);
    let start = std::time::Instant::now();
    let tree = FileTree::new(&crafted);
    (tree, start.elapsed())
}

/// `count` names of `letter` `times` times and each its own number, in a shuffled order.
fn alike_but_for_a_number(letter: &str, times: usize, count: u32) -> Vec<String> {
    shuffled(
        (0..count)
            .map(|n| format!("{}{n}", letter.repeat(times)))
            .collect(),
    )
}

/// `count` names of `times` letters, `lower` or `upper` by the bits of each name's own number in
/// the first 16, in a shuffled order.
fn alike_but_for_their_case([lower, upper]: [char; 2], times: u32, count: u32) -> Vec<String> {
    shuffled(
        (0..count)
            .map(|n| {
                (0..times)
                    .map(|bit| {
                        if bit < 16 && (n >> bit) & 1 == 1 {
                            upper
                        } else {
                            lower
                        }
                    })
                    .collect()
            })
            .collect(),
    )
}

/// Whether names beyond ASCII that took `beyond` were ordered as fast as names alike in the same
/// way in ASCII that took `ascii`: no more than 1.3 times as long, a margin for a busy machine,
/// and under 0.5 s in a release build.
fn as_fast_as_in_ascii(beyond: std::time::Duration, ascii: std::time::Duration) -> bool {
    beyond < ascii * 13 / 10
        && (cfg!(debug_assertions) || beyond < std::time::Duration::from_millis(500))
}

/// Asserts that a tree of `beyond`, names beyond ASCII, is made as fast as one of `ascii`, names
/// alike in the same way in ASCII, in the best of three tries: other tests may take the machine
/// from one of the two.
fn assert_ordered_as_fast_as_in_ascii(beyond: &[String], ascii: &[String]) {
    let mut tries = Vec::new();
    while tries.len() < 3 {
        let (tree, beyond) = timed_tree_of(beyond);
        let (_, ascii) = timed_tree_of(ascii);
        assert!(!tree.is_too_large());
        tries.push((beyond, ascii));
        if as_fast_as_in_ascii(beyond, ascii) {
            break;
        }
    }

    // Some 0.7 to 1 times as long in a debug build, and 0.6 to 0.75 times, some 0.2 s, in a
    // release one.
    assert!(
        tries
            .iter()
            .any(|&(beyond, ascii)| as_fast_as_in_ascii(beyond, ascii)),
        "{tries:?}, beyond ASCII and in ASCII"
    );
}

#[test]
fn names_beyond_ascii_alike_but_for_a_number_are_ordered_as_fast_as_in_ascii() {
    // "é" 128 times, 256 bytes, and a number: each comparison folded every letter of both names
    // to lower case. A tree of 36 000 of them took 2 s in a release build, at every read of the
    // details, and 1.8 times as long as one of "a" 256 times in a debug build.
    assert_ordered_as_fast_as_in_ascii(
        &alike_but_for_a_number("é", 128, 36_000),
        &alike_but_for_a_number("a", 256, 36_000),
    );
}

#[test]
fn names_beyond_ascii_alike_but_for_their_case_are_ordered_as_fast_as_in_ascii() {
    // 128 letters, 256 bytes, "é" or "É" by the bits of a number in the first 16: each comparison
    // folded every letter of both names, and again for the tie of their case. A tree of 36 000
    // of them took 3.8 s in a release build, at every read of the details, and 1.9 times as long
    // as one of "a" or "A" 256 times in a debug build.
    assert_ordered_as_fast_as_in_ascii(
        &alike_but_for_their_case(['é', 'É'], 128, 36_000),
        &alike_but_for_their_case(['a', 'A'], 256, 36_000),
    );
}

#[test]
fn names_crafted_to_compare_slowly_are_ordered_in_bounded_time() {
    // Each name has 600 runs of letters and digits to compare by number and without case; a
    // tree of them took 1.7 s to sort in a release build, at every read of the details.
    let paths: Vec<String> = names_crafted_to_compare_slowly()
        .iter()
        .map(|name| format!("T/{name}"))
        .collect();
    let files: Vec<super::Spec<'_>> = paths
        .iter()
        .map(|path| (path.as_str(), 1, 0, true, Normal))
        .collect();
    let crafted = details_of(&files);
    let start = std::time::Instant::now();

    let tree = FileTree::new(&crafted);

    let elapsed = start.elapsed();
    // Some 70 ms in a release build and 3 s in a debug one, where it took 55 s.
    assert!(elapsed < std::time::Duration::from_secs(20), "{elapsed:?}");
    assert!(!tree.is_too_large());
    // Past their first 32 runs the names compare byte by byte, so their numbers come in the
    // order of their digits.
    let rows = FileTreeView::new(&tree).rows(&tree);
    let alike = "a1".repeat(300);
    let names: Vec<&str> = rows
        .iter()
        .skip(1)
        .take(4)
        .map(|row| row.name.as_str())
        .collect();
    assert_eq!(
        names,
        ["0", "1", "10", "100"].map(|n| format!("{alike}{n}"))
    );
}
