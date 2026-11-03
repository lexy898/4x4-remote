//! The offline tests of `support.rs`: the checks of an add request, accepting the added torrent,
//! the folders and the local data. Only
//! x4-rpc's live tests declare this file; x4-core's include `support.rs` too and would run these a
//! second time.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use x4_rpc::{
    AddOutcome, AddSource, AddedTorrent, DownloadDir, HashString, MagnetLink, TorrentAdd, TorrentId,
};

use crate::metainfo::{Metainfo, random_seed, single_file_metainfo};
use crate::support::{
    KeptFile, LocalData, TestFolder, accept_added, check_add_again, check_first_add, file_is_gone,
    is_in_test_dir,
};

fn hash(digit: char) -> HashString {
    HashString::parse(&digit.to_string().repeat(40)).unwrap()
}

fn torrent(hash: HashString) -> AddedTorrent {
    AddedTorrent {
        id: TorrentId(9),
        name: "4x4-live-0.bin".to_owned(),
        hash,
    }
}

fn test_dir() -> DownloadDir {
    DownloadDir::new("/mnt/hdd/torrents/4x4-test").unwrap()
}

/// A small single-file torrent of this run, named `name`.
fn own_torrent(name: &str) -> Metainfo {
    single_file_metainfo(name, name.as_bytes(), 16 * 1024)
}

/// The request that adds `metainfo` into `folder`, paused.
fn add_request(metainfo: &Metainfo, folder: &TestFolder) -> TorrentAdd {
    TorrentAdd {
        source: AddSource::Metainfo(Arc::from(metainfo.bytes.as_slice())),
        download_dir: Some(folder.remote().clone()),
        paused: true,
        files_unwanted: Vec::new(),
        priority_high: Vec::new(),
        priority_low: Vec::new(),
        sequential_download: None,
    }
}

/// The same request with a magnet link of `metainfo`'s info hash instead of the metainfo.
fn by_magnet(request: TorrentAdd, metainfo: &Metainfo) -> TorrentAdd {
    let link = format!("magnet:?xt=urn:btih:{}", metainfo.info_hash);
    TorrentAdd {
        source: AddSource::Magnet(MagnetLink::parse(&link).unwrap()),
        ..request
    }
}

#[test]
fn a_first_add_of_this_runs_torrent_into_its_own_folder_passes() {
    let own = own_torrent("4x4-live-0.bin");
    for folder in [
        TestFolder::test_dir(&test_dir()),
        TestFolder::new_subfolder(&test_dir(), 1),
    ] {
        let request = add_request(&own, &folder);

        check_first_add(&request, &folder, &own);
        check_first_add(&by_magnet(request, &own), &folder, &own);
    }
}

#[test]
#[should_panic(expected = "a test torrent downloads into its own folder only; nothing was sent")]
fn a_first_add_into_another_folder_is_never_sent() {
    let own = own_torrent("4x4-live-0.bin");
    let folder = TestFolder::test_dir(&test_dir());
    let elsewhere = TestFolder::new_subfolder(&test_dir(), 1);

    check_first_add(&add_request(&own, &elsewhere), &folder, &own);
}

#[test]
#[should_panic(expected = "the request does not carry this run's torrent")]
fn a_first_add_of_another_torrent_is_never_sent() {
    let own = own_torrent("4x4-live-0.bin");
    let other = own_torrent("4x4-live-1.bin");
    let folder = TestFolder::test_dir(&test_dir());

    check_first_add(&add_request(&other, &folder), &folder, &own);
}

#[test]
#[should_panic(expected = "the request does not carry this run's torrent")]
fn a_first_add_by_a_magnet_link_of_another_torrent_is_never_sent() {
    let own = own_torrent("4x4-live-0.bin");
    let other = own_torrent("4x4-live-1.bin");
    let folder = TestFolder::test_dir(&test_dir());

    check_first_add(
        &by_magnet(add_request(&own, &folder), &other),
        &folder,
        &own,
    );
}

#[test]
fn a_second_add_of_the_same_torrent_into_a_folder_of_the_test_directory_passes() {
    let own = own_torrent("4x4-live-0.bin");
    let own_folder = TestFolder::test_dir(&test_dir());
    let first = add_request(&own, &own_folder);
    for folder in [
        own_folder.clone(),
        TestFolder::new_subfolder(&test_dir(), 2),
    ] {
        let again = add_request(&own, &folder);

        check_add_again(&first, &own.info_hash, &own_folder, &again, &folder);
        let by_link = by_magnet(again, &own);
        check_add_again(&first, &own.info_hash, &own_folder, &by_link, &folder);
    }
}

