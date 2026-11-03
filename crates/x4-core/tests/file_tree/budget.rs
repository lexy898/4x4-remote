//! A torrent whose file tree would take too much memory: more than 50 000 files, or keys beyond
//! the budget (every node is keyed by its whole path, so a deep path costs the square of its
//! depth). Such a tree is not made; the Files tab says the list is too large instead. Real
//! torrents stay far below both.

use std::time::{Duration, Instant};

use x4_core::test_support::details;
use x4_core::{
    FileTree, FileTreeView, MAX_CHOOSABLE_FILES, NodeKey, TREE_KEY_BUDGET, TorrentDetails,
};
use x4_rpc::FilePriority::Normal;

use super::{hash, indices, key, open_all};

/// Starts the peak of [`peak_kib`] again from the memory in use now, so that what was built
/// before does not hide what comes next; `false` where Linux does not allow it.
fn restart_peak() -> bool {
    std::fs::write("/proc/self/clear_refs", "5").is_ok()
}

/// The peak memory of this process since [`restart_peak`], in KiB, where Linux says it; the
/// tests run one per process.
fn peak_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmHWM:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

/// Makes the tree of `details` and asserts that it is refused within bounded time and memory.
fn assert_refused_cheaply(details: &TorrentDetails) {
    let measured = restart_peak();
    let peak_before = peak_kib();
    let start = Instant::now();

    let tree = FileTree::new(details);

    let elapsed = start.elapsed();
    assert!(tree.is_too_large());
    assert!(tree.is_empty());
    assert_eq!(FileTreeView::new(&tree).rows(&tree), Vec::new());
    // Some 25 ms in a release build and 400 ms in a debug one; the bound only says it ends.
    assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
    if let (true, Some(before), Some(after)) = (measured, peak_before, peak_kib()) {
        // The budget and the nodes around it, far from the hundreds of MiB of a whole tree.
        assert!(
            after.saturating_sub(before) < 64 * 1024,
            "{before} KiB to {after} KiB"
        );
    }
}

#[test]
fn a_deep_path_is_too_large_to_show() {
    // About 40 KB of names: 20 000 folders, one in the other. The whole tree would take some
    // 400 MiB of keys.
    let path = "a/".repeat(20_000) + "f";
    let deep = details(&hash(), &[(path.as_str(), 5, 0, true, Normal)]);

    assert_refused_cheaply(&deep);
    assert_eq!(indices(&FileTree::new(&deep), "a"), None);
    assert_eq!(FileTree::new(&deep).toggle_wanted_action(&key("a")), None);
}

#[test]
fn many_deep_chains_that_part_near_the_top_are_too_large() {
    // Each path alone is 4 KB and its chain some 4 MiB of keys; eight of them pass the budget.
    let paths: Vec<String> = (0..8)
        .map(|chain| format!("T/x{chain}/{}f", "a/".repeat(2_048)))
        .collect();
    let files: Vec<(&str, u64, u64, bool, x4_rpc::FilePriority)> = paths
        .iter()
        .map(|path| (path.as_str(), 1, 0, true, Normal))
        .collect();

    assert_refused_cheaply(&details(&hash(), &files));
    // Three of them fit.
    let fewer = FileTree::new(&details(&hash(), files.get(..3).unwrap()));
    assert!(!fewer.is_too_large());
    assert_eq!(indices(&fewer, "T"), Some(vec![0, 1, 2]));
}

#[test]
fn a_real_shaped_tree_is_not_affected() {
    // 5000 episodes four levels deep, with long names.
    let paths: Vec<String> = (0..5_000)
        .map(|n| {
            format!(
                "A Long Running Show/Season {:02}/Disc {}/Episode {n:04} - A Rather Long Title \
                 Of The Episode (1080p).mkv",
                n / 250,
                n / 50 % 5
            )
        })
        .collect();
    let files: Vec<(&str, u64, u64, bool, x4_rpc::FilePriority)> = paths
        .iter()
        .map(|path| (path.as_str(), 1 << 30, 0, true, Normal))
        .collect();

    let tree = FileTree::new(&details(&hash(), &files));

    assert!(!tree.is_too_large());
    assert_eq!(
        indices(&tree, "A Long Running Show"),
        Some((0..5_000).collect())
    );
    // Far below the budget: the keys take well under a MiB.
    let key_bytes: usize = open_all(&tree)
        .rows(&tree)
        .iter()
        .map(|row| row.key.as_str().len())
        .sum();
    assert!(key_bytes < TREE_KEY_BUDGET / 16, "{key_bytes}");
    assert!(tree.contains(&NodeKey::new("A Long Running Show/Season 19/Disc 4")));
}

/// The details of `count` files, `T/0` to `T/{count - 1}`.
fn numbered(count: usize) -> TorrentDetails {
    let paths: Vec<String> = (0..count).map(|n| format!("T/{n}")).collect();
    let files: Vec<(&str, u64, u64, bool, x4_rpc::FilePriority)> = paths
        .iter()
        .map(|path| (path.as_str(), 1, 0, true, Normal))
        .collect();
    details(&hash(), &files)
}

#[test]
fn more_than_50_000_files_are_too_large_to_show() {
    assert_eq!(MAX_CHOOSABLE_FILES, 50_000);

    let most = FileTree::new(&numbered(MAX_CHOOSABLE_FILES));
    assert!(!most.is_too_large());
    assert_eq!(
        indices(&most, "T"),
        Some((0..MAX_CHOOSABLE_FILES).collect())
    );

    // One more, and no tree is made at all, however short the paths.
    assert_refused_cheaply(&numbered(MAX_CHOOSABLE_FILES + 1));
}

/// The details of the files at `paths`, each one byte long and wanted.
fn details_at(paths: &[String]) -> TorrentDetails {
    let files: Vec<(&str, u64, u64, bool, x4_rpc::FilePriority)> = paths
        .iter()
        .map(|path| (path.as_str(), 1, 0, true, Normal))
        .collect();
    details(&hash(), &files)
}

#[test]
fn a_path_listed_50_000_times_is_shown_in_bounded_time() {
    // The daemon may list a path more than once; each copy is its own row, under a key with a
    // suffix. Finding each suffix by trying them all from the first took minutes.
    let paths = vec!["T/a".to_owned(); MAX_CHOOSABLE_FILES];
    let listed = details_at(&paths);
    let start = Instant::now();

    let tree = FileTree::new(&listed);

    let elapsed = start.elapsed();
    assert!(!tree.is_too_large());
    assert_eq!(
        indices(&tree, "T"),
        Some((0..MAX_CHOOSABLE_FILES).collect())
    );
    assert_eq!(indices(&tree, "T/a"), Some(vec![0]));
    assert_eq!(indices(&tree, "T/a\u{0}1"), Some(vec![1]));
    assert_eq!(indices(&tree, "T/a\u{0}49999"), Some(vec![49_999]));
    assert_eq!(indices(&tree, "T/a\u{0}50000"), None);
    // Some 30 ms in a release build; the bound only says it ends.
    assert!(elapsed < Duration::from_secs(10), "{elapsed:?}");
}
