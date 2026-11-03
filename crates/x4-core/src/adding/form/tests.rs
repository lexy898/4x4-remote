use std::path::PathBuf;
use std::sync::Arc;

use x4_rpc::{AddSource, DownloadDir, DownloadDirError, HashString, MagnetError, MagnetLink};

use super::*;
use crate::files::{FileRow, FileTreeView, NonEmptyIndices, RowKind, WantedState};
use crate::limits::KbPerSecond;
use crate::metainfo::{MetaFile, ParsedTorrent};
use crate::model::Torrent;
use crate::snapshot::{ConnectionState, SessionSummary, SpeedLimit};
use crate::test_support::torrent;

/// The session's default folder, as the daemon reports it: with a trailing `/`.
const SESSION_DIR: &str = "/data/torrents/";
/// The same folder as the form normalizes it.
const DEFAULT_DIR: &str = "/data/torrents";
const PATH: &str = "/home/user/Downloads/t.torrent";

fn hash() -> HashString {
    HashString::parse(&"ab".repeat(20)).unwrap()
}

fn link() -> MagnetLink {
    MagnetLink::parse(&format!("magnet:?xt=urn:btih:{}&dn=Film", "cd".repeat(20))).unwrap()
}

/// Bytes standing for the content of the file; the form only passes them on.
fn content() -> Arc<[u8]> {
    Arc::from(&b"d4:infod4:name1:Tee"[..])
}

/// A `.torrent` named "T" with these files (path and length) at their indices, or with `None`
/// when the parse left no choice of files.
fn loaded(files: Option<&[(&str, u64)]>) -> LoadedItem {
    let files: Option<Vec<MetaFile>> = files.map(|files| {
        files
            .iter()
            .map(|&(path, length)| MetaFile {
                path: path.to_owned(),
                length,
            })
            .collect()
    });
    let tree = match &files {
        Some(files) => Ok(FileChoice::first_tree(hash(), files).unwrap()),
        None => Err(NoChoiceReason::Ambiguous),
    };
    LoadedItem::Torrent(LoadedTorrent {
        path: PathBuf::from(PATH),
        parsed: ParsedTorrent {
            name: "T".to_owned(),
            hash: hash(),
            total_size: files
                .as_ref()
                .map(|files| files.iter().map(|file| file.length).sum()),
            files,
        },
        bytes: content(),
        tree,
    })
}

fn form_of(files: &[(&str, u64)]) -> AddForm {
    AddForm::new(loaded(Some(files)), DEFAULT_DIR.to_owned(), true)
}

fn magnet_form(link: Option<MagnetLink>) -> AddForm {
    AddForm::new(LoadedItem::Magnet(link), DEFAULT_DIR.to_owned(), true)
}

fn session() -> SessionSummary {
    let off = SpeedLimit {
        kbps: KbPerSecond(0),
        enabled: false,
    };
    SessionSummary {
        version: "4.1.4".to_owned(),
        download_dir: SESSION_DIR.to_owned(),
        alt_speed_enabled: false,
        speed_limit_down: off,
        speed_limit_up: off,
        alt_speed_down_kbps: KbPerSecond(0),
        alt_speed_up_kbps: KbPerSecond(0),
        start_added_torrents: true,
    }
}

fn snapshot(torrents: Vec<Torrent>) -> Snapshot {
    Snapshot {
        torrents,
        listed: true,
        session: Some(session()),
        free_space_bytes: None,
        rate_down: 0,
        rate_up: 0,
        connection: ConnectionState::Connected,
        stale: false,
    }
}

fn check(form: &AddForm) -> AddCheck {
    form.check(&snapshot(vec![torrent("other")]), true)
}

/// The request the form sends, which it must be ready to.
fn ready(form: &AddForm) -> TorrentAdd {
    match check(form) {
        AddCheck::Ready(add) => add,
        other => panic!("not ready: {other:?}"),
    }
}

fn unwanted(form: &AddForm) -> Vec<usize> {
    ready(form).files_unwanted
}

fn key(path: &str) -> NodeKey {
    NodeKey::new(path)
}

/// Every row of the form's tree, with every folder open.
fn rows(form: &AddForm) -> Vec<FileRow> {
    let tree = form.tree().unwrap();
    let mut view = FileTreeView::new(tree);
    loop {
        let closed: Vec<NodeKey> = view
            .rows(tree)
            .into_iter()
            .filter(|row| matches!(row.kind, RowKind::Folder { expanded: false }))
            .map(|row| row.key)
            .collect();
        if closed.is_empty() {
            return view.rows(tree);
        }
        for folder in &closed {
            view.toggle_expanded(tree, folder);
        }
    }
}

