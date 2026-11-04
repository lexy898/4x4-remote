//! The tests of the Files tab: what it holds as the tab, the panel and the target change, when
//! the list draws again, and how the tree of a big torrent is made off the window's thread.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{ElementId, ScrollDelta, Subscription, TestAppContext, WindowHandle, point, px};
use x4_core::test_support::{details, torrent};
use x4_core::{CHEAP_TREE_FILES, DetailsTab, DetailsTarget, Edge};
use x4_rpc::HashString;

use super::*;
use crate::app;

/// A read of the details of the torrent `hash`, a new one each time, as the service sends.
fn read(hash: &HashString) -> Arc<TorrentDetails> {
    Arc::new(details(
        hash,
        &[("Show/film.mkv", 1024, 0, true, FilePriority::Normal)],
    ))
}

/// The tree of the Files tab, if it has one.
fn tree_of(panel: WindowHandle<DetailsPanel>, cx: &mut TestAppContext) -> Option<Arc<FileTree>> {
    panel
        .update(cx, |panel, _, _| panel.files.tree.clone())
        .expect("the panel is open")
}

/// Runs `change` on the panel.
fn change(
    panel: WindowHandle<DetailsPanel>,
    cx: &mut TestAppContext,
    change: impl FnOnce(&mut DetailsPanel, &mut Context<'_, DetailsPanel>),
) {
    panel
        .update(cx, |panel, _, cx| change(panel, cx))
        .expect("the panel is open");
}

fn made_from_read(
    panel: WindowHandle<DetailsPanel>,
    read: &Arc<TorrentDetails>,
    cx: &mut TestAppContext,
) -> bool {
    tree_of(panel, cx).is_some_and(|tree| tree.is_made_from(read))
}

#[gpui_kit::test]
fn a_key_that_changes_nothing_draws_nothing_again(cx: &mut TestAppContext) {
    let mut tab = cx.update(FilesTab::new);
    let film = torrent("a");
    assert_eq!(tab.follow(Some(&read(&film.hash))), Follow::Changed);
    let step = |by: isize| {
        move |view: &mut FileTreeView, tree: &FileTree| {
            view.move_cursor(tree, by);
        }
    };

    assert!(
        tab.change_view(None, step(1)),
        "the cursor comes to the first row"
    );
    assert!(!tab.change_view(None, step(-1)), "Up on the first row");
    assert!(tab.change_view(None, step(1)), "Down to the film");
    assert!(!tab.change_view(None, step(1)), "Down on the last row");
    assert!(
        tab.change_view(None, FileTreeView::collapse_or_parent),
        "Left goes up to the folder"
    );
    assert!(
        tab.change_view(None, FileTreeView::collapse_or_parent),
        "Left closes it"
    );
}

/// What the Files tab of the panel holds: whether it has a tree, the keys of its rows and
/// the row of the cursor.
fn held(
    panel: WindowHandle<DetailsPanel>,
    cx: &mut TestAppContext,
) -> (bool, Vec<String>, Option<String>) {
    panel
        .update(cx, |panel, _, _| {
            let files = &panel.files;
            let rows = files
                .rows
                .iter()
                .map(|row| row.key.as_str().to_owned())
                .collect();
            let cursor = files
                .tree
                .as_ref()
                .and_then(|tree| files.view.cursor(tree))
                .map(|key| key.as_str().to_owned());
            (files.tree.is_some(), rows, cursor)
        })
        .expect("the panel is open")
}

/// A panel on the tab `tab`, about no torrent yet, which makes the trees of big torrents on the
/// executor, where the test runs them when it runs the tasks.
fn panel_on(tab: DetailsTab, cx: &mut TestAppContext) -> WindowHandle<DetailsPanel> {
    cx.update(app::init);
    cx.add_window(|window, cx| {
        let mut panel = DetailsPanel::new(tab, Rc::new(RefCell::default()), window, cx);
        panel.make_trees_on_the_executor();
        panel
    })
}

/// A read of the details of the torrent `hash`: a show with a season of two episodes.
fn season(hash: &HashString) -> Arc<TorrentDetails> {
    Arc::new(details(
        hash,
        &[
            ("Show/Season 1/E01.mkv", 1024, 0, true, FilePriority::Normal),
            ("Show/Season 1/E02.mkv", 1024, 0, true, FilePriority::Normal),
            ("Show/info.nfo", 1, 1, true, FilePriority::Normal),
        ],
    ))
}

/// Shows `show` on `panel`, with a read of [`season`], and has the user open the season and go
/// to its second episode; returns what the Files tab holds then.
fn at_the_second_episode(
    panel: WindowHandle<DetailsPanel>,
    show: &Torrent,
    cx: &mut TestAppContext,
) -> (bool, Vec<String>, Option<String>) {
    change(panel, cx, |panel, cx| {
        panel.show(DetailsTarget::One(show.hash.clone()), Some(show), cx);
        panel.take_details(season(&show.hash), cx);
        let folder = NodeKey::new("Show/Season 1");
        let episode = NodeKey::new("Show/Season 1/E02.mkv");
        panel.change_file_view(
            None,
            |view, tree| {
                view.toggle_expanded(tree, &folder);
                view.set_cursor(tree, &episode);
            },
            cx,
        );
    });
    let held = held(panel, cx);
    assert_eq!(
        held,
        (
            true,
            [
                "Show",
                "Show/Season 1",
                "Show/Season 1/E01.mkv",
                "Show/Season 1/E02.mkv",
                "Show/info.nfo",
            ]
            .map(str::to_owned)
            .to_vec(),
            Some("Show/Season 1/E02.mkv".to_owned())
        )
    );
    held
}

/// The progress of the row at `path` of the Files tab of `panel`, in whole percent.
fn percent_of(panel: WindowHandle<DetailsPanel>, path: &str, cx: &mut TestAppContext) -> u8 {
    panel
        .update(cx, |panel, _, _| {
            panel
                .files
                .rows
                .iter()
                .find(|row| row.key.as_str() == path)
                .map(|row| row.percent)
        })
        .expect("the panel is open")
        .expect("the row is listed")
}

#[gpui_kit::test]
fn a_read_of_the_same_files_keeps_the_tree_its_rows_and_the_cursor(cx: &mut TestAppContext) {
    let panel = panel_on(DetailsTab::Files, cx);
    let show = torrent("a");
    let before = at_the_second_episode(panel, &show, cx);
    let first = tree_of(panel, cx).expect("a tree");

    // The next read of the progress lists the same files: the tree is not made again, and shows
    // what the read says.
    let progressed = Arc::new(details(
        &show.hash,
        &[
            ("Show/Season 1/E01.mkv", 1024, 512, true, FilePriority::High),
            (
                "Show/Season 1/E02.mkv",
                1024,
                0,
                false,
                FilePriority::Normal,
            ),
            ("Show/info.nfo", 1, 1, true, FilePriority::Normal),
        ],
    ));
    change(panel, cx, |panel, cx| {
        panel.take_details(Arc::clone(&progressed), cx);
    });

    let second = tree_of(panel, cx).expect("a tree");
    assert!(second.shares_files_with(&first), "not made again");
    assert!(second.is_made_from(&progressed));
    assert_eq!(held(panel, cx), before);
    assert_eq!(percent_of(panel, "Show/Season 1/E01.mkv", cx), 50);

    // A read in which a file has another name lists other files: a new tree, fitted to them.
    let renamed = Arc::new(details(
        &show.hash,
        &[
            ("Show/Season 1/E01.mkv", 1024, 512, true, FilePriority::High),
            (
                "Show/Season 1/E02 (fixed).mkv",
                1024,
                0,
                false,
                FilePriority::Normal,
            ),
            ("Show/info.nfo", 1, 1, true, FilePriority::Normal),
        ],
    ));
    change(panel, cx, |panel, cx| panel.take_details(renamed, cx));

    let third = tree_of(panel, cx).expect("a tree");
    assert!(!third.shares_files_with(&second));
    let (_, rows, cursor) = held(panel, cx);
    assert!(rows.contains(&"Show/Season 1/E02 (fixed).mkv".to_owned()));
    assert_eq!(
        cursor.as_deref(),
        Some("Show/Season 1"),
        "the folder that is left"
    );
}

/// What the panel keeps of the reads it was given: whether it has details, a General model and a
/// tree, which keeps the details it was made from, and how many rows its Files tab has room for.
fn kept(panel: WindowHandle<DetailsPanel>, cx: &mut TestAppContext) -> (bool, bool, bool, usize) {
    panel
        .update(cx, |panel, _, _| {
            let files = &panel.files;
            (
                panel.details.is_some(),
                panel.general.is_some(),
                files.tree.is_some(),
                files.rows.capacity(),
            )
        })
        .expect("the panel is open")
}

#[gpui_kit::test]
fn leaving_the_files_tab_drops_the_tree_but_not_the_cursor_and_the_open_folders(
    cx: &mut TestAppContext,
) {
    let panel = panel_on(DetailsTab::Files, cx);
    let show = torrent("a");
    let before = at_the_second_episode(panel, &show, cx);

    change(panel, cx, |panel, cx| {
        panel.set_tab(DetailsTab::General, cx);
    });
    assert_eq!(held(panel, cx), (false, Vec::new(), None), "nothing held");

    // A read comes meanwhile; back on Files, the tree is made again, as it was left.
    change(panel, cx, |panel, cx| {
        panel.take_details(season(&show.hash), cx);
        panel.set_tab(DetailsTab::Files, cx);
    });
    assert_eq!(held(panel, cx), before);
}

#[gpui_kit::test]
fn a_hidden_panel_holds_no_details_tree_or_rows_but_keeps_the_cursor_and_the_open_folders(
    cx: &mut TestAppContext,
) {
    let panel = panel_on(DetailsTab::Files, cx);
    let show = torrent("a");
    let before = at_the_second_episode(panel, &show, cx);

    change(panel, cx, |panel, _| panel.hide());
    assert_eq!(kept(panel, cx), (false, false, false, 0), "nothing kept");
    // A read that was on its way when the panel hid is not kept either.
    change(panel, cx, |panel, cx| {
        panel.take_details(season(&show.hash), cx);
    });
    assert_eq!(kept(panel, cx), (false, false, false, 0));

    // Shown again on the same torrent, the panel waits for the next read, which brings the tree
    // back as it was left.
    change(panel, cx, |panel, cx| {
        panel.show(DetailsTarget::One(show.hash.clone()), Some(&show), cx);
    });
    assert_eq!(held(panel, cx), (false, Vec::new(), None), "no read yet");
    change(panel, cx, |panel, cx| {
        panel.take_details(season(&show.hash), cx);
    });
    assert_eq!(held(panel, cx), before);
}

#[gpui_kit::test]
fn a_target_of_several_torrents_or_another_one_drops_the_old_details(cx: &mut TestAppContext) {
    let panel = panel_on(DetailsTab::General, cx);
    let (a, b) = (torrent("a"), torrent("b"));
    change(panel, cx, |panel, cx| {
        panel.show(DetailsTarget::One(a.hash.clone()), Some(&a), cx);
        panel.take_details(read(&a.hash), cx);
    });
    assert_eq!(kept(panel, cx), (true, true, false, 0));

    change(panel, cx, |panel, cx| {
        panel.show(DetailsTarget::Many(2), None, cx);
    });
    assert_eq!(kept(panel, cx), (false, false, false, 0), "several");

    // Back on the one torrent, its details wait for the next read.
    change(panel, cx, |panel, cx| {
        panel.show(DetailsTarget::One(a.hash.clone()), Some(&a), cx);
    });
    assert_eq!(kept(panel, cx), (false, true, false, 0), "until the read");
    change(panel, cx, |panel, cx| {
        panel.take_details(read(&a.hash), cx);
        panel.show(DetailsTarget::One(b.hash.clone()), Some(&b), cx);
    });
    assert_eq!(kept(panel, cx), (false, true, false, 0), "another torrent");

    // A read of the old torrent that comes late is not kept for the new one.
    change(panel, cx, |panel, cx| {
        panel.take_details(read(&a.hash), cx);
    });
    assert_eq!(kept(panel, cx), (false, true, false, 0));
    change(panel, cx, |panel, cx| {
        panel.show(DetailsTarget::None, None, cx);
    });
    assert_eq!(kept(panel, cx), (false, false, false, 0), "none");
}

/// A read of the details of the torrent `hash`: an album of 2000 tracks.
fn album(hash: &HashString) -> Arc<TorrentDetails> {
    let names: Vec<String> = (1..=2000)
        .map(|track| format!("Album/Track {track:04}.flac"))
        .collect();
    let files: Vec<(&str, u64, u64, bool, FilePriority)> = names
        .iter()
        .map(|name| (name.as_str(), 1024, 0, true, FilePriority::Normal))
        .collect();
    Arc::new(details(hash, &files))
}

#[gpui_kit::test]
fn no_details_leave_no_room_kept_for_the_rows(cx: &mut TestAppContext) {
    let mut tab = cx.update(FilesTab::new);
    let album = album(&torrent("album").hash);
    // Too many files to make the tree at once: the panel has it made, and hands it over.
    assert_eq!(tab.follow(Some(&album)), Follow::Build(Arc::clone(&album)));
    assert_eq!(
        tab.take_built(FileTree::new(&album), Some(&album)),
        Follow::Changed
    );
    assert_eq!(tab.rows.len(), 2001);

    assert_eq!(tab.follow(None), Follow::Changed);

    assert_eq!(tab.rows.capacity(), 0, "the room of 2001 rows is let go");
    assert!(tab.tree.is_none());
}

/// Draws the window of `panel`.
fn draw(panel: WindowHandle<DetailsPanel>, cx: &mut TestAppContext) {
    cx.update_window(panel.into(), |_, window, cx| window.render_frame(cx))
        .expect("the panel is open");
}

/// Whether the row at `path` was drawn in the file list, which draws only the rows in sight.
fn row_drawn(panel: WindowHandle<DetailsPanel>, path: &str, cx: &mut TestAppContext) -> bool {
    let row = ElementId::NamedChild(
        Arc::new(ElementId::Name("file-row".into())),
        path.to_owned().into(),
    );
    cx.update_window(panel.into(), |_, window, _| {
        window.within(FILE_LIST_ID).try_find(row).is_some()
    })
    .expect("the panel is open")
}

/// Counts from now on how often `panel` asks to be drawn again; the count lasts as long as the
/// subscription.
fn count_redraws(
    panel: WindowHandle<DetailsPanel>,
    cx: &mut TestAppContext,
) -> (Rc<Cell<usize>>, Subscription) {
    let entity = panel.root(cx).expect("the panel is open");
    let count = Rc::new(Cell::new(0));
    let counter = Rc::clone(&count);
    let subscription =
        cx.update(|cx| cx.observe(&entity, move |_, _| counter.set(counter.get() + 1)));
    (count, subscription)
}

#[gpui_kit::test]
fn a_key_draws_the_list_again_to_bring_the_cursor_back_into_sight(cx: &mut TestAppContext) {
    let panel = panel_on(DetailsTab::Files, cx);
    let tracks = torrent("album");
    let last = "Album/Track 2000.flac";
    change(panel, cx, |panel, cx| {
        panel.show(DetailsTarget::One(tracks.hash.clone()), Some(&tracks), cx);
        panel.take_details(album(&tracks.hash), cx);
    });
    cx.run_until_parked();
    draw(panel, cx);
    // End: the cursor goes to the last row, and the list scrolls to it.
    change(panel, cx, |panel, cx| {
        panel.change_file_view(None, |view, tree| view.cursor_to_edge(tree, Edge::Last), cx);
    });
    draw(panel, cx);
    assert!(row_drawn(panel, last, cx) && !row_drawn(panel, "Album", cx));

    // Down on the last row, in sight, changes nothing: nothing is drawn again.
    let (redraws, _counting) = count_redraws(panel, cx);
    change(panel, cx, |panel, cx| {
        panel.change_file_view(None, |view, tree| view.move_cursor(tree, 1), cx);
    });
    assert_eq!(redraws.get(), 0, "in sight");

    // The wheel takes the list back to its top, away from the cursor.
    cx.update_window(panel.into(), |_, window, cx| {
        window.scroll(
            FILE_LIST_ID,
            ScrollDelta::Pixels(point(px(0.), px(100_000.))),
            cx,
        );
    })
    .expect("the panel is open");
    draw(panel, cx);
    assert!(row_drawn(panel, "Album", cx) && !row_drawn(panel, last, cx));
    redraws.set(0);

    // Down on the last row still moves nowhere, but the list is drawn again, back at the cursor.
    change(panel, cx, |panel, cx| {
        panel.change_file_view(None, |view, tree| view.move_cursor(tree, 1), cx);
    });
    assert_eq!(redraws.get(), 1, "out of sight");
    draw(panel, cx);
    assert!(row_drawn(panel, last, cx) && !row_drawn(panel, "Album", cx));
}

#[gpui_kit::test]
fn the_tree_is_made_only_while_the_files_tab_is_shown(cx: &mut TestAppContext) {
    cx.update(app::init);
    let panel = cx.add_window(|window, cx| {
        DetailsPanel::new(DetailsTab::General, Rc::new(RefCell::default()), window, cx)
    });
    let film = torrent("a");
    let first = read(&film.hash);
    change(panel, cx, |panel, cx| {
        panel.show(DetailsTarget::One(film.hash.clone()), Some(&film), cx);
        panel.take_details(Arc::clone(&first), cx);
    });
    assert!(tree_of(panel, cx).is_none(), "no tree on the General tab");

    // The progress is read again every few seconds; on the General tab that makes no tree.
    let second = read(&film.hash);
    change(panel, cx, |panel, cx| {
        panel.take_details(Arc::clone(&second), cx);
    });
    assert!(tree_of(panel, cx).is_none(), "still none");

    // The Files tab makes it from the newest read, and again from each newer one.
    change(panel, cx, |panel, cx| {
        panel.set_tab(DetailsTab::Files, cx);
    });
    assert!(made_from_read(panel, &second, cx));
    let third = read(&film.hash);
    change(panel, cx, |panel, cx| {
        panel.take_details(Arc::clone(&third), cx);
    });
    assert!(made_from_read(panel, &third, cx));

    // Back on General, the tree goes, and reads make none until the Files tab is shown again.
    change(panel, cx, |panel, cx| {
        panel.set_tab(DetailsTab::General, cx);
    });
    let fourth = read(&film.hash);
    change(panel, cx, |panel, cx| {
        panel.take_details(Arc::clone(&fourth), cx);
    });
    assert!(tree_of(panel, cx).is_none(), "none on General");
    change(panel, cx, |panel, cx| {
        panel.set_tab(DetailsTab::Files, cx);
    });
    assert!(made_from_read(panel, &fourth, cx));
}

/// The name of the last file of [`big`].
const LAST: &str = "Box/Last.tif";

/// A read of the details of the torrent `hash` with one file more than a tree is made of on the
/// window's thread ([`CHEAP_TREE_FILES`]): scans in parts of 100, and last the file `last`.
fn big(hash: &HashString, last: &str) -> Arc<TorrentDetails> {
    let mut names: Vec<String> = (1..=CHEAP_TREE_FILES)
        .map(|scan| format!("Box/Part {:02}/Scan {scan:04}.tif", scan / 100))
        .collect();
    names.push(last.to_owned());
    let files: Vec<(&str, u64, u64, bool, FilePriority)> = names
        .iter()
        .map(|name| (name.as_str(), 1024, 0, true, FilePriority::Normal))
        .collect();
    Arc::new(details(hash, &files))
}

/// Shows `scans` on the Files tab of `panel`, with the details `read`.
fn show_with(
    panel: WindowHandle<DetailsPanel>,
    scans: &Torrent,
    read: &Arc<TorrentDetails>,
    cx: &mut TestAppContext,
) {
    change(panel, cx, |panel, cx| {
        panel.show(DetailsTarget::One(scans.hash.clone()), Some(scans), cx);
        panel.take_details(Arc::clone(read), cx);
    });
}

#[gpui_kit::test]
fn a_big_torrents_tree_is_not_built_before_the_tasks_run(cx: &mut TestAppContext) {
    let panel = panel_on(DetailsTab::Files, cx);
    let scans = torrent("a");
    let read = big(&scans.hash, LAST);

    show_with(panel, &scans, &read, cx);
    assert!(tree_of(panel, cx).is_none(), "not on the window's thread");

    cx.run_until_parked();
    assert!(made_from_read(panel, &read, cx));
}

/// Whether the Files tab of `panel` draws the "—" of no tree, the switch and the file list.
fn drawn_parts(panel: WindowHandle<DetailsPanel>, cx: &mut TestAppContext) -> [bool; 3] {
    draw(panel, cx);
    cx.update_window(panel.into(), |_, window, _| {
        ["files-pending", "file-sequential", FILE_LIST_ID].map(|id| window.try_find(id).is_some())
    })
    .expect("the panel is open")
}

#[gpui_kit::test]
fn the_tab_shows_the_dash_and_the_switch_while_the_tree_is_built(cx: &mut TestAppContext) {
    let panel = panel_on(DetailsTab::Files, cx);
    let scans = torrent("a");

    show_with(panel, &scans, &big(&scans.hash, LAST), cx);
    assert_eq!(drawn_parts(panel, cx), [true, true, false]);

    cx.run_until_parked();
    assert_eq!(drawn_parts(panel, cx), [false, true, true]);
}

#[gpui_kit::test]
fn a_tree_for_a_torrent_no_longer_shown_is_dropped(cx: &mut TestAppContext) {
    let panel = panel_on(DetailsTab::Files, cx);
    let (a, b) = (torrent("a"), torrent("b"));
    show_with(panel, &a, &big(&a.hash, LAST), cx);
    assert!(tree_of(panel, cx).is_none(), "A's tree is being made");
    change(panel, cx, |panel, cx| {
        panel.show(DetailsTarget::One(b.hash.clone()), Some(&b), cx);
    });

    cx.run_until_parked();
    assert_eq!(held(panel, cx), (false, Vec::new(), None), "nothing of A");

    // B's own details, of a few files, give B's tree at once.
    change(panel, cx, |panel, cx| panel.take_details(read(&b.hash), cx));
    let tree = tree_of(panel, cx).expect("B's tree");
    assert_eq!(tree.hash(), &b.hash);
    cx.run_until_parked();
    assert!(tree_of(panel, cx).is_some_and(|after| Arc::ptr_eq(&after, &tree)));
}

#[gpui_kit::test]
fn a_tree_that_comes_for_another_torrent_is_dropped(cx: &mut TestAppContext) {
    let panel = panel_on(DetailsTab::Files, cx);
    let (a, b) = (torrent("a"), torrent("b"));
    let read = big(&a.hash, LAST);
    show_with(panel, &a, &read, cx);

    // As a tree on its way when the torrent changed would come.
    let other = FileTree::new(&big(&b.hash, LAST));
    change(panel, cx, |panel, cx| panel.tree_built(other, cx));
    assert!(tree_of(panel, cx).is_none(), "not B's");

    cx.run_until_parked();
    assert!(made_from_read(panel, &read, cx), "A's own still comes");
}

/// Makes no tree, as when no thread can be started.
fn make_nowhere(_: Arc<TorrentDetails>, _: MakeTree, _: &App) -> Task<Option<FileTree>> {
    Task::ready(None)
}

#[gpui_kit::test]
fn a_tree_no_other_thread_could_make_is_made_on_the_windows_thread(cx: &mut TestAppContext) {
    let panel = panel_on(DetailsTab::Files, cx);
    change(panel, cx, |panel, _| panel.files.make_on = make_nowhere);
    let scans = torrent("a");
    let read = big(&scans.hash, LAST);

    show_with(panel, &scans, &read, cx);
    cx.run_until_parked();

    assert!(made_from_read(panel, &read, cx), "late, rather than never");
}

#[gpui_kit::test]
fn closing_the_tab_while_the_tree_is_built_drops_it(cx: &mut TestAppContext) {
    let panel = panel_on(DetailsTab::Files, cx);
    let scans = torrent("a");
    let read = big(&scans.hash, LAST);
    show_with(panel, &scans, &read, cx);
    change(panel, cx, |panel, cx| {
        panel.set_tab(DetailsTab::General, cx);
    });

    cx.run_until_parked();
    assert!(tree_of(panel, cx).is_none(), "no tree for a closed tab");

    // Opened again, the tab has its tree made again.
    change(panel, cx, |panel, cx| panel.set_tab(DetailsTab::Files, cx));
    assert!(tree_of(panel, cx).is_none());
    cx.run_until_parked();
    assert!(made_from_read(panel, &read, cx));
}

/// How many trees [`make_and_count`] made; only
/// `a_newer_read_of_the_same_files_during_the_build_takes_the_finished_tree` uses it.
static MADE: AtomicUsize = AtomicUsize::new(0);

/// Makes the tree of `details`, as the app does, and counts it in [`MADE`].
fn make_and_count(details: &Arc<TorrentDetails>) -> FileTree {
    MADE.fetch_add(1, Ordering::SeqCst);
    FileTree::new(details)
}

#[gpui_kit::test]
fn a_newer_read_of_the_same_files_during_the_build_takes_the_finished_tree(
    cx: &mut TestAppContext,
) {
    let panel = panel_on(DetailsTab::Files, cx);
    change(panel, cx, |panel, _| panel.files.make = make_and_count);
    MADE.store(0, Ordering::SeqCst);
    let scans = torrent("a");
    let (first, second) = (big(&scans.hash, LAST), big(&scans.hash, LAST));

    show_with(panel, &scans, &first, cx);
    change(panel, cx, |panel, cx| {
        panel.take_details(Arc::clone(&second), cx);
    });
    assert!(tree_of(panel, cx).is_none());

    cx.run_until_parked();
    assert!(made_from_read(panel, &second, cx), "fitted to the newest");
    assert_eq!(MADE.load(Ordering::SeqCst), 1, "one tree made");
}

#[gpui_kit::test]
fn a_read_that_changes_the_files_during_the_build_starts_another_build(cx: &mut TestAppContext) {
    let panel = panel_on(DetailsTab::Files, cx);
    let scans = torrent("a");
    let renamed = big(&scans.hash, "Box/Last (fixed).tif");

    show_with(panel, &scans, &big(&scans.hash, LAST), cx);
    change(panel, cx, |panel, cx| {
        panel.take_details(Arc::clone(&renamed), cx);
    });
    assert!(tree_of(panel, cx).is_none(), "the first tree is being made");

    cx.run_until_parked();
    let tree = tree_of(panel, cx).expect("a tree");
    assert!(tree.is_made_from(&renamed));
    assert!(tree.contains(&NodeKey::new("Box/Last (fixed).tif")));
    assert!(!tree.contains(&NodeKey::new(LAST)));
}

#[gpui_kit::test]
fn the_open_folders_and_the_cursor_survive_a_built_tree(cx: &mut TestAppContext) {
    let panel = panel_on(DetailsTab::Files, cx);
    let scans = torrent("a");
    show_with(panel, &scans, &big(&scans.hash, LAST), cx);
    cx.run_until_parked();
    let (folder, scan) = (
        NodeKey::new("Box/Part 03"),
        NodeKey::new("Box/Part 03/Scan 0350.tif"),
    );
    change(panel, cx, |panel, cx| {
        panel.change_file_view(
            None,
            |view, tree| {
                view.toggle_expanded(tree, &folder);
                view.set_cursor(tree, &scan);
            },
            cx,
        );
    });
    let before = held(panel, cx);
    assert_eq!(before.2.as_deref(), Some(scan.as_str()));

    // Away from the tab and back, with a new read meanwhile: no tree while it is made again.
    change(panel, cx, |panel, cx| {
        panel.set_tab(DetailsTab::General, cx);
        panel.take_details(big(&scans.hash, LAST), cx);
        panel.set_tab(DetailsTab::Files, cx);
    });
    assert_eq!(held(panel, cx), (false, Vec::new(), None));

    cx.run_until_parked();
    assert_eq!(held(panel, cx), before);
}

/// The names of the threads [`make_and_note_the_thread`] made trees on. A `fn` cannot carry state,
/// so the note is made here; only `a_big_torrents_tree_is_made_on_a_thread_of_its_own` uses it.
static NOTED: Mutex<Vec<Option<String>>> = Mutex::new(Vec::new());

/// Makes the tree of `details`, as the app does, and notes in [`NOTED`] the thread it is made on.
fn make_and_note_the_thread(details: &Arc<TorrentDetails>) -> FileTree {
    let thread = std::thread::current().name().map(str::to_owned);
    NOTED.lock().expect("the note is kept").push(thread);
    FileTree::new(details)
}

#[gpui_kit::test]
fn a_big_torrents_tree_is_made_on_a_thread_of_its_own(cx: &mut TestAppContext) {
    cx.update(app::init);
    // Where the app makes it, not on the executor; only what makes it notes the thread.
    let panel = cx.add_window(|window, cx| {
        let mut panel =
            DetailsPanel::new(DetailsTab::Files, Rc::new(RefCell::default()), window, cx);
        panel.files.make = make_and_note_the_thread;
        panel
    });
    NOTED.lock().expect("the note is kept").clear();
    let scans = torrent("a");
    let read = big(&scans.hash, LAST);

    show_with(panel, &scans, &read, cx);

    // The thread wakes the window from outside the test's executor, at a time of its own.
    cx.executor().allow_parking();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !made_from_read(panel, &read, cx) {
        assert!(Instant::now() < deadline, "the tree never came");
        std::thread::sleep(Duration::from_millis(5));
        cx.run_until_parked();
    }
    assert_eq!(
        *NOTED.lock().expect("the note is kept"),
        [Some("file-tree".to_owned())]
    );
}
