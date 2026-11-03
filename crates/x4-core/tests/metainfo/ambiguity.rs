//! Files the daemon could read with other files or in another order than the strict reader (no
//! choice of files then, only "all files"), and files the daemon refuses (an error).

use x4_core::{MetainfoError, ParsedTorrent, parse_torrent};
use x4_rpc::HashString;

use super::writer::{Bencode, bytes, dict, int, list, unsorted_dict};
use super::{PIECE, file, folder_info, pieces, sha1_hex, single_info, torrent};

/// The info dictionary of the folder `Odd` holding `entries`, whose lengths add up to `total`.
fn odd_folder(entries: Vec<Bencode>, total: i64) -> Bencode {
    folder_info("Odd", entries, total)
}

/// Asserts that `file` is read, with its name and the hash of `info`, but without a choice of
/// files.
#[track_caller]
#[expect(
    clippy::unwrap_used,
    reason = "a test helper; a SHA-1 in hex is a valid hash"
)]
fn assert_no_choice(file: &[u8], info: &Bencode, case: &str) {
    assert_eq!(
        parse_torrent(file),
        Ok(ParsedTorrent {
            name: "Odd".to_owned(),
            hash: HashString::parse(&sha1_hex(&info.encode())).unwrap(),
            total_size: None,
            files: None,
        }),
        "{case}"
    );
}

#[test]
fn duplicate_or_unsorted_keys_leave_no_file_choice() {
    let good = || vec![file(&["a"], 1), file(&["b"], 2)];

    // The same torrent with its keys in order has a choice of files.
    let canonical = odd_folder(good(), 3);
    assert!(parse_torrent(&torrent(canonical)).unwrap().files.is_some());

    let unsorted_info = unsorted_dict([
        ("name", bytes("Odd")),
        ("files", Bencode::List(good())),
        ("piece length", int(PIECE)),
        ("pieces", pieces(3)),
    ]);
    assert_no_choice(
        &torrent(unsorted_info.clone()),
        &unsorted_info,
        "unsorted info",
    );

    let entry_with_two_paths = unsorted_dict([
        ("length", int(1)),
        ("path", list([bytes("a")])),
        ("path", list([bytes("b")])),
    ]);
    let info = odd_folder(vec![entry_with_two_paths, file(&["c"], 2)], 3);
    assert_no_choice(&torrent(info.clone()), &info, "a key twice in an entry");

    // `path.utf-8` before `path`: the daemon would join both.
    let utf8_first = unsorted_dict([
        ("length", int(1)),
        ("path.utf-8", list([bytes("a")])),
        ("path", list([bytes("b")])),
    ]);
    let info = odd_folder(vec![utf8_first, file(&["c"], 2)], 3);
    assert_no_choice(&torrent(info.clone()), &info, "unsorted entry");

    // Outside `info` too: R1 asks for no repeated or unsorted key anywhere.
    let info = odd_folder(good(), 3);
    let two_trackers = unsorted_dict([
        ("announce", bytes("http://one.test/")),
        ("announce", bytes("http://two.test/")),
        ("info", info.clone()),
    ])
    .encode();
    assert_no_choice(&two_trackers, &info, "a key twice at the top");
    let unsorted_elsewhere = dict([
        ("info", info.clone()),
        ("x", unsorted_dict([("b", int(1)), ("a", int(2))])),
    ])
    .encode();
    assert_no_choice(&unsorted_elsewhere, &info, "unsorted keys outside info");

    // Two `info`s: the daemon hashes the last one but may take files from the first.
    let first = odd_folder(vec![file(&["first"], 3)], 3);
    let second = odd_folder(good(), 3);
    let two_infos = unsorted_dict([("info", first.clone()), ("info", second.clone())]).encode();
    assert_no_choice(&two_infos, &second, "two infos");
    // The daemon keeps the piece hashes of the first `info`, so a last one without them is not
    // refused.
    let without_pieces = dict([
        ("files", Bencode::List(good())),
        ("name", bytes("Odd")),
        ("piece length", int(PIECE)),
    ]);
    let two_infos = unsorted_dict([("info", first), ("info", without_pieces.clone())]).encode();
    assert_no_choice(
        &two_infos,
        &without_pieces,
        "two infos, the last without pieces",
    );
}

#[test]
fn entries_the_daemon_could_count_differently_leave_no_file_choice() {
    let cases: Vec<(&str, Bencode)> = vec![
        // The daemon skips an entry that is not a dictionary, which shifts every later index.
        ("a string entry", bytes("junk")),
        ("an integer entry", int(7)),
        ("a list entry", list([bytes("x")])),
        // The daemon would read a negative length as a huge one.
        ("a negative length", file(&["x"], -1)),
        (
            "a length that is not an integer",
            dict([("length", bytes("1")), ("path", list([bytes("x")]))]),
        ),
        // A dictionary inside an entry ends like an entry: the daemon would add a file for it.
        (
            "a dictionary inside an entry",
            dict([
                ("length", int(1)),
                ("path", list([bytes("x")])),
                ("x-extra", dict([("path", list([bytes("y")]))])),
            ]),
        ),
        (
            "a dictionary in a list inside an entry",
            dict([
                ("length", int(1)),
                ("path", list([bytes("x")])),
                ("x-extra", list([dict([])])),
            ]),
        ),
        (
            "a list of lists inside an entry",
            dict([
                ("length", int(1)),
                ("md5sum", list([list([])])),
                ("path", list([bytes("x")])),
            ]),
        ),
        (
            "a list inside the path",
            dict([
                ("length", int(1)),
                ("path", list([list([bytes("x")]), bytes("y")])),
            ]),
        ),
        (
            "a path that is not a list",
            dict([("length", int(1)), ("path", bytes("x"))]),
        ),
        // The daemon stops reading the file list at the end of any list keyed `files`.
        (
            "an entry holding a files list",
            dict([
                ("files", list([])),
                ("length", int(1)),
                ("path", list([bytes("x")])),
            ]),
        ),
    ];
    for (case, odd_entry) in cases {
        let info = odd_folder(vec![file(&["a"], 1), odd_entry, file(&["b"], 2)], 3);
        assert_no_choice(&torrent(info.clone()), &info, case);
    }

    // `files` that is not a list.
    let info = dict([
        ("files", dict([("a", int(1))])),
        ("length", int(3)),
        ("name", bytes("Odd")),
        ("piece length", int(PIECE)),
        ("pieces", pieces(3)),
    ]);
    assert_no_choice(&torrent(info.clone()), &info, "files is a dictionary");

    // A single file with a negative length.
    let info = dict([
        ("length", int(-5)),
        ("name", bytes("Odd")),
        ("piece length", int(PIECE)),
        ("pieces", bytes([0; 20])),
    ]);
    assert_no_choice(&torrent(info.clone()), &info, "a negative single length");
}