/// The wanted state of every row, by path.
fn wanted_rows(form: &AddForm) -> Vec<(String, WantedState)> {
    rows(form)
        .into_iter()
        .map(|row| (row.key.as_str().to_owned(), row.wanted))
        .collect()
}

#[test]
fn a_torrent_form_starts_with_every_file_wanted() {
    let form = form_of(&[
        ("T/a.mkv", 10),
        ("T/sub/b.mkv", 20),
        ("T/.pad/2", 3),
        ("T/sub/c.txt", 1),
    ]);

    let tree = form.tree().unwrap();
    assert_eq!(tree.hash(), &hash(), "the tree is the torrent's own");
    let shown = wanted_rows(&form);
    assert_eq!(
        shown
            .iter()
            .map(|(path, _)| path.as_str())
            .collect::<Vec<_>>(),
        ["T", "T/sub", "T/sub/b.mkv", "T/sub/c.txt", "T/a.mkv"],
        "the padding file is hidden"
    );
    assert!(shown.iter().all(|(_, wanted)| *wanted == WantedState::All));
    // The folders reach the files at the daemon's indices, padding included.
    assert_eq!(
        tree.indices(&key("T")).map(NonEmptyIndices::into_vec),
        Some(vec![0, 1, 2, 3])
    );
    assert_eq!(
        tree.indices(&key("T/sub")).map(NonEmptyIndices::into_vec),
        Some(vec![1, 3])
    );
    // Nothing is downloaded yet.
    assert!(rows(&form).iter().all(|row| row.percent == 0));

    assert_eq!(
        check(&form),
        AddCheck::Ready(TorrentAdd {
            source: AddSource::Metainfo(content()),
            download_dir: None,
            paused: false,
            files_unwanted: Vec::new(),
            priority_high: Vec::new(),
            priority_low: Vec::new(),
            sequential_download: None,
        })
    );
    assert_eq!(form.name(), Some("T"));
    assert_eq!(form.path(), Some(Path::new(PATH)));
    assert!(!form.is_magnet());
    assert_eq!(form.choice_unavailable(), None, "there is a choice");
}

#[test]
fn unchecking_a_folder_unwants_its_pad_files_too() {
    let mut form = form_of(&[
        ("T/keep.mkv", 10),
        ("T/sub/a.mkv", 20),
        ("T/sub/_____padding_file_0", 4),
        ("T/sub/b.mkv", 20),
        ("T/.pad/4", 8),
    ]);

    form.toggle_wanted(&key("T/sub"));

    assert_eq!(
        unwanted(&form),
        [1, 2, 3],
        "the padding file of the folder too"
    );
    assert_eq!(
        wanted_rows(&form),
        [
            ("T".to_owned(), WantedState::Mixed),
            ("T/sub".to_owned(), WantedState::None),
            ("T/sub/a.mkv".to_owned(), WantedState::None),
            ("T/sub/b.mkv".to_owned(), WantedState::None),
            ("T/keep.mkv".to_owned(), WantedState::All),
        ],
        "the checkboxes show the choice"
    );

    // Checked again, its files and its padding come back.
    form.toggle_wanted(&key("T/sub"));
    assert_eq!(unwanted(&form), Vec::<usize>::new());

    // A file alone leaves the padding of its folder as it is: `.pad` belongs to the top folder.
    form.toggle_wanted(&key("T/keep.mkv"));
    assert_eq!(unwanted(&form), [0]);
    // The mixed top folder becomes wanted entirely, padding included.
    form.toggle_wanted(&key("T"));
    assert_eq!(unwanted(&form), Vec::<usize>::new());

    // A hidden padding file, a folder of padding only, or a key the tree does not have changes
    // nothing.
    for hidden in [
        "T/.pad/4",
        "T/.pad",
        "T/sub/_____padding_file_0",
        "T/none",
        "",
    ] {
        form.toggle_wanted(&key(hidden));
        assert_eq!(unwanted(&form), Vec::<usize>::new(), "{hidden:?}");
    }
}

