//! The files of a torrent, in the order the daemon numbers them, with the names it shows, and the
//! info hash it keys the torrent by.

use x4_core::{MetaFile, MetainfoError, parse_torrent};

use super::writer::{Bencode, bytes, dict, int, list};
use super::{PIECE, file, folder_info, pad, pieces, sha1_hex, single_info, torrent};

fn meta(path: &str, length: u64) -> MetaFile {
    MetaFile {
        path: path.to_owned(),
        length,
    }
}

#[test]
fn a_single_file_torrent_has_one_file_at_index_0() {
    let parsed = parse_torrent(&torrent(single_info("movie.mkv", 40_000))).unwrap();

    assert_eq!(parsed.name, "movie.mkv");
    assert_eq!(parsed.total_size, Some(40_000));
    assert_eq!(parsed.files, Some(vec![meta("movie.mkv", 40_000)]));
}

#[test]
fn files_keep_the_raw_info_files_order() {
    // Neither sorted by name nor grouped by folder: the daemon numbers them as listed.
    let entries = vec![
        file(&["b.txt"], 1),
        file(&["a.txt"], 2),
        file(&["sub", "z.bin"], 3),
        file(&["Sub", "a.bin"], 4),
        file(&["c.txt"], 5),
        file(&["sub", "a.bin"], 6),
    ];
    let parsed = parse_torrent(&torrent(folder_info("Top", entries, 21))).unwrap();

    assert_eq!(parsed.name, "Top");
    assert_eq!(parsed.total_size, Some(21));
    assert_eq!(
        parsed.files,
        Some(vec![
            meta("Top/b.txt", 1),
            meta("Top/a.txt", 2),
            meta("Top/sub/z.bin", 3),
            meta("Top/Sub/a.bin", 4),
            meta("Top/c.txt", 5),
            meta("Top/sub/a.bin", 6),
        ])
    );
}

#[test]
fn pad_files_keep_their_index() {
    let bitcomet_pad =
        "_____padding_file_0_if you see this file, please update to BitComet 0.85 or above____";
    let entries = vec![
        file(&["a.bin"], 1000),
        pad(PIECE - 1000),
        file(&["b.bin"], 5000),
        file(&[bitcomet_pad], PIECE - 5000),
        file(&["c.bin"], 0),
    ];
    let parsed = parse_torrent(&torrent(folder_info("Pads", entries, 2 * PIECE))).unwrap();

    let files = parsed.files.unwrap();
    let paths: Vec<_> = files.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "Pads/a.bin",
            "Pads/.pad/15384",
            "Pads/b.bin",
            &format!("Pads/{bitcomet_pad}"),
            "Pads/c.bin",
        ]
    );
    assert_eq!(files[1].length, 15384);
    assert_eq!(files[4].length, 0, "a zero-length file keeps its index too");
}

#[test]
fn a_hybrid_torrent_uses_the_v1_file_list() {
    // The v2 `file tree` is a dictionary, so it lists the files sorted; the v1 `files` list does
    // not, and the daemon ignores the tree.
    let leaf = |length: i64| {
        dict([(
            "",
            dict([("length", int(length)), ("pieces root", bytes([7; 32]))]),
        )])
    };
    let info = dict([
        (
            "file tree",
            dict([("a.bin", leaf(1000)), ("z.bin", leaf(5000))]),
        ),
        (
            "files",
            list([
                file(&["z.bin"], 5000),
                pad(PIECE - 5000),
                file(&["a.bin"], 1000),
            ]),
        ),
        ("meta version", int(2)),
        ("name", bytes("Hybrid")),
        ("piece length", int(PIECE)),
        ("pieces", pieces(PIECE + 1000)),
    ]);
    let layers = dict([("x".repeat(32).as_str(), bytes([9; 40]))]);
    let bytes = dict([("info", info), ("piece layers", layers)]).encode();

    let parsed = parse_torrent(&bytes).unwrap();

    assert_eq!(
        parsed.files,
        Some(vec![
            meta("Hybrid/z.bin", 5000),
            meta("Hybrid/.pad/11384", 11384),
            meta("Hybrid/a.bin", 1000),
        ])
    );
    assert_eq!(parsed.total_size, Some(PIECE.unsigned_abs() + 1000));
}

#[test]
fn a_v2_only_torrent_is_refused() {
    let tree = || {
        dict([(
            "a.bin",
            dict([(
                "",
                dict([("length", int(1000)), ("pieces root", bytes([7; 32]))]),
            )]),
        )])
    };
    let v2_only = dict([
        ("file tree", tree()),
        ("meta version", int(2)),
        ("name", bytes("V2")),
        ("piece length", int(PIECE)),
    ]);
    let tree_only = dict([
        ("file tree", tree()),
        ("name", bytes("V2")),
        ("piece length", int(PIECE)),
    ]);
    let version_and_empty_pieces = dict([
        ("length", int(1000)),
        ("meta version", int(2)),
        ("name", bytes("V2")),
        ("piece length", int(PIECE)),
        ("pieces", bytes([])),
    ]);
    for info in [v2_only, tree_only, version_and_empty_pieces] {
        assert_eq!(parse_torrent(&torrent(info)), Err(MetainfoError::V2Only));
    }

    // Without any sign of v2, missing piece hashes are just an invalid torrent.
    let no_pieces = dict([
        ("length", int(1000)),
        ("name", bytes("V1")),
        ("piece length", int(PIECE)),
    ]);
    assert!(matches!(
        parse_torrent(&torrent(no_pieces)),
        Err(MetainfoError::Invalid(_))
    ));
}

