//! The `.torrent` reader through its public API, on files built here with a small bencode writer
//! (`writer`): the files and their order, which is the daemon's index order (`files`); the
//! files the daemon could read differently, which leave no choice of files, and the ones it
//! refuses (`ambiguity`); and hostile input, which must never panic, overflow the stack or
//! bloat (`hostile`).

mod ambiguity;
mod files;
mod hostile;
mod writer;

use sha1::{Digest, Sha1};
use writer::{Bencode, bytes, dict, int, list};

/// The piece length of the test torrents.
const PIECE: i64 = 16384;

/// The `pieces` of a torrent of `total` bytes: one 20-byte hash per piece of [`PIECE`] bytes.
/// The reader only counts them, so they are all the same.
fn pieces(total: i64) -> Bencode {
    let count = usize::try_from(total.saturating_add(PIECE - 1) / PIECE).unwrap_or(0);
    bytes(vec![0xab; 20 * count])
}

/// An entry of `info.files`: the path's components and the length.
fn file(path: &[&str], length: i64) -> Bencode {
    dict([
        ("length", int(length)),
        ("path", list(path.iter().map(bytes))),
    ])
}

/// A padding file (BEP 47) in the `.pad` folder.
fn pad(length: i64) -> Bencode {
    dict([
        ("attr", bytes("p")),
        ("length", int(length)),
        ("path", list([bytes(".pad"), bytes(length.to_string())])),
    ])
}

/// The info dictionary of a folder `name` holding the `entries` of `info.files`, whose lengths
/// add up to `total`.
fn folder_info(name: &str, entries: Vec<Bencode>, total: i64) -> Bencode {
    dict([
        ("files", Bencode::List(entries)),
        ("name", bytes(name)),
        ("piece length", int(PIECE)),
        ("pieces", pieces(total)),
    ])
}

/// The info dictionary of a single file `name` of `length` bytes.
fn single_info(name: &str, length: i64) -> Bencode {
    dict([
        ("length", int(length)),
        ("name", bytes(name)),
        ("piece length", int(PIECE)),
        ("pieces", pieces(length)),
    ])
}

/// A `.torrent` file with a tracker and `info`.
fn torrent(info: Bencode) -> Vec<u8> {
    dict([
        ("announce", bytes("http://tracker.test/announce")),
        ("info", info),
    ])
    .encode()
}

/// The SHA-1 of `data` in lowercase hex, worked out here rather than by the reader.
fn sha1_hex(data: &[u8]) -> String {
    Sha1::digest(data)
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
        .collect()
}