#[test]
fn every_file_unwanted_blocks_add() {
    // The shown files unchecked one by one: the padding file of the top folder is still wanted,
    // but nothing would be downloaded.
    let mut form = form_of(&[("T/a", 1), ("T/b", 1), ("T/.pad/2", 1)]);
    form.toggle_wanted(&key("T/a"));
    assert_eq!(unwanted(&form), [0]);
    form.toggle_wanted(&key("T/b"));
    assert_eq!(check(&form), AddCheck::NothingWanted);

    // The whole torrent unchecked, padding included.
    let mut form = form_of(&[("T/a", 1), ("T/b", 1), ("T/.pad/2", 1)]);
    form.toggle_wanted(&key("T"));
    assert_eq!(check(&form), AddCheck::NothingWanted);
    form.toggle_wanted(&key("T/b"));
    assert_eq!(unwanted(&form), [0, 2]);

    // A torrent of padding files only shows nothing to download.
    let only_padding = form_of(&[("T/_____padding_file_0", 5), ("T/.pad/1", 5)]);
    assert!(only_padding.tree().unwrap().is_empty());
    assert_eq!(check(&only_padding), AddCheck::NothingWanted);
}

#[test]
fn every_choice_sends_sorted_indices_of_its_own_files_and_never_all_of_them() {
    let files = [
        ("T/a.mkv", 1),
        ("T/s/_____padding_file_1", 1),
        ("T/s/b.mkv", 1),
        ("T/.pad/3", 1),
        ("T/s/c.mkv", 1),
        ("T/s/t/d.mkv", 1),
    ];
    let shown = ["T/a.mkv", "T/s/b.mkv", "T/s/c.mkv", "T/s/t/d.mkv"];
    let nodes = [
        "T",
        "T/s",
        "T/s/t",
        "T/a.mkv",
        "T/s/b.mkv",
        "T/s/c.mkv",
        "T/s/t/d.mkv",
    ];

    // Every sequence of three clicks on the nodes.
    for first in nodes {
        for second in nodes {
            for third in nodes {
                let mut form = form_of(&files);
                for node in [first, second, third] {
                    form.toggle_wanted(&key(node));
                }
                let any_shown_wanted = wanted_rows(&form).iter().any(|(path, wanted)| {
                    shown.contains(&path.as_str()) && *wanted == WantedState::All
                });
                let clicks = [first, second, third];
                match check(&form) {
                    AddCheck::Ready(add) => {
                        assert!(any_shown_wanted, "{clicks:?}");
                        let unwanted = add.files_unwanted;
                        assert!(unwanted.is_sorted(), "{clicks:?}: {unwanted:?}");
                        assert!(
                            unwanted.windows(2).all(|pair| pair.first() != pair.get(1)),
                            "{clicks:?}: {unwanted:?}"
                        );
                        assert!(
                            unwanted.iter().all(|&index| index < files.len()),
                            "{clicks:?}"
                        );
                        assert!(unwanted.len() < files.len(), "{clicks:?}");
                    }
                    AddCheck::NothingWanted => assert!(!any_shown_wanted, "{clicks:?}"),
                    other => panic!("{clicks:?}: {other:?}"),
                }
            }
        }
    }
}

#[test]
fn an_ambiguous_torrent_offers_all_files_only() {
    let mut form = AddForm::new(loaded(None), DEFAULT_DIR.to_owned(), true);

    assert!(form.tree().is_none(), "no choice of files");
    assert_eq!(form.choice_unavailable(), Some(NoChoiceReason::Ambiguous));
    form.toggle_wanted(&key("T"));
    form.toggle_wanted(&key(""));

    let add = ready(&form);
    assert_eq!(add.files_unwanted, Vec::<usize>::new());
    assert_eq!(add.source, AddSource::Metainfo(content()));
    assert!(add.priority_high.is_empty() && add.priority_low.is_empty());
}

#[test]
fn a_loaded_torrent_the_daemon_could_number_otherwise_has_no_choice() {
    // `name` before `files`: keys out of order, which the daemon could read otherwise.
    let mut bytes = b"d4:infod4:name3:Odd5:filesld6:lengthi1e4:pathl1:aeed6:lengthi2e4:pathl1:beee\
        12:piece lengthi16384e6:pieces20:"
        .to_vec();
    bytes.extend([0xab; 20]);
    bytes.extend(b"ee");
    let bytes: Arc<[u8]> = Arc::from(bytes);
    assert_eq!(parse_torrent(&bytes).unwrap().files, None);

    let form = AddForm::new(
        LoadedItem::torrent(PathBuf::from(PATH), Arc::clone(&bytes)).unwrap(),
        DEFAULT_DIR.to_owned(),
        true,
    );

    assert_eq!(form.choice_unavailable(), Some(NoChoiceReason::Ambiguous));
    assert!(form.tree().is_none());
    assert_eq!(ready(&form).files_unwanted, Vec::<usize>::new());
}

