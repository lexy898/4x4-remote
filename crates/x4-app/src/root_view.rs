//! The content of the main window.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{Context, IntoElement, ParentElement as _, Render, Styled as _, Window, div};

use crate::PRODUCT_NAME;

/// The main window's content: the product name, centered on the theme background.
pub struct RootView;

impl Render for RootView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(PRODUCT_NAME)
    }
}
