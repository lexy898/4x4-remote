//! The filter sidebar on the left of the main window: one entry per [`Filter`], each with an icon
//! in the color of what it shows and, on the right, the number of torrents it matches.
//!
//! `x4-core` decides what the entries say: the order ([`Filter::ALL`]), the counts over every
//! torrent whatever the filter and the search ([`FilterCounts`]), whether they are known yet
//! ([`TableModel::shown_counts`](x4_core::TableModel::shown_counts)), and which count stands out
//! ([`FilterCounts::needs_attention`]). This module draws them, and a click on an entry hands its
//! filter to the main view, which shows it.
//!
//! The list is flat on purpose: a submenu of gpui-component's sidebar names its toggle with an
//! English sentence put together in code, which could not be translated.

use std::rc::Rc;

use gpui_kit::component::sidebar::{Sidebar, SidebarCollapsible, SidebarMenu, SidebarMenuItem};
use gpui_kit::component::{Icon, IconName, Theme};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, InteractiveElement as _, ParentElement as _, StatefulInteractiveElement as _, Styled as _,
    TestSupportExt as _, Window, div, px,
};
use x4_core::{Filter, FilterCounts};

use crate::i18n::{filter_count_text, filter_text};
use crate::theme::{count_color, filter_color, tabular_digits};

/// The width of the sidebar. The default columns of the table fit next to it in the default
/// window.
const SIDEBAR_WIDTH: f32 = 200.;

/// What a click on an entry does, given its filter.
type SelectHandler = Rc<dyn Fn(Filter, &mut Window, &mut App)>;

/// The sidebar with `counts` (none while they are not known yet) and `selected` highlighted, in
/// the colors of `theme`. A click on an entry calls `on_select` with its filter.
pub(crate) fn filter_sidebar(
    counts: Option<&FilterCounts>,
    selected: Filter,
    theme: &Theme,
    on_select: impl Fn(Filter, &mut Window, &mut App) + 'static,
) -> Sidebar<SidebarMenu> {
    let on_select: SelectHandler = Rc::new(on_select);
    let entries = Filter::ALL
        .into_iter()
        .enumerate()
        .map(|(index, filter)| entry(index, filter, counts, filter == selected, theme, &on_select));
    Sidebar::new("sidebar")
        // The sidebar stays as it is: it has no button to fold it away.
        .collapsible(SidebarCollapsible::None)
        .w(px(SIDEBAR_WIDTH))
        .child(SidebarMenu::new().children(entries))
}

/// The entry of `filter`, the `index`-th of the list.
fn entry(
    index: usize,
    filter: Filter,
    counts: Option<&FilterCounts>,
    selected: bool,
    theme: &Theme,
    on_select: &SelectHandler,
) -> SidebarMenuItem {
    let count = counts.map(|counts| {
        (
            filter_count_text(counts.get(filter)),
            count_color(counts.needs_attention(filter), theme),
        )
    });
    let on_select = Rc::clone(on_select);
    SidebarMenuItem::new(filter_text(filter))
        .icon(Icon::new(filter_icon(filter)).text_color(filter_color(filter, theme)))
        .active(selected)
        .when_some(count, |entry, (count, color)| {
            entry.suffix(move |_, _| {
                div()
                    .id(("filter-count", index))
                    .flex_none()
                    .font_features(tabular_digits())
                    .text_color(color)
                    .child(count.clone())
                    .aria_label(count.clone())
                    .test_support()
            })
        })
        .on_click(move |_, window, cx| on_select(filter, window, cx))
}

/// The icon of `filter`: the arrows of the transfers, the pause sign, a warning for errors.
fn filter_icon(filter: Filter) -> IconName {
    match filter {
        Filter::All => IconName::Inbox,
        Filter::Downloading => IconName::ArrowDown,
        Filter::Seeding => IconName::ArrowUp,
        Filter::Paused => IconName::Pause,
        Filter::Active => IconName::ChevronsUpDown,
        Filter::Error => IconName::CircleAlert,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use gpui_kit::AssetSource as _;
    use gpui_kit::assets::Assets;
    use gpui_kit::component::IconNamed as _;

    use super::*;

    #[test]
    fn every_filter_has_an_icon_of_its_own_in_the_bundled_assets() {
        let paths: BTreeSet<String> = Filter::ALL
            .map(|filter| filter_icon(filter).path().to_string())
            .into();

        assert_eq!(paths.len(), Filter::ALL.len(), "{paths:?}");
        for path in &paths {
            let svg = Assets.load(path);
            assert!(
                svg.as_ref().is_ok_and(Option::is_some),
                "{path} is not among the icons the app embeds: {:?}",
                svg.err()
            );
        }
    }
}
