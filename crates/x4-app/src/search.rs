//! The search box in the toolbar, which narrows the torrent list to the names that hold every word
//! typed, as the user types and without asking the daemon.
//!
//! The box only holds the text. The main view follows its changes into the search of its
//! [`ViewState`](x4_core::ViewState) and recomputes the rows from the last snapshot; `x4-core`
//! decides what matches ([`matches_search`](x4_core::matches_search)). Ctrl+F focuses the box
//! from anywhere in the window and `/` from anywhere but a text field, so that a slash can still
//! be typed in one. Escape in the box empties it and hands the focus back to the table, once the
//! box has nothing of its own to dismiss. The key bindings are in [`keymap`](crate::keymap).
//!
//! The box has no clear button: gpui-component names that button in English, outside our
//! messages. Escape clears it instead.

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{Icon, IconName, Sizable as _, Theme};
use gpui_kit::{
    App, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Styled as _,
    Window, div, px,
};

use crate::i18n::search_placeholder_text;
use crate::keymap::SEARCH_CONTEXT;

/// The width of the search box.
const SEARCH_WIDTH: f32 = 280.;

/// The state of an empty search box, which says what it is for.
pub(crate) fn new_search_state(
    window: &mut Window,
    cx: &mut Context<'_, InputState>,
) -> InputState {
    InputState::new(window, cx).placeholder(search_placeholder_text(cx.platform_info().os))
}

/// The search box showing `state`, in the colors of `theme`. Its key context lets Escape clear it.
pub(crate) fn search_box(state: &Entity<InputState>, theme: &Theme) -> impl IntoElement {
    div()
        .key_context(SEARCH_CONTEXT)
        .w(px(SEARCH_WIDTH))
        .min_w_0()
        .child(
            Input::new(state)
                .id("search")
                .small()
                .prefix(Icon::new(IconName::Search).text_color(theme.muted_foreground)),
        )
}

/// Moves the focus into the search box and selects its text, so that typing replaces it.
pub(crate) fn focus_search(state: &Entity<InputState>, window: &mut Window, cx: &mut App) {
    state.update(cx, |search, cx| {
        search.focus(window, cx);
        search.select_all(window, cx);
    });
}

/// Empties the search box. The box reports the change as it reports any edit.
pub(crate) fn clear_search(state: &Entity<InputState>, window: &mut Window, cx: &mut App) {
    state.update(cx, |search, cx| search.clean(window, cx));
}
