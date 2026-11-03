//! The offline tests of `metainfo.rs`: bencode and piece hashes. Only x4-rpc's live tests declare
//! this file; x4-core's include `metainfo.rs` too and would run these a second time.

use crate::metainfo::{Bencode, multi_file_metainfo, random_bytes, single_file_metainfo};

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
fn a_file_in_a_folder_of_the_torrent_has_one_path_element_per_part() {
    let files: [(&str, &[u8]); 2] = [("z.bin", b"hello"), ("mid/a.bin", b" world")];

    let metainfo = multi_file_metainfo("dir", &files, 4);

    let files_list = b"5:filesld6:lengthi5e4:pathl5:z.bineed6:lengthi6e4:pathl3:mid5:a.bineee";
    assert!(
        metainfo
            .bytes
            .windows(files_list.len())
            .any(|window| window == files_list),
        "{}",
        String::from_utf8_lossy(&metainfo.bytes)
    );
}

#[test]
fn random_bytes_have_the_requested_length_and_follow_the_seed() {
    let first = random_bytes(1, 13);

    assert_eq!(first.len(), 13);
    assert_eq!(random_bytes(1, 13), first);
    assert_ne!(random_bytes(2, 13), first);
}
