//! Which torrent the details panel is about.

use x4_rpc::HashString;

use super::row::Row;
use super::selection::Selection;

/// What the details panel is about, decided from the selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailsTarget {
    /// No torrent is selected.
    None,
    /// Exactly one torrent is selected, which the panel shows.
    One(HashString),
    /// Several torrents are selected, so the panel names their number and reads nothing.
    Many(usize),
}

/// The target of the details panel for the `selection` among the table's `rows`.
///
/// Only the selected rows that are shown count, as for an action: a selected torrent that a
/// filter or the search hides is not a target. The selection's lead does not decide it, since
/// the lead can sit on a row that is not selected.
#[must_use]
pub fn details_target(selection: &Selection, rows: &[Row]) -> DetailsTarget {
    match selection.targets(rows).as_slice() {
        [] => DetailsTarget::None,
        [one] => DetailsTarget::One(one.hash.clone()),
        many => DetailsTarget::Many(many.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::torrent;

    fn rows() -> Vec<Row> {
        ["a", "b", "c"]
            .iter()
            .map(|name| Row::from(&torrent(name)))
            .collect()
    }

    #[test]
    fn details_target_is_one_only_for_a_single_visible_selection() {
        let rows = rows();
        let (a, b) = (rows[0].hash.clone(), rows[1].hash.clone());
        let mut selection = Selection::default();
        assert_eq!(details_target(&selection, &rows), DetailsTarget::None);

        selection.click(&a);
        assert_eq!(
            details_target(&selection, &rows),
            DetailsTarget::One(a.clone())
        );

        selection.toggle(&b);
        assert_eq!(details_target(&selection, &rows), DetailsTarget::Many(2));

        // Ctrl-clicking the only selected row off leaves the lead on it, but selects nothing.
        selection.click(&a);
        selection.toggle(&a);
        assert_eq!(selection.lead(), Some(&a));
        assert_eq!(details_target(&selection, &rows), DetailsTarget::None);
    }

    #[test]
    fn a_selected_torrent_that_is_not_shown_is_not_a_target() {
        let rows = rows();
        let hidden = torrent("hidden").hash;
        let shown = rows[2].hash.clone();
        let mut selection = Selection::default();

        selection.click(&hidden);
        assert_eq!(details_target(&selection, &rows), DetailsTarget::None);

        selection.toggle(&shown);
        assert_eq!(selection.len(), 2);
        assert_eq!(details_target(&selection, &rows), DetailsTarget::One(shown));
    }
}