#[test]
fn a_single_file_torrent_without_a_name_still_shows_its_file() {
    // A single file named "/" has an empty path once sanitized.
    let mut form = form_of(&[("", 5)]);

    let shown = wanted_rows(&form);
    assert_eq!(shown, [(String::new(), WantedState::All)]);
    assert_eq!(unwanted(&form), Vec::<usize>::new());

    form.toggle_wanted(&key(""));
    assert_eq!(check(&form), AddCheck::NothingWanted);
    form.toggle_wanted(&key(""));
    assert_eq!(unwanted(&form), Vec::<usize>::new());
}

#[test]
fn a_magnet_form_never_sends_unwanted_files() {
    let mut form = magnet_form(Some(link()));

    assert!(form.tree().is_none(), "the files come with the metadata");
    form.toggle_wanted(&key(""));
    form.toggle_wanted(&key("Film"));
    let add = ready(&form);
    assert_eq!(add.source, AddSource::Magnet(link()));
    assert_eq!(add.files_unwanted, Vec::<usize>::new());
    assert!(add.priority_high.is_empty() && add.priority_low.is_empty());
    assert!(form.is_magnet());
    assert_eq!(
        form.choice_unavailable(),
        None,
        "the files come with the metadata"
    );
    assert_eq!(form.name(), Some("Film"));
    assert_eq!(form.path(), None);

    // The empty magnet form, filled in.
    let mut empty = magnet_form(None);
    assert_eq!(check(&empty), AddCheck::BadLink(MagnetError::Empty));
    assert_eq!(empty.name(), None);
    empty.set_link(&format!("  {}\n", link().as_str()));
    assert_eq!(ready(&empty).source, AddSource::Magnet(link()));
    assert_eq!(ready(&empty).files_unwanted, Vec::<usize>::new());
    empty.set_link("");
    assert_eq!(check(&empty), AddCheck::BadLink(MagnetError::Empty));

    // A `.torrent`'s form has no link to set.
    let mut torrent_form = form_of(&[("T/a", 1)]);
    torrent_form.set_link(link().as_str());
    assert_eq!(ready(&torrent_form).source, AddSource::Metainfo(content()));
}

#[test]
fn the_tree_can_be_kept_while_the_form_changes() {
    let mut form = form_of(&[("T/a", 1), ("T/b", 1)]);
    let drawn = Arc::clone(form.tree().unwrap());

    form.toggle_wanted(&key("T/a"));

    // The tree drawn stays as it was drawn; the form has a new one with the choice.
    let now = form.tree().unwrap();
    assert!(!Arc::ptr_eq(&drawn, now));
    assert_eq!(
        drawn.wanted_toggle(&key("T/a")).map(|(_, wanted)| wanted),
        Some(false),
        "wanted when it was drawn"
    );
    assert_eq!(
        now.wanted_toggle(&key("T/a")).map(|(_, wanted)| wanted),
        Some(true)
    );
    // A key that changes nothing keeps the tree.
    let before = Arc::clone(form.tree().unwrap());
    form.toggle_wanted(&key("T/none"));
    assert!(Arc::ptr_eq(&before, form.tree().unwrap()));
}

#[test]
fn a_torrent_already_in_the_list_is_named_not_sent() {
    let present = Torrent {
        hash: hash(),
        ..torrent("already here")
    };
    let listed = snapshot(vec![torrent("other"), present]);

    let form = form_of(&[("T/a", 1)]);
    assert_eq!(form.check(&listed, true), AddCheck::AlreadyInList(hash()));

    // Whatever else is wrong with the form: nothing is sent for it.
    let mut form = form_of(&[("T/a", 1)]);
    form.toggle_wanted(&key("T"));
    form.set_folder("relative".to_owned());
    assert_eq!(form.check(&listed, true), AddCheck::AlreadyInList(hash()));
    let ambiguous = AddForm::new(loaded(None), DEFAULT_DIR.to_owned(), true);
    assert_eq!(
        ambiguous.check(&listed, true),
        AddCheck::AlreadyInList(hash())
    );

    // A magnet link by its hash.
    let magnet_present = Torrent {
        hash: link().hash().clone(),
        ..torrent("magnet")
    };
    let listed = snapshot(vec![magnet_present]);
    assert_eq!(
        magnet_form(Some(link())).check(&listed, true),
        AddCheck::AlreadyInList(link().hash().clone())
    );
    assert!(matches!(
        form_of(&[("T/a", 1)]).check(&listed, true),
        AddCheck::Ready(_)
    ));
}

