//! One row of the file list, as x4-core's [`FileRow`] decides it, on one line: indented by its
//! depth, with an expander for a folder, its checkbox, its name (and after the name of a file that
//! downloads, muted, how much of it plays from the start), its size, its progress and its priority
//! button. A long name is cut short before that part is.
//!
//! The ids of a row and of its parts are keyed by the row's [`NodeKey`], never by its place in the
//! list: the toolkit keeps state under them (a button's focus, a menu), and a place would hand one
//! file's state to another as the list scrolls or a folder opens.
//!
//! Each control takes the tree of the frame it is drawn in, so a click acts on what was on screen
//! even when a newer read has come since. The checkbox and the priority button are not tab stops
//! and never take the focus, which stays in the list.

use std::sync::Arc;

use gpui_kit::base::{Checkbox, CheckboxIndicator, CheckboxState, Progress, ProgressIndicator};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, Theme, h_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, ElementId, Hsla, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, Role, SharedString, StatefulInteractiveElement, Styled as _,
    TestSupportExt as _, div, px, relative,
};
use x4_core::{FileRow, FileTree, NodeKey, PriorityView, RowKind, WantedState};
use x4_rpc::FilePriority;

use super::super::DetailsPanel;
use crate::i18n::{
    file_progress_label, files_wanted_label, percent_text, playable_text, priority_label,
    priority_text, size_text,
};
use crate::theme::{PROGRESS_TRACK_OPACITY, tabular_digits};
use crate::torrent_table::{BAR_HEIGHT, bar_fraction};

/// The height of a row: one line, with the controls' room around it.
pub(super) const ROW_HEIGHT: f32 = 28.;
/// How far each level of folders indents its entries.
const INDENT: f32 = 16.;
/// The width of the expander, and of the space a file leaves for it.
const EXPANDER_WIDTH: f32 = 16.;
/// The side of the checkbox.
const CHECKBOX_SIZE: f32 = 14.;
/// The widths of the columns after the name.
const SIZE_WIDTH: f32 = 76.;
const PROGRESS_WIDTH: f32 = 120.;
const PERCENT_WIDTH: f32 = 40.;
const PRIORITY_WIDTH: f32 = 92.;
/// The priorities of the menu, in its order.
const PRIORITIES: [FilePriority; 3] = [FilePriority::Low, FilePriority::Normal, FilePriority::High];

/// What every row of a frame shares.
pub(super) struct Drawn {
    /// The tree the rows are of, which their controls act on.
    pub(super) tree: Arc<FileTree>,
    /// The row the cursor is on.
    pub(super) cursor: Option<NodeKey>,
    /// Whether the controls can act: the daemon is connected.
    pub(super) enabled: bool,
    /// The color of the progress bars, the torrent's state's.
    pub(super) color: Hsla,
}

/// The element id of the row of `key`.
fn row_id(key: &NodeKey) -> ElementId {
    ElementId::NamedChild(
        Arc::new(ElementId::Name("file-row".into())),
        key.as_str().to_owned().into(),
    )
}

/// The row `row`, drawn in `drawn`, whose clicks go to the panel of `cx`.
pub(super) fn file_row(row: &FileRow, drawn: &Drawn, cx: &Context<'_, DetailsPanel>) -> AnyElement {
    let theme = cx.theme();
    let at_cursor = drawn.cursor.as_ref() == Some(&row.key);
    let expanded = match row.kind {
        RowKind::Folder { expanded } => Some(expanded),
        RowKind::File => None,
    };
    let (tree, key) = (Arc::clone(&drawn.tree), row.key.clone());
    h_flex()
        .id(row_id(&row.key))
        .test_support()
        .role(Role::TreeItem)
        .aria_label(SharedString::from(row.name.clone()))
        .aria_level(row.depth.saturating_add(1))
        .aria_selected(at_cursor)
        .when_some(expanded, StatefulInteractiveElement::aria_expanded)
        .h(px(ROW_HEIGHT))
        .w_full()
        .pr_2()
        .gap_2()
        .when(at_cursor, |row| row.bg(theme.list_active))
        .hover(|style| style.bg(theme.list_hover))
        .on_click(cx.listener(move |panel, _, _, cx| panel.click_file_row(&tree, &key, cx)))
        .child(div().flex_none().w(px(INDENT * level(row.depth) + 4.)))
        .child(expander(row, expanded, drawn, cx))
        .child(wanted_box(row, drawn, theme, cx))
        .child(name_cell(row, theme))
        .child(
            div()
                .flex_none()
                .w(px(SIZE_WIDTH))
                .whitespace_nowrap()
                .text_right()
                .text_xs()
                .font_features(tabular_digits())
                .text_color(theme.muted_foreground)
                .child(size_text(row.size)),
        )
        .child(progress_cell(row, drawn, theme))
        .child(priority_button(row, drawn, cx))
        .into_any_element()
}

