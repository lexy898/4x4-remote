//! Headless tests for the main window and the application-wide key bindings.
//!
//! Quitting when the last window closes (`QuitMode`) is not covered: the test platform's `quit`
//! does nothing and `App` has no getter for the quit mode.

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
fn init_installs_a_global_quit_handler(cx: &mut TestAppContext) {
    // With no window open, only a global listener can make the action available.
    let available = cx.update(|cx| {
        app::init(cx);
        cx.is_action_available(&Quit)
    });

    assert!(available);
}

#[gpui_kit::test]
fn ctrl_q_dispatches_quit(cx: &mut TestAppContext) {
    let quit_requested = Rc::new(Cell::new(false));
    let (window, _) = cx.update(|cx| {
        app::init(cx);
        let quit_requested = Rc::clone(&quit_requested);
        // The test platform ignores `quit`, so a second listener records the dispatch. Global
        // listeners run newest first and stop the dispatch unless they propagate it, so this
        // one propagates to keep `init`'s handler in the chain.
        cx.on_action(move |_: &Quit, cx| {
            quit_requested.set(true);
            cx.propagate();
        });
        app::open_main_window(cx).unwrap()
    });

    cx.simulate_keystrokes(window, "ctrl-q");

    assert!(quit_requested.get());
}
