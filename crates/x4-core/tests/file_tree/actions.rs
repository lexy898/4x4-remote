//! The file actions the tree builds itself, so that the hash, the files and the new value always
//! come from the tree that was drawn.

use x4_core::test_support::details;
use x4_core::{Action, FileTree};
use x4_rpc::FilePriority::{self, High, Low, Normal};

use super::{Spec, hash, key, other_hash, tree_of};

fn wanted(hash: x4_rpc::HashString, files: &[usize], wanted: bool) -> Action {
    Action::SetFilesWanted {
        hash,
        files: files.to_vec(),
        wanted,
    }
}

fn priority(hash: x4_rpc::HashString, files: &[usize], priority: FilePriority) -> Action {
    Action::SetFilePriority {
        hash,
        files: files.to_vec(),
        priority,
    }
}

#[test]
fn a_file_action_takes_hash_files_and_value_from_its_own_tree() {
    // The same paths in two torrents with other hashes, other file order and other states, as
    // when the selection changes between the moment a row is drawn and the moment it is clicked.
    let a_files: [Spec<'_>; 3] = [
        ("T/a.mkv", 1, 0, true, Normal),
        ("T/sub/b.mkv", 1, 0, false, Low),
        ("T/sub/c.mkv", 1, 0, false, Low),
    ];
    let b_files: [Spec<'_>; 3] = [
        ("T/sub/c.mkv", 1, 0, true, High),
        ("T/sub/b.mkv", 1, 0, true, High),
        ("T/a.mkv", 1, 0, false, Normal),
    ];
    let a = FileTree::new(&details(&hash(), &a_files));
    let b = FileTree::new(&details(&other_hash(), &b_files));

    // A file: A's file is wanted, B's is not.
    let file = key("T/a.mkv");
    assert_eq!(
        a.toggle_wanted_action(&file),
        Some(wanted(hash(), &[0], false))
    );
    assert_eq!(
        b.toggle_wanted_action(&file),
        Some(wanted(other_hash(), &[2], true))
    );
    assert_eq!(
        a.priority_action(&file, High),
        Some(priority(hash(), &[0], High))
    );
    assert_eq!(
        b.priority_action(&file, High),
        Some(priority(other_hash(), &[2], High))
    );
    assert_eq!(
        a.step_priority_action(&file, true),
        Some(priority(hash(), &[0], High)),
        "from A's Normal"
    );
    assert_eq!(
        b.step_priority_action(&file, false),
        Some(priority(other_hash(), &[2], Low)),
        "from B's Normal"
    );

    // A folder: its files are those of its own tree, in the order of its own torrent.
    let folder = key("T/sub");
    assert_eq!(
        a.toggle_wanted_action(&folder),
        Some(wanted(hash(), &[1, 2], true)),
        "A's folder is not wanted at all"
    );
    assert_eq!(
        b.toggle_wanted_action(&folder),
        Some(wanted(other_hash(), &[0, 1], false)),
        "B's folder is wanted entirely"
    );
    assert_eq!(
        a.step_priority_action(&folder, true),
        Some(priority(hash(), &[1, 2], Normal)),
        "from A's Low"
    );
    assert_eq!(
        b.step_priority_action(&folder, true),
        Some(priority(other_hash(), &[0, 1], High)),
        "B's High stays High"
    );
}

#[test]
fn a_key_unknown_to_the_tree_gives_no_action() {
    let tree = tree_of(&[
        ("T/a.mkv", 1, 0, true, Normal),
        ("T/sub/b.mkv", 1, 0, true, Normal),
    ]);

    for unknown in ["T/missing", "T/sub/a.mkv", "", "T/", "U", "t/a.mkv"] {
        let unknown = key(unknown);
        assert_eq!(tree.toggle_wanted_action(&unknown), None, "{unknown:?}");
        assert_eq!(tree.priority_action(&unknown, High), None, "{unknown:?}");
        assert_eq!(
            tree.step_priority_action(&unknown, true),
            None,
            "{unknown:?}"
        );
        assert_eq!(
            tree.step_priority_action(&unknown, false),
            None,
            "{unknown:?}"
        );
    }

    // A key of another torrent's tree means nothing here.
    let other = FileTree::new(&details(
        &other_hash(),
        &[("T/only_in_the_other.mkv", 1, 0, true, Normal)],
    ));
    assert_eq!(
        tree.toggle_wanted_action(&key("T/only_in_the_other.mkv")),
        None
    );
    assert!(
        other
            .toggle_wanted_action(&key("T/only_in_the_other.mkv"))
            .is_some()
    );

    // An empty tree has nothing to act on.
    let empty = tree_of(&[]);
    assert_eq!(empty.toggle_wanted_action(&key("T")), None);
    assert_eq!(empty.priority_action(&key("T"), Low), None);
    assert_eq!(empty.step_priority_action(&key("T"), true), None);
}