#[test]
fn a_bad_folder_or_link_blocks_add() {
    let mut form = form_of(&[("T/a", 1)]);
    for (folder, error) in [
        ("", DownloadDirError::Empty),
        ("\n", DownloadDirError::Empty),
        ("data/torrents", DownloadDirError::NotAbsolute),
        ("~/torrents", DownloadDirError::NotAbsolute),
        ("/data/../etc", DownloadDirError::DotPart),
        (
            "/data/a\tb",
            DownloadDirError::ControlCharacter { offset: 7 },
        ),
    ] {
        form.set_folder(folder.to_owned());
        assert_eq!(check(&form), AddCheck::BadFolder(error), "{folder:?}");
        assert_eq!(form.folder(), Err(error), "{folder:?}");
    }
    form.set_folder("/data/films/".to_owned());
    assert_eq!(form.folder(), DownloadDir::new("/data/films"));
    assert!(matches!(check(&form), AddCheck::Ready(_)));

    // The folder is checked before the files.
    form.toggle_wanted(&key("T"));
    assert_eq!(check(&form), AddCheck::NothingWanted);
    form.set_folder("films".to_owned());
    assert_eq!(
        check(&form),
        AddCheck::BadFolder(DownloadDirError::NotAbsolute)
    );

    // The link is checked before the folder.
    let mut magnet = magnet_form(None);
    magnet.set_folder("films".to_owned());
    for (text, error) in [
        (
            "https://tracker.example/file.torrent",
            MagnetError::NotMagnet,
        ),
        ("/home/user/t.torrent", MagnetError::NotMagnet),
        (&"cd".repeat(20), MagnetError::BareHash),
        ("magnet:?dn=Film", MagnetError::NoHash),
    ] {
        magnet.set_link(text);
        assert_eq!(check(&magnet), AddCheck::BadLink(error), "{text:?}");
    }
    magnet.set_link(link().as_str());
    assert_eq!(
        check(&magnet),
        AddCheck::BadFolder(DownloadDirError::NotAbsolute)
    );
    magnet.set_folder(DEFAULT_DIR.to_owned());
    assert!(matches!(check(&magnet), AddCheck::Ready(_)));
}

#[test]
fn nothing_is_ready_while_the_daemon_is_not_connected() {
    let mut form = form_of(&[("T/a", 1)]);
    let present = snapshot(vec![Torrent {
        hash: hash(),
        ..torrent("already here")
    }]);
    assert_eq!(
        form.check(&snapshot(Vec::new()), false),
        AddCheck::NotConnected
    );
    assert_eq!(form.check(&present, false), AddCheck::NotConnected);
    form.toggle_wanted(&key("T"));
    form.set_folder(String::new());
    assert_eq!(form.check(&present, false), AddCheck::NotConnected);
    assert_eq!(
        magnet_form(None).check(&snapshot(Vec::new()), false),
        AddCheck::NotConnected
    );
}

#[test]
fn the_request_names_a_folder_only_when_it_is_not_the_default() {
    let mut form = form_of(&[("T/a", 1)]);

    // The session's default folder, however it is written, is left to the daemon.
    for folder in [
        DEFAULT_DIR,
        SESSION_DIR,
        "//data//torrents//",
        "/data/torrents\n",
    ] {
        form.set_folder(folder.to_owned());
        assert_eq!(ready(&form).download_dir, None, "{folder:?}");
    }
    // Another one is sent normalized.
    form.set_folder("/data//films/".to_owned());
    assert_eq!(
        ready(&form).download_dir,
        DownloadDir::new("/data/films").ok()
    );
    // Case matters on the server.
    form.set_folder("/data/Torrents".to_owned());
    assert_eq!(
        ready(&form).download_dir,
        DownloadDir::new("/data/Torrents").ok()
    );
    // While the session is not known, the folder is always named.
    form.set_folder(DEFAULT_DIR.to_owned());
    let unknown = Snapshot {
        session: None,
        ..snapshot(Vec::new())
    };
    let AddCheck::Ready(add) = form.check(&unknown, true) else {
        panic!("not ready");
    };
    assert_eq!(add.download_dir, DownloadDir::new(DEFAULT_DIR).ok());
}

