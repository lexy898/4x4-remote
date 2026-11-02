//! Opening and closing folders and moving the cursor.

use x4_core::{Edge, FileTreeView, NodeKey};
use x4_rpc::FilePriority::Normal;

use super::{key, shape, tree_of};

#[test]
fn expand_collapse_and_cursor_moves() {
    let tree = tree_of(&[
        ("T/sub/b.mkv", 1, 0, true, Normal),
        ("T/sub/c.mkv", 1, 0, true, Normal),
        ("T/a.mkv", 1, 0, true, Normal),
    ]);
    let mut view = FileTreeView::new(&tree);
    assert_eq!(view.cursor(&tree), None);
    let cursor = |view: &FileTreeView| view.cursor(&tree).map(NodeKey::as_str).map(str::to_owned);

    // Down goes to the first row, then on, and stops at the last.
    view.move_cursor(&tree, 1);
    assert_eq!(cursor(&view).as_deref(), Some("T"));
    view.move_cursor(&tree, 1);
    assert_eq!(cursor(&view).as_deref(), Some("T/sub"));
    view.move_cursor(&tree, 1);
    assert_eq!(cursor(&view).as_deref(), Some("T/a.mkv"));
    view.move_cursor(&tree, 1);
    assert_eq!(cursor(&view).as_deref(), Some("T/a.mkv"));
    view.move_cursor(&tree, -1);
    assert_eq!(cursor(&view).as_deref(), Some("T/sub"));

    // Opening a folder lists its entries; the cursor goes in and out of it.
    assert!(!view.is_expanded(&tree, &key("T/sub")));
    view.toggle_expanded(&tree, &key("T/sub"));
    assert!(view.is_expanded(&tree, &key("T/sub")));
    assert_eq!(
        shape(&view.rows(&tree)),
        [
            ("T", 0),
            ("T/sub", 1),
            ("T/sub/b.mkv", 2),
            ("T/sub/c.mkv", 2),
            ("T/a.mkv", 1)
        ]
    );
    view.cursor_to_first_child(&tree);
    assert_eq!(cursor(&view).as_deref(), Some("T/sub/b.mkv"));
    view.cursor_to_first_child(&tree);
    assert_eq!(
        cursor(&view).as_deref(),
        Some("T/sub/b.mkv"),
        "a file has no entries"
    );
    view.cursor_to_parent(&tree);
    assert_eq!(cursor(&view).as_deref(), Some("T/sub"));
    view.cursor_to_parent(&tree);
    assert_eq!(cursor(&view).as_deref(), Some("T"));
    view.cursor_to_parent(&tree);
    assert_eq!(
        cursor(&view).as_deref(),
        Some("T"),
        "the top folder has no folder around it"
    );

    // Home and End are steps as far as the list goes.
    view.move_cursor(&tree, isize::MAX);
    assert_eq!(cursor(&view).as_deref(), Some("T/a.mkv"));
    view.move_cursor(&tree, isize::MIN);
    assert_eq!(cursor(&view).as_deref(), Some("T"));

    // Closing a folder that holds the cursor takes the cursor to the folder.
    view.set_cursor(&tree, &key("T/sub/c.mkv"));
    assert_eq!(cursor(&view).as_deref(), Some("T/sub/c.mkv"));
    view.toggle_expanded(&tree, &key("T/sub"));
    assert_eq!(cursor(&view).as_deref(), Some("T/sub"));
    assert_eq!(
        shape(&view.rows(&tree)),
        [("T", 0), ("T/sub", 1), ("T/a.mkv", 1)]
    );

    // A file cannot be opened, and a key that is not in the tree is not a place for the cursor.
    view.toggle_expanded(&tree, &key("T/a.mkv"));
    assert!(!view.is_expanded(&tree, &key("T/a.mkv")));
    view.set_cursor(&tree, &key("T/missing"));
    assert_eq!(cursor(&view).as_deref(), Some("T/sub"));
}

#[test]
fn a_step_without_a_cursor_starts_at_the_first_row_either_way() {
    let tree = tree_of(&[
        ("T/a.mkv", 1, 0, true, Normal),
        ("T/b.mkv", 1, 0, true, Normal),
    ]);

    // As in the table with nothing selected: Down, Up, Page Down and Page Up all start at the
    // top, and only End goes to the bottom.
    for step in [1, -1, 5, -5] {
        let mut view = FileTreeView::new(&tree);
        view.move_cursor(&tree, step);
        assert_eq!(view.cursor(&tree), Some(&key("T")), "step {step}");
    }
}

