//! Which rows of the table the user has selected.

use std::collections::HashSet;

use x4_rpc::HashString;

use super::row::Row;

/// The rows the user has selected, by hash, with the two rows that keyboard and mouse work from.
///
/// Rows are identified by their [`HashString`], so a refresh that renumbers or reorders the
/// torrents does not change the selection. The display order is always the order of the `rows`
/// that the methods are given; the selection itself has none.
///
/// The `anchor` is where a range starts (the last plain or Ctrl click), the `lead` is the row the
/// arrow keys move from and the end of the range. Both can be a row that is not selected: toggling
/// the only selected row off leaves the lead on it, so that Shift+arrows can go on from there.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    selected: HashSet<HashString>,
    anchor: Option<HashString>,
    lead: Option<HashString>,
}

impl Selection {
    /// A plain click: only this row is selected, and it becomes the anchor and the lead.
    pub fn click(&mut self, hash: &HashString) {
        self.selected.clear();
        self.selected.insert(hash.clone());
        self.anchor = Some(hash.clone());
        self.lead = Some(hash.clone());
    }

    /// Ctrl+click: adds the row to the selection, or removes it when it is selected. Either way it
    /// becomes the anchor and the lead.
    pub fn toggle(&mut self, hash: &HashString) {
        if !self.selected.remove(hash) {
            self.selected.insert(hash.clone());
        }
        self.anchor = Some(hash.clone());
        self.lead = Some(hash.clone());
    }

    /// Shift+click: selects the rows from the anchor to `hash` in the display order of `rows`,
    /// replacing the previous range. The anchor stays and `hash` becomes the lead.
    ///
    /// It acts like [`click`](Self::click) when there is no anchor, or when the anchor is not
    /// among `rows` any more. It does nothing when `hash` is not among `rows`.
    pub fn extend_to(&mut self, hash: &HashString, rows: &[Row]) {
        let Some(end) = position(rows, hash) else {
            return;
        };
        let Some(start) = self
            .anchor
            .as_ref()
            .and_then(|anchor| position(rows, anchor))
        else {
            self.click(hash);
            return;
        };
        let range = rows
            .get(start.min(end)..=start.max(end))
            .unwrap_or_default();
        self.selected = range.iter().map(|row| row.hash.clone()).collect();
        self.lead = Some(hash.clone());
    }

    /// Moves the lead by `step` rows (negative is up), stopping at the first and last row.
    ///
    /// With `extend` (Shift+arrow) the range from the anchor follows the lead; without it the new
    /// lead is selected alone, like a click. With no lead (or one that is not among `rows`) a
    /// positive step lands on the first row and a negative one on the last, as in a file manager.
    /// A step of zero and an empty `rows` change nothing.
    pub fn move_lead(&mut self, step: isize, rows: &[Row], extend: bool) {
        if step == 0 {
            return;
        }
        let current = self.lead.as_ref().and_then(|lead| position(rows, lead));
        let target = match current {
            Some(index) if step > 0 => index.saturating_add(step.unsigned_abs()),
            Some(index) => index.saturating_sub(step.unsigned_abs()),
            None if step > 0 => 0,
            None => usize::MAX,
        };
        // A target past the end lands on the last row; there is none when `rows` is empty.
        let Some(row) = rows.get(target).or(rows.last()) else {
            return;
        };
        if extend {
            self.extend_to(&row.hash, rows);
        } else {
            self.click(&row.hash);
        }
    }

    /// Ctrl+A: selects exactly `rows`. The anchor and the lead stay when they are among them.
    pub fn select_all(&mut self, rows: &[Row]) {
        self.selected = rows.iter().map(|row| row.hash.clone()).collect();
        self.anchor
            .take_if(|anchor| !self.selected.contains(anchor));
        self.lead.take_if(|lead| !self.selected.contains(lead));
    }

    /// Selects nothing and forgets the anchor and the lead.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Drops what `rows` (the rows now shown) no longer holds: the selected hashes, and the
    /// anchor and the lead when their rows are gone. Called after the filter, the search or a
    /// refresh changes the rows, so that an action never reaches a row the user cannot see.
    pub fn retain_visible(&mut self, rows: &[Row]) {
        let visible: HashSet<&HashString> = rows.iter().map(|row| &row.hash).collect();
        self.selected.retain(|hash| visible.contains(hash));
        self.anchor.take_if(|anchor| !visible.contains(&*anchor));
        self.lead.take_if(|lead| !visible.contains(&*lead));
    }

    /// Whether the row is selected.
    #[must_use]
    pub fn contains(&self, hash: &HashString) -> bool {
        self.selected.contains(hash)
    }