#[test]
#[should_panic(expected = "is not in the test directory of")]
fn a_second_add_outside_the_test_directory_is_never_sent() {
    let own = own_torrent("4x4-live-0.bin");
    let own_folder = TestFolder::test_dir(&test_dir());
    let first = add_request(&own, &own_folder);
    let outside = TestFolder::test_dir(&DownloadDir::new("/mnt/hdd/torrents").unwrap());

    check_add_again(
        &first,
        &own.info_hash,
        &own_folder,
        &add_request(&own, &outside),
        &outside,
    );
}

#[test]
#[should_panic(
    expected = "a second add names a folder of the test directory only; nothing was sent"
)]
fn a_second_add_into_another_folder_than_the_one_it_names_is_never_sent() {
    let own = own_torrent("4x4-live-0.bin");
    let own_folder = TestFolder::test_dir(&test_dir());
    let first = add_request(&own, &own_folder);
    let subfolder = TestFolder::new_subfolder(&test_dir(), 2);

    check_add_again(
        &first,
        &own.info_hash,
        &own_folder,
        &add_request(&own, &subfolder),
        &own_folder,
    );
}

#[test]
#[should_panic(expected = "a second add carries this run's torrent")]
fn a_second_add_of_another_torrent_is_never_sent() {
    let own = own_torrent("4x4-live-0.bin");
    let other = own_torrent("4x4-live-1.bin");
    let folder = TestFolder::test_dir(&test_dir());
    let first = add_request(&own, &folder);

    check_add_again(
        &first,
        &own.info_hash,
        &folder,
        &add_request(&other, &folder),
        &folder,
    );
}

#[test]
#[should_panic(expected = "a second add carries this run's torrent")]
fn a_second_add_by_a_magnet_link_of_another_torrent_is_never_sent() {
    let own = own_torrent("4x4-live-0.bin");
    let other = own_torrent("4x4-live-1.bin");
    let folder = TestFolder::test_dir(&test_dir());
    let first = add_request(&own, &folder);

    check_add_again(
        &first,
        &own.info_hash,
        &folder,
        &by_magnet(add_request(&own, &folder), &other),
        &folder,
    );
}

#[test]
#[should_panic(expected = "a second add carries this run's torrent")]
fn a_second_add_from_metainfo_of_a_torrent_first_added_by_its_magnet_link_is_never_sent() {
    // The metainfo cannot be compared with a magnet link's hash without parsing it, so it is
    // refused.
    let own = own_torrent("4x4-live-0.bin");
    let folder = TestFolder::test_dir(&test_dir());
    let first = by_magnet(add_request(&own, &folder), &own);

    check_add_again(
        &first,
        &own.info_hash,
        &folder,
        &add_request(&own, &folder),
        &folder,
    );
}

#[test]
fn a_new_torrent_with_the_built_hash_is_accepted() {
    let outcome = AddOutcome::Added(torrent(hash('a')));

    assert_eq!(accept_added(&outcome, &hash('a')), Ok(TorrentId(9)));
}

#[test]
fn a_new_torrent_with_another_hash_is_rejected() {
    let outcome = AddOutcome::Added(torrent(hash('b')));

    assert!(accept_added(&outcome, &hash('a')).is_err());
}

#[test]
fn a_duplicate_is_rejected_even_with_the_built_hash() {
    let outcome = AddOutcome::Duplicate(torrent(hash('a')));

    assert!(accept_added(&outcome, &hash('a')).is_err());
}

/// A local test file in a directory of its own, which the file creates.
fn local_file() -> (PathBuf, LocalData) {
    let dir = env::temp_dir().join(format!("x4-live-support-{:016x}", random_seed()));
    let file = LocalData::write_file(&dir, "kept.bin", b"data").unwrap();
    (dir, file)
}

#[test]
fn a_kept_file_outlives_the_torrent_and_is_deleted_by_the_test() {
    let (dir, mut file) = local_file();

    let kept = KeptFile(file.hand_over());
    drop(file);

    assert!(
        !file_is_gone(kept.path()),
        "the torrent's guard deleted the kept file"
    );
    let path = kept.path().to_owned();
    kept.delete();
    assert!(file_is_gone(&path), "{} is still there", path.display());
    assert!(!dir.exists(), "{} is left behind", dir.display());
}

