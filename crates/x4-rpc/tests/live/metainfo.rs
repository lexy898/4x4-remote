//! The test torrent's content: random file data and its metainfo, of one file or of a folder of
//! files (a bencode writer and SHA-1 piece hashes). Pure code, no daemon or filesystem access.

use std::hash::{BuildHasher, RandomState};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use sha1::{Digest, Sha1};
use x4_rpc::HashString;

/// A seed that differs between runs: the standard library's per-process random hash keys,
/// mixed with the time and the process id.
pub(crate) fn random_seed() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    RandomState::new().hash_one((nanos, process::id()))
}

/// `len` pseudo-random bytes from `SplitMix64` seeded with `seed`: fresh content (so a fresh info
/// hash) for every run, without a dependency.
pub(crate) fn random_bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed;
    let mut bytes = Vec::with_capacity(len + 8);
    while bytes.len() < len {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        bytes.extend_from_slice(&(value ^ (value >> 31)).to_le_bytes());
    }
    bytes.truncate(len);
    bytes
}

/// A bencode value: as much of the format as the test torrents' metainfo needs.
#[derive(Debug)]
enum Bencode<'a> {
    Int(usize),
    Bytes(&'a [u8]),
    List(Vec<Bencode<'a>>),
    /// Encoded with its keys sorted, as bencode requires.
    Dict(Vec<(&'a str, Bencode<'a>)>),
}

impl Bencode<'_> {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode_into(&mut out);
        out
    }

    fn encode_into(&self, out: &mut Vec<u8>) {
        match self {
            Self::Int(value) => out.extend_from_slice(format!("i{value}e").as_bytes()),
            Self::Bytes(bytes) => {
                out.extend_from_slice(format!("{}:", bytes.len()).as_bytes());
                out.extend_from_slice(bytes);
            }
            Self::List(items) => {
                out.push(b'l');
                for item in items {
                    item.encode_into(out);
                }
                out.push(b'e');
            }
            Self::Dict(entries) => {
                let mut sorted: Vec<_> = entries.iter().collect();
                // `str` orders by bytes, which is the order bencode wants.
                sorted.sort_by_key(|(key, _)| *key);
                out.push(b'd');
                for (key, value) in sorted {
                    Bencode::Bytes(key.as_bytes()).encode_into(out);
                    value.encode_into(out);
                }
                out.push(b'e');
            }
        }
    }
}

/// A `.torrent` file and the info hash the daemon will report for it.
#[derive(Debug)]
pub(crate) struct Metainfo {
    /// The content of the `.torrent` file.
    pub(crate) bytes: Vec<u8>,
    /// The SHA-1 of the bencoded info dictionary.
    pub(crate) info_hash: HashString,
}

/// The metainfo of a single-file torrent `name` holding `data`, without trackers.
#[expect(
    clippy::expect_used,
    reason = "a SHA-1 digest in hex is always a valid hash"
)]
pub(crate) fn single_file_metainfo(name: &str, data: &[u8], piece_length: usize) -> Metainfo {
    let pieces: Vec<u8> = data.chunks(piece_length).flat_map(Sha1::digest).collect();
    let info = Bencode::Dict(vec![
        ("length", Bencode::Int(data.len())),
        ("name", Bencode::Bytes(name.as_bytes())),
        ("piece length", Bencode::Int(piece_length)),
        ("pieces", Bencode::Bytes(&pieces)),
    ]);
    let info_hash = HashString::parse(&to_hex(&Sha1::digest(info.encode())))
        .expect("a SHA-1 digest in hex is a valid hash");
    let bytes = Bencode::Dict(vec![("info", info)]).encode();
    Metainfo { bytes, info_hash }
}

