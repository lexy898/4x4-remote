//! `torrent_set` on the files of the synthetic torrents, as Transmission 4.1 reads it: the lists
//! of file indices, the empty list that means every file, and the index out of range.

use x4_rpc::{FilePriority, Status};

use super::support::*;
use super::*;

/// A torrent of 3 to 12 files, and one of a single file.
fn a_folder_and_a_single(client: &mut Client<SyntheticDaemon>) -> (HashString, HashString) {
    (
        pick(client, |_, files| (3..=12).contains(&files)),
        pick(client, |_, files| files == 1),
    )
}

/// What a `torrent_set` can change in the torrents `hashes`: their switches and priorities, and
/// the sequential download.
fn state_of(
    client: &mut Client<SyntheticDaemon>,
    hashes: &[&HashString],
) -> Vec<(Vec<bool>, Vec<FilePriority>, bool)> {
    hashes
        .iter()
        .map(|hash| {
            let (wanted, priorities) = switches(client, hash);
            (wanted, priorities, is_sequential(client, hash))
        })
        .collect()
}

#[test]
fn synthetic_torrent_set_files_follows_transmission() {
    let mut client = client(Activity::Static);
    let (hash, single) = a_folder_and_a_single(&mut client);
    let count = switches(&mut client, &hash).0.len();

    // An empty list means all files, in each of the five lists.
    set_raw(&mut client, &[&hash], json!({ "files_unwanted": [] })).expect("set");
    assert_eq!(switches(&mut client, &hash).0, vec![false; count]);
    set_raw(&mut client, &[&hash], json!({ "files_wanted": [] })).expect("set");
    assert_eq!(switches(&mut client, &hash).0, vec![true; count]);
    for (list, priority) in [
        ("priority_high", FilePriority::High),
        ("priority_low", FilePriority::Low),
        ("priority_normal", FilePriority::Normal),
    ] {
        set_raw(&mut client, &[&hash], json!({ list: [] })).expect("set");
        assert_eq!(
            switches(&mut client, &hash).1,
            vec![priority; count],
            "{list}"
        );
    }

    // An index out of range is error 6, "file index out of range", and the call changes nothing:
    // not its other lists, not the sequential download, not the other torrents it names.
    let unchanged = state_of(&mut client, &[&hash, &single]);
    recently_active(&mut client);
    let beyond = json!({ "files_wanted": [count] });
    let negative = json!({ "priority_high": [-1] });
    let after_a_valid_list = json!({
        "files_unwanted": [0], "priority_high": [0], "sequential_download": true,
        "priority_low": [1, count + 40],
    });
    for (settings, torrents) in [
        (beyond.clone(), vec![&hash]),
        (negative, vec![&hash]),
        (after_a_valid_list, vec![&hash]),
        // The single file has no file 1, so the torrent before and after it stays as it was.
        (json!({ "files_unwanted": [1] }), vec![&hash, &single]),
        (json!({ "priority_high": [1] }), vec![&single, &hash]),
    ] {
        let refused = set_raw(&mut client, &torrents, settings.clone());
        assert!(
            matches!(
                &refused,
                Err(RpcError::Server { code: 6, message, .. }) if message == "file index out of range"
            ),
            "{settings}: {refused:?}"
        );
        assert_eq!(
            state_of(&mut client, &[&hash, &single]),
            unchanged,
            "{settings}"
        );
    }
    assert_eq!(
        recently_active(&mut client),
        [] as [HashString; 0],
        "no torrent was touched"
    );
}

#[test]
fn named_files_change_and_the_size_to_download_follows() {
    let mut client = client(Activity::Static);
    let (hash, other) = a_folder_and_a_single(&mut client);
    let before = read(&mut client, &hash, &full_and(&[TorrentField::SizeWhenDone]));
    let others = state_of(&mut client, &[&other]);
    recently_active(&mut client);

    let set = TorrentSet {
        files_unwanted: vec![1],
        priority_high: vec![0],
        priority_low: vec![2],
        ..TorrentSet::default()
    };
    client.torrent_set(&selection(&[&hash]), &set).expect("set");

    let (wanted, priorities) = switches(&mut client, &hash);
    assert_eq!(wanted.iter().filter(|&&wanted| !wanted).count(), 1);
    assert_eq!(wanted.get(1), Some(&false));
    assert_eq!(priorities.first(), Some(&FilePriority::High));
    assert_eq!(priorities.get(1), Some(&FilePriority::Normal));
    assert_eq!(priorities.get(2), Some(&FilePriority::Low));
    let after = read(&mut client, &hash, &[TorrentField::SizeWhenDone]);
    let length = before
        .files
        .as_deref()
        .and_then(|files| files.get(1))
        .expect("file 1")
        .length;
    assert_eq!(
        after.size_when_done,
        before.size_when_done.map(|size| size - length),
        "the unwanted file is not downloaded"
    );
    assert_eq!(
        recently_active(&mut client),
        slice::from_ref(&hash),
        "the torrent changed"
    );
    assert_eq!(
        state_of(&mut client, &[&other]),
        others,
        "and only that one"
    );

    let back = TorrentSet {
        files_wanted: vec![1],
        priority_normal: vec![0, 2],
        ..TorrentSet::default()
    };
    client
        .torrent_set(&selection(&[&hash]), &back)
        .expect("set");
    let count = wanted.len();
    assert_eq!(
        switches(&mut client, &hash),
        (vec![true; count], vec![FilePriority::Normal; count])
    );
    let again = read(&mut client, &hash, &[TorrentField::SizeWhenDone]);
    assert_eq!(again.size_when_done, before.size_when_done);
}

