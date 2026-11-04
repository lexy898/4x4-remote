//! The lists of the Peers and the Trackers tab: a header row over a `uniform_list` of one-line
//! rows, which builds only the rows in sight, all in the same [`Column`]s, so that the cells of a
//! column line up under its header.
//!
//! Not gpui-component's `DataTable`: the keymap binds Up, Down, Home, End, Page Up and Page Down
//! in the bare `DataTable` context to moving the torrent selection, so a second table with the
//! focus would move it, and the tab would then switch to another torrent. These lists are no tab
//! stops, take no keys and have no menu; the wheel scrolls them. A row's id is keyed by what it
//! shows (a peer's address and port, a tracker's id), never by its place in the list, which
//! changes with every read and every sort.
//!
//! A click on a header sorts by its column, where the column has a sort ([`Column::sort`]); the
//! header of the column sorted by has the mark of the direction after its title, as the torrent
//! table's headers do. The list is a table for screen readers: the header row's cells are column
//! headers, and each row and cell is named.

use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Div, ElementId, InteractiveElement as _, IntoElement, ParentElement as _,
    Role, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _,
    TestSupportExt as _, UniformListScrollHandle, Window, div, px, uniform_list,
};
use x4_core::SortDirection;

use crate::i18n::{sort_mark_label, sort_mark_text};

/// The height of a row, and of the header: one line, as in the file list.
pub(crate) const ROW_HEIGHT: f32 = crate::file_list::ROW_HEIGHT;

/// How wide a column is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Width {
    /// This many logical pixels.
    Fixed(f32),
    /// An equal share, with the other columns of this kind, of the room the fixed columns leave,
    /// and at least this many logical pixels.
    Flex(f32),
}

/// Where a column puts what its cells hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Align {
    /// At the start: text.
    Start,
    /// At the end: numbers, whose digits line up so.
    End,
}

/// A column of a list.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Column<K> {
    /// How wide it is.
    pub(crate) width: Width,
    /// Where its cells put what they hold.
    pub(crate) align: Align,
    /// The title in the header; empty for a column of icons.
    pub(crate) header: SharedString,
    /// What screen readers call the column: the title, or more where the title is short, such as
    /// an arrow.
    pub(crate) name: SharedString,
    /// What a click on the header sorts by; `None` for a header a click does nothing on.
    pub(crate) sort: Option<K>,
}

/// The room of a cell of `column`, in the header or in a row: as wide as the column, its content
/// on one line, cut off at its edge and put where the column puts it. The caller gives it an id
/// and fills it.
pub(crate) fn cell<K>(column: &Column<K>) -> Div {
    let cell = h_flex().h_full().overflow_hidden();
    let cell = match column.width {
        Width::Fixed(width) => cell.flex_none().w(px(width)),
        Width::Flex(min) => cell.flex_1().min_w(px(min)),
    };
    match column.align {
        Align::Start => cell,
        Align::End => cell.justify_end(),
    }
}

/// One line of a list, the header or a row: the cells side by side, with the same room around
/// them on every line, so that the cells of a column line up.
fn line() -> Div {
    h_flex()
        .flex_none()
        .w_full()
        .h(px(ROW_HEIGHT))
        .px_3()
        .gap_2()
        .overflow_hidden()
}

/// The row `id`, the row at `index` of the list, named `label` for screen readers. The caller
/// fills it with one [`cell`] per column, in the order of the columns.
pub(crate) fn row(id: ElementId, index: usize, label: SharedString) -> Stateful<Div> {
    line()
        .id(id)
        .role(Role::Row)
        // The header is the first row.
        .aria_row_index(index + 1)
        .aria_label(label)
}

/// The header row `id` of `columns`: each title, with the mark of `sorted`'s direction after the
/// title of the column sorted by. A click on a header that has a sort calls `on_sort` with it.
/// Each header is `("column", index)` in the row.
pub(crate) fn header<K, F>(
    id: &'static str,
    columns: &[Column<K>],
    sorted: Option<(K, SortDirection)>,
    on_sort: F,
    cx: &App,
) -> impl IntoElement + use<K, F>
where
    K: Copy + PartialEq + 'static,
    F: Fn(K, &mut Window, &mut App) + 'static,
{
    let theme = cx.theme();
    let on_sort = Rc::new(on_sort);
    line()
        .id(id)
        .role(Role::Row)
        .aria_row_index(0)
        .text_xs()
        .bg(theme.table_head)
        .text_color(theme.table_head_foreground)
        .border_b_1()
        .border_color(theme.border)
        .children(columns.iter().enumerate().map(|(index, column)| {
            let mark = sorted
                .filter(|(by, _)| column.sort == Some(*by))
                .map(|(_, direction)| direction);
            let header = cell(column)
                .id(("column", index))
                .role(Role::ColumnHeader)
                .aria_label(column.name.clone())
                .aria_column_index(index)
                .gap_1()
                .child(div().truncate().child(column.header.clone()))
                .when_some(mark, |header, direction| {
                    header.child(
                        div()
                            .id("sort-mark")
                            .flex_none()
                            .child(sort_mark_text(direction))
                            .aria_label(sort_mark_label(direction))
                            .test_support(),
                    )
                });
            match column.sort {
                Some(key) => {
                    let on_sort = Rc::clone(&on_sort);
                    header
                        .cursor_pointer()
                        .on_click(move |_, window, cx| on_sort(key, window, cx))
                }
                None => header,
            }
            .test_support()
        }))
        .test_support()
}

