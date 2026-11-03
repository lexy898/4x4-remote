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
        magnet @ AddItem::Magnet(_) => read_on_the_executor(magnet, cx),
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

/// A `.torrent` of one file of one byte, named `name`.
fn one_file_torrent(name: &str) -> Vec<u8> {
    let mut bytes = format!(
        "d4:infod6:lengthi1e4:name{}:{name}12:piece lengthi16384e6:pieces20:",
        name.len()
    )
    .into_bytes();
    bytes.extend([0; 20]);
    bytes.extend(b"ee");
    bytes
}

/// Runs the window's tasks until `done` says so, letting real time pass for the threads that read
/// files, which wake the window from outside the test's executor; `false` after 10 seconds.
fn run_until(cx: &mut TestAppContext, mut done: impl FnMut(&mut TestAppContext) -> bool) -> bool {
    cx.executor().allow_parking();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if done(cx) {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Reads a file as the app does, on a thread of its own, and fails a read that is not on such a
/// thread.
fn read_and_check_the_thread(item: AddItem, cx: &App) -> Task<Result<LoadedItem, LoadError>> {
    fn load_on_the_reader(item: AddItem) -> Result<LoadedItem, LoadError> {
        if std::thread::current().name() != Some("torrent-reader") {
            return Err(LoadError::NotAFile);
        }
        load(item)
    }
    adding::read_on_a_thread(item, load_on_the_reader, cx)
}

#[gpui_kit::test]
fn a_torrent_file_is_read_on_a_thread_of_its_own(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("film.torrent");
    std::fs::write(&path, one_file_torrent("Film")).expect("the file is written");
    let (handle, view) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.adds.read = read_and_check_the_thread;
            view.take_add_request(AddRequest::Items(vec![AddItem::File(path)]), window, cx);
        });
    })
    .expect("the window is open");

    assert!(run_until(cx, |cx| label(handle, "add-name", cx).is_some()));
    assert_eq!(label(handle, "add-name", cx).as_deref(), Some("Film"));
}

/// Reads a file as the app does, on a thread of its own, but never finishes reading one in a
/// folder named `hangs`, as on a network file system that hangs.
fn read_hanging_in_one_folder(item: AddItem, cx: &App) -> Task<Result<LoadedItem, LoadError>> {
    fn load_or_hang(item: AddItem) -> Result<LoadedItem, LoadError> {
        if let AddItem::File(path) = &item
            && path.parent().is_some_and(|dir| dir.ends_with("hangs"))
        {
            loop {
                std::thread::park();
            }
        }
        load(item)
    }
    adding::read_on_a_thread(item, load_or_hang, cx)
}

#[gpui_kit::test]
fn a_skipped_read_that_never_ends_does_not_hold_the_next_one(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let hangs = dir.path().join("hangs");
    std::fs::create_dir(&hangs).expect("the folder is made");
    // More reads that never end than GPUI's executor has threads, all skipped, and then a file
    // that reads.
    let mut items: Vec<AddItem> = (0..8)
        .map(|n| AddItem::File(hangs.join(format!("{n}.torrent"))))
        .collect();
    let film = dir.path().join("film.torrent");
    std::fs::write(&film, one_file_torrent("Film")).expect("the file is written");
    items.push(AddItem::File(film));
    let (handle, view) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.adds.read = read_hanging_in_one_folder;
            view.take_add_request(AddRequest::Items(items), window, cx);
        });
    })
    .expect("the window is open");

    for n in 1..=8 {
        cx.executor().advance_clock(READ_GRACE);
        assert!(
            run_until(cx, |cx| label(handle, "add-reading", cx).is_some()),
            "the read of file {n} is said to take long"
        );
        assert_eq!(
            label(handle, "add-title", cx),
            Some(format!("Add torrent ({n} of 9)"))
        );
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("add-cancel", cx);
        })
        .expect("the window is open");
    }

    assert!(
        run_until(cx, |cx| label(handle, "add-name", cx).is_some()),
        "the last file is read, though none of the reads before it ended"
    );
    assert_eq!(label(handle, "add-name", cx).as_deref(), Some("Film"));
    assert_eq!(label(handle, "add-reading", cx), None);
}
