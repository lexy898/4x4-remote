//! What the main window shows, decided here so the views only draw it.
//!
//! [`table_model`] turns a [`Snapshot`] and the user's [`ViewState`] (filter, search, sort) into
//! the [`Row`]s of the table and the [`FilterCounts`] of the sidebar. A [`Row`] holds values, not
//! text: the [`Subtitle`] under the name, the [`EtaView`], the dimming and the whole percent are
//! picked here, and the UI words them through its i18n layer. [`status_model`] does the same for
//! the status bar, and [`visibility`] tells the engine whether the user can see the window.

mod columns;
mod eta;
mod percent;
mod row;
mod status;
mod subtitle;

pub use columns::{ColumnId, ViewState};
pub use eta::{CompactDuration, EtaView, TimeUnit};
pub use row::Row;
pub use status::{StatusModel, status_model, visibility};
pub use subtitle::Subtitle;

use crate::filter::{FilterCounts, counts, matches_search};
use crate::model::Torrent;
use crate::snapshot::Snapshot;
use crate::sort::sort;

/// The rows of the table and the counts of the sidebar.
#[derive(Debug, Clone, PartialEq)]
pub struct TableModel {
    /// The torrents that match the filter and the search, in the chosen order.
    pub rows: Vec<Row>,
    /// How many torrents each filter matches, over all torrents: neither the filter nor the
    /// search changes them.
    pub counts: FilterCounts,
}

/// The table for `snapshot` as the user's `view` asks for it.
#[must_use]
pub fn table_model(snapshot: &Snapshot, view: &ViewState) -> TableModel {
    let mut shown: Vec<&Torrent> = snapshot
        .torrents
        .iter()
        .filter(|torrent| {
            view.filter.matches(torrent) && matches_search(&torrent.name, &view.search)
        })
        .collect();
    sort(&mut shown, view.sort_key, view.sort_direction);
    TableModel {
        // Only the rows that are shown are built, so the strings of the others are never copied.
        rows: shown.into_iter().map(Row::from).collect(),
        counts: counts(&snapshot.torrents),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::Filter;
    use crate::snapshot::ConnectionState;
    use crate::sort::{SortDirection, SortKey};
    use crate::state::DisplayState;
    use crate::test_support::torrent;

    fn snapshot(torrents: Vec<Torrent>) -> Snapshot {
        Snapshot {
            torrents,
            session: None,
            free_space_bytes: None,
            rate_down: 0,
            rate_up: 0,
            connection: ConnectionState::Connected,
            stale: false,
        }
    }

    fn named(name: &str, state: DisplayState) -> Torrent {
        Torrent {
            state,
            ..torrent(name)
        }
    }

    fn names(model: &TableModel) -> Vec<&str> {
        model.rows.iter().map(|row| row.name.as_str()).collect()
    }

    #[test]
    fn counts_ignore_filter_and_search() {
        let snapshot = snapshot(vec![
            named("x seed", DisplayState::Seeding),
            named("y seed", DisplayState::Seeding),
            named("x paused", DisplayState::Paused),
            named("x download", DisplayState::Downloading),
            named("z seed x", DisplayState::Seeding),
        ]);
        let view = ViewState {
            filter: Filter::Seeding,
            search: "x".to_owned(),
            ..ViewState::default()
        };

        let model = table_model(&snapshot, &view);

        assert_eq!(model.counts.all, 5);
        assert_eq!(model.counts.seeding, 3);
        assert_eq!(model.counts.paused, 1);
        assert_eq!(model.counts.downloading, 1);
        assert_eq!(names(&model), ["x seed", "z seed x"]);
    }

    #[test]
    fn rows_need_both_the_filter_and_the_search() {
        let snapshot = snapshot(vec![
            named("Alpha", DisplayState::Paused),
            named("Beta", DisplayState::Paused),
            named("Alpha two", DisplayState::Seeding),
        ]);

        let both = table_model(
            &snapshot,
            &ViewState {
                filter: Filter::Paused,
                search: "alpha".to_owned(),
                ..ViewState::default()
            },
        );
        let nothing = table_model(
            &snapshot,
            &ViewState {
                filter: Filter::Downloading,
                ..ViewState::default()
            },
        );

        assert_eq!(names(&both), ["Alpha"]);
        assert_eq!(names(&nothing), Vec::<&str>::new());
        assert_eq!(nothing.counts.all, 3);
    }

    #[test]
    fn rows_follow_sort_and_direction() {
        let sized = |name: &str, size_when_done: u64| Torrent {
            size_when_done,
            ..torrent(name)
        };
        let snapshot = snapshot(vec![
            sized("b", 10),
            sized("small", 1),
            sized("a", 10),
            sized("big", 500),
        ]);
        let mut view = ViewState {
            sort_key: SortKey::Size,
            sort_direction: SortDirection::Descending,
            ..ViewState::default()
        };

        // The ties on size are ordered by name, also when the sizes go down.
        assert_eq!(
            names(&table_model(&snapshot, &view)),
            ["big", "a", "b", "small"]
        );

        view.sort_direction = SortDirection::Ascending;
        assert_eq!(
            names(&table_model(&snapshot, &view)),
            ["small", "a", "b", "big"]
        );

        // The default is by name, ignoring case.
        let mixed = self::snapshot(vec![torrent("b"), torrent("A"), torrent("c")]);
        assert_eq!(
            names(&table_model(&mixed, &ViewState::default())),
            ["A", "b", "c"]
        );
    }

    #[test]
    fn an_empty_snapshot_gives_an_empty_table() {
        let model = table_model(&snapshot(Vec::new()), &ViewState::default());

        assert_eq!(names(&model), Vec::<&str>::new());
        assert_eq!(model.counts, FilterCounts::default());
    }

    #[test]
    fn a_row_is_built_from_its_torrent() {
        let source = Torrent {
            size_when_done: 2048,
            ..named("one", DisplayState::Paused)
        };

        let model = table_model(&snapshot(vec![source.clone()]), &ViewState::default());

        assert_eq!(model.rows, [Row::from(&source)]);
    }
}