#[test]
fn start_immediately_decides_whether_the_torrent_is_added_paused() {
    let mut form = AddForm::new(loaded(Some(&[("T/a", 1)])), DEFAULT_DIR.to_owned(), false);
    assert!(!form.start());
    assert!(ready(&form).paused);

    form.set_start(true);
    assert!(form.start());
    assert!(!ready(&form).paused);

    let mut magnet = AddForm::new(
        LoadedItem::Magnet(Some(link())),
        DEFAULT_DIR.to_owned(),
        false,
    );
    assert!(ready(&magnet).paused);
    magnet.set_start(true);
    assert!(!ready(&magnet).paused);
}

/// A real `.torrent`: one file "a.bin" of 5 bytes in one piece, announced to `announce`.
fn single_file_torrent(announce: &str) -> Arc<[u8]> {
    let mut bytes = format!(
        "d8:announce{}:{announce}4:infod6:lengthi5e4:name5:a.bin12:piece lengthi16384e6:pieces20:",
        announce.len()
    )
    .into_bytes();
    bytes.extend([0xab; 20]);
    bytes.extend(b"ee");
    Arc::from(bytes)
}

#[test]
fn a_loaded_torrent_holds_the_parse_of_its_own_bytes() {
    let bytes = single_file_torrent("http://t.test/announce");

    let item = LoadedItem::torrent(PathBuf::from(PATH), Arc::clone(&bytes)).unwrap();

    let LoadedItem::Torrent(torrent) = &item else {
        panic!("a torrent");
    };
    assert_eq!(*torrent.parsed(), parse_torrent(&bytes).unwrap());
    assert_eq!(torrent.bytes(), &*bytes);
    assert_eq!(torrent.path(), Path::new(PATH));
    let form = AddForm::new(item, DEFAULT_DIR.to_owned(), true);
    assert_eq!(ready(&form).source, AddSource::Metainfo(Arc::clone(&bytes)));
    assert_eq!(form.name(), Some("a.bin"));
    assert_eq!(wanted_rows(&form), [("a.bin".to_owned(), WantedState::All)]);

    assert_eq!(
        LoadedItem::torrent(PathBuf::from(PATH), Arc::from(&b"not bencode"[..])).err(),
        Some(MetainfoError::NotBencode { offset: 0 })
    );
}

#[test]
fn the_request_shares_the_bytes_that_were_parsed() {
    let bytes = single_file_torrent("http://t.test/announce");
    let item = LoadedItem::torrent(PathBuf::from(PATH), Arc::clone(&bytes)).unwrap();
    let form = AddForm::new(item, DEFAULT_DIR.to_owned(), true);

    // The very bytes, not a copy, however often the form is checked.
    for _ in 0..3 {
        let AddSource::Metainfo(sent) = ready(&form).source else {
            panic!("a .torrent goes as its content");
        };
        assert!(Arc::ptr_eq(&sent, &bytes));
    }
}

/// A real multi-file `.torrent` named `name`, its files given by their path components, each one
/// byte long.
fn folder_torrent(name: &str, paths: &[Vec<String>]) -> Arc<[u8]> {
    let mut bytes = b"d4:infod5:filesl".to_vec();
    for path in paths {
        bytes.extend(b"d6:lengthi1e4:pathl");
        for part in path {
            bytes.extend(format!("{}:{part}", part.len()).into_bytes());
        }
        bytes.extend(b"ee");
    }
    let pieces = paths.len().div_ceil(16_384);
    bytes.extend(
        format!(
            "e4:name{}:{name}12:piece lengthi16384e6:pieces{}:",
            name.len(),
            20 * pieces
        )
        .into_bytes(),
    );
    bytes.extend(std::iter::repeat_n(0xab, 20 * pieces));
    bytes.extend(b"ee");
    Arc::from(bytes)
}

/// The peak memory of this process since [`restart_peak`], in KiB, where Linux says it; the
/// tests run one per process.
fn peak_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmHWM:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

/// Starts the peak of [`peak_kib`] again from the memory in use now, so that what was built
/// before does not hide what comes next; `false` where Linux does not allow it.
fn restart_peak() -> bool {
    std::fs::write("/proc/self/clear_refs", "5").is_ok()
}

