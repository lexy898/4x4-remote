//! The changes the user makes to the table's columns: showing and hiding them, moving them and
//! resizing them.
//!
//! The table on screen counts only the shown columns, so a move or a list of widths comes in by
//! the position among those; the hidden columns keep their place in [`UiState::columns`]
//! (`super::UiState`), so that a column shown again comes back where it was.

use super::{ColumnState, MAX_COLUMN_WIDTH, MIN_COLUMN_WIDTH};
use crate::view::ColumnId;

/// The shown columns of `columns`, in order.
pub fn shown_columns(columns: &[ColumnState]) -> impl Iterator<Item = &ColumnState> {
    columns.iter().filter(|column| column.visible)
}

/// Shows or hides the column `id`, and returns whether that changed anything. A column that
/// cannot be hidden ([`ColumnId::can_hide`]) stays shown.
pub fn set_column_shown(columns: &mut [ColumnState], id: ColumnId, shown: bool) -> bool {
    let shown = shown || !id.can_hide();
    match columns.iter_mut().find(|column| column.id == id) {
        Some(column) if column.visible != shown => {
            column.visible = shown;
            true
        }
        _ => false,
    }
}

/// Moves the shown column at `from` so that it ends up at `to`, both counted among the shown
/// columns, as the table does when the user drags a header. Returns whether anything moved; an
/// index past the shown columns moves nothing.
pub fn move_shown_column(columns: &mut Vec<ColumnState>, from: usize, to: usize) -> bool {
    if from == to {
        return false;
    }
    let shown: Vec<usize> = columns
        .iter()
        .enumerate()
        .filter(|(_, column)| column.visible)
        .map(|(index, _)| index)
        .collect();
    let (Some(&source), Some(&target)) = (shown.get(from), shown.get(to)) else {
        return false;
    };
    // Moving right, the target column slides one place left once the moved one is out, so
    // inserting at its old index puts the moved column right after it; moving left, the
    // target stays and the moved column goes right before it. Either way it lands at `to`
    // among the shown columns.
    let column = columns.remove(source);
    columns.insert(target, column);
    true
}

