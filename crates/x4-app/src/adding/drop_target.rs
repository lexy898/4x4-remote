//! How the places that take dropped files (the main window and the add dialog) show that they would
//! take them, and how a drop still lands after a key press.
//!
//! gpui takes a key press as the start of keyboard use: no element counts as hovered until the
//! mouse moves again, and both the highlight and the drop go only to the element hovered. Files
//! dragged in from another app move no mouse of this window, only the drag, so a drop after a key
//! press would land nowhere. [`hover_while_dragging`] makes the drag count as the mouse.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    App, DragMoveEvent, ExternalPaths, InputEvent as _, StyleRefinement, Styled as _, Window,
};

/// `style` with the highlight of a place files may be dropped on: the theme's drop color and a
/// border, which shows on top of whatever the place holds.
pub(crate) fn drop_highlight(
    style: StyleRefinement,
    _: &ExternalPaths,
    _: &mut Window,
    cx: &mut App,
) -> StyleRefinement {
    let theme = cx.theme();
    style
        .bg(theme.drop_target)
        .border_2()
        .border_color(theme.drag_border)
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
