//! The folders on the server that the folder field of the add dialog offers.

use std::collections::HashMap;
use std::time::SystemTime;

use x4_rpc::DownloadDir;

use crate::model::Torrent;

/// The most folders [`folder_picks`] offers.
pub const MAX_FOLDER_PICKS: usize = 10;

/// The folders to offer for a torrent to add, the likeliest first:
/// 1. `session_dir`, the session's default folder;
/// 2. the `history` of folders torrents were added to, which is kept the newest first;
/// 3. the folders of the `torrents` in the list, the one with the newest
///    [`added`](Torrent::added) torrent first. Those whose torrents were added at an unknown time
///    come last; folders that tie are ordered by name.
///
/// Every folder is normalized as [`DownloadDir::new`] does, so `/a/` and `/a` are one; a folder
/// the daemon could not use is left out. Each folder comes once, where it comes first, and there
/// are at most [`MAX_FOLDER_PICKS`].
#[must_use]
pub fn folder_picks(
    session_dir: Option<&str>,
    history: &[String],
    torrents: &[Torrent],
) -> Vec<String> {
    let by_torrents = torrent_folders(torrents);
    let candidates = session_dir
        .into_iter()
        .chain(history.iter().map(String::as_str))
        .chain(by_torrents.iter().map(DownloadDir::as_str));
    let mut picks: Vec<String> = Vec::with_capacity(MAX_FOLDER_PICKS);
    for candidate in candidates {
        if picks.len() == MAX_FOLDER_PICKS {
            break;
        }
        let Ok(folder) = DownloadDir::new(candidate) else {
            continue;
        };
        if !picks.iter().any(|pick| pick == folder.as_str()) {
            picks.push(folder.as_str().to_owned());
        }
    }
    picks
}

/// The folders of `torrents`, each once, by their newest torrent and then by name.
fn torrent_folders(torrents: &[Torrent]) -> Vec<DownloadDir> {
    // Most torrents share a few folders, so each text is normalized once.
    let mut by_text: HashMap<&str, Option<SystemTime>> = HashMap::new();
    for torrent in torrents {
        let newest = by_text.entry(torrent.download_dir.as_str()).or_default();
        *newest = (*newest).max(torrent.added);
    }
    let mut by_folder: HashMap<DownloadDir, Option<SystemTime>> = HashMap::new();
    for (text, added) in by_text {
        if let Ok(folder) = DownloadDir::new(text) {
            let newest = by_folder.entry(folder).or_default();
            *newest = (*newest).max(added);
        }
    }
    let mut folders: Vec<(DownloadDir, Option<SystemTime>)> = by_folder.into_iter().collect();
    // `None` sorts below every time, so the folders of unknown age come last.
    folders.sort_unstable_by(|(a, a_added), (b, b_added)| {
        b_added
            .cmp(a_added)
            .then_with(|| a.as_str().cmp(b.as_str()))
    });
    folders.into_iter().map(|(folder, _)| folder).collect()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::*;
    use crate::test_support::torrent;

    /// A torrent in `folder`, added `at` seconds after the epoch, or at an unknown time.
    fn in_folder(name: &str, folder: &str, at: Option<u64>) -> Torrent {
        Torrent {
            download_dir: folder.to_owned(),
            added: at.map(|seconds| SystemTime::UNIX_EPOCH + Duration::from_secs(seconds)),
            ..torrent(name)
        }
    }

    fn strings(folders: &[&str]) -> Vec<String> {
        folders.iter().map(|&folder| folder.to_owned()).collect()
    }

    #[test]
    fn folder_picks_put_the_session_folder_history_and_torrents_in_order() {
        let history = strings(&["/data/films/", "/data/torrents", "relative", "/data//music"]);
        let torrents = [
            in_folder("a", "/data/torrents", Some(50)),
            in_folder("b", "/data/books", Some(30)),
            in_folder("c", "/srv/unknown", None),
            in_folder("d", "/data/books/", Some(90)),
            in_folder("e", "/data/films", Some(10)),
            in_folder("f", "", Some(99)),
            in_folder("g", "/data/old", Some(20)),
            in_folder("h", "/srv/also unknown", None),
            in_folder("i", "/data/../etc", Some(95)),
        ];

        let picks = folder_picks(Some("/data/torrents/"), &history, &torrents);

        assert_eq!(
            picks,
            [
                // The session's folder, then the history from the newest, then the torrents'
                // folders by their newest torrent, and those of unknown age by name; each once,
                // normalized, and only the folders the daemon could use.
                "/data/torrents",
                "/data/films",
                "/data/music",
                "/data/books",
                "/data/old",
                "/srv/also unknown",
                "/srv/unknown",
            ]
        );

        // Without the session, the history comes first.
        assert_eq!(
            folder_picks(None, &history, &torrents)
                .first()
                .map(String::as_str),
            Some("/data/films")
        );
        assert_eq!(folder_picks(None, &[], &[]), Vec::<String>::new());
        // A session folder the daemon could not use is left out.
        assert_eq!(folder_picks(Some(""), &[], &[]), Vec::<String>::new());
    }

    #[test]
    fn folder_picks_are_capped_at_ten() {
        assert_eq!(MAX_FOLDER_PICKS, 10);
        let history: Vec<String> = (0..7).map(|n| format!("/h/{n}")).collect();
        let torrents: Vec<Torrent> = (0..7)
            .map(|n| in_folder(&n.to_string(), &format!("/t/{n}"), Some(n)))
            .collect();

        let picks = folder_picks(Some("/s"), &history, &torrents);

        let mut expected = vec!["/s".to_owned()];
        expected.extend(history.iter().cloned());
        expected.extend(["/t/6".to_owned(), "/t/5".to_owned()]);
        assert_eq!(picks, expected);
    }

    #[test]
    fn folder_picks_do_not_depend_on_the_order_of_the_torrents() {
        let mut torrents = vec![
            in_folder("a", "/x", Some(5)),
            in_folder("b", "/y", Some(5)),
            in_folder("c", "/z", None),
            in_folder("d", "/w", None),
            in_folder("e", "/x", Some(1)),
        ];
        let expected = folder_picks(None, &[], &torrents);
        assert_eq!(expected, ["/x", "/y", "/w", "/z"], "ties by name");

        torrents.reverse();
        assert_eq!(folder_picks(None, &[], &torrents), expected);
        torrents.rotate_left(2);
        assert_eq!(folder_picks(None, &[], &torrents), expected);
    }
}
