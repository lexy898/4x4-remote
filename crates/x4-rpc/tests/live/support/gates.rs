//! The checks of an add request before it is sent: a test adds only this run's torrent, and only
//! into a folder of its own in the test directory. A request that fails one is never sent.

use x4_rpc::{AddSource, HashString, TorrentAdd};

use super::TestFolder;
use crate::metainfo::Metainfo;

/// The check of a first add: `request` must name `folder`, the torrent's own, and carry
/// `metainfo`, the one this run built, or a magnet link of its info hash.
pub(crate) fn check_first_add(request: &TorrentAdd, folder: &TestFolder, metainfo: &Metainfo) {
    assert_eq!(
        request.download_dir.as_ref(),
        Some(folder.remote()),
        "a test torrent downloads into its own folder only; nothing was sent"
    );
    let hash = &metainfo.info_hash;
    let carries_own = match &request.source {
        AddSource::Metainfo(bytes) => **bytes == *metainfo.bytes,
        AddSource::Magnet(link) => link.hash() == hash,
    };
    assert!(
        carries_own,
        "the request does not carry this run's torrent {hash}; nothing was sent"
    );
}

/// The check of a second add of the torrent `hash`, which `first` added into `own`: `folder` must
/// be in the same test directory, `request` must name it, and carry the same metainfo or a magnet
/// link of `hash`.
pub(crate) fn check_add_again(
    first: &TorrentAdd,
    hash: &HashString,
    own: &TestFolder,
    request: &TorrentAdd,
    folder: &TestFolder,
) {
    assert!(
        folder.test_dir == own.test_dir,
        "{folder:?} is not in the test directory of {own:?}; nothing was sent"
    );
    assert_eq!(
        request.download_dir.as_ref(),
        Some(folder.remote()),
        "a second add names a folder of the test directory only; nothing was sent"
    );
    let same_torrent = match (&request.source, &first.source) {
        (AddSource::Metainfo(again), AddSource::Metainfo(first)) => again == first,
        (AddSource::Magnet(link), _) => link.hash() == hash,
        (AddSource::Metainfo(_), AddSource::Magnet(_)) => false,
    };
    assert!(
        same_torrent,
        "a second add carries this run's torrent {hash} only; nothing was sent"
    );
}