/// Takes the widths of the shown columns, in order, as the table reports them after a resize,
/// and returns whether any changed. Each width is clamped to [`MIN_COLUMN_WIDTH`] to
/// [`MAX_COLUMN_WIDTH`]; one that is not a number leaves its column as it is, and widths past
/// the shown columns are ignored.
pub fn set_shown_widths(columns: &mut [ColumnState], widths: &[f32]) -> bool {
    let mut changed = false;
    let shown = columns.iter_mut().filter(|column| column.visible);
    for (column, &width) in shown.zip(widths) {
        if !width.is_finite() {
            continue;
        }
        let width = width.clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH);
        if column.width.total_cmp(&width).is_ne() {
            column.width = width;
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_state::default_columns;

    fn ids(columns: &[ColumnState]) -> Vec<ColumnId> {
        columns.iter().map(|column| column.id).collect()
    }

    fn shown_ids(columns: &[ColumnState]) -> Vec<ColumnId> {
        shown_columns(columns).map(|column| column.id).collect()
    }

    fn widths(columns: &[ColumnState]) -> Vec<f32> {
        columns.iter().map(|column| column.width).collect()
    }

    /// The default columns with `hidden` hidden.
    fn with_hidden(hidden: &[ColumnId]) -> Vec<ColumnState> {
        let mut columns = default_columns();
        for column in &mut columns {
            column.visible = !hidden.contains(&column.id);
        }
        columns
    }

    #[test]
    fn hiding_and_showing_a_column() {
        let mut columns = default_columns();

        assert!(set_column_shown(&mut columns, ColumnId::Peers, false));
        assert!(!shown_ids(&columns).contains(&ColumnId::Peers));
        assert_eq!(
            ids(&columns),
            ColumnId::ALL,
            "a hidden column keeps its place"
        );
        assert!(
            !set_column_shown(&mut columns, ColumnId::Peers, false),
            "hidden already"
        );

        assert!(set_column_shown(&mut columns, ColumnId::Peers, true));
        assert_eq!(shown_ids(&columns), ColumnId::ALL);
        assert!(!set_column_shown(&mut columns, ColumnId::Peers, true));
    }

    #[test]
    fn the_name_column_cannot_be_hidden() {
        let mut columns = default_columns();

        assert!(!set_column_shown(&mut columns, ColumnId::Name, false));
        assert_eq!(shown_ids(&columns), ColumnId::ALL);
    }

    #[test]
    fn a_column_that_is_not_in_the_list_changes_nothing() {
        let mut columns: Vec<ColumnState> = default_columns()
            .into_iter()
            .filter(|column| column.id != ColumnId::Eta)
            .collect();
        let before = columns.clone();

        assert!(!set_column_shown(&mut columns, ColumnId::Eta, false));
        assert_eq!(columns, before);
    }

    #[test]
    fn moving_a_column_without_hidden_ones_is_a_plain_move() {
        use ColumnId::{Down, Eta, Name, Peers, Progress, Ratio, Size, State, Up};
        let mut columns = default_columns();

        assert!(move_shown_column(&mut columns, 1, 3));
        assert_eq!(
            ids(&columns),
            [Name, Progress, State, Size, Down, Up, Eta, Ratio, Peers]
        );

        assert!(move_shown_column(&mut columns, 8, 0));
        assert_eq!(
            ids(&columns),
            [Peers, Name, Progress, State, Size, Down, Up, Eta, Ratio]
        );
    }

    #[test]
    fn moving_counts_only_the_shown_columns() {
        use ColumnId::{Down, Eta, Name, Peers, Progress, Ratio, Size, State, Up};
        // Shown: Name, Size, State, Up, Eta, Ratio, Peers.
        let mut columns = with_hidden(&[Progress, Down]);

        // Size (shown 1) goes after Up (shown 3).
        assert!(move_shown_column(&mut columns, 1, 3));
        assert_eq!(
            shown_ids(&columns),
            [Name, State, Up, Size, Eta, Ratio, Peers]
        );
        assert_eq!(
            ids(&columns),
            [Name, Progress, State, Down, Up, Size, Eta, Ratio, Peers]
        );

        // Peers (shown 6) goes before State (shown 1), and the hidden Progress stays before it.
        assert!(move_shown_column(&mut columns, 6, 1));
        assert_eq!(
            shown_ids(&columns),
            [Name, Peers, State, Up, Size, Eta, Ratio]
        );
        assert_eq!(
            ids(&columns),
            [Name, Progress, Peers, State, Down, Up, Size, Eta, Ratio]
        );
    }

    #[test]
    fn a_move_to_the_same_place_or_out_of_range_moves_nothing() {
        let mut columns = with_hidden(&[ColumnId::Peers]);
        let before = columns.clone();

        assert!(!move_shown_column(&mut columns, 2, 2));
        assert!(!move_shown_column(&mut columns, 8, 0), "only 8 are shown");
        assert!(!move_shown_column(&mut columns, 0, 8));
        assert_eq!(columns, before);
    }

    #[test]
    fn widths_go_to_the_shown_columns_in_order() {
        let mut columns = with_hidden(&[ColumnId::Size]);
        let mut expected = widths(&columns);

        // Name, Progress, State, Down: Size is hidden and keeps its width.
        assert!(set_shown_widths(&mut columns, &[300.0, 150.0, 110.0, 95.5]));
        expected.splice(0..5, [300.0, 90.0, 150.0, 110.0, 95.5]);
        assert_eq!(widths(&columns), expected);

        assert!(!set_shown_widths(&mut columns, &[300.0]), "nothing changed");
    }

    #[test]
    fn widths_are_clamped_and_bad_ones_are_ignored() {
        let mut columns = default_columns();
        let mut extra = vec![10.0, 5000.0, f32::NAN, f32::INFINITY];
        extra.extend([1.0; 6]);

        assert!(set_shown_widths(&mut columns, &extra));

        let got = widths(&columns);
        assert_eq!(got.first(), Some(&MIN_COLUMN_WIDTH));
        assert_eq!(got.get(1), Some(&MAX_COLUMN_WIDTH));
        let defaults = widths(&default_columns());
        assert_eq!(got.get(2), defaults.get(2), "not a number: unchanged");
        assert_eq!(got.get(3), defaults.get(3), "infinite: unchanged");
        assert_eq!(got.len(), ColumnId::ALL.len(), "the extra width is ignored");
    }
}