/// The metainfo of a multi-file torrent: the folder `name` holding `files`, each a name and its
/// data, in that order, without trackers. The pieces run across the files: the data is hashed
/// end to end.
#[expect(
    clippy::expect_used,
    reason = "a SHA-1 digest in hex is always a valid hash"
)]
pub(crate) fn multi_file_metainfo(
    name: &str,
    files: &[(&str, &[u8])],
    piece_length: usize,
) -> Metainfo {
    let data: Vec<u8> = files
        .iter()
        .flat_map(|(_, data)| data.iter().copied())
        .collect();
    let pieces: Vec<u8> = data.chunks(piece_length).flat_map(Sha1::digest).collect();
    let entries = files
        .iter()
        .map(|(file, data)| {
            Bencode::Dict(vec![
                ("length", Bencode::Int(data.len())),
                ("path", Bencode::List(vec![Bencode::Bytes(file.as_bytes())])),
            ])
        })
        .collect();
    let info = Bencode::Dict(vec![
        ("files", Bencode::List(entries)),
        ("name", Bencode::Bytes(name.as_bytes())),
        ("piece length", Bencode::Int(piece_length)),
        ("pieces", Bencode::Bytes(&pieces)),
    ]);
    let info_hash = HashString::parse(&to_hex(&Sha1::digest(info.encode())))
        .expect("a SHA-1 digest in hex is a valid hash");
    let bytes = Bencode::Dict(vec![("info", info)]).encode();
    Metainfo { bytes, info_hash }
}

fn to_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn from_hex(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn bencode_sorts_keys_and_prefixes_strings_with_their_length() {
        let value = Bencode::Dict(vec![
            ("name", Bencode::Bytes(b"a")),
            ("length", Bencode::Int(11)),
            ("info", Bencode::Dict(Vec::new())),
        ]);

        assert_eq!(value.encode(), b"d4:infode6:lengthi11e4:name1:ae");
    }

    #[test]
    fn metainfo_hashes_every_piece_and_the_info_dictionary() {
        // Expected values computed independently with Python's hashlib.
        let pieces = from_hex(concat!(
            "a5cec7af5f7aab769cf0d4aa440e01c7bfc371b2", // "hell"
            "1085cd1a2f46b0902dac61890f31072561978deb", // "o wo"
            "c466b3a33e6f6dc170438d34fcddf7860d3658fb", // "rld"
        ));
        let mut info = b"d6:lengthi11e4:name5:a.bin12:piece lengthi4e6:pieces60:".to_vec();
        info.extend_from_slice(&pieces);
        info.push(b'e');
        let mut torrent = b"d4:info".to_vec();
        torrent.extend_from_slice(&info);
        torrent.push(b'e');

        let metainfo = single_file_metainfo("a.bin", b"hello world", 4);

        assert_eq!(metainfo.bytes, torrent);
        assert_eq!(
            metainfo.info_hash.as_str(),
            "420ae9d2244908beea4a3022d6a186dd308f0dca"
        );
    }

    #[test]
    fn a_multi_file_metainfo_lists_the_files_and_hashes_their_data_as_one() {
        // Expected values computed independently with Python's hashlib: the pieces run across
        // the files, so they are those of "hello world" in pieces of 4.
        let pieces = from_hex(concat!(
            "a5cec7af5f7aab769cf0d4aa440e01c7bfc371b2", // "hell"
            "1085cd1a2f46b0902dac61890f31072561978deb", // "o wo"
            "c466b3a33e6f6dc170438d34fcddf7860d3658fb", // "rld"
        ));
        let mut info = b"d5:filesld6:lengthi5e4:pathl5:a.bineed6:lengthi6e4:pathl5:b.bineee\
                         4:name3:dir12:piece lengthi4e6:pieces60:"
            .to_vec();
        info.extend_from_slice(&pieces);
        info.push(b'e');
        let mut torrent = b"d4:info".to_vec();
        torrent.extend_from_slice(&info);
        torrent.push(b'e');

        let files: [(&str, &[u8]); 2] = [("a.bin", b"hello"), ("b.bin", b" world")];

        let metainfo = multi_file_metainfo("dir", &files, 4);

        assert_eq!(metainfo.bytes, torrent);
        assert_eq!(
            metainfo.info_hash.as_str(),
            "8418e8506e0fc7207271ba21236d85e9be6c64d0"
        );
    }

    #[test]
    fn random_bytes_have_the_requested_length_and_follow_the_seed() {
        let first = random_bytes(1, 13);

        assert_eq!(first.len(), 13);
        assert_eq!(random_bytes(1, 13), first);
        assert_ne!(random_bytes(2, 13), first);
    }
}
