//! The tests of the Files tab: what it holds as the tab, the panel and the target change, and
//! when the list draws again.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::{TestAppContext, WindowHandle};
use x4_core::test_support::{details, torrent};
use x4_core::{DetailsTab, DetailsTarget};
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

/// The details the tree of the Files tab was made from, if it has one.
fn made_from(
    panel: WindowHandle<DetailsPanel>,
    cx: &mut TestAppContext,
) -> Option<Arc<TorrentDetails>> {
    panel
        .update(cx, |panel, _, _| panel.files.made_from.clone())
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
    made_from(panel, cx).is_some_and(|made_from| Arc::ptr_eq(&made_from, read))
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

#[gpui_kit::test]
fn leaving_the_files_tab_drops_the_tree_but_not_the_cursor_and_the_open_folders(
    cx: &mut TestAppContext,
) {
    cx.update(app::init);
    let panel = cx.add_window(|window, cx| {
        DetailsPanel::new(DetailsTab::Files, Rc::new(RefCell::default()), window, cx)
    });
    let show = torrent("a");
    let season = |hash: &HashString| {
        Arc::new(details(
            hash,
            &[
                ("Show/Season 1/E01.mkv", 1024, 0, true, FilePriority::Normal),
                ("Show/Season 1/E02.mkv", 1024, 0, true, FilePriority::Normal),
                ("Show/info.nfo", 1, 1, true, FilePriority::Normal),
            ],
        ))
    };
    change(panel, cx, |panel, cx| {
        panel.show(DetailsTarget::One(show.hash.clone()), Some(&show), cx);
        panel.take_details(season(&show.hash), cx);
        // The user opens the season and goes to its second episode.
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
    let before = held(panel, cx);
    assert_eq!(
        before,
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
    assert!(made_from(panel, cx).is_none(), "no tree on the General tab");

    // The progress is read again every few seconds; on the General tab that makes no tree.
    let second = read(&film.hash);
    change(panel, cx, |panel, cx| {
        panel.take_details(Arc::clone(&second), cx);
    });
    assert!(made_from(panel, cx).is_none(), "still none");

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
    assert!(made_from(panel, cx).is_none(), "none on General");
    change(panel, cx, |panel, cx| {
        panel.set_tab(DetailsTab::Files, cx);
    });
    assert!(made_from_read(panel, &fourth, cx));
}
