use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, Entity, TestAppContext};
use x4_core::UiState;

use super::*;
use crate::app;

/// Reads a magnet link as the app does, and never finishes reading a file, as on a network file
/// system that hangs.
fn stuck_on_files(item: AddItem, cx: &App) -> Task<Result<LoadedItem, LoadError>> {
    match item {
        AddItem::File(_) => cx.background_executor().spawn(std::future::pending()),
        magnet @ AddItem::Magnet(_) => read_in_background(magnet, cx),
    }
}

fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<MainView>) {
    let (_events, events_rx) = async_channel::unbounded();
    let (commands, _commands_rx) = crossbeam_channel::unbounded();
    cx.update(|cx| cx.set_reduce_motion(true));
    let (handle, view) = cx
        .update(|cx| {
            app::init(cx);
            app::open_main_window(events_rx, commands, None, &UiState::default(), cx)
        })
        .expect("the window opens");
    cx.update_window(handle, |_, window, _| window.activate_window())
        .expect("the window is open");
    cx.run_until_parked();
    (handle, view)
}

/// What the element `id` says, after a frame is drawn.
fn label(handle: AnyWindowHandle, id: &'static str, cx: &mut TestAppContext) -> Option<String> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find(id)
            .and_then(|found| found.label().map(str::to_owned))
    })
    .expect("the window is open")
}

#[gpui_kit::test]
fn a_read_that_never_ends_can_be_skipped(cx: &mut TestAppContext) {
    let (handle, view) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.adds.read = stuck_on_files;
            view.take_add_request(
                AddRequest::Items(vec![
                    AddItem::File("/srv/hangs/Show.torrent".into()),
                    AddItem::Magnet(None),
                ]),
                window,
                cx,
            );
        });
    })
    .expect("the window is open");
    cx.run_until_parked();
    assert_eq!(
        label(handle, "add-title", cx),
        None,
        "a short read shows nothing"
    );

    cx.executor().advance_clock(READ_GRACE);
    cx.run_until_parked();

    assert_eq!(
        label(handle, "add-title", cx).as_deref(),
        Some("Add torrent (1 of 2)")
    );
    assert_eq!(
        label(handle, "add-reading", cx).as_deref(),
        Some("Reading the file…")
    );
    assert_eq!(label(handle, "add-cancel", cx).as_deref(), Some("Skip"));
    assert_eq!(label(handle, "add-confirm", cx).as_deref(), Some("Add"));

    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("add-cancel", cx);
    })
    .expect("the window is open");
    cx.run_until_parked();

    assert_eq!(
        label(handle, "add-title", cx).as_deref(),
        Some("Add torrent (2 of 2)"),
        "the next torrent, though the read of the first never ends"
    );
    assert_eq!(label(handle, "add-reading", cx), None);
    assert_eq!(
        label(handle, "add-link", cx).as_deref(),
        Some("Magnet link")
    );
    assert_eq!(label(handle, "add-cancel", cx).as_deref(), Some("Cancel"));
    assert!(
        cx.update_window(handle, |_, window, cx| window.has_active_dialog(cx))
            .expect("the window is open")
    );
}

#[gpui_kit::test]
fn a_quick_read_opens_the_dialog_on_the_torrent_at_once(cx: &mut TestAppContext) {
    let (handle, view) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.take_add_request(AddRequest::Items(vec![AddItem::Magnet(None)]), window, cx);
        });
    })
    .expect("the window is open");
    cx.run_until_parked();

    assert_eq!(
        label(handle, "add-title", cx).as_deref(),
        Some("Add torrent")
    );
    assert_eq!(
        label(handle, "add-reading", cx),
        None,
        "never said to be read"
    );
    assert_eq!(label(handle, "add-cancel", cx).as_deref(), Some("Cancel"));
}

#[gpui_kit::test]
fn a_read_that_never_ends_behind_another_dialog_can_be_skipped_once_it_closes(
    cx: &mut TestAppContext,
) {
    let (handle, view) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.adds.read = stuck_on_files;
            view.take_add_request(
                AddRequest::Items(vec![AddItem::File("/srv/hangs/Show.torrent".into())]),
                window,
                cx,
            );
        });
    })
    .expect("the window is open");
    cx.run_until_parked();
    // Another dialog opens while the file is read, and is still open when the read is slow.
    cx.update_window(handle, |_, window, cx| {
        window.open_dialog(cx, |dialog, _, _| dialog);
    })
    .expect("the window is open");
    cx.executor().advance_clock(READ_GRACE * 2);
    cx.run_until_parked();
    assert_eq!(
        label(handle, "add-title", cx),
        None,
        "the other dialog is open"
    );

    cx.update_window(handle, |_, window, cx| {
        window.close_dialog(cx);
        view.update(cx, |view, cx| view.dialog_closed(window, cx));
    })
    .expect("the window is open");
    cx.run_until_parked();

    assert_eq!(
        label(handle, "add-reading", cx).as_deref(),
        Some("Reading the file…")
    );
    assert_eq!(label(handle, "add-cancel", cx).as_deref(), Some("Skip"));
}
