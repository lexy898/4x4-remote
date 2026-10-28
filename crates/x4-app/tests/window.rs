//! Headless tests for the main window and the application-wide key bindings.

use std::cell::Cell;
use std::rc::Rc;

use gpui_kit::TestAppContext;
use x4_app::app::{self, Quit};

#[gpui_kit::test]
fn main_window_opens_with_root_view(cx: &mut TestAppContext) {
    let opened = cx.update(|cx| {
        gpui_kit::init(cx);
        app::open_main_window(cx)
    });

    assert!(opened.is_ok(), "main window failed to open: {opened:?}");
    assert_eq!(cx.windows().len(), 1);
}

#[gpui_kit::test]
fn ctrl_q_dispatches_quit(cx: &mut TestAppContext) {
    let quit_requested = Rc::new(Cell::new(false));
    let (window, _) = cx.update(|cx| {
        app::init(cx);
        let quit_requested = Rc::clone(&quit_requested);
        // The test platform ignores `quit`, so a second listener records the dispatch.
        cx.on_action(move |_: &Quit, _| quit_requested.set(true));
        app::open_main_window(cx).unwrap()
    });

    cx.simulate_keystrokes(window, "ctrl-q");

    assert!(quit_requested.get());
}