#[test]
fn the_lists_apply_in_the_order_of_the_daemon_and_skip_what_is_not_an_integer() {
    let mut client = client(Activity::Static);
    let (hash, _) = a_folder_and_a_single(&mut client);

    // Unwanted, then wanted; high, low, then normal: the later list wins for a file in both.
    let both = json!({
        "files_wanted": [0], "files_unwanted": [0],
        "priority_normal": [0], "priority_low": [0], "priority_high": [0],
    });
    set_raw(&mut client, &[&hash], both).expect("set");
    let (wanted, priorities) = switches(&mut client, &hash);
    assert_eq!(wanted.first(), Some(&true), "wanted comes after unwanted");
    assert_eq!(
        priorities.first(),
        Some(&FilePriority::Normal),
        "normal comes last"
    );

    // Entries that are not integers are skipped, the integers around them are not.
    let odd = json!({ "files_unwanted": ["1", 2.5, null, true, 0] });
    set_raw(&mut client, &[&hash], odd).expect("set");
    let (wanted, _) = switches(&mut client, &hash);
    assert_eq!(wanted.iter().filter(|&&wanted| !wanted).count(), 1);
    assert_eq!(wanted.first(), Some(&false));
    // A list of nothing but such entries is not empty, so it does not mean every file.
    set_raw(&mut client, &[&hash], json!({ "priority_high": ["all"] })).expect("set");
    assert!(
        switches(&mut client, &hash)
            .1
            .iter()
            .all(|&priority| priority != FilePriority::High)
    );
}

#[test]
fn a_list_of_the_wrong_type_is_refused_and_changes_nothing() {
    let mut client = client(Activity::Static);
    let (hash, single) = a_folder_and_a_single(&mut client);
    let unchanged = state_of(&mut client, &[&hash, &single]);

    for settings in [
        json!({ "files_unwanted": "all" }),
        json!({ "priority_high": 1 }),
        json!({ "files_wanted": null }),
        json!({ "priority_low": [0], "priority_normal": {} }),
        json!({ "files_unwanted": [0], "sequential_download": "yes" }),
    ] {
        let refused = set_raw(&mut client, &[&hash], settings.clone());
        assert!(
            matches!(&refused, Err(RpcError::Server { code, .. }) if *code == INVALID_PARAMS),
            "{settings}: {refused:?}"
        );
        assert_eq!(
            state_of(&mut client, &[&hash, &single]),
            unchanged,
            "{settings}"
        );
    }
}

#[test]
fn the_sequential_download_is_stored_for_the_torrents_named() {
    let mut client = client(Activity::Static);
    let (hash, other) = a_folder_and_a_single(&mut client);

    set_sequential(&mut client, &hash, true);
    assert!(is_sequential(&mut client, &hash));
    assert!(!is_sequential(&mut client, &other));
    set_sequential(&mut client, &hash, false);
    assert!(!is_sequential(&mut client, &hash));
}

#[test]
fn a_set_without_ids_reaches_every_torrent_and_an_empty_list_every_file() {
    let mut client = Client::new(SyntheticDaemon::new(40, Activity::Static));

    client
        .call_raw("torrent_set", Some(json!({ "files_unwanted": [] })))
        .expect("set");

    for (hash, ..) in counts(&mut client) {
        assert!(
            switches(&mut client, &hash).0.iter().all(|&wanted| !wanted),
            "{hash}"
        );
    }
    // The list shows what is to be downloaded: nothing, so nothing is left and it is all done.
    let all = shown(list(&mut client, &GetIds::All));
    assert!(all.iter().all(|torrent| torrent.size_when_done == 0));
    assert!(all.iter().all(|torrent| torrent.left_until_done == 0));
    assert!(
        all.iter()
            .all(|torrent| torrent.total_size > 0 && torrent.progress >= 1.0)
    );
}

#[test]
fn a_download_with_nothing_wanted_left_to_get_becomes_a_seed() {
    let mut client = client(Activity::Ticking);
    let hash = pick(&mut client, |status, _| status == Status::Downloading);
    assert_eq!(find(&mut client, &hash).state, DisplayState::Downloading);

    set_raw(&mut client, &[&hash], json!({ "files_unwanted": [] })).expect("set");

    let torrent = find(&mut client, &hash);
    assert_eq!(
        torrent.state,
        DisplayState::Seeding,
        "nothing is left to download"
    );
    assert_eq!((torrent.rate_down, torrent.peers_sending_to_us), (0, 0));
    // Wanting the files again brings back what they lack, and it downloads again.
    set_raw(&mut client, &[&hash], json!({ "files_wanted": [] })).expect("set");
    let again = find(&mut client, &hash);
    assert!(again.left_until_done > 0);
    assert_eq!(again.state, DisplayState::Downloading);
}
