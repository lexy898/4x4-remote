//! The tests of the Files tab: what it holds as the tab, the panel and the target change, and
//! when the list draws again.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, ElementId, ScrollDelta, Subscription, TestAppContext, WindowHandle, point, px,
};
use x4_core::test_support::{details, torrent};
use x4_core::{DetailsTab, DetailsTarget, Edge};
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
    tab.follow(Some(&read(&film.hash)));
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

/// A panel on the tab `tab`, about no torrent yet.
fn panel_on(tab: DetailsTab, cx: &mut TestAppContext) -> WindowHandle<DetailsPanel> {
    cx.update(app::init);
    cx.add_window(|window, cx| DetailsPanel::new(tab, Rc::new(RefCell::default()), window, cx))
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
    tab.follow(Some(&album(&torrent("album").hash)));
    assert_eq!(tab.rows.len(), 2001);

    assert!(tab.follow(None));

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
