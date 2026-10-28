//! The table's columns and the user's choices of what to see.

use crate::filter::Filter;
use crate::sort::{SortDirection, SortKey};

/// A column of the torrent table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColumnId {
    /// The name, with the subtitle under it.
    Name,
    /// The size of the wanted files.
    Size,
    /// The progress bar.
    Progress,
    /// The display state.
    State,
    /// The download rate.
    Down,
    /// The upload rate.
    Up,
    /// The time to finish.
    Eta,
    /// The share ratio.
    Ratio,
    /// The connected peers.
    Peers,
}

impl ColumnId {
    /// Every column, in the order the table shows them.
    pub const ALL: [ColumnId; 9] = [
        Self::Name,
        Self::Size,
        Self::Progress,
        Self::State,
        Self::Down,
        Self::Up,
        Self::Eta,
        Self::Ratio,
        Self::Peers,
    ];

    /// A stable name for the column, for the state file and the table's column keys. It never
    /// changes between versions and is never shown to the user.
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Size => "size",
            Self::Progress => "progress",
            Self::State => "state",
            Self::Down => "down",
            Self::Up => "up",
            Self::Eta => "eta",
            Self::Ratio => "ratio",
            Self::Peers => "peers",
        }
    }

    /// The key a click on this column's header sorts the list by.
    #[must_use]
    pub fn sort_key(self) -> SortKey {
        match self {
            Self::Name => SortKey::Name,
            Self::Size => SortKey::Size,
            Self::Progress => SortKey::Progress,
            Self::State => SortKey::State,
            Self::Down => SortKey::DownloadSpeed,
            Self::Up => SortKey::UploadSpeed,
            Self::Eta => SortKey::Eta,
            Self::Ratio => SortKey::Ratio,
            Self::Peers => SortKey::Peers,
        }
    }

    /// The direction a column is sorted in when it is chosen: largest first for the columns where
    /// that is what the user looks for, otherwise smallest first.
    fn first_direction(self) -> SortDirection {
        match self {
            Self::Size | Self::Progress | Self::Down | Self::Up | Self::Ratio | Self::Peers => {
                SortDirection::Descending
            }
            Self::Name | Self::State | Self::Eta => SortDirection::Ascending,
        }
    }
}

/// What the user chose to see: the filter, the search and the sort order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewState {
    /// The sidebar filter.
    pub filter: Filter,
    /// The text in the search box; empty matches everything.
    pub search: String,
    /// The column the list is sorted by.
    pub sort_key: SortKey,
    /// Which way `sort_key` orders the list.
    pub sort_direction: SortDirection,
}

impl Default for ViewState {
    /// Every torrent, no search, sorted by name from A to Z.
    fn default() -> Self {
        Self {
            filter: Filter::All,
            search: String::new(),
            sort_key: SortKey::Name,
            sort_direction: SortDirection::Ascending,
        }
    }
}

impl ViewState {
    /// Handles a click on the header of `column`: the column that is already sorted flips its
    /// direction. Any other column starts largest first if it is [`ColumnId::Size`],
    /// [`ColumnId::Progress`], [`ColumnId::Down`], [`ColumnId::Up`], [`ColumnId::Ratio`] or
    /// [`ColumnId::Peers`], and smallest first otherwise.
    pub fn toggle_sort(&mut self, column: ColumnId) {
        let key = column.sort_key();
        if self.sort_key == key {
            self.sort_direction = match self.sort_direction {
                SortDirection::Ascending => SortDirection::Descending,
                SortDirection::Descending => SortDirection::Ascending,
            };
        } else {
            self.sort_key = key;
            self.sort_direction = column.first_direction();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_state_starts_with_everything_sorted_by_name() {
        let view = ViewState::default();

        assert_eq!(view.filter, Filter::All);
        assert_eq!(view.search, "");
        assert_eq!(view.sort_key, SortKey::Name);
        assert_eq!(view.sort_direction, SortDirection::Ascending);
    }

    #[test]
    fn toggle_sort_flips_and_starts_by_kind() {
        let mut view = ViewState::default();

        view.toggle_sort(ColumnId::Name);
        assert_eq!(
            (view.sort_key, view.sort_direction),
            (SortKey::Name, SortDirection::Descending)
        );

        view.toggle_sort(ColumnId::Size);
        assert_eq!(
            (view.sort_key, view.sort_direction),
            (SortKey::Size, SortDirection::Descending)
        );

        view.toggle_sort(ColumnId::Name);
        assert_eq!(
            (view.sort_key, view.sort_direction),
            (SortKey::Name, SortDirection::Ascending)
        );

        // Clicking the same column twice comes back to where it was.
        view.toggle_sort(ColumnId::Name);
        view.toggle_sort(ColumnId::Name);
        assert_eq!(view.sort_direction, SortDirection::Ascending);
    }

    #[test]
    fn a_new_column_starts_in_its_own_direction() {
        let descending = [
            ColumnId::Size,
            ColumnId::Progress,
            ColumnId::Down,
            ColumnId::Up,
            ColumnId::Ratio,
            ColumnId::Peers,
        ];
        let ascending = [ColumnId::Name, ColumnId::State, ColumnId::Eta];

        for column in ColumnId::ALL {
            // Start from a column that is none of the above, so the click always switches.
            let mut view = ViewState {
                sort_key: SortKey::Added,
                ..ViewState::default()
            };
            view.toggle_sort(column);

            let expected = if descending.contains(&column) {
                SortDirection::Descending
            } else {
                assert!(ascending.contains(&column), "{column:?} is in neither list");
                SortDirection::Ascending
            };
            assert_eq!(view.sort_key, column.sort_key(), "{column:?}");
            assert_eq!(view.sort_direction, expected, "{column:?}");
        }
    }

    #[test]
    fn toggle_sort_keeps_the_filter_and_the_search() {
        let mut view = ViewState {
            filter: Filter::Paused,
            search: "abc".to_owned(),
            ..ViewState::default()
        };

        view.toggle_sort(ColumnId::Down);

        assert_eq!(view.filter, Filter::Paused);
        assert_eq!(view.search, "abc");
    }

    #[test]
    fn column_keys_are_unique_snake_case_names() {
        let keys = ColumnId::ALL.map(ColumnId::key);

        assert_eq!(
            keys,
            [
                "name", "size", "progress", "state", "down", "up", "eta", "ratio", "peers"
            ]
        );
        let unique: std::collections::HashSet<_> = keys.into_iter().collect();
        assert_eq!(unique.len(), ColumnId::ALL.len());
    }

    #[test]
    fn column_sort_keys() {
        let keys = ColumnId::ALL.map(ColumnId::sort_key);

        assert_eq!(
            keys,
            [
                SortKey::Name,
                SortKey::Size,
                SortKey::Progress,
                SortKey::State,
                SortKey::DownloadSpeed,
                SortKey::UploadSpeed,
                SortKey::Eta,
                SortKey::Ratio,
                SortKey::Peers,
            ]
        );
        assert_eq!(
            ColumnId::ALL,
            [
                ColumnId::Name,
                ColumnId::Size,
                ColumnId::Progress,
                ColumnId::State,
                ColumnId::Down,
                ColumnId::Up,
                ColumnId::Eta,
                ColumnId::Ratio,
                ColumnId::Peers,
            ]
        );
    }
}