/// The list `id`, named `label` for screen readers: `header` over `rows` rows, of which
/// `render_rows` draws those in sight, scrolled by `scroll`. The caller gives the list its size.
pub(crate) fn list(
    id: &'static str,
    label: SharedString,
    header: impl IntoElement,
    rows: usize,
    scroll: &UniformListScrollHandle,
    render_rows: impl Fn(Range<usize>, &mut Window, &mut App) -> Vec<AnyElement> + 'static,
) -> Stateful<Div> {
    v_flex()
        .id(id)
        .role(Role::Table)
        .aria_label(label)
        .aria_row_count(rows + 1)
        .child(header)
        .child(
            uniform_list("rows", rows, render_rows)
                .track_scroll(scroll)
                .flex_1()
                .min_h_0()
                .w_full(),
        )
}

#[cfg(test)]
mod tests {
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{AppContext as _, Bounds, Context, Pixels, Render, TestAppContext, size};

    use super::*;
    use crate::app;

    /// A list of two columns, a fixed one and one that takes the rest, with two rows, in a
    /// window 400 wide.
    struct Grid(Vec<Column<u8>>);

    impl Render for Grid {
        fn render(&mut self, _: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
            let columns = self.0.clone();
            let header = header(
                "head",
                &self.0,
                Some((1, SortDirection::Descending)),
                |_, _, _| {},
                cx,
            );
            list(
                "grid",
                "Grid".into(),
                header,
                2,
                &UniformListScrollHandle::new(),
                move |range, _, _| {
                    range
                        .map(|index| {
                            row(("row", index).into(), index, "a row".into())
                                .children(columns.iter().enumerate().map(|(column, spec)| {
                                    cell(spec).id(("cell", column)).test_support()
                                }))
                                .test_support()
                                .into_any_element()
                        })
                        .collect()
                },
            )
            .size_full()
            .test_support()
        }
    }

    fn column(width: Width, sort: Option<u8>) -> Column<u8> {
        Column {
            width,
            align: Align::Start,
            header: "Title".into(),
            name: "Title".into(),
            sort,
        }
    }

    #[gpui_kit::test]
    fn the_cells_of_a_column_line_up_under_its_header(cx: &mut TestAppContext) {
        cx.update(app::init);
        let columns = vec![
            column(Width::Fixed(52.), Some(0)),
            column(Width::Flex(140.), Some(1)),
            column(Width::Fixed(76.), None),
        ];
        let window = cx.add_window(|_, _| Grid(columns));
        cx.simulate_window_resize(window.into(), size(px(400.), px(300.)));
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
            let bounds =
                |window: &mut Window, scope: ElementId, cell: ElementId| -> Bounds<Pixels> {
                    window.within(scope).find(cell).bounds()
                };
            for column in 0..3usize {
                let head = bounds(window, "head".into(), ("column", column).into());
                for row in 0..2usize {
                    let cell = bounds(window, ("row", row).into(), ("cell", column).into());
                    assert_eq!(cell.left(), head.left(), "column {column}, row {row}");
                    assert_eq!(
                        cell.size.width, head.size.width,
                        "column {column}, row {row}"
                    );
                    assert_eq!(cell.size.height, px(ROW_HEIGHT));
                }
            }
            let widths: Vec<Pixels> = (0..3usize)
                .map(|column| {
                    bounds(window, "head".into(), ("column", column).into())
                        .size
                        .width
                })
                .collect();
            assert_eq!(widths.first(), Some(&px(52.)));
            assert_eq!(widths.get(2), Some(&px(76.)));
            // The rest of the 400: less the room around and between the cells.
            assert_eq!(widths.get(1), Some(&px(400. - 24. - 16. - 52. - 76.)));

            // Only the header sorted by has the mark.
            assert!(
                window
                    .within("head")
                    .within(("column", 1usize))
                    .try_find("sort-mark")
                    .is_some()
            );
            assert!(
                window
                    .within("head")
                    .within(("column", 0usize))
                    .try_find("sort-mark")
                    .is_none()
            );
        })
        .expect("the window is open");
    }
}
