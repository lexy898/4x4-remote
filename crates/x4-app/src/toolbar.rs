//! The toolbar along the top of the main window: the search box, the buttons that add torrents
//! (Add file and Add magnet), the buttons that start and pause the selected torrents and the Remove
//! menu on the left, the Connection button, the theme switch and the column menu on the right.
//!
//! Add file and Add magnet are always enabled, as the add dialog says itself whether the daemon is
//! connected; a click goes through the same request as their keys, Ctrl+O and Ctrl+U.
//!
//! Start, Pause and Remove are enabled as the selection's [`Availability`] says, and a click on
//! Start or Pause goes through the same request as its key, which reads the availability again: a
//! disabled action does nothing from either. Remove opens a menu of the two ways to remove, "Remove
//! from list…" and "Remove and delete files…"; each item opens the dialog that asks first, as its
//! key does. The tooltips name the action and its key. The main view draws the toolbar again
//! whenever the availability changes.
//!
//! The Connection button opens the Connection dialog, as Ctrl+, does; it is always enabled, and
//! the main view opens nothing while another dialog is open.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::InputState;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, IconName, Sizable as _, h_flex};
use gpui_kit::{
    Action, Anchor, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, Styled as _, div,
};
use x4_core::{Availability, ColumnId, ThemeChoice};
use x4_rpc::RemoveData;

use crate::actions::{
    PauseTorrents, RemoveTorrents, RemoveTorrentsWithData, StartTorrents, TorrentRequest,
};
use crate::i18n::{column_name_text, t};
use crate::keymap::{AddMagnetLink, AddTorrentFiles, MAIN_CONTEXT, TORRENT_LIST_CONTEXT};
use crate::main_view::{AddButton, MainView};
use crate::search::search_box;

/// What the toolbar shows besides the search box.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ToolbarState {
    /// The theme choice the switch names.
    pub(crate) theme: ThemeChoice,
    /// Whether each column is shown, in the order of [`ColumnId::ALL`], as the column menu
    /// lists them.
    pub(crate) columns: [(ColumnId, bool); ColumnId::ALL.len()],
    /// Which actions the selection allows, for the Start, Pause and Remove buttons.
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
                .child(add_button(
                    "add-torrent-files",
                    gpui_kit::assets::IconName::FilePlus,
                    t!("action-add-file").into(),
                    &AddTorrentFiles,
                    AddButton::Files,
                    cx,
                ))
                .child(add_button(
                    "add-magnet-link",
                    gpui_kit::assets::IconName::Magnet,
                    t!("action-add-magnet").into(),
                    &AddMagnetLink,
                    AddButton::Magnet,
                    cx,
                )),
        )
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
                ))
                .child(remove_menu(state.availability.remove, cx)),
        )
        .child(div().flex_1())
        .child(connection_button(cx))
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

/// The icon button that opens the Connection dialog of the view of `cx`; its tooltip names the key,
/// Ctrl+, too.
fn connection_button(cx: &Context<'_, MainView>) -> Button {
    Button::new("toolbar-connection")
        .ghost()
        .small()
        .icon(gpui_kit::assets::IconName::Settings)
        .accessibility_label(SharedString::from(t!("toolbar-connection")))
        .tooltip(t!("toolbar-connection-tooltip"))
        .on_click(cx.listener(|view, _, window, cx| view.open_connection(window, cx)))
}

/// The icon button `id` of the toolbar that adds torrents, named `name`: a click carries out
/// `button` on the view of `cx`, and the tooltip shows the key of `action`, which does the same.
fn add_button(
    id: &'static str,
    icon: gpui_kit::assets::IconName,
    name: SharedString,
    action: &dyn Action,
    button: AddButton,
    cx: &Context<'_, MainView>,
) -> Button {
    Button::new(id)
        .ghost()
        .small()
        .icon(icon)
        .accessibility_label(name.clone())
        .tooltip_with_action(name, action, Some(MAIN_CONTEXT))
        .on_click(cx.listener(move |view, _, window, cx| {
            view.press_add_button(button, window, cx);
        }))
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
        .on_click(cx.listener(move |view, _, window, cx| {
            view.press_button(request, window, cx);
        }))
}

/// The toolbar button whose menu offers the two ways to remove the selected torrents, enabled when
/// the selection can be removed. Each item opens the dialog of the view of `cx` that asks first.
fn remove_menu(enabled: bool, cx: &Context<'_, MainView>) -> impl IntoElement + use<> {
    let this = cx.weak_entity();
    let name: SharedString = t!("action-remove").into();
    Button::new("remove-torrents")
        .ghost()
        .small()
        .icon(gpui_kit::assets::IconName::Trash)
        .accessibility_label(name.clone())
        .tooltip_with_action(name, &RemoveTorrents, Some(TORRENT_LIST_CONTEXT))
        .disabled(!enabled)
        .dropdown_menu(move |menu, _, _| {
            [
                (
                    RemoveData::Keep,
                    t!("remove-from-list-menu"),
                    Box::new(RemoveTorrents) as Box<dyn Action>,
                ),
                (
                    RemoveData::Delete,
                    t!("remove-with-data-menu"),
                    Box::new(RemoveTorrentsWithData),
                ),
            ]
            .into_iter()
            .fold(menu, |menu, (data, label, action)| {
                let this = this.clone();
                // The action only shows its key in the menu; a click asks the view directly.
                menu.item(PopupMenuItem::new(label).action(action).on_click(
                    move |_, window, cx| {
                        if let Err(error) =
                            this.update(cx, |view, cx| view.ask_removal(data, window, cx))
                        {
                            tracing::debug!(%error, "a removal was chosen after the window closed");
                        }
                    },
                ))
            })
        })
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