#[test]
fn the_hash_is_sha1_over_the_raw_info_bytes() {
    // A known answer, worked out with `sha1sum` over the info dictionary below.
    let known: &[u8] = b"d8:announce28:http://tracker.test/announce13:creation datei1700000000e\
        4:infod6:lengthi12345e4:name8:file.bin12:piece lengthi16384e6:pieces20:xxxxxxxxxxxxxxxxxxxxee";
    let parsed = parse_torrent(known).unwrap();
    assert_eq!(
        parsed.hash.as_str(),
        "ba61769b455228b4bb7de0e75298c3c177d54f8a"
    );
    assert_eq!(parsed.files, Some(vec![meta("file.bin", 12345)]));

    // Bencode the daemon reads but would not write: a string length with a leading zero and a
    // `-0`. The hash is over these bytes as they are, not over a re-encoding, and what follows
    // the top dictionary is ignored, as the daemon does.
    let mut info = b"d6:lengthi5e4:name05:a.txt12:piece lengthi16384e6:pieces20:".to_vec();
    info.extend_from_slice(&[0xab; 20]);
    info.extend_from_slice(b"1:xi-0ee");
    let mut file = b"d4:info".to_vec();
    file.extend_from_slice(&info);
    file.extend_from_slice(b"etrailing bytes");

    let parsed = parse_torrent(&file).unwrap();

    assert_eq!(parsed.hash.as_str(), sha1_hex(&info));
    let canonical = dict([
        ("length", int(5)),
        ("name", bytes("a.txt")),
        ("piece length", int(PIECE)),
        ("pieces", bytes([0xab; 20])),
        ("x", int(0)),
    ])
    .encode();
    assert_ne!(parsed.hash.as_str(), sha1_hex(&canonical));
    assert_eq!(parsed.files, Some(vec![meta("a.txt", 5)]));
}

#[test]
fn names_are_sanitized_for_display_only() {
    let entries = vec![
        file(&["..", "a"], 1),
        file(&[".", "b"], 2),
        file(&["", "c", ""], 3),
        file(&["d/e", "f"], 4),
        dict([("length", int(5)), ("path", list([bytes(b"g\xffh")]))]),
        dict([("length", int(6)), ("path", list([bytes(b"i\0hidden")]))]),
        file(&["...", "j"], 7),
    ];
    let info = dict([
        ("files", Bencode::List(entries)),
        ("name", bytes(b"Top/.\xfe")),
        ("piece length", int(PIECE)),
        ("pieces", pieces(28)),
    ]);

    let parsed = parse_torrent(&torrent(info)).unwrap();

    // The torrent's own name is only made valid text; its parts are sanitized where it becomes the
    // files' top folder.
    assert_eq!(parsed.name, "Top/.\u{fffd}");
    assert_eq!(
        parsed.files,
        Some(vec![
            meta("Top/.\u{fffd}/_../a", 1),
            meta("Top/.\u{fffd}/_./b", 2),
            meta("Top/.\u{fffd}/c", 3),
            meta("Top/.\u{fffd}/d/e/f", 4),
            meta("Top/.\u{fffd}/g\u{fffd}h", 5),
            meta("Top/.\u{fffd}/i", 6),
            meta("Top/.\u{fffd}/.../j", 7),
        ])
    );

    // A name that is only `.` becomes `_.` as the top folder.
    let info = folder_info(".", vec![file(&["a"], 1)], 1);
    let parsed = parse_torrent(&torrent(info)).unwrap();
    assert_eq!(parsed.name, ".");
    assert_eq!(parsed.files, Some(vec![meta("_./a", 1)]));
}

#[test]
fn utf8_keys_replace_the_plain_ones() {
    let entry = dict([
        ("length", int(3)),
        ("path", list([bytes(b"\xe9t\xe9.txt")])),
        ("path.utf-8", list([bytes("été.txt")])),
    ]);
    let info = dict([
        ("files", list([entry, file(&["plain.txt"], 4)])),
        ("name", bytes(b"\xc9t\xe9")),
        ("name.utf-8", bytes("Été")),
        ("piece length", int(PIECE)),
        ("pieces", pieces(7)),
    ]);

    let parsed = parse_torrent(&torrent(info)).unwrap();

    assert_eq!(parsed.name, "Été");
    assert_eq!(
        parsed.files,
        Some(vec![meta("Été/été.txt", 3), meta("Été/plain.txt", 4)])
    );
}

#[test]
fn a_top_level_piece_length_counts_like_the_info_one() {
    // libtransmission reads `piece length` outside `info` too; it comes after `info`, so it wins.
    let info = dict([
        ("length", int(3 * 1024)),
        ("name", bytes("a.bin")),
        ("piece length", int(PIECE)),
        ("pieces", bytes([0xab; 60])),
    ]);
    let file = dict([("info", info), ("piece length", int(1024))]).encode();

    let parsed = parse_torrent(&file).unwrap();

    assert_eq!(parsed.files, Some(vec![meta("a.bin", 3 * 1024)]));
}
