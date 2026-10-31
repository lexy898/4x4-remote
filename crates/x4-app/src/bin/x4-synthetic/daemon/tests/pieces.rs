//! The pieces of the synthetic torrents: they follow the progress, the files agree with them, and
//! with the sequential download on they come from the start.

use x4_rpc::Status;

use super::support::*;
use super::*;

/// The hashes of the torrents that download and have less than half of more than 4 GiB, so that
/// a few dozen polls never finish them, in the order of the list.
fn long_downloads(client: &mut Client<SyntheticDaemon>) -> Vec<HashString> {
    list(client, &GetIds::All)
        .torrents
        .into_iter()
        .filter(|record| record.status == Some(Status::Downloading))
        .filter(|record| record.percent_done < Some(0.5) && record.total_size > Some(4 << 30))
        .map(|record| record.hash_string.expect("a hash"))
        .map(|hash| HashString::parse(&hash).expect("valid"))
        .collect()
}

/// Polls the recently active torrents `times`, as the engine does while the window is visible.
fn poll(client: &mut Client<SyntheticDaemon>, times: u32) {
    for _ in 0..times {
        recently_active(client);
    }
}

#[test]
fn synthetic_pieces_follow_progress_and_sequential() {
    let mut client = client(Activity::Ticking);
    let hash = long_downloads(&mut client).remove(0);

    // The pieces hold exactly what the list says is done, and each file agrees with them.
    let first = View::of(&mut client, &hash);
    assert_eq!(first.present_bytes(), first.total - first.left);
    assert!((share(first.present_bytes(), first.total) - first.percent_done).abs() < 0.01);
    assert_eq!(first.reported(), first.covered());
    assert!(
        first
            .have
            .iter()
            .skip(first.first_missing())
            .any(|&have| have),
        "without sequential the pieces are scattered, not a prefix"
    );

    // As it downloads pieces are added, never lost, and still scattered.
    poll(&mut client, 20);
    let later = View::of(&mut client, &hash);
    let added = later.gained_since(&first);
    assert!(!added.is_empty(), "the download went on");
    assert_eq!(later.present(), first.present() + added.len());
    assert!(later.left < first.left);
    assert_eq!(later.present_bytes(), later.total - later.left);
    assert_eq!(later.reported(), later.covered());
    assert!(
        added.iter().any(|&piece| piece > later.first_missing()),
        "the new pieces are scattered"
    );

    // With sequential on, the pieces that come are the lowest ones missing, so what plays from
    // the start grows.
    set_sequential(&mut client, &hash, true);
    poll(&mut client, 20);
    let ordered = View::of(&mut client, &hash);
    let added = ordered.gained_since(&later);
    assert!(!added.is_empty(), "the download went on");
    assert!(
        added.iter().all(|&piece| piece < ordered.first_missing()),
        "{added:?} are not the first of the missing, {} is missing",
        ordered.first_missing()
    );
    assert!(ordered.first_missing() > later.first_missing());
    assert_eq!(ordered.reported(), ordered.covered());

    // Turned off again, the pieces that come are scattered again, and none is lost.
    set_sequential(&mut client, &hash, false);
    poll(&mut client, 20);
    let scattered = View::of(&mut client, &hash);
    let added = scattered.gained_since(&ordered);
    assert!(added.iter().any(|&piece| piece > scattered.first_missing()));
}

#[test]
fn pieces_are_a_prefix_when_sequential_is_on_from_the_start() {
    let mut client = client(Activity::Ticking);
    let hash = long_downloads(&mut client).remove(1);
    set_sequential(&mut client, &hash, true);

    let first = View::of(&mut client, &hash);

    let present = first.present();
    assert!(present > 0);
    assert_eq!(
        first.first_missing(),
        present,
        "the pieces are the first ones"
    );
    assert_eq!(first.present_bytes(), first.total - first.left);
    poll(&mut client, 10);
    let grown = View::of(&mut client, &hash);
    assert!(grown.present() > present);
    assert_eq!(
        grown.first_missing(),
        grown.present(),
        "and they stay a prefix"
    );
    assert_eq!(grown.reported(), grown.covered());
    // The first file holds what the prefix reaches into it.
    let first_file = grown.files.first().expect("a file");
    let prefix_bytes = u64::try_from(grown.first_missing()).expect("fits") * grown.piece_size;
    assert_eq!(
        grown.reported().first(),
        Some(&prefix_bytes.min(first_file.length))
    );
}