/// The depth of a row as a number of indents.
fn level(depth: usize) -> f32 {
    f32::from(u16::try_from(depth).unwrap_or(u16::MAX))
}

/// The chevron that opens and closes a folder; the same room, empty, for a file.
fn expander(
    row: &FileRow,
    expanded: Option<bool>,
    drawn: &Drawn,
    cx: &Context<'_, DetailsPanel>,
) -> AnyElement {
    let room = div()
        .flex_none()
        .w(px(EXPANDER_WIDTH))
        .h_full()
        .flex()
        .items_center()
        .justify_center();
    let Some(expanded) = expanded else {
        return room.into_any_element();
    };
    let (tree, key) = (Arc::clone(&drawn.tree), row.key.clone());
    room.id("file-expander")
        .test_support()
        .text_color(cx.theme().muted_foreground)
        .child(
            Icon::new(if expanded {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .xsmall(),
        )
        .on_click(cx.listener(move |panel, _, _, cx| {
            // The row's own click would put the cursor back where it was.
            cx.stop_propagation();
            panel.click_expander(&tree, &key, cx);
        }))
        .into_any_element()
}

/// The checkbox of whether the row is downloaded: checked, empty, or a dash for a folder of both.
fn wanted_box(
    row: &FileRow,
    drawn: &Drawn,
    theme: &Theme,
    cx: &Context<'_, DetailsPanel>,
) -> impl IntoElement {
    let state = match row.wanted {
        WantedState::All => CheckboxState::Checked,
        WantedState::None => CheckboxState::Unchecked,
        WantedState::Mixed => CheckboxState::Indeterminate,
    };
    let mark = match state {
        CheckboxState::Checked => Some(IconName::Check),
        CheckboxState::Indeterminate => Some(IconName::Minus),
        CheckboxState::Unchecked => None,
    };
    let filled = mark.is_some();
    let enabled = drawn.enabled;
    let panel = cx.weak_entity();
    let (tree, key) = (Arc::clone(&drawn.tree), row.key.clone());
    let accent = if enabled {
        theme.primary
    } else {
        theme.primary.opacity(0.5)
    };
    Checkbox::new("file-wanted")
        .state(state)
        .tab_stop(false)
        .disabled(!enabled)
        .accessibility_label(files_wanted_label(&row.name))
        .flex_none()
        // The keys stay in the list, which the press gave the focus to.
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .on_change(move |_, _, _, cx| {
            // The row's own click would put the cursor there again, and that is all.
            cx.stop_propagation();
            if let Err(error) = panel.update(cx, |panel, cx| panel.click_wanted(&tree, &key, cx)) {
                tracing::debug!(%error, "a checkbox was clicked after the panel closed");
            }
        })
        .child(
            CheckboxIndicator::new()
                .state(state)
                .size(px(CHECKBOX_SIZE))
                .flex()
                .items_center()
                .justify_center()
                .border_1()
                .rounded(px(3.))
                .map(|indicator| {
                    if filled {
                        indicator
                            .border_color(accent)
                            .bg(accent)
                            .text_color(theme.primary_foreground)
                    } else {
                        indicator.border_color(theme.input).bg(theme.background)
                    }
                })
                .children(mark.map(|mark| Icon::new(mark).size(px(CHECKBOX_SIZE - 4.)))),
        )
}

/// The name, and after the name of a file that downloads, muted, the part of it that plays from
/// the start; the name is cut short first.
fn name_cell(row: &FileRow, theme: &Theme) -> impl IntoElement {
    let muted = row.wanted == WantedState::None;
    h_flex()
        .flex_1()
        .min_w_0()
        .items_baseline()
        .gap_2()
        .child(
            div()
                .id("file-name")
                .flex_initial()
                .min_w_0()
                .truncate()
                .when(muted, |name| name.text_color(theme.muted_foreground))
                .child(SharedString::from(row.name.clone()))
                .test_support(),
        )
        .when_some(row.playable, |cell, playable| {
            let text = playable_text(playable);
            cell.child(
                div()
                    .id("file-playable")
                    .flex_none()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .aria_label(text.clone())
                    .child(text)
                    .test_support(),
            )
        })
}

/// A thin bar of the progress, in the torrent's color (muted for what is skipped), and the
/// percent beside it.
fn progress_cell(row: &FileRow, drawn: &Drawn, theme: &Theme) -> impl IntoElement {
    let color = if row.wanted == WantedState::None {
        theme.muted_foreground
    } else {
        drawn.color
    };
    let percent = percent_text(row.percent);
    h_flex()
        .flex_none()
        .w(px(PROGRESS_WIDTH))
        .gap_2()
        .child(
            Progress::new("file-progress")
                .value(f32::from(row.percent))
                .accessibility_label(file_progress_label(&row.name))
                .aria_value(percent.clone())
                .flex_1()
                .h(px(BAR_HEIGHT))
                .rounded_full()
                .bg(color.opacity(PROGRESS_TRACK_OPACITY))
                .child(
                    ProgressIndicator::new()
                        .h_full()
                        .w(relative(bar_fraction(row.progress)))
                        .rounded_full()
                        .bg(color),
                ),
        )
        .child(
            div()
                .flex_none()
                .w(px(PERCENT_WIDTH))
                .whitespace_nowrap()
                .text_right()
                .text_xs()
                .font_features(tabular_digits())
                .text_color(theme.muted_foreground)
                .child(percent),
        )
}

/// The button that shows the row's priority and opens the menu of the three, with the current
/// one checked; a folder of several shows "Mixed" and checks none.
fn priority_button(
    row: &FileRow,
    drawn: &Drawn,
    cx: &Context<'_, DetailsPanel>,
) -> impl IntoElement {
    let panel = cx.weak_entity();
    let (tree, key, current) = (Arc::clone(&drawn.tree), row.key.clone(), row.priority);
    div().flex_none().w(px(PRIORITY_WIDTH)).child(
        Button::new("file-priority")
            .ghost()
            .xsmall()
            .tab_stop(false)
            .dropdown_caret(true)
            .disabled(!drawn.enabled)
            .label(priority_text(row.priority))
            .accessibility_label(priority_label(&row.name, row.priority))
            // The menu is built when it opens, from the tree of the frame it opened on.
            .dropdown_menu(move |menu, _, _| {
                PRIORITIES.into_iter().fold(menu, |menu, priority| {
                    let (panel, tree, key) = (panel.clone(), Arc::clone(&tree), key.clone());
                    menu.item(
                        PopupMenuItem::new(priority_text(PriorityView::One(priority)))
                            .checked(current == PriorityView::One(priority))
                            .on_click(move |_, _, cx| {
                                if let Err(error) = panel.update(cx, |panel, cx| {
                                    panel.choose_priority(&tree, &key, priority, cx);
                                }) {
                                    tracing::debug!(
                                        %error,
                                        "a priority was chosen after the panel closed"
                                    );
                                }
                            }),
                    )
                })
            }),
    )
}
