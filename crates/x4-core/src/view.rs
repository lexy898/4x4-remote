//! What the main window shows, decided here so the views only draw it.
//!
//! [`table_model`] turns a [`Snapshot`] and the user's [`ViewState`] (filter, search, sort) into
//! the [`Row`]s of the table and the [`FilterCounts`] of the sidebar. A [`Row`] holds values, not
//! text: the [`Subtitle`] under the name, the [`EtaView`], the dimming and the whole percent are
//! picked here, and the UI words them through its i18n layer. [`status_model`] does the same for
//! the status bar, and [`visibility`] tells the engine whether the user can see the window.
//!
//! A [`Selection`] is the set of selected rows by hash, which clicks and arrow keys change and
//! whose `targets` are the rows an action reaches. [`availability()`] tells which actions those
//! targets allow, and [`toggle_kind`] what the Space key does to them. [`removal_summary`] tells
//! the removal dialog what it is about to remove.

mod availability;
mod columns;
mod eta;
mod percent;
mod removal;
mod row;
mod selection;
mod status;
mod subtitle;

pub use availability::{Availability, ToggleKind, availability, toggle_kind};
pub use columns::{ColumnId, ViewState};
pub use eta::{CompactDuration, EtaView, TimeUnit};
pub use removal::{RemovalSummary, removal_summary};
pub use row::Row;
pub use selection::Selection;
pub use status::{Indicator, ServiceStatus, StatusModel, status_model, visibility};
pub use subtitle::Subtitle;

use std::sync::Arc;

use crate::filter::{FilterCounts, counts, matches_search};
use crate::model::Torrent;
use crate::snapshot::Snapshot;
use crate::sort::sort;

/// The rows of the table and the counts of the sidebar.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TableModel {
    /// The torrents that match the filter and the search, in the chosen order. Shared, so the
    /// view and the table behind it hold the rows of one snapshot without copying thousands of
    /// them.
    pub rows: Arc<[Row]>,
    /// How many torrents each filter matches, over all torrents: neither the filter nor the
    /// search changes them.
    pub counts: FilterCounts,
    /// Whether the daemon's list of torrents has arrived ([`Snapshot::listed`]); `false` before
    /// the first snapshot.
    pub listed: bool,
}

/// Why the table has no rows, which its placeholder tells the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyTable {
    /// The daemon's list of torrents has not arrived yet: nothing is known, so nothing is said.
    Waiting,
    /// The daemon has no torrents.
    NoTorrents,
    /// There are torrents, but the filter and the search hide every one of them.
    NoMatches,
}

impl TableModel {
    /// The counts the sidebar shows, or `None` before the daemon's list of torrents has arrived
    /// while no torrent is known: zeros would say that there are none, so nothing is said, as in
    /// the table ([`EmptyTable::Waiting`]).
    #[must_use]
    pub fn shown_counts(&self) -> Option<FilterCounts> {
        (self.listed || self.counts.all > 0).then_some(self.counts)
    }

    /// Why the table is empty, or `None` when it has rows. It never says that there are no
    /// torrents before the daemon's list has arrived.
    #[must_use]
    pub fn empty(&self) -> Option<EmptyTable> {
        if !self.rows.is_empty() {
            None
        } else if self.counts.all > 0 {
            Some(EmptyTable::NoMatches)
        } else if self.listed {
            Some(EmptyTable::NoTorrents)
        } else {
            Some(EmptyTable::Waiting)
        }
    }
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
    // Only the rows that are shown are built, so the strings of the others are never copied.
    let mut rows = Vec::with_capacity(shown.len());
    rows.extend(shown.into_iter().map(Row::from));
    TableModel {
        rows: rows.into(),
        counts: counts(&snapshot.torrents),
        listed: snapshot.listed,
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
            listed: true,
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
        assert_eq!(model.empty(), Some(EmptyTable::NoTorrents));
        assert!(model.listed);
    }

    #[test]
    fn no_torrents_is_said_only_once_the_list_has_come() {
        assert_eq!(
            TableModel::default().empty(),
            Some(EmptyTable::Waiting),
            "before the first snapshot"
        );
        for connection in [
            ConnectionState::Connecting,
            ConnectionState::Connected,
            ConnectionState::Reconnecting {
                attempt: 3,
                last_error: crate::snapshot::ConnectionError::Unreachable,
            },
        ] {
            let waiting = Snapshot {
                listed: false,
                connection: connection.clone(),
                ..snapshot(Vec::new())
            };
            assert_eq!(
                table_model(&waiting, &ViewState::default()).empty(),
                Some(EmptyTable::Waiting),
                "{connection:?}"
            );
        }
    }

    #[test]
    fn counts_are_shown_once_the_list_has_come_or_torrents_are_there() {
        assert_eq!(
            TableModel::default().shown_counts(),
            None,
            "before the first snapshot"
        );
        let waiting = Snapshot {
            listed: false,
            ..snapshot(Vec::new())
        };
        assert_eq!(
            table_model(&waiting, &ViewState::default()).shown_counts(),
            None,
            "zeros would say there are no torrents"
        );

        let partial = Snapshot {
            listed: false,
            ..snapshot(vec![named("Alpha", DisplayState::Seeding)])
        };
        let partial = table_model(&partial, &ViewState::default());
        assert_eq!(partial.shown_counts(), Some(partial.counts));
        assert_eq!(partial.counts.all, 1);

        let none = table_model(&snapshot(Vec::new()), &ViewState::default());
        assert_eq!(none.shown_counts(), Some(FilterCounts::default()));
    }

    #[test]
    fn torrents_that_came_before_the_whole_list_are_shown_as_such() {
        let partial = Snapshot {
            listed: false,
            ..snapshot(vec![named("Alpha", DisplayState::Seeding)])
        };

        let shown = table_model(&partial, &ViewState::default());
        let hidden = table_model(
            &partial,
            &ViewState {
                filter: Filter::Paused,
                ..ViewState::default()
            },
        );

        assert_eq!(shown.empty(), None);
        assert_eq!(hidden.empty(), Some(EmptyTable::NoMatches));
    }

    #[test]
    fn a_table_that_hides_every_torrent_says_nothing_matches() {
        let snapshot = snapshot(vec![named("Alpha", DisplayState::Seeding)]);

        let hidden = |view: ViewState| table_model(&snapshot, &view).empty();

        assert_eq!(hidden(ViewState::default()), None);
        assert_eq!(
            hidden(ViewState {
                filter: Filter::Paused,
                ..ViewState::default()
            }),
            Some(EmptyTable::NoMatches)
        );
        assert_eq!(
            hidden(ViewState {
                search: "beta".to_owned(),
                ..ViewState::default()
            }),
            Some(EmptyTable::NoMatches)
        );
    }
}