#[test]
fn a_folder_of_files_is_deleted_with_the_directory_it_made() {
    let dir = env::temp_dir().join(format!("x4-live-support-{:016x}", random_seed()));
    let files: [(&str, &[u8]); 2] = [("a.bin", b"a"), ("b.bin", b"bb")];
    let data = LocalData::write_folder(&dir, "folder", &files).unwrap();
    assert_eq!(fs::read(dir.join("folder/b.bin")).unwrap(), b"bb");
    assert!(!data.files_are_gone());

    drop(data);

    assert!(!dir.exists(), "{} is left behind", dir.display());
}

#[test]
fn a_folder_that_is_already_there_is_left_alone() {
    let dir = env::temp_dir().join(format!("x4-live-support-{:016x}", random_seed()));
    let theirs = dir.join("folder");
    fs::create_dir_all(&theirs).unwrap();
    fs::write(theirs.join("a.bin"), b"theirs").unwrap();
    let files: [(&str, &[u8]); 1] = [("a.bin", b"ours")];

    let written = LocalData::write_folder(&dir, "folder", &files);

    assert!(written.is_err());
    assert_eq!(fs::read(theirs.join("a.bin")).unwrap(), b"theirs");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn files_the_daemon_deleted_leave_the_folder_to_remove() {
    let dir = env::temp_dir().join(format!("x4-live-support-{:016x}", random_seed()));
    let files: [(&str, &[u8]); 2] = [("a.bin", b"a"), ("b.bin", b"bb")];
    let mut data = LocalData::write_folder(&dir, "folder", &files).unwrap();
    // As the daemon does when it deletes a torrent's data, the files go first.
    fs::remove_file(dir.join("folder/a.bin")).unwrap();
    fs::remove_file(dir.join("folder/b.bin")).unwrap();
    assert!(data.files_are_gone());

    assert_eq!(data.delete(), Vec::<String>::new());

    assert!(data.is_clean());
    assert!(!dir.exists(), "{} is left behind", dir.display());
}

#[test]
fn a_kept_file_is_deleted_when_the_test_fails_before_deleting_it() {
    let (dir, mut file) = local_file();
    let kept = KeptFile(file.hand_over());
    drop(file);

    drop(kept);

    assert!(!dir.exists(), "{} is left behind", dir.display());
}

#[test]
fn the_test_directory_is_compared_in_its_normalized_form() {
    let test_dir = DownloadDir::new("//mnt/hdd//torrents/4x4-test/").unwrap();
    let folder = TestFolder::test_dir(&test_dir);

    for dir in [
        "/mnt/hdd/torrents/4x4-test",
        "/mnt/hdd/torrents/4x4-test/",
        "//mnt/hdd//torrents/4x4-test//",
    ] {
        assert!(folder.is(dir), "{dir}");
        assert!(is_in_test_dir(dir, &test_dir), "{dir}");
    }
    // Inside it. A folder that does not normalize is compared by its text, so it is counted
    // rather than missed.
    for dir in [
        "/mnt/hdd/torrents/4x4-test/sub",
        "//mnt/hdd/torrents//4x4-test//sub/",
        "/mnt/hdd/torrents/4x4-test/../other",
    ] {
        assert!(!folder.is(dir), "{dir}");
        assert!(is_in_test_dir(dir, &test_dir), "{dir}");
    }
    for dir in [
        "/mnt/hdd/torrents",
        "/mnt/hdd/torrents/4x4-test2",
        "/mnt/hdd/torrents/4x4-tes",
        "mnt/hdd/torrents/4x4-test",
        "",
    ] {
        assert!(!folder.is(dir), "{dir}");
        assert!(!is_in_test_dir(dir, &test_dir), "{dir}");
    }
}

#[test]
fn a_folder_with_a_control_character_is_never_the_test_directory() {
    let test_dir = DownloadDir::new("/mnt/hdd/torrents/4x4-test").unwrap();
    let folder = TestFolder::test_dir(&test_dir);

    // `DownloadDir::new` trims these at either end, so they must be refused before it does.
    for dir in [
        "/mnt/hdd/torrents/4x4-test\n",
        "/mnt/hdd/torrents/4x4-test/\r\n",
        "\t/mnt/hdd/torrents/4x4-test",
        "\0/mnt/hdd/torrents/4x4-test",
        "/mnt/hdd/torrents/4x4-test\u{7f}",
        "/mnt/hdd/torrents/4x4-test\u{85}",
        "/mnt/hdd/torrents/4x4\u{1b}-test",
    ] {
        assert!(!folder.is(dir), "{dir:?}");
    }
}

#[test]
fn a_subfolder_is_its_own_name_directly_in_the_test_directory_and_nothing_else() {
    let test_dir = DownloadDir::new("/mnt/hdd/torrents/4x4-test").unwrap();
    let folder = TestFolder::new_subfolder(&test_dir, 0xabc);
    let own = "/mnt/hdd/torrents/4x4-test/4x4-live-new-0000000000000abc";

    assert_eq!(folder.remote().as_str(), own);
    assert_eq!(folder.subfolder(), Some("4x4-live-new-0000000000000abc"));
    assert_eq!(TestFolder::test_dir(&test_dir).subfolder(), None);
    for dir in [
        own,
        "/mnt/hdd/torrents/4x4-test/4x4-live-new-0000000000000abc/",
        "//mnt/hdd/torrents//4x4-test//4x4-live-new-0000000000000abc",
    ] {
        assert!(folder.is(dir), "{dir}");
    }
    for dir in [
        // The test directory itself, and the folders around it.
        "/mnt/hdd/torrents/4x4-test",
        "/mnt/hdd/torrents",
        "/mnt/hdd/torrents/4x4-live-new-0000000000000abc",
        // Another run's folder, and folders inside this one or beside it.
        "/mnt/hdd/torrents/4x4-test/4x4-live-new-0000000000000abd",
        "/mnt/hdd/torrents/4x4-test/4x4-live-new-0000000000000abc/sub",
        "/mnt/hdd/torrents/4x4-test/4x4-live-new-0000000000000abc2",
        "/mnt/hdd/torrents/4x4-test/sub/4x4-live-new-0000000000000abc",
        "/mnt/hdd/torrents/4x4-test/4x4-live-new-0000000000000abc/..",
        "/mnt/hdd/torrents/4x4-test2/4x4-live-new-0000000000000abc",
        "mnt/hdd/torrents/4x4-test/4x4-live-new-0000000000000abc",
        // Control characters, even where `DownloadDir::new` would trim them.
        "/mnt/hdd/torrents/4x4-test/4x4-live-new-0000000000000abc\n",
        "\n/mnt/hdd/torrents/4x4-test/4x4-live-new-0000000000000abc",
        "",
    ] {
        assert!(!folder.is(dir), "{dir:?}");
    }
    assert!(!TestFolder::test_dir(&test_dir).is(own));
}

#[test]
fn a_subfolder_the_daemon_made_is_deleted_with_what_it_put_there() {
    let dir = env::temp_dir().join(format!("x4-live-support-{:016x}", random_seed()));
    let data = LocalData::unwritten(
        &dir,
        Some("4x4-live-new-1"),
        "torrent",
        &["empty.bin", "data.bin"],
    )
    .unwrap();
    // As the daemon does when the torrent starts: it makes the folders and the empty file.
    fs::create_dir_all(dir.join("4x4-live-new-1/torrent")).unwrap();
    fs::write(dir.join("4x4-live-new-1/torrent/empty.bin"), b"").unwrap();
    assert!(!data.files_are_gone());

    drop(data);

    assert!(!dir.exists(), "{} is left behind", dir.display());
}

#[test]
fn a_subfolder_the_daemon_never_made_leaves_nothing_to_delete() {
    let dir = env::temp_dir().join(format!("x4-live-support-{:016x}", random_seed()));
    let mut data =
        LocalData::unwritten(&dir, Some("4x4-live-new-2"), "torrent", &["a.bin"]).unwrap();
    assert!(data.files_are_gone());

    assert_eq!(data.delete(), Vec::<String>::new());

    assert!(data.is_clean());
    assert!(!dir.exists(), "{} is left behind", dir.display());
}

#[test]
fn unwritten_data_in_the_test_directory_is_never_deleted_here() {
    let dir = env::temp_dir().join(format!("x4-live-support-{:016x}", random_seed()));
    fs::create_dir(&dir).unwrap();
    let data = LocalData::unwritten(&dir, None, "torrent", &["a.bin"]).unwrap();
    // Whatever is there under the torrent's name, this run did not write it.
    fs::create_dir(dir.join("torrent")).unwrap();
    fs::write(dir.join("torrent/a.bin"), b"theirs").unwrap();
    assert!(data.is_clean());

    drop(data);

    assert_eq!(fs::read(dir.join("torrent/a.bin")).unwrap(), b"theirs");
    fs::remove_dir_all(&dir).unwrap();
}
