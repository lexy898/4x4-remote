use gpui_kit::TestAppContext;
use gpui_kit::px;
use x4_core::test_support::torrent;
use x4_core::{DisplayState, Torrent, default_columns};

use super::*;

fn rows(names: &[&str]) -> Arc<[Row]> {
    names.iter().map(|name| Row::from(&torrent(name))).collect()
}

fn table() -> TorrentTable {
    TorrentTable::new(default_columns(), (SortKey::Name, SortDirection::Ascending))
}

#[test]
fn set_rows_keeps_the_selected_torrent_by_hash() {
    let mut table = table();
    table.set_rows(rows(&["a", "b", "c"]));
    table.select_row(1);
    let selected = table.selected().cloned();

    table.set_rows(rows(&["0", "c", "b", "a"]));

    assert_eq!(table.selected().cloned(), selected);
    assert_eq!(table.selected_index(), Some(2));
}

#[test]
fn set_rows_drops_a_torrent_that_is_gone() {
    let mut table = table();
    table.set_rows(rows(&["a", "b"]));
    table.select_row(0);

    table.set_rows(rows(&["b"]));
    assert_eq!(table.selected(), None);
    assert_eq!(table.selected_index(), None);

    table.set_rows(rows(&["a", "b"]));
    assert_eq!(
        table.selected(),
        None,
        "a torrent that comes back is not selected again"
    );
}

#[test]
fn selecting_past_the_rows_selects_nothing() {
    let mut table = table();
    table.set_rows(rows(&["a"]));
    table.select_row(0);

    table.select_row(5);
    assert_eq!(table.selected(), None);

    table.select_row(0);
    table.clear_selected();
    assert_eq!(table.selected_index(), None);
}

#[gpui_kit::test]
fn hidden_columns_are_left_out(cx: &mut TestAppContext) {
    let mut columns = default_columns();
    for column in &mut columns {
        column.visible = !matches!(column.id, ColumnId::Size | ColumnId::Peers);
    }
    let table = TorrentTable::new(columns, (SortKey::Name, SortDirection::Ascending));

    cx.update(|cx| {
        assert_eq!(table.columns_count(cx), 7);
        let keys: Vec<SharedString> = (0..7).map(|ix| table.column(ix, cx).key).collect();
        assert_eq!(
            keys,
            ["name", "progress", "state", "down", "up", "eta", "ratio"].map(SharedString::from)
        );
        assert_eq!(table.column(1, cx).name, "Progress");
        assert_eq!(table.column(3, cx).name, "Download speed");
    });
}

#[gpui_kit::test]
fn columns_carry_the_stored_widths(cx: &mut TestAppContext) {
    let mut table = table();

    assert!(table.set_widths(&[px(300.), px(120.)]));
    assert!(!table.set_widths(&[px(300.)]), "nothing changed");

    cx.update(|cx| {
        assert_eq!(table.column(0, cx).width, px(300.));
        assert_eq!(table.column(1, cx).width, px(120.));
        assert_eq!(table.column(2, cx).width, px(140.), "the default");
        let size = table.column(1, cx);
        assert_eq!(size.min_width, px(MIN_COLUMN_WIDTH));
        assert_eq!(size.max_width, px(MAX_COLUMN_WIDTH));
        assert!(!size.selectable, "a click on a header selects no column");
        assert!(
            size.sort.is_none(),
            "the table draws no sort icon of its own"
        );
    });
}

#[gpui_kit::test]
fn numbers_are_right_aligned(cx: &mut TestAppContext) {
    use gpui_kit::TextAlign;

    let table = table();

    cx.update(|cx| {
        let aligns: Vec<TextAlign> = (0..9).map(|ix| table.column(ix, cx).align).collect();
        assert_eq!(
            aligns,
            [
                TextAlign::Left,
                TextAlign::Right,
                TextAlign::Left,
                TextAlign::Left,
                TextAlign::Right,
                TextAlign::Right,
                TextAlign::Right,
                TextAlign::Right,
                TextAlign::Right,
            ]
        );
    });
}

#[gpui_kit::test]
fn cell_labels_name_the_row_and_column(cx: &mut TestAppContext) {
    let mut table = table();
    table.set_rows(rows(&["Alpha", "Beta"]));

    cx.update(|cx| {
        assert_eq!(table.cell_label(1, 0, cx).as_deref(), Some("Beta"));
        assert_eq!(table.cell_label(0, 3, cx).as_deref(), Some("Seeding"));
        assert_eq!(table.cell_label(2, 0, cx), None, "past the rows");
        assert_eq!(table.cell_label(0, 9, cx), None, "past the columns");
    });
}

#[gpui_kit::test]
fn idle_rows_are_dimmed_and_active_rows_are_not(cx: &mut TestAppContext) {
    let mut table = table();
    let downloading = Torrent {
        state: DisplayState::Downloading,
        ..torrent("active")
    };
    // A seeder with nobody downloading from it is idle.
    table.set_rows([Row::from(&torrent("idle")), Row::from(&downloading)].into());
    cx.update(gpui_kit::init);
    let window = cx.add_window(|window, cx| new_table_state(table, window, cx));

    let opacities = window
        .update(cx, |state, window, cx| {
            [0, 1].map(|ix| {
                state
                    .delegate_mut()
                    .render_tr(ix, window, cx)
                    .style()
                    .opacity
            })
        })
        .expect("the window is open");

    assert_eq!(opacities, [Some(DIMMED_OPACITY), None]);
}