    /// The row the arrow keys move from, if there is one. It need not be selected.
    #[must_use]
    pub fn lead(&self) -> Option<&HashString> {
        self.lead.as_ref()
    }

    /// How many rows are selected.
    #[must_use]
    pub fn len(&self) -> usize {
        self.selected.len()
    }

    /// Whether no row is selected.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }

    /// The selected rows among `rows`, in the display order of `rows` and not in the order the
    /// user clicked them. Selected hashes that are not in `rows` are skipped.
    #[must_use]
    pub fn targets<'r>(&self, rows: &'r [Row]) -> Vec<&'r Row> {
        rows.iter()
            .filter(|row| self.selected.contains(&row.hash))
            .collect()
    }
}

/// The index of the row with `hash`.
fn position(rows: &[Row], hash: &HashString) -> Option<usize> {
    rows.iter().position(|row| row.hash == *hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::torrent;

    /// `count` rows in display order, named `row 0`, `row 1` and so on. Their hashes are not in
    /// the order of the rows, so a test cannot pass by sorting hashes.
    fn rows(count: usize) -> Vec<Row> {
        (0..count)
            .map(|index| Row::from(&torrent(&format!("row {index}"))))
            .collect()
    }

    fn hash_of(rows: &[Row], index: usize) -> &HashString {
        &rows[index].hash
    }

    fn names<'r>(rows: &[&'r Row]) -> Vec<&'r str> {
        rows.iter().map(|row| row.name.as_str()).collect()
    }

    fn selected_names<'r>(selection: &Selection, rows: &'r [Row]) -> Vec<&'r str> {
        names(&selection.targets(rows))
    }

    #[test]
    fn click_selects_only_that_row() {
        let rows = rows(3);
        let mut selection = Selection::default();
        selection.click(hash_of(&rows, 0));
        selection.toggle(hash_of(&rows, 1));

        selection.click(hash_of(&rows, 2));

        assert_eq!(selected_names(&selection, &rows), ["row 2"]);
        assert_eq!(selection.len(), 1);
        assert!(!selection.is_empty());
        assert!(selection.contains(hash_of(&rows, 2)));
        assert!(!selection.contains(hash_of(&rows, 0)));
        assert_eq!(selection.lead(), Some(hash_of(&rows, 2)));
        // The anchor is the clicked row: Shift+click extends from it.
        let mut extended = selection.clone();
        extended.extend_to(hash_of(&rows, 0), &rows);
        assert_eq!(
            selected_names(&extended, &rows),
            ["row 0", "row 1", "row 2"]
        );
    }

    #[test]
    fn toggle_adds_then_removes_and_moves_the_lead() {
        let rows = rows(3);
        let mut selection = Selection::default();
        selection.click(hash_of(&rows, 0));

        selection.toggle(hash_of(&rows, 2));
        assert_eq!(selected_names(&selection, &rows), ["row 0", "row 2"]);
        assert_eq!(selection.lead(), Some(hash_of(&rows, 2)));

        selection.toggle(hash_of(&rows, 0));
        assert_eq!(selected_names(&selection, &rows), ["row 2"]);
        assert_eq!(
            selection.lead(),
            Some(hash_of(&rows, 0)),
            "the lead goes to the toggled row, also when it was removed"
        );

        // The toggled row is the anchor of the next Shift+click.
        selection.extend_to(hash_of(&rows, 1), &rows);
        assert_eq!(selected_names(&selection, &rows), ["row 0", "row 1"]);
    }

    #[test]
    fn toggling_the_only_selected_row_empties_the_selection_but_keeps_the_lead() {
        let rows = rows(4);
        let mut selection = Selection::default();
        selection.click(hash_of(&rows, 1));

        selection.toggle(hash_of(&rows, 1));

        assert!(selection.is_empty());
        assert_eq!(selection.len(), 0);
        assert_eq!(selected_names(&selection, &rows), Vec::<&str>::new());
        assert_eq!(selection.lead(), Some(hash_of(&rows, 1)));

        // Shift+Down continues from the row the lead stayed on.
        selection.move_lead(1, &rows, true);
        assert_eq!(selected_names(&selection, &rows), ["row 1", "row 2"]);
        assert_eq!(selection.lead(), Some(hash_of(&rows, 2)));
    }

    #[test]
    fn shift_click_selects_the_range_in_display_order() {
        let rows = rows(5);
        let mut selection = Selection::default();
        selection.click(hash_of(&rows, 1));

        selection.extend_to(hash_of(&rows, 3), &rows);
        assert_eq!(
            selected_names(&selection, &rows),
            ["row 1", "row 2", "row 3"]
        );
        assert_eq!(selection.lead(), Some(hash_of(&rows, 3)));

        // The anchor stays on row 1: the new range replaces the old one.
        selection.extend_to(hash_of(&rows, 0), &rows);
        assert_eq!(selected_names(&selection, &rows), ["row 0", "row 1"]);
        assert_eq!(selection.len(), 2);
        assert_eq!(selection.lead(), Some(hash_of(&rows, 0)));
    }

    #[test]
    fn shift_click_without_anchor_acts_like_click() {
        let rows = rows(4);
        let mut selection = Selection::default();

        selection.extend_to(hash_of(&rows, 2), &rows);

        assert_eq!(selected_names(&selection, &rows), ["row 2"]);
        assert_eq!(selection.lead(), Some(hash_of(&rows, 2)));

        // It also became the anchor.
        selection.extend_to(hash_of(&rows, 3), &rows);
        assert_eq!(selected_names(&selection, &rows), ["row 2", "row 3"]);
    }

    #[test]
    fn shift_click_with_an_anchor_that_left_the_rows_acts_like_click() {
        let all = rows(4);
        let mut selection = Selection::default();
        selection.click(hash_of(&all, 0));

        let shown = &all[1..];
        selection.extend_to(hash_of(&all, 3), shown);

        assert_eq!(selected_names(&selection, shown), ["row 3"]);
        assert_eq!(selection.lead(), Some(hash_of(&all, 3)));
    }

    #[test]
    fn shift_click_on_a_row_that_is_not_shown_changes_nothing() {
        let all = rows(4);
        let shown = &all[..2];
        let mut selection = Selection::default();
        selection.click(hash_of(&all, 0));

        selection.extend_to(hash_of(&all, 3), shown);

        assert_eq!(selected_names(&selection, shown), ["row 0"]);
        assert_eq!(selection.lead(), Some(hash_of(&all, 0)));
    }

    #[test]
    fn move_lead_clamps_and_extends() {
        let rows = rows(4);

        // With no lead, a step down starts at the first row and a step up at the last.
        let mut selection = Selection::default();
        selection.move_lead(1, &rows, false);
        assert_eq!(selected_names(&selection, &rows), ["row 0"]);
        assert_eq!(selection.lead(), Some(hash_of(&rows, 0)));

        let mut selection = Selection::default();
        selection.move_lead(-1, &rows, false);
        assert_eq!(selected_names(&selection, &rows), ["row 3"]);
        assert_eq!(selection.lead(), Some(hash_of(&rows, 3)));

        // A plain move selects only the new lead and clamps at both ends.
        selection.move_lead(-1, &rows, false);
        assert_eq!(selected_names(&selection, &rows), ["row 2"]);
        selection.move_lead(-10, &rows, false);
        assert_eq!(selected_names(&selection, &rows), ["row 0"]);
        selection.move_lead(-1, &rows, false);
        assert_eq!(selected_names(&selection, &rows), ["row 0"]);
        selection.move_lead(2, &rows, false);
        assert_eq!(selected_names(&selection, &rows), ["row 2"]);
        selection.move_lead(isize::MAX, &rows, false);
        assert_eq!(selected_names(&selection, &rows), ["row 3"]);
        selection.move_lead(isize::MIN, &rows, false);
        assert_eq!(selected_names(&selection, &rows), ["row 0"]);

        // Extending keeps the anchor and grows or shrinks the range with the lead.
        selection.click(hash_of(&rows, 1));
        selection.move_lead(1, &rows, true);
        assert_eq!(selected_names(&selection, &rows), ["row 1", "row 2"]);
        selection.move_lead(1, &rows, true);
        selection.move_lead(1, &rows, true);
        assert_eq!(
            selected_names(&selection, &rows),
            ["row 1", "row 2", "row 3"]
        );
        assert_eq!(selection.lead(), Some(hash_of(&rows, 3)));
        selection.move_lead(-3, &rows, true);
        assert_eq!(selected_names(&selection, &rows), ["row 0", "row 1"]);
        assert_eq!(selection.lead(), Some(hash_of(&rows, 0)));

        // A plain move after extending drops the range.
        selection.move_lead(1, &rows, false);
        assert_eq!(selected_names(&selection, &rows), ["row 1"]);

        // Extending with no lead starts a selection at the end the step points to.
        let mut selection = Selection::default();
        selection.move_lead(1, &rows, true);
        assert_eq!(selected_names(&selection, &rows), ["row 0"]);
    }

    #[test]
    fn move_lead_changes_nothing_for_no_rows_or_no_step() {
        let rows = rows(3);
        let mut selection = Selection::default();
        selection.move_lead(1, &[], false);
        selection.move_lead(0, &rows, false);
        assert!(selection.is_empty());
        assert_eq!(selection.lead(), None);

        selection.click(hash_of(&rows, 1));
        selection.move_lead(0, &rows, true);
        selection.move_lead(1, &[], true);
        assert_eq!(selected_names(&selection, &rows), ["row 1"]);
        assert_eq!(selection.lead(), Some(hash_of(&rows, 1)));
    }

    #[test]
    fn move_lead_from_a_lead_that_left_the_rows_starts_at_an_end() {
        let all = rows(4);
        let mut selection = Selection::default();
        selection.click(hash_of(&all, 0));

        let shown = &all[1..];
        selection.move_lead(1, shown, false);

        assert_eq!(selected_names(&selection, shown), ["row 1"]);
    }

    #[test]
    fn select_all_selects_only_given_rows() {
        let all = rows(5);
        let mut selection = Selection::default();
        selection.click(hash_of(&all, 4));
        selection.toggle(hash_of(&all, 0));

        let shown = &all[1..4];
        selection.select_all(shown);

        assert_eq!(
            selected_names(&selection, &all),
            ["row 1", "row 2", "row 3"]
        );
        assert_eq!(selection.len(), 3);
        assert!(!selection.contains(hash_of(&all, 0)));
        assert!(!selection.contains(hash_of(&all, 4)));
        assert_eq!(
            selection.lead(),
            None,
            "the lead was a row that is no longer selected"
        );

        // A lead that is still among the rows stays.
        selection.click(hash_of(&all, 2));
        selection.select_all(shown);
        assert_eq!(selection.lead(), Some(hash_of(&all, 2)));
        assert_eq!(selection.len(), 3);

        selection.select_all(&[]);
        assert!(selection.is_empty());
    }

    #[test]
    fn clear_forgets_everything() {
        let rows = rows(3);
        let mut selection = Selection::default();
        selection.click(hash_of(&rows, 1));
        selection.extend_to(hash_of(&rows, 2), &rows);

        selection.clear();

        assert_eq!(selection, Selection::default());
        assert_eq!(selection.lead(), None);
        // With no anchor left, Shift+click acts like click.
        selection.extend_to(hash_of(&rows, 0), &rows);
        assert_eq!(selected_names(&selection, &rows), ["row 0"]);
    }

    #[test]
    fn retain_visible_drops_hidden_rows_and_their_anchor() {
        let all = rows(5);
        let mut selection = Selection::default();
        selection.click(hash_of(&all, 0));
        selection.extend_to(hash_of(&all, 3), &all);
        assert_eq!(selection.len(), 4);

        // Row 0 (the anchor) and row 1 are filtered away.
        let shown = &all[2..];
        selection.retain_visible(shown);

        assert_eq!(selected_names(&selection, &all), ["row 2", "row 3"]);
        assert_eq!(selection.len(), 2);
        assert_eq!(selection.lead(), Some(hash_of(&all, 3)));
        assert!(!selection.contains(hash_of(&all, 0)));
        assert!(!selection.contains(hash_of(&all, 1)));
        // The anchor went with its row, so Shift+click starts afresh.
        selection.extend_to(hash_of(&all, 4), shown);
        assert_eq!(selected_names(&selection, &all), ["row 4"]);

        // A lead that is hidden goes too.
        selection.retain_visible(&all[..2]);
        assert!(selection.is_empty());
        assert_eq!(selection.lead(), None);
    }

    #[test]
    fn retain_visible_keeps_what_is_still_shown() {
        let rows = rows(3);
        let mut selection = Selection::default();
        selection.click(hash_of(&rows, 0));
        selection.extend_to(hash_of(&rows, 2), &rows);
        let before = selection.clone();

        selection.retain_visible(&rows);

        assert_eq!(selection, before);
    }

    #[test]
    fn targets_are_in_display_order_not_click_order() {
        let rows = rows(5);
        let mut selection = Selection::default();
        selection.click(hash_of(&rows, 3));
        selection.toggle(hash_of(&rows, 0));
        selection.toggle(hash_of(&rows, 4));

        assert_eq!(
            selected_names(&selection, &rows),
            ["row 0", "row 3", "row 4"]
        );

        // The order follows the rows given, whatever they are.
        let reversed: Vec<Row> = rows.iter().rev().cloned().collect();
        assert_eq!(
            selected_names(&selection, &reversed),
            ["row 4", "row 3", "row 0"]
        );
    }

    #[test]
    fn targets_skip_selected_rows_that_are_not_shown() {
        let all = rows(3);
        let mut selection = Selection::default();
        selection.click(hash_of(&all, 0));
        selection.toggle(hash_of(&all, 2));

        // `targets` does not change the selection, it only looks at the rows given.
        assert_eq!(selected_names(&selection, &all[1..]), ["row 2"]);
        assert_eq!(selection.len(), 2);
    }
}