#[test]
fn a_folder_of_only_pad_files_gives_no_action() {
    let tree = tree_of(&[
        ("T/a.mkv", 100, 0, true, Normal),
        ("T/pads/_____padding_file_1", 28, 0, true, Normal),
        ("T/pads/deeper/_____padding_file_2", 28, 0, true, Normal),
        ("T/.pad/3", 40, 0, true, Normal),
    ]);

    for hidden in [
        "T/pads",
        "T/pads/deeper",
        "T/pads/_____padding_file_1",
        "T/.pad",
        "T/.pad/3",
    ] {
        let hidden = key(hidden);
        assert_eq!(tree.toggle_wanted_action(&hidden), None, "{hidden:?}");
        assert_eq!(tree.priority_action(&hidden, High), None, "{hidden:?}");
        assert_eq!(tree.step_priority_action(&hidden, true), None, "{hidden:?}");
    }
    // The torrent of padding only has nothing to act on either, not even the whole torrent.
    let only_padding = tree_of(&[("T/_____padding_file_0", 5, 0, true, Normal)]);
    assert_eq!(only_padding.toggle_wanted_action(&key("T")), None);
    assert_eq!(only_padding.priority_action(&key("T"), High), None);
    assert_eq!(only_padding.step_priority_action(&key("T"), false), None);
}

#[test]
fn toggle_wanted_on_a_mixed_folder_wants_all_its_files_including_pad_files() {
    let tree = tree_of(&[
        ("T/a.mkv", 100, 0, true, Normal),
        ("T/_____padding_file_1", 28, 0, false, Normal),
        ("T/sub/b.mkv", 50, 0, false, Normal),
        ("T/sub/_____padding_file_3", 14, 0, false, Normal),
        ("T/off/c.mkv", 50, 0, false, Normal),
        ("T/on/d.mkv", 50, 0, true, Normal),
    ]);

    // Mixed: some of its files are wanted and some are not.
    assert_eq!(
        tree.toggle_wanted_action(&key("T")),
        Some(wanted(hash(), &[0, 1, 2, 3, 4, 5], true)),
        "every file of the folder, the padding ones too"
    );
    // Not wanted at all becomes wanted; wanted entirely becomes not wanted.
    assert_eq!(
        tree.toggle_wanted_action(&key("T/sub")),
        Some(wanted(hash(), &[2, 3], true))
    );
    assert_eq!(
        tree.toggle_wanted_action(&key("T/off")),
        Some(wanted(hash(), &[4], true))
    );
    assert_eq!(
        tree.toggle_wanted_action(&key("T/on")),
        Some(wanted(hash(), &[5], false))
    );
}

#[test]
fn step_priority_saturates_at_high_and_low() {
    let tree = tree_of(&[
        ("T/high.mkv", 1, 0, true, High),
        ("T/normal.mkv", 1, 0, true, Normal),
        ("T/low.mkv", 1, 0, true, Low),
    ]);
    let step = |path: &str, up: bool| tree.step_priority_action(&key(path), up);

    assert_eq!(step("T/high.mkv", true), Some(priority(hash(), &[0], High)));
    assert_eq!(
        step("T/high.mkv", false),
        Some(priority(hash(), &[0], Normal))
    );
    assert_eq!(
        step("T/normal.mkv", true),
        Some(priority(hash(), &[1], High))
    );
    assert_eq!(
        step("T/normal.mkv", false),
        Some(priority(hash(), &[1], Low))
    );
    assert_eq!(
        step("T/low.mkv", true),
        Some(priority(hash(), &[2], Normal))
    );
    assert_eq!(step("T/low.mkv", false), Some(priority(hash(), &[2], Low)));

    // A folder whose files differ becomes the same throughout, the same either way.
    let mixed = priority(hash(), &[0, 1, 2], Normal);
    assert_eq!(step("T", true), Some(mixed.clone()));
    assert_eq!(step("T", false), Some(mixed));
}