#[test]
fn home_and_end_go_to_the_first_and_the_last_row_shown() {
    let tree = tree_of(&[
        ("T/sub/b.mkv", 1, 0, true, Normal),
        ("T/a.mkv", 1, 0, true, Normal),
        ("T/c.mkv", 1, 0, true, Normal),
    ]);

    // Without a cursor, Home goes to the first row and End to the last, not the other way.
    let mut home = FileTreeView::new(&tree);
    home.cursor_to_edge(&tree, Edge::First);
    assert_eq!(home.cursor(&tree), Some(&key("T")));
    let mut end = FileTreeView::new(&tree);
    end.cursor_to_edge(&tree, Edge::Last);
    assert_eq!(end.cursor(&tree), Some(&key("T/c.mkv")));

    // From a cursor, the last row is the last one shown: the closed folder hides its file.
    end.cursor_to_edge(&tree, Edge::First);
    assert_eq!(end.cursor(&tree), Some(&key("T")));
    end.toggle_expanded(&tree, &key("T/sub"));
    end.cursor_to_edge(&tree, Edge::Last);
    assert_eq!(end.cursor(&tree), Some(&key("T/c.mkv")));
    end.toggle_expanded(&tree, &key("T"));
    end.cursor_to_edge(&tree, Edge::Last);
    assert_eq!(
        end.cursor(&tree),
        Some(&key("T")),
        "only the top folder is shown"
    );

    // A tree with nothing to show has no place for the cursor.
    let empty = tree_of(&[]);
    let mut nowhere = FileTreeView::new(&empty);
    nowhere.cursor_to_edge(&empty, Edge::Last);
    assert_eq!(nowhere.cursor(&empty), None);
}

#[test]
fn the_cursor_stays_on_rows_that_are_shown() {
    let tree = tree_of(&[
        ("T/sub/b.mkv", 1, 0, true, Normal),
        ("T/a.mkv", 1, 0, true, Normal),
    ]);
    let mut view = FileTreeView::new(&tree);

    // The file is inside a folder that is closed.
    view.set_cursor(&tree, &key("T/sub/b.mkv"));
    assert_eq!(view.cursor(&tree), None);

    view.toggle_expanded(&tree, &key("T/sub"));
    view.set_cursor(&tree, &key("T/sub/b.mkv"));
    assert_eq!(view.cursor(&tree), Some(&key("T/sub/b.mkv")));

    // Closing the top folder takes the cursor out of the hidden rows.
    view.toggle_expanded(&tree, &key("T"));
    assert_eq!(view.cursor(&tree), Some(&key("T")));
}

#[test]
fn left_and_right_open_close_and_enter_folders() {
    let tree = tree_of(&[
        ("T/sub/b.mkv", 1, 0, true, Normal),
        ("T/a.mkv", 1, 0, true, Normal),
    ]);
    let mut view = FileTreeView::new(&tree);
    view.set_cursor(&tree, &key("T/sub"));

    view.expand_or_first_child(&tree);
    assert!(view.is_expanded(&tree, &key("T/sub")));
    assert_eq!(
        view.cursor(&tree),
        Some(&key("T/sub")),
        "the first press opens"
    );
    view.expand_or_first_child(&tree);
    assert_eq!(
        view.cursor(&tree),
        Some(&key("T/sub/b.mkv")),
        "the second press goes in"
    );
    view.expand_or_first_child(&tree);
    assert_eq!(
        view.cursor(&tree),
        Some(&key("T/sub/b.mkv")),
        "a file has nothing to open"
    );

    view.collapse_or_parent(&tree);
    assert_eq!(
        view.cursor(&tree),
        Some(&key("T/sub")),
        "from a file, to its folder"
    );
    view.collapse_or_parent(&tree);
    assert!(
        !view.is_expanded(&tree, &key("T/sub")),
        "an open folder closes"
    );
    assert_eq!(view.cursor(&tree), Some(&key("T/sub")));
    view.collapse_or_parent(&tree);
    assert_eq!(
        view.cursor(&tree),
        Some(&key("T")),
        "a closed folder goes to the one around it"
    );
}