/// Loads `bytes` and asserts that the form offers them with all their files and no choice,
/// because the list is too large, within bounded time and memory: little beyond what parsing them
/// takes.
fn assert_offered_with_all_files(bytes: &Arc<[u8]>) {
    // What the parse alone takes, so that the bound below is on what the form adds to it.
    let measured = restart_peak();
    drop(parse_torrent(bytes).unwrap());
    let peak_parsed = peak_kib();
    let measured = measured && restart_peak();
    let start = std::time::Instant::now();

    let item = LoadedItem::torrent(PathBuf::from(PATH), Arc::clone(bytes)).unwrap();
    let form = AddForm::new(item, DEFAULT_DIR.to_owned(), true);
    let add = ready(&form);

    let elapsed = start.elapsed();
    assert_eq!(form.choice_unavailable(), Some(NoChoiceReason::TooLarge));
    assert!(form.tree().is_none());
    assert_eq!(add.files_unwanted, Vec::<usize>::new());
    let AddSource::Metainfo(sent) = add.source else {
        panic!("a .torrent goes as its content");
    };
    assert!(Arc::ptr_eq(&sent, bytes));
    // The bound only says it ends: half a million files take some 3 s in a debug build.
    assert!(elapsed < std::time::Duration::from_secs(60), "{elapsed:?}");
    if let (true, Some(parsed), Some(after)) = (measured, peak_parsed, peak_kib()) {
        // At most the budget of a tree that was given up, 32 MiB. A tree of 500 000 files would
        // take some 250 MiB, and copying their entries for it alone 40.
        assert!(
            after.saturating_sub(parsed) < 40 * 1024,
            "{parsed} KiB after the parse, {after} KiB after the form"
        );
    }
}

#[test]
fn a_torrent_with_a_deep_path_is_offered_with_all_files() {
    // A single file whose name holds 20 000 folders, one in the other: about 40 KB, which the
    // parser takes, and whose tree would take some 400 MiB.
    let name = "a/".repeat(20_000) + "f";
    let mut bytes = format!(
        "d4:infod6:lengthi5e4:name{}:{name}12:piece lengthi16384e6:pieces20:",
        name.len()
    )
    .into_bytes();
    bytes.extend([0xab; 20]);
    bytes.extend(b"ee");

    assert_offered_with_all_files(&Arc::from(bytes));
}

#[test]
fn a_torrent_of_many_deep_chains_is_offered_with_all_files() {
    // Eight paths of 2 050 parts that part at the top: each is 6 KB, its chain some 4 MiB of
    // keys.
    let paths: Vec<Vec<String>> = (0..8)
        .map(|chain| {
            std::iter::once(format!("x{chain}"))
                .chain(std::iter::repeat_n("a".to_owned(), 2_048))
                .chain(std::iter::once("f".to_owned()))
                .collect()
        })
        .collect();

    assert_offered_with_all_files(&folder_torrent("T", &paths));

    // Three such chains fit, and give a choice.
    let fewer = folder_torrent("T", paths.get(..3).unwrap());
    let form = AddForm::new(
        LoadedItem::torrent(PathBuf::from(PATH), fewer).unwrap(),
        DEFAULT_DIR.to_owned(),
        true,
    );
    assert!(!form.tree().unwrap().is_too_large());
    assert_eq!(form.choice_unavailable(), None);
}

#[test]
fn a_torrent_of_500_000_tiny_files_is_offered_with_all_files() {
    // 14 MB, within the 16 MiB a file may have.
    let paths: Vec<Vec<String>> = (0..500_000).map(|n| vec![n.to_string()]).collect();
    let bytes = folder_torrent("T", &paths);
    drop(paths);

    assert_offered_with_all_files(&bytes);
}

#[test]
fn a_torrent_of_80_000_files_with_long_paths_is_offered_with_all_files() {
    // Paths of 200 bytes: a folder name of 100 and a file name of 99.
    let name = "N".repeat(100);
    let paths: Vec<Vec<String>> = (0..80_000)
        .map(|n| vec![format!("{n:05}{}", "f".repeat(94))])
        .collect();
    let bytes = folder_torrent(&name, &paths);
    drop(paths);
    let parsed = parse_torrent(&bytes).unwrap();
    assert_eq!(
        parsed
            .files
            .as_ref()
            .and_then(|files| files.first())
            .map(|file| file.path.len()),
        Some(200)
    );

    assert_offered_with_all_files(&bytes);
}

