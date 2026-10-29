//! The toolbar along the top of the main window: the search box and the buttons that start and
//! pause the selected torrents on the left, the theme switch and the column menu on the right.
//!
//! Start and Pause are enabled as the selection's [`Availability`] says, and a click on one goes
//! through the same request as its key, which reads the availability again: a disabled action does
//! nothing from either. Their tooltips name the action and its key. The main view draws the toolbar
//! again whenever the availability changes.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::InputState;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, IconName, Sizable as _, h_flex};
use gpui_kit::{
    Action, Anchor, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Styled as _, div,
};
use x4_core::{Availability, ColumnId, ThemeChoice};

use crate::actions::{PauseTorrents, StartTorrents, TorrentRequest};
use crate::i18n::{column_name_text, t};
use crate::keymap::TORRENT_LIST_CONTEXT;
use crate::main_view::MainView;
use crate::search::search_box;

/// What the toolbar shows besides the search box.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ToolbarState {
    /// The theme choice the switch names.
    pub(crate) theme: ThemeChoice,
    /// Whether each column is shown, in the order of [`ColumnId::ALL`], as the column menu
    /// lists them.
    pub(crate) columns: [(ColumnId, bool); ColumnId::ALL.len()],
    /// Which actions the selection allows, for the Start and Pause buttons.
    pub(crate) availability: Availability,
}

/// The toolbar with the search box `search`, showing `state`. Its buttons act on the view of `cx`.
pub(crate) fn toolbar(
    search: &Entity<InputState>,
    state: ToolbarState,
    cx: &Context<'_, MainView>,
) -> impl IntoElement + use<> {
    let theme_label = match state.theme {
        ThemeChoice::System => t!("theme-system"),
        ThemeChoice::Light => t!("theme-light"),
        ThemeChoice::Dark => t!("theme-dark"),
    };
    let border = cx.theme().border;
    h_flex()
        .id("toolbar")
        .flex_none()
        .gap_2()
        .px_2()
        .py_1()
        .border_b_1()
        .border_color(border)
        .child(search_box(search, cx.theme()))
        .child(
            h_flex()
                .gap_1()
                .child(action_button(
                    ActionButton {
                        id: "start-torrents",
                        icon: IconName::Play,
                        name: t!("action-start").into(),
                        action: &StartTorrents,
                        request: TorrentRequest::Start,
                        enabled: state.availability.start,
                    },
                    cx,
                ))
                .child(action_button(
                    ActionButton {
                        id: "pause-torrents",
                        icon: IconName::Pause,
                        name: t!("action-pause").into(),
                        action: &PauseTorrents,
                        request: TorrentRequest::Pause,
                        enabled: state.availability.pause,
                    },
                    cx,
                )),
        )
        .child(div().flex_1())
        .child(
            Button::new("theme-toggle")
                .ghost()
                .small()
                .label(theme_label)
                .tooltip(t!("theme-switch-tooltip"))
                .on_click(cx.listener(|view, _, window, cx| view.switch_theme(window, cx))),
        )
        .child(column_menu(state.columns, cx))
}

/// An icon button of the toolbar that acts on the selected torrents.
struct ActionButton<'a> {
    /// The element id.
    id: &'static str,
    icon: IconName,
    /// The action's name, for screen readers and the tooltip.
    name: SharedString,
    /// The action its key runs, whose key the tooltip shows.
    action: &'a dyn Action,
    /// What a click asks for.
    request: TorrentRequest,
    /// Whether the selection allows it.
    enabled: bool,
}

/// The button `button`, which asks the view of `cx` for its request when clicked.
fn action_button(button: ActionButton<'_>, cx: &Context<'_, MainView>) -> Button {
    let request = button.request;
    Button::new(button.id)
        .ghost()
        .small()
        .icon(button.icon)
        .accessibility_label(button.name.clone())
        .tooltip_with_action(button.name, button.action, Some(TORRENT_LIST_CONTEXT))
        .disabled(!button.enabled)
        .on_click(cx.listener(move |view, _, window, cx| view.request(request, window, cx)))
}

/// The toolbar button whose menu shows and hides the columns, with a check on each shown one
/// (`columns`). The name column's entry cannot be unchecked.
fn column_menu(
    columns: [(ColumnId, bool); ColumnId::ALL.len()],
    cx: &Context<'_, MainView>,
) -> impl IntoElement + use<> {
    let this = cx.weak_entity();
    Button::new("column-menu")
        .ghost()
        .small()
        .label(t!("column-menu"))
        .tooltip(t!("column-menu-tooltip"))
        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
            columns.into_iter().fold(menu, |menu, (column, shown)| {
                let this = this.clone();
                menu.item(
                    PopupMenuItem::new(column_name_text(column))
                        .checked(shown)
                        .disabled(!column.can_hide())
                        .on_click(move |_, _, cx| {
                            if let Err(error) =
                                this.update(cx, |view, cx| view.toggle_column(column, cx))
                            {
                                tracing::debug!(%error, "a column was toggled after the window closed");
                            }
                        }),
                )
            })
        })
}
