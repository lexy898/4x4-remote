//! How the places that take dropped files (the main window and the add dialog) show that they would
//! take them, and how a drop still lands after a key press.
//!
//! Each place has a layer over all of it ([`drop_highlight_layer`]) that is tinted, and its border
//! coloured, while files are dragged over the place. The layer is drawn over the place's content,
//! so the tint shows over opaque parts too, and it lies outside the layout with its border always
//! there, transparent until a drag, so nothing moves when a drag comes or goes.
//!
//! gpui takes a key press as the start of keyboard use: no element counts as hovered until the
//! mouse moves again, and both the highlight and the drop go only to the element hovered. Files
//! dragged in from another app move no mouse of this window, only the drag, so a drop after a key
//! press would land nowhere. [`hover_while_dragging`] makes the drag count as the mouse.
//!
//! That is how gpui-pre 0.3.11 behaves: `Window::dispatch_event` sets the input modality from the
//! raw event before it turns a `FileDrop` into mouse events (window.rs:5849-5855), and `on_drop`
//! and `drag_over` ask `Hitbox::is_hovered`, which is false while the keyboard is in use
//! (window.rs:790, div.rs:2970). Check again at each gpui-kit bump whether the workaround is still
//! needed; the test `a_drop_after_a_key_press_still_lands` guards that drops land either way.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    App, DragMoveEvent, ExternalPaths, InputEvent as _, InteractiveElement as _, IntoElement,
    Pixels, StyleRefinement, Styled as _, TestSupportExt as _, Window, div, transparent_black,
};

/// The layer, `id`, to put last into a place that takes dropped files, whose corners are rounded
/// by `radius`: it covers the whole place and draws the highlight while files are dragged over it
/// ([`drop_highlight`]). It takes no click, hover or scroll from what lies under it: its only
/// hitbox is the one of a highlight, which blocks nothing.
pub(crate) fn drop_highlight_layer(id: &'static str, radius: Pixels) -> impl IntoElement {
    div()
        .id(id)
        .absolute()
        .inset_0()
        .rounded(radius)
        .border_2()
        .border_color(transparent_black())
        .drag_over::<ExternalPaths>(drop_highlight)
        .test_support()
}

/// `style` with the highlight of a place files may be dropped on: the theme's drop color and the
/// color of its border, which the layer always has.
fn drop_highlight(
    style: StyleRefinement,
    _: &ExternalPaths,
    _: &mut Window,
    cx: &mut App,
) -> StyleRefinement {
    let theme = cx.theme();
    style.bg(theme.drop_target).border_color(theme.drag_border)
}

/// Follows files dragged over the window (`event`): while the window takes the keys to be in use,
/// sends the drag's move once more as a move of the mouse, right after this one, so that the
/// element under the drag is hovered again and the drop lands there. The move it sends ends
/// keyboard use, so it sends no other.
pub(crate) fn hover_while_dragging(
    event: &DragMoveEvent<ExternalPaths>,
    window: &mut Window,
    cx: &mut App,
) {
    if !window.last_input_was_keyboard() {
        return;
    }
    let mouse_move = event.event.clone();
    window.defer(cx, move |window, cx| {
        window.dispatch_event(mouse_move.to_platform_input(), cx);
    });
}