#[test]
fn paths_that_match_only_once_made_valid_text_leave_no_file_choice() {
    // Both become `a\u{fffd}` here; whether the daemon's UTF-8 repair makes them equal, and so
    // refuses the torrent, is not certain.
    let invalid =
        |name: &[u8], length| dict([("length", int(length)), ("path", list([bytes(name)]))]);
    let info = odd_folder(vec![invalid(b"a\xff", 1), invalid(b"a\xfe", 2)], 3);
    assert_no_choice(&torrent(info.clone()), &info, "two invalid names");
    let info = odd_folder(vec![file(&["a\u{fffd}"], 1), invalid(b"a\xff", 2)], 3);
    assert_no_choice(&torrent(info.clone()), &info, "a replacement character");
}

#[test]
fn torrents_the_daemon_refuses_are_refused() {
    let invalid = |file: Vec<u8>, case: &str| {
        let result = parse_torrent(&file);
        assert!(
            matches!(result, Err(MetainfoError::Invalid(_))),
            "{case}: {result:?}"
        );
    };

    // The piece count would refuse it too, but the daemon finds that it has no files first.
    let empty = dict([
        ("length", int(0)),
        ("name", bytes("empty.bin")),
        ("piece length", int(PIECE)),
        ("pieces", bytes([0; 20])),
    ]);
    assert_eq!(
        parse_torrent(&torrent(empty)),
        Err(MetainfoError::Invalid("the torrent has no files"))
    );
    invalid(torrent(single_info("", 10)), "a single file without a name");
    invalid(
        torrent(single_info("\0hidden", 10)),
        "a name that ends at once",
    );
    let info = odd_folder(vec![file(&["a"], 1), file(&[], 2)], 3);
    invalid(torrent(info), "an entry without a path");
    let info = odd_folder(vec![file(&["a"], 1), file(&["", ""], 2)], 3);
    invalid(torrent(info), "an entry whose path is empty");
    let info = odd_folder(vec![file(&["a"], 1), dict([("length", int(2))])], 3);
    invalid(torrent(info), "an entry with no path key");
    let info = odd_folder(vec![file(&["a", "b"], 1), file(&["a/b"], 2)], 3);
    invalid(torrent(info), "two files with the same path");
    let info = odd_folder(vec![file(&["a"], 1), file(&["", "a", ""], 2)], 3);
    invalid(torrent(info), "two files with the same sanitized path");

    let with = |piece_length: Bencode, hashes: Bencode| {
        dict([
            ("length", int(3 * PIECE)),
            ("name", bytes("a.bin")),
            ("piece length", piece_length),
            ("pieces", hashes),
        ])
    };
    invalid(
        torrent(with(int(PIECE), bytes([0; 59]))),
        "pieces that are not whole hashes",
    );
    invalid(
        torrent(with(int(PIECE), bytes([0; 40]))),
        "too few pieces for the size",
    );
    invalid(
        torrent(with(int(PIECE), bytes([0; 80]))),
        "too many pieces for the size",
    );
    invalid(torrent(with(int(0), bytes([0; 60]))), "a piece length of 0");
    invalid(
        torrent(with(int(-PIECE), bytes([0; 60]))),
        "a negative piece length",
    );
    invalid(
        torrent(with(int(1 << 32), bytes([0; 20]))),
        "a piece length beyond 32 bits",
    );
    let no_piece_length = dict([
        ("length", int(10)),
        ("name", bytes("a.bin")),
        ("pieces", bytes([0; 20])),
    ]);
    invalid(torrent(no_piece_length), "no piece length");
    // All files empty: no piece can cover them, and `pieces` must not be empty.
    let info = odd_folder(vec![file(&["a"], 0)], 0);
    invalid(torrent(info), "a folder of empty files");
}

#[test]
fn a_file_without_an_info_dictionary_is_refused() {
    assert_eq!(
        parse_torrent(&dict([("announce", bytes("x"))]).encode()),
        Err(MetainfoError::NoInfo)
    );
    assert_eq!(
        parse_torrent(&dict([("info", bytes("x"))]).encode()),
        Err(MetainfoError::NoInfo)
    );
    assert_eq!(
        parse_torrent(&list([dict([("info", single_info("a", 1))])]).encode()),
        Err(MetainfoError::NoInfo)
    );
    assert_eq!(parse_torrent(b"i5e"), Err(MetainfoError::NoInfo));
    assert_eq!(
        parse_torrent(b"hello"),
        Err(MetainfoError::NotBencode { offset: 0 })
    );
}
