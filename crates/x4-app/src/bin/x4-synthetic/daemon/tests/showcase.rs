//! The showcase daemon, which a screenshot is taken of: a few torrents with names of freely
//! distributable content and the sizes such content has, in a mix of states that its first rows
//! already show; and the default daemon, which the tests and the measurements depend on, unchanged
//! beside it.

use x4_core::{SortDirection, SortKey};

use super::*;

/// The torrents of a showcase daemon that behaves as `activity` says, in the daemon's order.
fn showcase(activity: Activity) -> Vec<ShownTorrent> {
    let mut client = Client::new(SyntheticDaemon::showcase(activity));
    shown(list(&mut client, &GetIds::All))
}

/// How many rows the main window shows without scrolling in its first-start size.
const ROWS_SHOWN: usize = 8;

/// One MiB, in bytes.
const MIB: u64 = 1024 * 1024;

/// The names of the table, in its order.
fn table_names() -> Vec<&'static str> {
    SHOWCASE.iter().map(|torrent| torrent.name).collect()
}

#[test]
fn the_showcase_names_its_torrents_from_the_list() {
    let torrents = showcase(Activity::Ticking);

    let names: Vec<&str> = torrents
        .iter()
        .map(|torrent| torrent.name.as_str())
        .collect();
    assert_eq!(names, table_names());
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
    assert_eq!(names, table_names(), "row N is the torrent N + 1");
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

/// How many files each torrent of `daemon` has, in the daemon's order.
fn file_counts(daemon: SyntheticDaemon) -> Vec<u32> {
    let mut client = Client::new(daemon);
    client
        .torrent_get(&GetIds::All, &[TorrentField::FileCount])
        .expect("the client parses the answer")
        .torrents
        .iter()
        .map(|record| record.file_count.expect("a file count"))
        .collect()
}

/// The most files an ordinary plan makes: a series of 40, each followed by a padding file.
const MOST_ORDINARY_FILES: u32 = 80;

#[test]
fn no_showcase_torrent_has_the_album_or_the_archive_plan() {
    // The premise: the seed gives the first 24 torrents of any daemon its album and its archive.
    let default = file_counts(SyntheticDaemon::new(24, Activity::Ticking));
    assert!(
        default.contains(&2000) && default.contains(&50_000),
        "{default:?}"
    );

    for activity in [Activity::Ticking, Activity::Static] {
        let counts = file_counts(SyntheticDaemon::showcase(activity));

        assert!(
            counts.iter().all(|&count| count <= MOST_ORDINARY_FILES),
            "the {activity:?} showcase has the files {counts:?}"
        );
    }
}

#[test]
fn each_showcase_torrent_has_the_size_of_its_row() {
    for activity in [Activity::Ticking, Activity::Static] {
        let torrents = showcase(activity);

        assert_eq!(torrents.len(), SHOWCASE.len());
        for (torrent, row) in torrents.iter().zip(&SHOWCASE) {
            assert_eq!(
                torrent.total_size,
                u64::from(row.mib) * MIB,
                "{} in the {activity:?} showcase",
                row.name
            );
            assert_eq!(torrent.size_when_done, torrent.total_size, "{}", row.name);
        }
    }
}

#[test]
fn a_showcase_torrent_keeps_how_much_of_it_is_done_when_its_size_changes() {
    let mut client = Client::new(SyntheticDaemon::new(24, Activity::Ticking));
    let seeded = shown(list(&mut client, &GetIds::All));

    let resized = showcase(Activity::Ticking);

    let mut unfinished = 0;
    for (seeded, torrent) in seeded.iter().zip(&resized) {
        assert_eq!(seeded.state, torrent.state, "{}", torrent.name);
        assert!(
            (seeded.progress - torrent.progress).abs() < 0.001,
            "{}: {} done before, {} after",
            torrent.name,
            seeded.progress,
            torrent.progress
        );
        assert!(
            torrent.left_until_done <= torrent.total_size,
            "{}",
            torrent.name
        );
        assert_eq!(
            torrent.left_until_done == 0,
            seeded.left_until_done == 0,
            "{}: what is left is still left, or still nothing",
            torrent.name
        );
        unfinished += usize::from(torrent.left_until_done > 0);
    }
    assert!(unfinished >= 5, "only {unfinished} torrents lack data");
}

#[test]
fn the_files_and_pieces_of_a_showcase_torrent_add_up_to_its_size() {
    let mut client = Client::new(SyntheticDaemon::showcase(Activity::Ticking));
    let fields = [
        TorrentField::Name,
        TorrentField::TotalSize,
        TorrentField::LeftUntilDone,
        TorrentField::BytesCompleted,
        TorrentField::PieceSize,
        TorrentField::PieceCount,
        TorrentField::Files,
    ];

    let records = client
        .torrent_get(&GetIds::All, &fields)
        .expect("the client parses the answer")
        .torrents;

    assert_eq!(records.len(), SHOWCASE.len());
    for record in &records {
        let name = record.name.as_deref().expect("a name");
        let total = record.total_size.expect("a size");
        let files = record.files.as_deref().expect("files");
        assert_eq!(
            files.iter().map(|file| file.length).sum::<u64>(),
            total,
            "{name}"
        );
        let piece_size = record.piece_size.expect("a piece size");
        assert_eq!(
            u64::from(record.piece_count.expect("a piece count")),
            total.div_ceil(piece_size),
            "{name}: the pieces cover the size"
        );
        assert_eq!(
            record
                .bytes_completed
                .as_deref()
                .expect("bytes completed")
                .iter()
                .sum::<u64>()
                + record.left_until_done.expect("what is left"),
            total,
            "{name}: what is had and what is left make the size"
        );
    }
}