#[test]
fn up_to_50_000_files_can_be_chosen_from() {
    let paths: Vec<Vec<String>> = (0..=50_000).map(|n| vec![n.to_string()]).collect();

    let most = folder_torrent("T", paths.get(..50_000).unwrap());
    let mut form = AddForm::new(
        LoadedItem::torrent(PathBuf::from(PATH), most).unwrap(),
        DEFAULT_DIR.to_owned(),
        true,
    );
    assert_eq!(form.choice_unavailable(), None);
    assert!(!form.tree().unwrap().is_too_large());
    form.toggle_wanted(&key("T/7"));
    assert_eq!(unwanted(&form), [7]);

    assert_offered_with_all_files(&folder_torrent("T", &paths));
}

#[test]
fn a_torrent_of_50_000_paths_the_tree_shows_alike_loads_in_bounded_time() {
    // The parser keeps "a//a" and "a/a" apart, and the tree shows both as "a/a": these are 50 000
    // copies of one path 17 folders deep, each under a key with its own suffix. Finding each
    // suffix by trying them all from the first took minutes, and as long again for each click.
    let paths: Vec<Vec<String>> = (0..50_000_u32)
        .map(|n| {
            let path: String = std::iter::once("a")
                .chain((0..16).map(|bit| if (n >> bit) & 1 == 1 { "//a" } else { "/a" }))
                .collect();
            vec![path]
        })
        .collect();
    let bytes = folder_torrent("T", &paths);
    let shown = format!("T/{}", ["a"; 17].join("/"));
    let start = std::time::Instant::now();

    let mut form = AddForm::new(
        LoadedItem::torrent(PathBuf::from(PATH), bytes).unwrap(),
        DEFAULT_DIR.to_owned(),
        true,
    );

    let loaded = start.elapsed();
    let tree = form.tree().unwrap();
    assert!(!tree.is_too_large());
    assert_eq!(
        tree.indices(&key(&shown)).map(NonEmptyIndices::into_vec),
        Some(vec![0])
    );
    let start = std::time::Instant::now();
    form.toggle_wanted(&key(&format!("{shown}\u{0}7")));
    let clicked = start.elapsed();
    assert_eq!(unwanted(&form), [7]);
    // Some 100 ms each in a release build; the bounds only say they end.
    assert!(loaded < std::time::Duration::from_secs(10), "{loaded:?}");
    assert!(clicked < std::time::Duration::from_secs(10), "{clicked:?}");
}

#[test]
fn a_real_shaped_torrent_loads_with_its_tree() {
    // 5 000 episodes, four levels deep with the torrent's folder, with long names.
    let paths: Vec<Vec<String>> = (0..5_000)
        .map(|n| {
            vec![
                format!("Season {:02}", n / 250),
                format!("Disc {}", n / 50 % 5),
                format!("Episode {n:04} - A Rather Long Title Of The Episode (1080p).mkv"),
            ]
        })
        .collect();
    let bytes = folder_torrent("A Long Running Show", &paths);

    let mut form = AddForm::new(
        LoadedItem::torrent(PathBuf::from(PATH), bytes).unwrap(),
        DEFAULT_DIR.to_owned(),
        true,
    );

    let tree = form.tree().unwrap();
    assert!(!tree.is_too_large());
    assert_eq!(
        tree.indices(&key("A Long Running Show"))
            .map(NonEmptyIndices::into_vec),
        Some((0..5_000).collect())
    );
    form.toggle_wanted(&key("A Long Running Show/Season 19"));
    assert_eq!(unwanted(&form), (4_750..5_000).collect::<Vec<_>>());
}

#[test]
fn debug_shows_neither_the_content_nor_the_link() {
    let bytes = single_file_torrent("http://t.test/a?pk=SECRETSECRET");
    let item = LoadedItem::torrent(PathBuf::from(PATH), bytes).unwrap();
    let secret_link = MagnetLink::parse(&format!(
        "magnet:?xt=urn:btih:{}&tr=http://t.test/a?pk=SECRETSECRET",
        "cd".repeat(20)
    ))
    .unwrap();

    let shown = [
        format!("{item:?}"),
        format!(
            "{:?}",
            AddForm::new(item.clone(), DEFAULT_DIR.to_owned(), true)
        ),
        format!("{:?}", LoadedItem::Magnet(Some(secret_link.clone()))),
        format!("{:?}", magnet_form(Some(secret_link))),
    ];
    for text in shown {
        assert!(!text.contains("SECRET"), "{text}");
        assert!(!text.contains("100, 56"), "no bytes: {text}");
        assert!(text.len() < 300, "{text}");
    }
}
