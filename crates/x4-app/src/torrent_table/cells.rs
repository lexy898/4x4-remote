//! What each column of the torrent table says about a row, and how its cells and headers are
//! drawn: a state dot and two lines in the name cell, a thin bar with a caption in the progress
//! cell, and numbers right-aligned in tabular digits, the rates in the state's color only while
//! data moves.

use gpui_kit::component::{Theme, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Div, FontWeight, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, Stateful, StatefulInteractiveElement as _, Styled,
    TestSupportExt as _, Window, div, px, relative,
};
use x4_core::{ColumnId, HumanSpeed, Row, SortDirection};

use crate::i18n::{
    column_title_text, eta_text, peer_count_text, progress_text, ratio_text, size_text,
    sort_mark_label, sort_mark_text, speed_text, state_text, subtitle_text,
};
use crate::theme::{
    PROGRESS_TRACK_OPACITY, rate_color, state_color, subtitle_color, tabular_digits,
};

/// The height of a progress bar.
const BAR_HEIGHT: f32 = 4.;

/// Whether `column` holds numbers, which are right-aligned in tabular digits.
pub(super) fn is_numeric(column: ColumnId) -> bool {
    match column {
        ColumnId::Size
        | ColumnId::Down
        | ColumnId::Up
        | ColumnId::Eta
        | ColumnId::Ratio
        | ColumnId::Peers => true,
        ColumnId::Name | ColumnId::Progress | ColumnId::State => false,
    }
}

/// What the cell of `column` says about `row`, for screen readers: the name, the caption of the
/// progress bar, or the text the cell shows.
pub(super) fn cell_text(row: &Row, column: ColumnId) -> SharedString {
    match column {
        ColumnId::Name => row.name.clone().into(),
        ColumnId::Size => size_text(row.size),
        ColumnId::Progress => progress_text(row.percent, row.size),
        ColumnId::State => state_text(row.state),
        ColumnId::Down => speed_text(row.rate_down),
        ColumnId::Up => speed_text(row.rate_up),
        ColumnId::Eta => eta_text(&row.eta),
        ColumnId::Ratio => ratio_text(row.ratio),
        ColumnId::Peers => peer_count_text(row.peers),
    }
}

/// The cell of `column` for `row`, in the colors of `theme`.
pub(super) fn cell(row: &Row, column: ColumnId, theme: &Theme) -> AnyElement {
    match column {
        ColumnId::Name => name_cell(row, theme),
        ColumnId::Progress => progress_cell(row, theme),
        ColumnId::State => h_flex()
            .size_full()
            .text_color(state_color(row.state, theme))
            .child(div().truncate().child(state_text(row.state)))
            .into_any_element(),
        ColumnId::Down => rate_cell(row, row.rate_down, row.down_active, theme),
        ColumnId::Up => rate_cell(row, row.rate_up, row.up_active, theme),
        ColumnId::Size | ColumnId::Eta | ColumnId::Ratio | ColumnId::Peers => {
            number(cell_text(row, column), theme.muted_foreground).into_any_element()
        }
    }
}

/// The state dot, the name and, under it, the subtitle, muted or in the color of an error.
fn name_cell(row: &Row, theme: &Theme) -> AnyElement {
    h_flex()
        .size_full()
        .gap_2()
        .overflow_hidden()
        .child(
            div()
                .flex_none()
                .size_2()
                .rounded_full()
                .bg(state_color(row.state, theme)),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .child(div().truncate().child(SharedString::from(row.name.clone())))
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(subtitle_color(&row.subtitle, theme))
                        .child(subtitle_text(&row.subtitle)),
                ),
        )
        .into_any_element()
}

/// A thin bar in the state's color over a pale track of it, and the caption under it, such as
/// "63% of 22.9 GiB". The bar does not animate, so a new snapshot draws one frame.
fn progress_cell(row: &Row, theme: &Theme) -> AnyElement {
    let color = state_color(row.state, theme);
    v_flex()
        .size_full()
        .justify_center()
        .gap_1()
        .child(
            div()
                .w_full()
                .h(px(BAR_HEIGHT))
                .flex_none()
                .rounded_full()
                .bg(color.opacity(PROGRESS_TRACK_OPACITY))
                .child(
                    div()
                        .h_full()
                        .w(relative(bar_fraction(row.progress)))
                        .rounded_full()
                        .bg(color),
                ),
        )
        .child(
            div()
                .truncate()
                .text_xs()
                .font_features(tabular_digits())
                .text_color(theme.muted_foreground)
                .child(progress_text(row.percent, row.size)),
        )
        .into_any_element()
}