#[test]
fn complete_and_idle_torrents_have_steady_pieces() {
    let mut client = client(Activity::Static);
    let seeding = pick(&mut client, |status, _| status == Status::Seeding);

    let done = View::of(&mut client, &seeding);

    assert!(
        done.have.iter().all(|&have| have),
        "a seeding torrent has every piece"
    );
    assert!(
        done.files
            .iter()
            .all(|file| file.bytes_completed == file.length)
    );
    assert_eq!(done.left, 0);

    let downloading = pick(&mut client, |status, _| status == Status::Downloading);
    let before = read(&mut client, &downloading, TorrentField::DETAILS_FULL);
    poll(&mut client, 3);
    let after = read(&mut client, &downloading, TorrentField::DETAILS_FULL);
    assert_eq!(before, after, "a static daemon stands still");
}

#[test]
fn a_download_stops_short_of_the_end_and_keeps_going() {
    // A small daemon polls quickly; its download closest to the end gets there within a while.
    let mut client = Client::new(SyntheticDaemon::new(300, Activity::Ticking));
    let hash = list(&mut client, &GetIds::All)
        .torrents
        .into_iter()
        .filter(|record| record.status == Some(Status::Downloading))
        .min_by_key(|record| record.left_until_done)
        .and_then(|record| record.hash_string)
        .map(|hash| HashString::parse(&hash).expect("valid"))
        .expect("a download");
    set_sequential(&mut client, &hash, true);

    let mut last = View::of(&mut client, &hash);
    for _ in 0..50 {
        poll(&mut client, 100);
        let now = View::of(&mut client, &hash);
        let stopped = now.left == last.left;
        last = now;
        if stopped {
            break;
        }
    }

    // As before the torrents had pieces: it comes close to the end and stays there.
    assert!(last.left > 0 && last.left <= last.total / 50);
    assert!(last.have.iter().any(|&have| !have));
    assert!(last.percent_done > 0.98 && last.percent_done < 1.0);
    assert_eq!(last.present_bytes(), last.total - last.left);
}

/// The first tracks of the album that a small choice of files wants.
const WANTED_TRACKS: usize = 10;

/// The bytes the first `WANTED_TRACKS` files of `view` still lack.
fn tracks_missing(view: &View) -> u64 {
    view.files
        .iter()
        .take(WANTED_TRACKS)
        .map(|file| file.length - file.bytes_completed)
        .sum()
}

#[test]
fn wanting_one_part_of_the_album_still_downloads() {
    let mut client = client(Activity::Ticking);
    let album = pick(&mut client, |status, files| {
        status == Status::Downloading && files == 2000
    });
    let wanted: Vec<usize> = (0..WANTED_TRACKS).collect();
    set_raw(
        &mut client,
        &[&album],
        json!({ "files_unwanted": [], "files_wanted": wanted }),
    )
    .expect("only the first tracks are wanted");

    // The wanted tracks are a small part of the pieces: less than the last percent that a
    // download used to stay short of, counted over all the pieces.
    let first = View::of(&mut client, &album);
    let wanted_pieces = first
        .files
        .get(WANTED_TRACKS - 1)
        .map(|file| usize::try_from(file.end_piece).expect("fits"))
        .expect("the last wanted track");
    assert!(
        wanted_pieces <= first.have.len() / 100,
        "{wanted_pieces} of {} pieces are wanted",
        first.have.len()
    );
    let mut missing = tracks_missing(&first);
    assert!(
        missing > first.piece_size,
        "the wanted tracks lack {missing} bytes, so there is something to download"
    );

    // Each few polls bring more of them, until what is left is the one piece that the download
    // stays short of.
    let mut rounds = 0;
    while missing > first.piece_size {
        rounds += 1;
        assert!(rounds <= 60, "{missing} bytes are still missing");
        poll(&mut client, 5);
        let now = tracks_missing(&View::of(&mut client, &album));
        assert!(
            now < missing,
            "{missing} bytes were missing and {now} are after {rounds} rounds of polls"
        );
        missing = now;
    }
}
