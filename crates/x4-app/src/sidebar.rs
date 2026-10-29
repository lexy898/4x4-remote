//! The filter sidebar on the left of the main window: one entry per [`Filter`], each with an icon
//! in the color of what it shows and, on the right, the number of torrents it matches.
//!
//! `x4-core` decides what the entries say: the order ([`Filter::ALL`]), the counts over every
//! torrent whatever the filter and the search ([`FilterCounts`]), whether they are known yet
//! ([`TableModel::shown_counts`](x4_core::TableModel::shown_counts)), and which count stands out
//! ([`FilterCounts::needs_attention`]). This module draws them, dims the counts while the data is
//! stale, as the table and the status bar are, and names each entry for screen readers with its
//! count. A click on an entry hands its filter to the main view, which shows it.
//!
//! The list is flat on purpose: a submenu of gpui-component's sidebar names its toggle with an
//! English sentence put together in code, which could not be translated.

use std::rc::Rc;

use gpui_kit::component::sidebar::{Sidebar, SidebarCollapsible, SidebarMenu, SidebarMenuItem};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Theme};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, Div, InteractiveElement as _, ParentElement as _, Stateful,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, div, px,
};
use x4_core::{Filter, FilterCounts};

use crate::i18n::{filter_count_text, filter_label_text, filter_text};
use crate::theme::{STALE_OPACITY, count_color, filter_color, tabular_digits};

/// The width of the sidebar. The default columns of the table fit next to it in the default
/// window.
const SIDEBAR_WIDTH: f32 = 200.;

/// What a click on an entry does, given its filter.
type SelectHandler = Rc<dyn Fn(Filter, &mut Window, &mut App)>;

/// The sidebar with `counts` (none while they are not known yet), dimmed while they are `stale`,
/// and `selected` highlighted, in the colors of `theme`. A click on an entry calls `on_select`
/// with its filter.
pub(crate) fn filter_sidebar(
    counts: Option<&FilterCounts>,
    stale: bool,
    selected: Filter,
    theme: &Theme,
    on_select: impl Fn(Filter, &mut Window, &mut App) + 'static,
) -> Sidebar<SidebarMenu> {
    let on_select: SelectHandler = Rc::new(on_select);
    let entries = Filter::ALL.into_iter().enumerate().map(|(index, filter)| {
        entry(
            index,
            filter,
            counts,
            filter == selected,
            stale,
            theme,
            &on_select,
        )
    });
    Sidebar::new("sidebar")
        // The sidebar stays as it is: it has no button to fold it away.
        .collapsible(SidebarCollapsible::None)
        .w(px(SIDEBAR_WIDTH))
        .child(SidebarMenu::new().children(entries))
}

/// The entry of `filter`, the `index`-th of the list, with its count from `counts` if they are
/// known.
fn entry(
    index: usize,
    filter: Filter,
    counts: Option<&FilterCounts>,
    selected: bool,
    stale: bool,
    theme: &Theme,
    on_select: &SelectHandler,
) -> SidebarMenuItem {
    let counts = counts.copied();
    let on_select = Rc::clone(on_select);
    SidebarMenuItem::new(filter_text(filter))
        .accessibility_label(filter_label_text(
            filter,
            counts.map(|counts| counts.get(filter)),
        ))
        .icon(Icon::new(filter_icon(filter)).text_color(filter_color(filter, theme)))
        .active(selected)
        .when_some(counts, |entry, counts| {
            entry.suffix(move |_, cx| {
                count_badge(index, filter, &counts, stale, cx.theme()).test_support()
            })
        })
        .on_click(move |_, window, cx| on_select(filter, window, cx))
}

/// The count of `filter`, the `index`-th entry, from `counts`: in tabular digits, in the danger
/// color when it needs attention, and dimmed while `stale`. The name and the icon of the entry
/// stay as they are, as the user can still pick the filter.
fn count_badge(
    index: usize,
    filter: Filter,
    counts: &FilterCounts,
    stale: bool,
    theme: &Theme,
) -> Stateful<Div> {
    let text = filter_count_text(counts.get(filter));
    div()
        .id(("filter-count", index))
        .flex_none()
        .font_features(tabular_digits())
        .text_color(count_color(counts.needs_attention(filter), theme))
        .when(stale, |badge| badge.opacity(STALE_OPACITY))
        .child(text.clone())
        .aria_label(text)
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

    use gpui_kit::assets::Assets;
    use gpui_kit::component::IconNamed as _;
    use gpui_kit::{AssetSource as _, TestAppContext};

    use super::*;

    fn theme(cx: &mut TestAppContext) -> Theme {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.theme().clone()
        })
    }

    /// Counts that differ for every filter, with `error` torrents in error.
    fn counts_with_errors(error: usize) -> FilterCounts {
        FilterCounts {
            all: 21,
            downloading: 2,
            seeding: 3,
            paused: 4,
            active: 5,
            error,
        }
    }

    #[gpui_kit::test]
    fn a_count_is_dimmed_only_while_the_data_is_stale(cx: &mut TestAppContext) {
        let theme = theme(cx);
        let counts = counts_with_errors(1);
        let opacity = |filter, stale| {
            count_badge(0, filter, &counts, stale, &theme)
                .style()
                .opacity
        };

        for filter in Filter::ALL {
            assert_eq!(opacity(filter, true), Some(STALE_OPACITY), "{filter:?}");
            assert_eq!(opacity(filter, false), None, "{filter:?}");
        }
    }

    #[gpui_kit::test]
    fn only_an_error_count_above_zero_is_in_the_danger_color(cx: &mut TestAppContext) {
        let theme = theme(cx);
        let color = |filter, counts: &FilterCounts| {
            count_badge(0, filter, counts, false, &theme)
                .style()
                .text
                .color
        };

        for filter in Filter::ALL {
            let expected = if filter == Filter::Error {
                theme.danger
            } else {
                theme.muted_foreground
            };
            assert_eq!(
                color(filter, &counts_with_errors(1)),
                Some(expected),
                "{filter:?}"
            );
            assert_eq!(
                color(filter, &counts_with_errors(0)),
                Some(theme.muted_foreground),
                "{filter:?} without errors"
            );
        }
    }

    #[gpui_kit::test]
    fn counts_are_in_tabular_digits(cx: &mut TestAppContext) {
        let theme = theme(cx);
        let counts = counts_with_errors(1);

        for filter in Filter::ALL {
            let features = count_badge(0, filter, &counts, false, &theme)
                .style()
                .text
                .font_features
                .clone();
            assert_eq!(
                features.map(|features| features.tag_value_list().to_vec()),
                Some(vec![("tnum".to_owned(), 1)]),
                "{filter:?}"
            );
        }
    }

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
