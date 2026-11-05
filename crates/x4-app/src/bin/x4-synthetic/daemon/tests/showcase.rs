//! The showcase daemon, which a screenshot is taken of: a few torrents with names of freely
//! distributable content, in a mix of states that its first rows already show; and the default
//! daemon, which the tests and the measurements depend on, unchanged beside it.

use x4_core::{SortDirection, SortKey};

use super::*;

/// The torrents of a showcase daemon that behaves as `activity` says, in the daemon's order.
fn showcase(activity: Activity) -> Vec<ShownTorrent> {
    let mut client = Client::new(SyntheticDaemon::showcase(activity));
    shown(list(&mut client, &GetIds::All))
}

/// How many rows the main window shows without scrolling in its first-start size.
const ROWS_SHOWN: usize = 8;

#[test]
fn the_showcase_names_its_torrents_from_the_list() {
    let torrents = showcase(Activity::Ticking);

    let names: Vec<&str> = torrents
        .iter()
        .map(|torrent| torrent.name.as_str())
        .collect();
    assert_eq!(names, SHOWCASE_NAMES);
    assert_eq!(names.len(), 24);
    let unique: HashSet<&str> = names.iter().copied().collect();
    assert_eq!(unique.len(), names.len(), "every name is unique");
    let hashes: HashSet<_> = torrents.iter().map(|torrent| &torrent.hash).collect();
    assert_eq!(hashes.len(), torrents.len(), "every hash is unique");
}

#[test]
fn the_showcase_has_every_main_state_whether_ticking_or_static() {
    for activity in [Activity::Ticking, Activity::Static] {
        let states: HashSet<DisplayState> = showcase(activity)
            .iter()
            .map(|torrent| torrent.state)
            .collect();

        for (wanted, name) in [
            (DisplayState::Downloading, "downloading"),
            (DisplayState::Seeding, "seeding"),
            (DisplayState::Paused, "paused"),
            (DisplayState::Error, "error"),
        ] {
            assert!(
                states.contains(&wanted),
                "no {name} torrent in the {activity:?} showcase: {states:?}"
            );
        }
        assert!(
            states
                .iter()
                .any(|state| matches!(state, DisplayState::Queued(_))),
            "no queued torrent in the {activity:?} showcase: {states:?}"
        );
    }
}

#[test]
fn the_static_showcase_is_the_same_torrents_without_rates() {
    let ticking = showcase(Activity::Ticking);
    let idle = showcase(Activity::Static);

    let states = |torrents: &[ShownTorrent]| -> Vec<(String, DisplayState)> {
        torrents
            .iter()
            .map(|torrent| (torrent.name.clone(), torrent.state))
            .collect()
    };
    assert_eq!(states(&idle), states(&ticking));
    assert!(
        idle.iter()
            .all(|torrent| torrent.rate_down == 0 && torrent.rate_up == 0),
        "every rate stays at 0"
    );
    let mut client = Client::new(SyntheticDaemon::showcase(Activity::Static));
    assert_eq!(
        list(&mut client, &GetIds::RecentlyActive).torrents,
        Vec::<TorrentRecord>::new(),
        "no torrent is ever recently active"
    );
}

#[test]
fn the_showcase_is_listed_in_the_order_the_default_sort_shows() {
    let torrents = showcase(Activity::Ticking);
    let mut shown_order: Vec<&ShownTorrent> = torrents.iter().collect();

    x4_core::sort(&mut shown_order, SortKey::Name, SortDirection::Ascending);

    let names: Vec<&str> = shown_order
        .iter()
        .map(|torrent| torrent.name.as_str())
        .collect();
    assert_eq!(names, SHOWCASE_NAMES, "row N is the torrent N + 1");
}

#[test]
fn the_first_rows_of_the_showcase_mix_states() {
    let torrents = showcase(Activity::Ticking);

    let first: HashSet<DisplayState> = torrents
        .iter()
        .take(ROWS_SHOWN)
        .map(|torrent| torrent.state)
        .collect();

    assert!(first.len() >= 4, "the first rows show only {first:?}");
}

#[test]
fn the_showcase_has_a_download_with_peers_for_the_peers_tab() {
    let busiest = showcase(Activity::Ticking)
        .iter()
        .filter(|torrent| torrent.state == DisplayState::Downloading)
        .map(|torrent| torrent.peers_connected)
        .max();

    assert!(
        busiest.is_some_and(|peers| peers >= 10),
        "the busiest download has {busiest:?} peers"
    );
}

#[test]
fn the_default_daemon_is_unchanged() {
    let mut client = Client::new(SyntheticDaemon::new(COUNT, Activity::Static));

    let all = shown(list(&mut client, &GetIds::All));

    assert_eq!(all.len(), 2000);
    assert_eq!(
        all.first().map(|torrent| torrent.name.as_str()),
        Some("Synthetic torrent 0001")
    );
    assert_eq!(
        all.last().map(|torrent| torrent.name.as_str()),
        Some("Synthetic torrent 2000")
    );
}