/// A download or upload rate: in the state's color and a heavier weight while data moves that
/// way, muted otherwise.
fn rate_cell(row: &Row, speed: HumanSpeed, active: bool, theme: &Theme) -> AnyElement {
    number(speed_text(speed), rate_color(row.state, active, theme))
        .when(active, |cell| cell.font_weight(FontWeight::SEMIBOLD))
        .into_any_element()
}

/// A number, right-aligned in tabular digits, so that the digits of the rows line up and a value
/// that changes every tick does not jitter.
fn number(text: SharedString, color: Hsla) -> Div {
    h_flex()
        .size_full()
        .justify_end()
        .font_features(tabular_digits())
        .text_color(color)
        .child(div().truncate().child(text))
}

/// The fraction of the progress bar's track that is filled.
#[expect(
    clippy::cast_possible_truncation,
    reason = "a fraction from 0 to 1 loses only digits far finer than a pixel"
)]
fn bar_fraction(progress: f64) -> f32 {
    progress as f32
}

/// The header of `column`: its title, followed by a mark for `sorted` on the column the list is
/// sorted by. A click calls `on_click`; dragging the header moves the column, which the table
/// handles.
pub(super) fn header(
    column: ColumnId,
    sorted: Option<SortDirection>,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    h_flex()
        .id("column-title")
        .size_full()
        .gap_1()
        .overflow_hidden()
        .cursor_pointer()
        .when(is_numeric(column), Styled::justify_end)
        .child(div().truncate().child(column_title_text(column)))
        .when_some(sorted, |header, direction| {
            header.child(
                div()
                    .id("sort-mark")
                    .flex_none()
                    .text_xs()
                    .child(sort_mark_text(direction))
                    .aria_label(sort_mark_label(direction))
                    .test_support(),
            )
        })
        .on_click(move |_, window, cx| {
            // A short flick on a header starts moving the column, and the release still lands
            // on the header it started on; that is a move, not a click.
            if !cx.has_active_drag() {
                on_click(window, cx);
            }
        })
}

#[cfg(test)]
mod tests {
    use x4_core::test_support::torrent;
    use x4_core::{DisplayState, Torrent};

    use super::*;

    #[test]
    fn numbers_are_the_size_rates_eta_ratio_and_peers() {
        let numeric: Vec<ColumnId> = ColumnId::ALL
            .into_iter()
            .filter(|column| is_numeric(*column))
            .collect();

        assert_eq!(
            numeric,
            [
                ColumnId::Size,
                ColumnId::Down,
                ColumnId::Up,
                ColumnId::Eta,
                ColumnId::Ratio,
                ColumnId::Peers
            ]
        );
    }

    #[test]
    fn every_cell_has_a_text() {
        let row = Row::from(&Torrent {
            state: DisplayState::Downloading,
            progress: 0.5,
            size_when_done: 2048,
            rate_down: 1536,
            rate_up: 0,
            peers_connected: 7,
            ..torrent("Alpha")
        });

        let texts = ColumnId::ALL.map(|column| cell_text(&row, column));

        assert_eq!(
            texts.each_ref().map(SharedString::as_ref),
            [
                "Alpha",
                "2.0 KiB",
                "50% of 2.0 KiB",
                "Downloading",
                "1.5 KiB/s",
                "0 B/s",
                "—",
                "—",
                "7"
            ]
        );
    }

    #[test]
    fn the_bar_fraction_is_the_progress() {
        assert!((bar_fraction(0.25) - 0.25).abs() < f32::EPSILON);
        assert!((bar_fraction(1.0) - 1.0).abs() < f32::EPSILON);
        assert!(bar_fraction(0.0).abs() < f32::EPSILON);
    }
}
