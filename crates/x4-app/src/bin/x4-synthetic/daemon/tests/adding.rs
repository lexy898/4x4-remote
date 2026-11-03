//! Adding torrents to the synthetic daemon, and asking it for the free space of a folder: what
//! `torrent_add` makes of the content of a `.torrent` file and of a magnet link, what it refuses
//! with which words, and which folders `free_space` knows.

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use x4_core::parse_torrent;
use x4_rpc::{AddOutcome, AddSource, DownloadDir, MagnetLink, Status, TorrentAdd};

use super::support::*;
use super::*;

const KIB: u64 = 1024;
const MIB: u64 = 1024 * KIB;
/// The piece length of the test torrents.
const PIECE: u64 = 16 * KIB;
/// The hash of the magnet links of these tests: no torrent of the daemon has it.
const HASH: &str = "00112233445566778899aabbccddeeff00112233";

/// A bencoded string.
fn string(value: &[u8]) -> Vec<u8> {
    let mut out = format!("{}:", value.len()).into_bytes();
    out.extend_from_slice(value);
    out
}

/// An entry of `info.files`: a path, a length, and whether it is a padding file (BEP 47).
type Entry<'a> = (&'a [&'a str], u64, bool);

fn file_entry((path, length, pad): &Entry<'_>) -> Vec<u8> {
    let mut out = b"d".to_vec();
    if *pad {
        out.extend(string(b"attr"));
        out.extend(string(b"p"));
    }
    out.extend(string(b"length"));
    out.extend(format!("i{length}e").bytes());
    out.extend(string(b"path"));
    out.push(b'l');
    for part in *path {
        out.extend(string(part.as_bytes()));
    }
    out.extend(b"ee");
    out
}

/// The `pieces` entry of a torrent of `total` bytes: a hash for each piece.
fn pieces(total: u64) -> Vec<u8> {
    let count = usize::try_from(total.div_ceil(PIECE)).expect("fits");
    let mut out = string(b"pieces");
    out.extend(string(&vec![0xab; 20 * count]));
    out
}

/// A `.torrent` file of a folder `name` with `entries`. With `unsorted` its `info` keys are out
/// of order, so that the reader cannot tell which file has which index and gives no files.
fn folder_torrent(name: &str, entries: &[Entry<'_>], unsorted: bool) -> Vec<u8> {
    let total = entries.iter().map(|&(_, length, _)| length).sum();
    let mut files = string(b"files");
    files.push(b'l');
    for entry in entries {
        files.extend(file_entry(entry));
    }
    files.push(b'e');
    let mut named = string(b"name");
    named.extend(string(name.as_bytes()));
    let mut piece_length = string(b"piece length");
    piece_length.extend(format!("i{PIECE}e").bytes());
    let parts = if unsorted {
        [named, files, piece_length, pieces(total)]
    } else {
        [files, named, piece_length, pieces(total)]
    };
    let mut out = b"d4:infod".to_vec();
    out.extend(parts.concat());
    out.extend(b"ee");
    out
}

/// A `.torrent` file of one file `name` of `length` bytes.
fn single_torrent(name: &str, length: u64) -> Vec<u8> {
    let mut out = b"d4:infod".to_vec();
    out.extend(string(b"length"));
    out.extend(format!("i{length}e").bytes());
    out.extend(string(b"name"));
    out.extend(string(name.as_bytes()));
    out.extend(string(b"piece length"));
    out.extend(format!("i{PIECE}e").bytes());
    out.extend(pieces(length));
    out.extend(b"ee");
    out
}

/// A folder of four files, the second of them padding: the files the tests add and look at.
fn show() -> Vec<u8> {
    folder_torrent(
        "Show",
        &[
            (&["a.mkv"], 3 * MIB, false),
            (&[".pad", "1"], 64 * KIB, true),
            (&["b.mkv"], 2 * MIB, false),
            (&["Sample", "s.mkv"], MIB, false),
        ],
        false,
    )
}

fn metainfo(content: &[u8]) -> AddSource {
    AddSource::Metainfo(Arc::from(content))
}

fn magnet(text: &str) -> AddSource {
    AddSource::Magnet(MagnetLink::parse(text).expect("a valid magnet link"))
}

fn add(source: AddSource) -> TorrentAdd {
    TorrentAdd {
        source,
        download_dir: None,
        paused: false,
        files_unwanted: Vec::new(),
        priority_high: Vec::new(),
        priority_low: Vec::new(),
        sequential_download: None,
    }
}

fn dir(path: &str) -> DownloadDir {
    DownloadDir::new(path).expect("a valid folder")
}

/// Adds a torrent that must be new, and returns what the daemon says it added.
fn add_new(client: &mut Client<SyntheticDaemon>, add: &TorrentAdd) -> x4_rpc::AddedTorrent {
    match client.torrent_add(add).expect("the daemon adds it") {
        AddOutcome::Added(added) => added,
        AddOutcome::Duplicate(duplicate) => panic!("a duplicate of {duplicate:?}"),
    }
}

/// The record of the torrent `hash` with its details, name, state, folder and sizes.
fn look(client: &mut Client<SyntheticDaemon>, hash: &HashString) -> TorrentRecord {
    read(
        client,
        hash,
        &full_and(&[
            TorrentField::Name,
            TorrentField::Status,
            TorrentField::DownloadDir,
            TorrentField::TotalSize,
            TorrentField::SizeWhenDone,
            TorrentField::LeftUntilDone,
            TorrentField::PercentDone,
            TorrentField::MetadataPercentComplete,
            TorrentField::AddedDate,
        ]),
    )
}

/// The download and upload rates of the torrent `hash`.
fn rates(client: &mut Client<SyntheticDaemon>, hash: &HashString) -> (Option<u64>, Option<u64>) {
    let record = read(
        client,
        hash,
        &[TorrentField::RateDownload, TorrentField::RateUpload],
    );
    (record.rate_download, record.rate_upload)
}

/// The files of a record as (name, length).
fn names(record: &TorrentRecord) -> Vec<(&str, u64)> {
    record
        .files
        .as_deref()
        .expect("files")
        .iter()
        .map(|file| (file.name.as_str(), file.length))
        .collect()
}

/// The `torrent_add` `params` posted as they are, for what the client would not send.
fn post_add(daemon: &SyntheticDaemon, params: &Value) -> Value {
    let request = json!({
        "jsonrpc": "2.0", "id": 4, "method": "torrent_add", "params": params,
    });
    post(daemon, &request.to_string())
}

#[test]
fn synthetic_add_parses_metainfo_and_honours_paused_and_unwanted() {
    let mut client = client(Activity::Static);
    let content = show();
    let parsed = parse_torrent(&content).expect("a valid torrent");
    let before = counts(&mut client).len();
    recently_active(&mut client);

    // Paused, into a folder of its own, without file 2, file 3 and a file past the end.
    let request = TorrentAdd {
        download_dir: Some(dir("/data/shows")),
        paused: true,
        files_unwanted: vec![2, 3, 99],
        ..add(metainfo(&content))
    };
    let added = add_new(&mut client, &request);

    assert_eq!(added.name, "Show");
    assert_eq!(added.hash, parsed.hash);
    assert_eq!(counts(&mut client).len(), before + 1);
    let record = look(&mut client, &added.hash);
    assert_eq!(record.id, Some(added.id));
    assert_eq!(record.name.as_deref(), Some("Show"));
    assert_eq!(
        names(&record),
        [
            ("Show/a.mkv", 3 * MIB),
            ("Show/.pad/1", 64 * KIB),
            ("Show/b.mkv", 2 * MIB),
            ("Show/Sample/s.mkv", MIB),
        ],
        "the files are the metainfo's, in its order, padding included"
    );
    assert_eq!(record.file_count, Some(4));
    assert_eq!(record.status, Some(Status::Stopped));
    assert_eq!(record.download_dir.as_deref(), Some("/data/shows"));
    assert_eq!(
        record.wanted.as_deref(),
        Some([true, true, false, false].as_slice()),
        "an index past the end is ignored and the others hold"
    );
    let total = 3 * MIB + 64 * KIB + 2 * MIB + MIB;
    assert_eq!(record.total_size, Some(total));
    assert_eq!(record.size_when_done, Some(3 * MIB + 64 * KIB));
    assert_eq!(record.left_until_done, Some(3 * MIB + 64 * KIB));
    assert_eq!(record.percent_done, Some(0.0));
    assert_eq!(record.metadata_percent_complete, Some(1.0));
    assert_eq!(
        recently_active(&mut client),
        slice::from_ref(&added.hash),
        "the new torrent is among the recently active ones at the next poll"
    );

    // Not paused, into the session's folder: it downloads.
    let movie = single_torrent("Movie.mkv", 5 * MIB);
    let started = add_new(&mut client, &add(metainfo(&movie)));
    let record = look(&mut client, &started.hash);
    assert_eq!(record.status, Some(Status::Downloading));
    assert_eq!(record.download_dir.as_deref(), Some(DOWNLOAD_DIR));
    assert_eq!(names(&record), [("Movie.mkv", 5 * MIB)]);
    assert_eq!(record.wanted.as_deref(), Some([true].as_slice()));
    assert_eq!(record.total_size, Some(5 * MIB));
    assert!(started.id > added.id, "ids are not reused");
    assert!(
        record.added_date > look(&mut client, &added.hash).added_date,
        "a later torrent is added later"
    );
}

#[test]
fn synthetic_add_counts_file_sizes_in_whole_kib_and_a_listless_torrent_as_one_file() {
    let mut client = client(Activity::Static);

    // The model counts in KiB, so a file of 3 MiB and a byte is 3 MiB and a KiB.
    let odd = folder_torrent(
        "Odd",
        &[
            (&["a"], 3 * MIB + 1, false),
            (&["b"], 0, false),
            (&["c"], 10, false),
        ],
        false,
    );
    let rounded = add_new(&mut client, &add(metainfo(&odd)));
    let record = look(&mut client, &rounded.hash);
    assert_eq!(
        names(&record),
        [("Odd/a", 3 * MIB + KIB), ("Odd/b", 0), ("Odd/c", KIB)]
    );
    assert_eq!(record.total_size, Some(3 * MIB + 2 * KIB));

    // A torrent the reader cannot number with certainty is one file named after the torrent.
    let tangled = folder_torrent(
        "Tangled",
        &[(&["x"], MIB, false), (&["y"], MIB, false)],
        true,
    );
    assert!(
        parse_torrent(&tangled).expect("readable").files.is_none(),
        "the test torrent is ambiguous"
    );
    let one = add_new(&mut client, &add(metainfo(&tangled)));
    assert_eq!(one.name, "Tangled");
    let record = look(&mut client, &one.hash);
    assert_eq!(names(&record), [("Tangled", 0)]);
    assert_eq!(record.file_count, Some(1));
}

#[test]
fn synthetic_add_of_a_magnet_has_no_metadata() {
    // A ticking daemon, in which every download moves: this one must not.
    let mut client = client(Activity::Ticking);
    let link =
        format!("magnet:?xt=urn:btih:{HASH}&dn=Some%20Show&tr=http%3A%2F%2Ftracker.test%2Fa");
    let request = add(magnet(&link));
    let hash = HashString::parse(HASH).expect("a hash");

    let added = add_new(&mut client, &request);

    assert_eq!(added.name, "Some Show", "named from dn");
    assert_eq!(added.hash, hash);
    let record = look(&mut client, &hash);
    assert_eq!(record.id, Some(added.id));
    assert_eq!(record.name.as_deref(), Some("Some Show"));
    assert_eq!(record.file_count, Some(0));
    assert_eq!(names(&record), [] as [(&str, u64); 0]);
    assert_eq!(record.metadata_percent_complete, Some(0.0));
    assert_eq!(record.status, Some(Status::Downloading));
    assert_eq!(record.total_size, Some(0));
    assert_eq!(record.percent_done, Some(0.0), "nothing of it is known");
    assert_eq!(record.download_dir.as_deref(), Some(DOWNLOAD_DIR));
    let shown = find(&mut client, &hash);
    assert_eq!(shown.state, DisplayState::Downloading);
    assert_eq!(shown.attention, Some(Attention::FetchingMetadata));

    // Time passes and it stays what it is: nothing comes in before the metadata does.
    for _ in 0..3 {
        recently_active(&mut client);
    }
    let later = look(&mut client, &hash);
    assert_eq!(later.metadata_percent_complete, Some(0.0));
    assert_eq!(later.file_count, Some(0));
    assert_eq!(rates(&mut client, &hash), (Some(0), Some(0)));

    // Emptying the wanted files of a torrent with none changes nothing: it does not become a seed.
    set_raw(&mut client, &[&hash], json!({ "files_wanted": [] })).expect("an answer");
    assert_eq!(look(&mut client, &hash).status, Some(Status::Downloading));

    // Without a name it is named by its hash; paused, it is stopped, in the folder given.
    let other = "ffeeddccbbaa99887766554433221100ffeeddcc";
    let nameless = TorrentAdd {
        paused: true,
        download_dir: Some(dir("/data/magnets")),
        ..add(magnet(&format!("magnet:?xt=urn:btih:{other}")))
    };
    let added = add_new(&mut client, &nameless);
    assert_eq!(added.name, other);
    let record = look(&mut client, &added.hash);
    assert_eq!(record.status, Some(Status::Stopped));
    assert_eq!(record.download_dir.as_deref(), Some("/data/magnets"));
    assert_eq!(record.metadata_percent_complete, Some(0.0));
    act(&mut client, TorrentAction::Start, &[&added.hash]);
    assert_eq!(
        look(&mut client, &added.hash).status,
        Some(Status::Downloading),
        "started, it looks for its metadata; it has no data to be a seed with"
    );
    assert_eq!(rates(&mut client, &added.hash), (Some(0), Some(0)));

    // File choices have no file to reach: the daemon takes them and ignores them.
    let third = "0123456789012345678901234567890123456789";
    let answer = post_add(
        &SyntheticDaemon::new(COUNT, Activity::Static),
        &json!({
            "filename": format!("magnet:?xt=urn:btih:{third}"),
            "files_unwanted": [0, 1],
            "priority_high": [2],
        }),
    );
    assert_eq!(
        answer.pointer("/result/torrent_added/hash_string"),
        Some(&json!(third)),
        "{answer}"
    );
}

#[test]
fn synthetic_duplicate_answers_torrent_duplicate() {
    let mut client = client(Activity::Static);
    let content = show();
    let first = add_new(&mut client, &add(metainfo(&content)));
    let everything = list(&mut client, &GetIds::All);
    let details = look(&mut client, &first.hash);
    recently_active(&mut client);

    // The same file again, with other wishes: the old torrent is reported and nothing changes.
    let again = TorrentAdd {
        download_dir: Some(dir("/data/elsewhere")),
        paused: true,
        files_unwanted: vec![0],
        ..add(metainfo(&content))
    };
    let outcome = client.torrent_add(&again).expect("a duplicate is no error");
    assert_eq!(outcome, AddOutcome::Duplicate(first.clone()));
    assert_eq!(list(&mut client, &GetIds::All), everything);
    assert_eq!(look(&mut client, &first.hash), details);
    assert_eq!(
        recently_active(&mut client),
        [] as [HashString; 0],
        "no torrent was touched"
    );

    // The magnet link of the same hash is a duplicate of it.
    let link = format!("magnet:?xt=urn:btih:{}", first.hash);
    let outcome = client.torrent_add(&add(magnet(&link))).expect("an answer");
    assert_eq!(outcome, AddOutcome::Duplicate(first.clone()));

    // So is the magnet link of a torrent the daemon made up, whatever case its hash is in.
    let made_up = first_hash(&mut client);
    let link = format!("magnet:?xt=urn:btih:{}", made_up.as_str().to_uppercase());
    let outcome = client.torrent_add(&add(magnet(&link))).expect("an answer");
    let AddOutcome::Duplicate(duplicate) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(duplicate.hash, made_up);
    assert_eq!(duplicate.id, TorrentId(1));
    assert_eq!(duplicate.name, "Synthetic torrent 0001");
    assert_eq!(
        counts(&mut client).len(),
        usize::try_from(COUNT).unwrap() + 1
    );
}

/// The hash of the first torrent the daemon made up.
fn first_hash(client: &mut Client<SyntheticDaemon>) -> HashString {
    counts(client).first().expect("a torrent").0.clone()
}

/// The error of an answer: its code, message and `data.error_string`.
fn error_of(answer: &Value) -> (i64, &str, Option<&str>) {
    let error = answer.get("error").unwrap_or_else(|| panic!("{answer}"));
    (
        error.get("code").and_then(Value::as_i64).expect("a code"),
        error
            .get("message")
            .and_then(Value::as_str)
            .expect("a message"),
        error.pointer("/data/error_string").and_then(Value::as_str),
    )
}

#[test]
fn synthetic_add_refuses_with_the_daemons_codes_and_words() {
    let daemon = SyntheticDaemon::new(COUNT, Activity::Static);
    let link = format!("magnet:?xt=urn:btih:{HASH}");
    let garbage = STANDARD.encode(b"this is not bencode");
    let torrents = || {
        post(
            &daemon,
            r#"{"jsonrpc":"2.0","id":1,"method":"torrent_get"}"#,
        )
    };
    let before = torrents();

    let invalid = (-32602, "Invalid params");
    let relative = (3, "path is not absolute");
    let unrecognized = (4, "unrecognized info");
    let cases = [
        // Neither source, or an empty one.
        (
            json!({}),
            invalid,
            Some("no filename or metainfo specified"),
        ),
        (
            json!({ "filename": "", "metainfo": "" }),
            invalid,
            Some("no filename or metainfo specified"),
        ),
        // A folder that is not absolute, checked before the content is read.
        (
            json!({ "filename": link, "download_dir": "downloads" }),
            relative,
            Some("download directory path is not absolute"),
        ),
        (
            json!({ "filename": link, "download_dir": "" }),
            relative,
            Some("download directory path is not absolute"),
        ),
        (
            json!({ "metainfo": garbage, "download_dir": "~/x" }),
            relative,
            Some("download directory path is not absolute"),
        ),
        // Content the daemon does not recognize, with no more to say.
        (
            json!({ "metainfo": "!!! not base64 !!!" }),
            unrecognized,
            None,
        ),
        (json!({ "metainfo": garbage }), unrecognized, None),
        (
            json!({ "filename": "https://example.org/a.torrent" }),
            unrecognized,
            None,
        ),
        (json!({ "filename": "/etc/hostname" }), unrecognized, None),
        (
            json!({ "filename": "magnet:?dn=No%20hash" }),
            unrecognized,
            None,
        ),
        (
            json!({ "filename": format!("magnet:?xt=urn:btmh:1220{}", "ab".repeat(32)) }),
            unrecognized,
            None,
        ),
        // The filename wins over the metainfo, even a good one.
        (
            json!({ "filename": "nonsense", "metainfo": STANDARD.encode(show()) }),
            unrecognized,
            None,
        ),
    ];

    for (params, (code, message), detail) in cases {
        let answer = post_add(&daemon, &params);
        assert_eq!(answer.get("id"), Some(&json!(4)), "{params}");
        assert_eq!(error_of(&answer), (code, message, detail), "{params}");
        assert_eq!(
            answer.pointer("/error/data").is_some(),
            detail.is_some(),
            "{params}: {answer}"
        );
    }
    assert_eq!(torrents(), before, "none of them added a torrent");

    // The client reads the refusal as a server error, with the detail.
    let mut client = client(Activity::Static);
    let refused = client
        .call_raw("torrent_add", Some(json!({ "filename": "" })))
        .expect_err("refused");
    assert_eq!(
        refused,
        RpcError::Server {
            code: -32602,
            message: "Invalid params".to_owned(),
            detail: Some("no filename or metainfo specified".to_owned()),
        }
    );
}

#[test]
fn synthetic_add_takes_the_wishes_as_the_daemon_does() {
    let daemon = SyntheticDaemon::new(COUNT, Activity::Static);
    let content = STANDARD.encode(show());
    // Wrong types are not errors: the daemon reads what it can and leaves the rest as if it were
    // not there. An index that is not an integer is skipped, one past the end ignored.
    let answer = post_add(
        &daemon,
        &json!({
            "metainfo": content,
            "paused": "yes",
            "files_unwanted": [1, "2", 2.5, null, -1, 4, 1_000_000],
            "files_wanted": "all",
            "priority_high": [0],
            "priority_low": [3, 77],
        }),
    );
    let hash = answer
        .pointer("/result/torrent_added/hash_string")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{answer}"))
        .to_owned();
    let mut client = Client::new(daemon);
    let hash = HashString::parse(&hash).expect("a hash");

    let record = look(&mut client, &hash);
    assert_eq!(
        record.status,
        Some(Status::Downloading),
        "a paused that is not a boolean is not there"
    );
    assert_eq!(
        record.wanted.as_deref(),
        Some([true, false, true, true].as_slice())
    );
    let (_, priorities) = switches(&mut client, &hash);
    assert_eq!(
        priorities,
        [
            x4_rpc::FilePriority::High,
            x4_rpc::FilePriority::Normal,
            x4_rpc::FilePriority::Normal,
            x4_rpc::FilePriority::Normal
        ],
        "a priority list with an index past the end is ignored as a whole"
    );
}

/// The `free_space` of `path` as the daemon sent it.
fn free_space_of(daemon: &SyntheticDaemon, params: &Value) -> Value {
    let request = json!({
        "jsonrpc": "2.0", "id": 8, "method": "free_space", "params": params,
    });
    post(daemon, &request.to_string())
}

#[test]
fn synthetic_free_space_errors_for_unknown_folders() {
    let mut client = client(Activity::Static);

    // The session's folder keeps answering, as the status bar reads it.
    let known = client.free_space(DOWNLOAD_DIR).expect("a known folder");
    assert_eq!(
        (known.path.as_str(), known.size_bytes, known.total_size),
        (DOWNLOAD_DIR, 800 << 30, 1 << 40)
    );
    let slash = format!("{DOWNLOAD_DIR}/");
    assert_eq!(
        client.free_space(&slash).expect("the same folder").path,
        slash,
        "a trailing slash is the same folder, and the path comes back as asked"
    );

    // A folder nothing is in is not there.
    let refused = client.free_space("/data/new").expect_err("no such folder");
    assert_eq!(
        refused,
        RpcError::Server {
            code: 5,
            message: "system error".to_owned(),
            detail: Some("No such file or directory".to_owned()),
        }
    );
    let daemon = SyntheticDaemon::new(COUNT, Activity::Static);
    let answer = free_space_of(&daemon, &json!({ "path": "/data/new" }));
    assert_eq!(
        answer.pointer("/error"),
        Some(&json!({
            "code": 5,
            "message": "system error",
            "data": {
                "error_string": "No such file or directory",
                "result": { "path": "/data/new", "size_bytes": -1, "total_size": -1 },
            },
        })),
        "{answer}"
    );
    assert!(
        client.free_space(&format!("{DOWNLOAD_DIR}/new")).is_err(),
        "a folder below a known one is not known"
    );

    // A torrent's folder is there once the torrent is.
    let content = single_torrent("Movie.mkv", MIB);
    let request = TorrentAdd {
        download_dir: Some(dir("/data/new")),
        ..add(metainfo(&content))
    };
    let added = add_new(&mut client, &request);
    let space = client
        .free_space("/data/new")
        .expect("a folder of a torrent");
    assert_eq!((space.size_bytes, space.total_size), (800 << 30, 1 << 40));
    client
        .torrent_remove(&selection(&[&added.hash]), RemoveData::Keep)
        .expect("removed");
    assert!(
        client.free_space("/data/new").is_err(),
        "the folder goes with its last torrent"
    );

    // What the daemon says to a path it cannot use.
    for (params, code, message, detail) in [
        (
            json!({}),
            -32602,
            "Invalid params",
            "directory path argument is missing",
        ),
        (
            json!({ "path": 5 }),
            -32602,
            "Invalid params",
            "directory path argument is missing",
        ),
        (
            json!({ "path": "data" }),
            3,
            "path is not absolute",
            "directory path is not absolute",
        ),
        (
            json!({ "path": "" }),
            3,
            "path is not absolute",
            "directory path is not absolute",
        ),
    ] {
        let answer = free_space_of(&daemon, &params);
        assert_eq!(error_of(&answer), (code, message, Some(detail)), "{params}");
        assert_eq!(
            answer.pointer("/error/data/result"),
            None,
            "{params}: no sizes without a path to measure"
        );
    }
}

#[test]
fn synthetic_free_space_answers_for_the_parents_of_known_folders() {
    let mut client = client(Activity::Static);

    // The folders above the session's folder are there, as on a real disk.
    for parent in ["/", "/data", "/data/"] {
        let space = client
            .free_space(parent)
            .expect("a parent of a known folder");
        assert_eq!(
            (space.path.as_str(), space.size_bytes, space.total_size),
            (parent, 800 << 30, 1 << 40)
        );
    }
    // Beside it and below it, nothing is.
    for missing in [
        "/data/other",
        "/data/torrents/sub",
        "/dat",
        "/data/torrentsx",
    ] {
        assert!(
            client.free_space(missing).is_err(),
            "{missing} is neither a known folder nor a parent of one"
        );
    }

    // The parents of a torrent's folder are there while the torrent is.
    let content = single_torrent("Movie.mkv", MIB);
    let request = TorrentAdd {
        download_dir: Some(dir("/srv/media/new")),
        ..add(metainfo(&content))
    };
    let added = add_new(&mut client, &request);
    for parent in ["/srv", "/srv/media", "/srv/media/new"] {
        assert!(client.free_space(parent).is_ok(), "{parent} is there");
    }
    client
        .torrent_remove(&selection(&[&added.hash]), RemoveData::Keep)
        .expect("removed");
    for parent in ["/srv", "/srv/media", "/srv/media/new"] {
        assert!(
            client.free_space(parent).is_err(),
            "{parent} goes with the last torrent below it"
        );
    }
    assert!(client.free_space("/").is_ok(), "the root stays");
}
