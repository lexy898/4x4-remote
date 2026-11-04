//! Tests of the country table, of [`CountryCode`] and, on files that [`build_country_file`] and
//! `encode` write, of the reader and the routing of [`GeoIp`]. No test reads the installed files.

use std::io::{Cursor, Read, Seek, SeekFrom};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use super::build::{INFO, build_file, encode};
use super::*;

fn code(letters: &str) -> CountryCode {
    CountryCode::new(letters).unwrap()
}

fn ip(text: &str) -> IpAddr {
    text.parse().unwrap()
}

fn open(file: Vec<u8>, edition: Edition) -> Result<CountryDb, GeoIpError> {
    CountryDb::open(Box::new(Cursor::new(file)), edition)
}

/// What a [`Counted`] source was asked for, and when its reads start to fail.
struct Counters {
    reads: AtomicUsize,
    bytes: AtomicU64,
    /// How many more reads succeed; `usize::MAX` for all of them.
    reads_before_failure: AtomicUsize,
}

impl Counters {
    fn reads(&self) -> usize {
        self.reads.load(Ordering::Relaxed)
    }

    fn bytes(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }

    fn fail_after(&self, reads: usize) {
        self.reads_before_failure.store(reads, Ordering::Relaxed);
    }
}

/// A file in memory that counts the reads made of it, and fails them when told to.
struct Counted {
    file: Cursor<Vec<u8>>,
    counters: Arc<Counters>,
}

impl Read for Counted {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.counters.reads_before_failure.load(Ordering::Relaxed);
        if left == 0 {
            return Err(io::Error::other("the disk went away"));
        }
        if left != usize::MAX {
            self.counters
                .reads_before_failure
                .store(left - 1, Ordering::Relaxed);
        }
        let read = self.file.read(buf)?;
        self.counters.reads.fetch_add(1, Ordering::Relaxed);
        self.counters
            .bytes
            .fetch_add(u64::try_from(read).unwrap(), Ordering::Relaxed);
        Ok(read)
    }
}

impl Seek for Counted {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }
}

fn counted(file: Vec<u8>) -> (Box<dyn Source>, Arc<Counters>) {
    let counters = Arc::new(Counters {
        reads: AtomicUsize::new(0),
        bytes: AtomicU64::new(0),
        reads_before_failure: AtomicUsize::new(usize::MAX),
    });
    let source = Counted {
        file: Cursor::new(file),
        counters: Arc::clone(&counters),
    };
    (Box::new(source), counters)
}

/// Blocks in the documentation ranges of RFC 5737.
fn v4_blocks() -> Vec<(IpAddr, u8, CountryCode)> {
    vec![
        (ip("192.0.2.0"), 24, code("GB")),
        (ip("198.51.100.0"), 24, code("US")),
        (ip("203.0.113.0"), 25, code("DE")),
    ]
}

fn v4_db() -> CountryDb {
    open(build_country_file(Edition::V4, &v4_blocks()), Edition::V4).unwrap()
}

/// The documentation range of RFC 3849, and all of the IPv4-mapped addresses, as the newer IPv6
/// files have them.
fn v6_db() -> CountryDb {
    let blocks = [
        (ip("2001:db8::"), 32, code("DE")),
        (ip("::ffff:0:0"), 96, code("FR")),
    ];
    open(build_country_file(Edition::V6, &blocks), Edition::V6).unwrap()
}

/// A leaf record of the table entry at `index`.
fn leaf(index: u32) -> u32 {
    COUNTRY_BEGIN + index
}

const US: u32 = 225;
const DE: u32 = 56;

// --- The table -------------------------------------------------------------------------------

#[test]
fn table_has_255_entries() {
    let entries = (0..).map_while(country_codes::code_at).count();
    assert_eq!(entries, 255);
    assert_eq!(country_codes::TABLE_LEN, 255);
    assert_eq!(country_codes::code_at(255), None);
    assert_eq!(country_codes::code_at(u32::MAX), None);
}

#[test]
fn entry_zero_is_not_found() {
    assert_eq!(country_codes::code_at(0), Some("--"));
}

#[test]
fn every_other_entry_is_two_upper_case_letters_or_digits() {
    for index in 1..country_codes::TABLE_LEN {
        let entry = country_codes::code_at(index).unwrap();
        assert_eq!(entry.len(), 2, "entry {index}: {entry:?}");
        assert!(
            entry
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit()),
            "entry {index}: {entry:?}"
        );
    }
}

#[test]
fn entries_are_distinct() {
    let mut entries: Vec<&str> = (0..country_codes::TABLE_LEN)
        .filter_map(country_codes::code_at)
        .collect();
    entries.sort_unstable();
    entries.dedup();
    assert_eq!(entries.len(), 255);
}

/// The rule the table is built by: between the three regions and the codes appended later, the
/// ISO 3166-1 codes of 2003 in alphabetical order, five of them replaced in place by their
/// successors.
#[test]
fn iso_block_is_alphabetical_once_the_five_successors_are_put_back() {
    let replaced = [
        ("CW", "AN", 10),
        ("SX", "FX", 75),
        ("TL", "TP", 216),
        ("RS", "YU", 239),
        ("ME", "ZR", 242),
    ];
    for (successor, _, index) in replaced {
        assert_eq!(country_codes::code_at(index), Some(successor));
    }
    let of_2003: Vec<&str> = (3..=243)
        .map(|index| {
            let entry = country_codes::code_at(index).unwrap();
            replaced
                .iter()
                .find(|(successor, _, _)| *successor == entry)
                .map_or(entry, |(_, retired, _)| *retired)
        })
        .collect();
    assert_eq!(of_2003.first(), Some(&"AD"));
    assert_eq!(of_2003.last(), Some(&"ZW"));
    for pair in of_2003.windows(2) {
        assert!(pair[0] < pair[1], "{} before {}", pair[0], pair[1]);
    }
}

#[test]
fn regions_come_first_and_later_codes_last() {
    let first: Vec<_> = (0..3).filter_map(country_codes::code_at).collect();
    assert_eq!(first, ["--", "AP", "EU"]);
    let last: Vec<_> = (244..255).filter_map(country_codes::code_at).collect();
    assert_eq!(
        last,
        [
            "A1", "A2", "O1", "AX", "GG", "IM", "JE", "BL", "MF", "BQ", "SS"
        ]
    );
}

#[test]
fn pinned_indexes() {
    let pinned = [
        ("US", 225),
        ("RU", 185),
        ("DE", 56),
        ("GB", 77),
        ("CW", 10),
        ("SX", 75),
        ("AP", 1),
        ("EU", 2),
        ("A1", 244),
        ("A2", 245),
        ("O1", 246),
        ("AX", 247),
        ("SS", 254),
    ];
    for (entry, index) in pinned {
        assert_eq!(country_codes::code_at(index), Some(entry), "index {index}");
    }
}

#[test]
fn entries_that_are_not_countries_give_none() {
    for index in [0, 1, 2, 244, 245, 246, 255, 256, u32::MAX] {
        assert_eq!(country_codes::country(index), None, "index {index}");
    }
}

#[test]
fn every_other_entry_is_its_country() {
    for index in (3..244).chain(247..255) {
        let entry = country_codes::code_at(index).unwrap();
        assert_eq!(
            country_codes::country(index).map(|code| code.as_str().to_owned()),
            Some(entry.to_owned())
        );
    }
}

#[test]
fn index_of_finds_every_letter_entry() {
    assert_eq!(country_codes::index_of(code("US")), Some(225));
    assert_eq!(country_codes::index_of(code("EU")), Some(2));
    assert_eq!(country_codes::index_of(code("AP")), Some(1));
    assert_eq!(country_codes::index_of(code("SS")), Some(254));
    assert_eq!(country_codes::index_of(code("XK")), None);
}

#[test]
fn known_countries_are_the_table_without_the_non_countries() {
    let known: Vec<CountryCode> = known_countries().collect();
    assert_eq!(known.len(), 249);
    assert_eq!(known.first(), Some(&code("AD")));
    assert_eq!(known.last(), Some(&code("SS")));
    for not_a_country in ["AP", "EU"] {
        assert!(!known.contains(&code(not_a_country)));
    }
    assert!(known.contains(&code("CW")) && known.contains(&code("SX")));
}

// --- CountryCode -------------------------------------------------------------------------------

#[test]
fn country_code_is_two_upper_case_letters() {
    assert_eq!(code("US").as_str(), "US");
    for not_a_code in ["", "U", "us", "Us", "USA", "A1", "--", "\u{dc}S", "U "] {
        assert_eq!(CountryCode::new(not_a_code), None, "{not_a_code:?}");
    }
}

#[test]
fn country_code_debug_shows_its_letters() {
    assert_eq!(format!("{:?}", code("DE")), "CountryCode(\"DE\")");
}

// --- Builder and reader ------------------------------------------------------------------------

#[test]
fn a_hit_gives_the_country_of_its_block() {
    let mut db = v4_db();
    assert_eq!(db.lookup(ip("192.0.2.77")), Some(code("GB")));
    assert_eq!(db.lookup(ip("198.51.100.1")), Some(code("US")));
    assert_eq!(db.lookup(ip("203.0.113.100")), Some(code("DE")));
    assert!(!db.is_broken());
}

#[test]
fn a_miss_gives_none_and_the_database_stays_usable() {
    let mut db = v4_db();
    for miss in ["192.0.3.1", "8.8.8.8", "0.0.0.0", "255.255.255.255"] {
        assert_eq!(db.lookup(ip(miss)), None, "{miss}");
    }
    assert!(!db.is_broken());
    assert_eq!(db.lookup(ip("192.0.2.77")), Some(code("GB")));
}

#[test]
fn the_first_and_the_last_address_of_a_block_are_in_it() {
    let mut db = v4_db();
    assert_eq!(db.lookup(ip("192.0.2.0")), Some(code("GB")));
    assert_eq!(db.lookup(ip("192.0.2.255")), Some(code("GB")));
    assert_eq!(db.lookup(ip("192.0.1.255")), None);
    assert_eq!(db.lookup(ip("192.0.3.0")), None);
    assert_eq!(db.lookup(ip("203.0.113.127")), Some(code("DE")));
    assert_eq!(db.lookup(ip("203.0.113.128")), None);
}

#[test]
fn a_slash_one_is_a_leaf_at_depth_one() {
    let file = build_country_file(Edition::V4, &[(ip("128.0.0.0"), 1, code("DE"))]);
    let (source, counters) = counted(file);
    let mut db = CountryDb::open(source, Edition::V4).unwrap();
    for (address, country) in [
        ("128.0.0.0", Some(code("DE"))),
        ("200.1.2.3", Some(code("DE"))),
        ("255.255.255.255", Some(code("DE"))),
        ("127.255.255.255", None),
    ] {
        let before = counters.reads();
        assert_eq!(db.lookup(ip(address)), country, "{address}");
        assert_eq!(counters.reads() - before, 1, "{address}");
    }
}

#[test]
fn a_slash_32_is_found_at_full_depth() {
    let file = build_country_file(Edition::V4, &[(ip("203.0.113.7"), 32, code("RU"))]);
    let (source, counters) = counted(file);
    let mut db = CountryDb::open(source, Edition::V4).unwrap();
    let before = counters.reads();
    assert_eq!(db.lookup(ip("203.0.113.7")), Some(code("RU")));
    assert_eq!(counters.reads() - before, 32);
    assert_eq!(db.lookup(ip("203.0.113.6")), None);
    assert_eq!(db.lookup(ip("203.0.113.8")), None);
    assert!(!db.is_broken());
}

#[test]
fn a_slash_128_is_found_at_full_depth() {
    let file = build_country_file(Edition::V6, &[(ip("2001:db8::1"), 128, code("FR"))]);
    let (source, counters) = counted(file);
    let mut db = CountryDb::open(source, Edition::V6).unwrap();
    let before = counters.reads();
    assert_eq!(db.lookup(ip("2001:db8::1")), Some(code("FR")));
    assert_eq!(counters.reads() - before, 128);
    assert_eq!(db.lookup(ip("2001:db8::")), None);
    assert_eq!(db.lookup(ip("2001:db8::2")), None);
    assert!(!db.is_broken());
}

#[test]
fn not_found_and_entries_that_are_not_countries_give_none() {
    // "--", AP, EU, A1, A2, O1 and an index past the table, each in a /16 of 10.0.0.0/8.
    let indexes = [0, 1, 2, 244, 245, 246, 255];
    let mut blocks: Vec<(IpAddr, u8, u8)> = (0u8..)
        .zip(indexes)
        .map(|(second, index)| (IpAddr::V4(Ipv4Addr::new(10, second, 0, 0)), 16, index))
        .collect();
    blocks.push((ip("10.200.0.0"), 16, 225));
    let mut db = open(build_file(Edition::V4, &blocks, true), Edition::V4).unwrap();
    for second in 0..7 {
        let address = IpAddr::V4(Ipv4Addr::new(10, second, 1, 1));
        assert_eq!(db.lookup(address), None, "{address}");
    }
    assert!(!db.is_broken());
    assert_eq!(db.lookup(ip("10.200.1.1")), Some(code("US")));
}

#[test]
fn curacao_and_sint_maarten_keep_their_codes() {
    let blocks = [
        (ip("192.0.2.0"), 25, code("CW")),
        (ip("192.0.2.128"), 25, code("SX")),
    ];
    let mut db = open(build_country_file(Edition::V4, &blocks), Edition::V4).unwrap();
    assert_eq!(db.lookup(ip("192.0.2.1")), Some(code("CW")));
    assert_eq!(db.lookup(ip("192.0.2.129")), Some(code("SX")));
}

#[test]
fn the_builder_writes_the_index_of_a_code_and_an_unknown_code_as_not_found() {
    for (letters, index) in [("US", 225), ("CW", 10), ("SX", 75), ("EU", 2), ("XK", 0)] {
        let file = build_country_file(Edition::V4, &[(ip("128.0.0.0"), 1, code(letters))]);
        // One node: the record for a 0 bit says "not found", the one for a 1 bit the block's.
        let [_, _, _, last] = leaf(0).to_le_bytes();
        assert_eq!(last, 0);
        assert_eq!(file[..3], leaf(0).to_le_bytes()[..3], "{letters}");
        assert_eq!(file[3..6], leaf(index).to_le_bytes()[..3], "{letters}");
    }
}

#[test]
fn a_longer_prefix_wins_over_the_block_around_it_in_either_order() {
    let outer = (ip("10.0.0.0"), 8, code("US"));
    let inner = (ip("10.1.0.0"), 16, code("DE"));
    for blocks in [[outer, inner], [inner, outer]] {
        let mut db = open(build_country_file(Edition::V4, &blocks), Edition::V4).unwrap();
        assert_eq!(db.lookup(ip("10.1.2.3")), Some(code("DE")));
        assert_eq!(db.lookup(ip("10.1.255.255")), Some(code("DE")));
        assert_eq!(db.lookup(ip("10.0.255.255")), Some(code("US")));
        assert_eq!(db.lookup(ip("10.2.0.0")), Some(code("US")));
        assert_eq!(db.lookup(ip("11.0.0.0")), None);
    }
}

#[test]
fn a_slash_zero_covers_every_address() {
    let blocks = [
        (ip("192.0.2.0"), 24, code("GB")),
        (ip("0.0.0.0"), 0, code("US")),
    ];
    let mut db = open(build_country_file(Edition::V4, &blocks), Edition::V4).unwrap();
    assert_eq!(db.lookup(ip("0.0.0.0")), Some(code("US")));
    assert_eq!(db.lookup(ip("255.255.255.255")), Some(code("US")));
    assert_eq!(db.lookup(ip("192.0.2.1")), Some(code("GB")));
}

#[test]
fn the_host_bits_of_a_block_do_not_matter() {
    let blocks = [(ip("192.0.2.77"), 24, code("GB"))];
    let mut db = open(build_country_file(Edition::V4, &blocks), Edition::V4).unwrap();
    assert_eq!(db.lookup(ip("192.0.2.1")), Some(code("GB")));
}

#[test]
fn a_prefix_longer_than_the_address_is_the_whole_address() {
    let blocks = [(ip("203.0.113.7"), 40, code("RU"))];
    let mut db = open(build_country_file(Edition::V4, &blocks), Edition::V4).unwrap();
    assert_eq!(db.lookup(ip("203.0.113.7")), Some(code("RU")));
    assert_eq!(db.lookup(ip("203.0.113.6")), None);
    let blocks = [(ip("2001:db8::1"), 200, code("FR"))];
    let mut db = open(build_country_file(Edition::V6, &blocks), Edition::V6).unwrap();
    assert_eq!(db.lookup(ip("2001:db8::1")), Some(code("FR")));
    assert_eq!(db.lookup(ip("2001:db8::")), None);
}

#[test]
fn blocks_of_the_other_family_are_left_out() {
    let v4 = (ip("192.0.2.0"), 24, code("GB"));
    let v6 = (ip("2001:db8::"), 32, code("DE"));
    assert_eq!(
        build_country_file(Edition::V4, &[v6, v4]),
        build_country_file(Edition::V4, &[v4])
    );
    assert_eq!(
        build_country_file(Edition::V6, &[v4, v6]),
        build_country_file(Edition::V6, &[v6])
    );
}

#[test]
fn a_file_without_blocks_knows_no_address() {
    let mut db = open(build_country_file(Edition::V4, &[]), Edition::V4).unwrap();
    assert_eq!(db.lookup(ip("192.0.2.1")), None);
    let mut db = open(build_country_file(Edition::V6, &[]), Edition::V6).unwrap();
    assert_eq!(db.lookup(ip("2001:db8::1")), None);
    assert!(!db.is_broken());
}

#[test]
fn the_file_ends_with_the_info_the_marker_the_type_and_the_node_count() {
    for (edition, type_byte) in [(Edition::V4, 1), (Edition::V6, 12)] {
        let block = match edition {
            Edition::V4 => ip("128.0.0.0"),
            Edition::V6 => ip("8000::"),
        };
        let file = build_country_file(edition, &[(block, 1, code("DE"))]);
        let mut expected = file[..6].to_vec();
        expected.extend([0, 0, 0]);
        expected.extend(INFO.as_bytes());
        expected.extend([0xFF, 0xFF, 0xFF, type_byte, 1, 0, 0]);
        assert_eq!(file, expected, "{edition}");
    }
}

#[test]
fn both_trailer_forms_open_with_the_info_of_the_build() {
    let with_count = build_file(Edition::V4, &[(ip("192.0.2.0"), 24, 77)], true);
    let without = build_file(Edition::V4, &[(ip("192.0.2.0"), 24, 77)], false);
    assert_eq!(with_count.len(), without.len() + 3);
    for file in [with_count, without] {
        let mut db = open(file, Edition::V4).unwrap();
        assert_eq!(db.info(), INFO);
        assert!(db.info().starts_with("4x4"));
        assert_eq!(db.lookup(ip("192.0.2.1")), Some(code("GB")));
    }
}

#[test]
fn the_info_keeps_only_printable_ascii() {
    let mut file = encode(&[[leaf(US), leaf(US)]], Edition::V4, true);
    let marker = file.len() - 7;
    // Replace the info text with one that has a line break, a control character and UTF-8.
    file.splice(9..marker, "Build\n1\u{7}é".bytes());
    let db = open(file, Edition::V4).unwrap();
    assert_eq!(db.info(), "Build?1???");
}

// --- Routing -----------------------------------------------------------------------------------

#[test]
fn ipv4_mapped_ipv6_goes_to_the_v4_database() {
    let mut geo = GeoIp::new(Some(v4_db()), Some(v6_db()));
    assert_eq!(geo.country(ip("::ffff:192.0.2.1")), Some(code("GB")));
    assert_eq!(geo.country(ip("192.0.2.1")), Some(code("GB")));
    // Not in the v4 database, so no country, although the v6 database would say FR.
    assert_eq!(geo.country(ip("::ffff:8.8.8.8")), None);
    assert_eq!(geo.country(ip("2001:db8::1")), Some(code("DE")));
}

#[test]
fn ipv4_is_never_looked_up_in_the_v6_database() {
    let mut geo = GeoIp::new(None, Some(v6_db()));
    assert_eq!(geo.country(ip("192.0.2.1")), None);
    assert_eq!(geo.country(ip("::ffff:192.0.2.1")), None);
    assert_eq!(geo.country(ip("2001:db8::1")), Some(code("DE")));
    let mut v6 = v6_db();
    assert_eq!(v6.lookup(ip("192.0.2.1")), None);
    assert!(!v6.is_broken());
}

#[test]
fn only_a_v4_database_knows_only_ipv4() {
    let mut geo = GeoIp::new(Some(v4_db()), None);
    assert_eq!(geo.country(ip("192.0.2.1")), Some(code("GB")));
    assert_eq!(geo.country(ip("2001:db8::1")), None);
}

#[test]
fn only_a_v6_database_knows_only_ipv6() {
    let mut geo = GeoIp::new(None, Some(v6_db()));
    assert_eq!(geo.country(ip("2001:db8::1")), Some(code("DE")));
    assert_eq!(geo.country(ip("198.51.100.1")), None);
}

#[test]
fn no_database_knows_no_country() {
    let mut geo = GeoIp::none();
    assert_eq!(geo.country(ip("192.0.2.1")), None);
    assert_eq!(geo.country(ip("2001:db8::1")), None);
}

#[test]
fn the_v4_database_takes_ipv4_mapped_addresses_and_reads_nothing_for_others() {
    let (source, counters) = counted(build_country_file(Edition::V4, &v4_blocks()));
    let mut db = CountryDb::open(source, Edition::V4).unwrap();
    assert_eq!(db.lookup(ip("::ffff:192.0.2.1")), Some(code("GB")));
    let before = counters.reads();
    assert_eq!(db.lookup(ip("2001:db8::1")), None);
    assert_eq!(counters.reads(), before);
    assert!(!db.is_broken());
}

// --- Checks on opening -------------------------------------------------------------------------

#[test]
fn a_file_shorter_than_10_bytes_is_too_short() {
    for size in [0, 1, 9] {
        let error = open(vec![0xFF; size], Edition::V4).unwrap_err();
        assert!(
            matches!(error, GeoIpError::TooShort { size: found } if found == u64::try_from(size).unwrap()),
            "{error:?}"
        );
    }
}

#[test]
fn the_smallest_file_opens() {
    // One node, the marker and the type: no delimiter, no info, no node count.
    let mut file = Vec::new();
    file.extend(&leaf(US).to_le_bytes()[..3]);
    file.extend(&leaf(DE).to_le_bytes()[..3]);
    file.extend([0xFF, 0xFF, 0xFF, 1]);
    assert_eq!(file.len(), 10);
    let mut db = open(file, Edition::V4).unwrap();
    assert_eq!(db.info(), "");
    assert_eq!(db.lookup(ip("1.2.3.4")), Some(code("US")));
    assert_eq!(db.lookup(ip("200.1.2.3")), Some(code("DE")));
}

#[test]
fn a_file_without_the_marker_in_its_last_22_bytes_is_rejected() {
    let error = open(vec![0; 64], Edition::V4).unwrap_err();
    assert!(matches!(error, GeoIpError::NoMarker), "{error:?}");
    // The marker of a file without a node count starts 4 bytes before its end: 18 more bytes
    // still leave it in the last 22, 19 do not.
    let mut file = build_file(Edition::V4, &v4_blocks_indexed(), false);
    file.extend([0; 18]);
    let error = open(file.clone(), Edition::V4).unwrap_err();
    assert!(
        matches!(error, GeoIpError::CutTrailer { after_marker: 19 }),
        "{error:?}"
    );
    file.push(0);
    let error = open(file, Edition::V4).unwrap_err();
    assert!(matches!(error, GeoIpError::NoMarker), "{error:?}");
}

fn v4_blocks_indexed() -> Vec<(IpAddr, u8, u8)> {
    vec![(ip("192.0.2.0"), 24, 77), (ip("198.51.100.0"), 24, 225)]
}

#[test]
fn the_latest_marker_wins() {
    // A marker and the IPv6 type in the info text come before the real marker and type.
    let mut file = encode(&[[leaf(US), leaf(DE)]], Edition::V4, false);
    let marker = file.len() - 4;
    file.splice(marker..marker, [0xFF, 0xFF, 0xFF, 12]);
    let mut db = open(file, Edition::V4).unwrap();
    assert_eq!(db.lookup(ip("1.2.3.4")), Some(code("US")));
}

#[test]
fn a_file_of_the_other_edition_is_rejected_with_its_type() {
    let v6 = build_country_file(Edition::V6, &[]);
    let error = open(v6, Edition::V4).unwrap_err();
    assert!(
        matches!(
            error,
            GeoIpError::WrongEdition {
                edition: Edition::V4,
                found: 12
            }
        ),
        "{error:?}"
    );
    let v4 = build_country_file(Edition::V4, &[]);
    let error = open(v4, Edition::V6).unwrap_err();
    assert!(
        matches!(
            error,
            GeoIpError::WrongEdition {
                edition: Edition::V6,
                found: 1
            }
        ),
        "{error:?}"
    );
}

#[test]
fn a_database_that_is_not_of_countries_is_rejected() {
    // City, region, proxy, netspeed, large country, and a country file from before 2003.
    for type_byte in [0, 2, 3, 8, 10, 17, 18, 106, 254] {
        let mut file = build_country_file(Edition::V4, &v4_blocks());
        let at = file.len() - 4;
        file[at] = type_byte;
        let error = open(file, Edition::V4).unwrap_err();
        assert!(
            matches!(error, GeoIpError::WrongEdition { found, .. } if found == type_byte),
            "{type_byte}: {error:?}"
        );
    }
}

#[test]
fn a_cut_or_extended_trailer_is_rejected() {
    let file = build_country_file(Edition::V4, &v4_blocks());
    for (cut, after_marker) in [(1, 3), (2, 2), (4, 0)] {
        let error = open(file[..file.len() - cut].to_vec(), Edition::V4).unwrap_err();
        assert!(
            matches!(error, GeoIpError::CutTrailer { after_marker: found } if found == after_marker),
            "cut by {cut}: {error:?}"
        );
    }
    let mut longer = file.clone();
    longer.push(0);
    let error = open(longer, Edition::V4).unwrap_err();
    assert!(
        matches!(error, GeoIpError::CutTrailer { after_marker: 5 }),
        "{error:?}"
    );
    // Cut by exactly the node count, the file ends after its type, as files may.
    let mut db = open(file[..file.len() - 3].to_vec(), Edition::V4).unwrap();
    assert_eq!(db.lookup(ip("192.0.2.1")), Some(code("GB")));
}

#[test]
fn a_node_count_out_of_range_is_rejected() {
    let file = build_country_file(Edition::V4, &v4_blocks());
    let marker = u64::try_from(file.len() - 7).unwrap();
    let with_count = |count: u32| {
        let mut file = file.clone();
        let at = file.len() - 3;
        file[at..].copy_from_slice(&count.to_le_bytes()[..3]);
        file
    };
    let room = u32::try_from(marker / 6).unwrap();
    // Not 0xFF_FFFF: that would end the file with a marker.
    for count in [0, room + 1, 0x7F_FFFF] {
        let error = open(with_count(count), Edition::V4).unwrap_err();
        assert!(
            matches!(error, GeoIpError::NodeCount { count: found, room } if found == count && room == marker),
            "{count}: {error:?}"
        );
    }
    // As many nodes as fit before the marker are accepted: the walks never reach the extra ones.
    let mut db = open(with_count(room), Edition::V4).unwrap();
    assert_eq!(db.lookup(ip("192.0.2.1")), Some(code("GB")));
}

#[test]
fn a_probe_walk_that_leaves_the_file_is_rejected() {
    for (edition, nodes, with_count) in [
        // The lowest address goes left, past the only node.
        (Edition::V4, [[1, leaf(US)]], true),
        // The highest goes right, far past the end.
        (Edition::V4, [[leaf(US), 1000]], false),
        (Edition::V6, [[leaf(US), 1]], true),
    ] {
        let error = open(encode(&nodes, edition, with_count), edition).unwrap_err();
        assert!(matches!(error, GeoIpError::ProbeFailed), "{error:?}");
    }
}

#[test]
fn a_probe_walk_that_never_ends_is_rejected() {
    for edition in [Edition::V4, Edition::V6] {
        // The root points to itself for a 0 bit: the walk runs out of bits.
        let (source, counters) = counted(encode(&[[0, leaf(US)]], edition, true));
        let error = CountryDb::open(source, edition).unwrap_err();
        assert!(matches!(error, GeoIpError::ProbeFailed), "{error:?}");
        // The end of the file, then one read per bit, and no more.
        assert_eq!(
            counters.reads(),
            1 + usize::try_from(edition.bits()).unwrap(),
            "{edition}"
        );
    }
}

#[test]
fn a_read_error_while_opening_is_reported() {
    for reads in [0, 1] {
        let (source, counters) = counted(build_country_file(Edition::V4, &v4_blocks()));
        counters.fail_after(reads);
        let error = CountryDb::open(source, Edition::V4).unwrap_err();
        assert!(matches!(error, GeoIpError::Read(_)), "{error:?}");
    }
}

#[test]
fn opening_reads_the_end_of_the_file_and_two_walks_only() {
    // 4096 blocks of /24 make a file of about 25 KB.
    let blocks: Vec<(IpAddr, u8, CountryCode)> = (0..16)
        .flat_map(|second| {
            (0..=255).map(move |third| {
                (
                    IpAddr::V4(Ipv4Addr::new(10, second, third, 0)),
                    24,
                    code("US"),
                )
            })
        })
        .collect();
    let file = build_country_file(Edition::V4, &blocks);
    assert!(file.len() > 20_000, "{}", file.len());
    let (source, counters) = counted(file);
    let mut db = CountryDb::open(source, Edition::V4).unwrap();
    assert!(counters.bytes() <= 150 + 2 * 32 * 6, "{}", counters.bytes());
    let before = counters.bytes();
    assert_eq!(db.lookup(ip("10.5.6.7")), Some(code("US")));
    assert_eq!(counters.bytes() - before, 24 * 6);
}

// --- Lookups in corrupt files ------------------------------------------------------------------

#[test]
fn a_pointer_past_the_end_breaks_the_database() {
    // Both probes end in a leaf; an address that starts with 1 then 0 meets node 99.
    let file = encode(&[[leaf(US), 1], [99, leaf(DE)]], Edition::V4, true);
    let (source, counters) = counted(file);
    let mut db = CountryDb::open(source, Edition::V4).unwrap();
    assert_eq!(db.lookup(ip("255.0.0.0")), Some(code("DE")));
    assert_eq!(db.lookup(ip("128.0.0.0")), None);
    assert!(db.is_broken());
    // A broken database answers nothing more, and reads nothing more.
    let before = counters.reads();
    assert_eq!(db.lookup(ip("255.0.0.0")), None);
    assert_eq!(db.lookup(ip("1.2.3.4")), None);
    assert_eq!(counters.reads(), before);
}

#[test]
fn a_node_pointing_to_itself_breaks_the_database_after_one_read_per_bit() {
    let file = encode(&[[leaf(US), 1], [1, leaf(DE)]], Edition::V4, true);
    let (source, counters) = counted(file);
    let mut db = CountryDb::open(source, Edition::V4).unwrap();
    let before = counters.reads();
    assert_eq!(db.lookup(ip("128.0.0.0")), None);
    assert_eq!(counters.reads() - before, 32);
    assert!(db.is_broken());
}

#[test]
fn a_read_error_in_the_middle_of_a_walk_breaks_the_database() {
    let file = build_country_file(Edition::V4, &[(ip("203.0.113.7"), 32, code("RU"))]);
    let (source, counters) = counted(file);
    let mut db = CountryDb::open(source, Edition::V4).unwrap();
    counters.fail_after(10);
    assert_eq!(db.lookup(ip("203.0.113.7")), None);
    assert!(db.is_broken());
    counters.fail_after(usize::MAX);
    assert_eq!(db.lookup(ip("203.0.113.7")), None);
}

#[test]
fn every_lookup_reads_the_file_again() {
    let (source, counters) = counted(build_country_file(Edition::V4, &v4_blocks()));
    let mut db = CountryDb::open(source, Edition::V4).unwrap();
    let start = counters.reads();
    assert_eq!(db.lookup(ip("192.0.2.1")), Some(code("GB")));
    let once = counters.reads() - start;
    assert_eq!(db.lookup(ip("192.0.2.1")), Some(code("GB")));
    assert_eq!(counters.reads() - start, 2 * once);
}

/// A small generator (`SplitMix64`) with a fixed seed, so that a failure can be replayed.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut mixed = self.0;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^ (mixed >> 31)
    }

    fn below(&mut self, bound: u32) -> u32 {
        u32::try_from(self.next() % u64::from(bound)).unwrap()
    }

    /// A record of a tree of `nodes` nodes: a leaf, a node of the tree, a node just past it, or
    /// anything at all.
    fn record(&mut self, nodes: u32) -> u32 {
        match self.below(100) {
            0..40 => leaf(self.below(256)),
            40..85 => self.below(nodes),
            85..95 => nodes + self.below(4),
            _ => self.below(1 << 24),
        }
    }

    fn address(&mut self) -> IpAddr {
        let bits = u128::from(self.next()) << 64 | u128::from(self.next());
        match self.below(3) {
            0 => IpAddr::V4(Ipv4Addr::from(u32::try_from(bits >> 96).unwrap())),
            1 => IpAddr::V6(Ipv4Addr::from(u32::try_from(bits >> 96).unwrap()).to_ipv6_mapped()),
            _ => IpAddr::V6(Ipv6Addr::from(bits)),
        }
    }
}

#[test]
fn random_bytes_with_a_valid_trailer_never_panic() {
    let known: Vec<CountryCode> = known_countries().collect();
    let mut random = Random(0x0404_0404);
    let (mut opened, mut lookups, mut node_reads) = (0, 0, 0);
    for round in 0..4000 {
        let edition = if round % 2 == 0 {
            Edition::V4
        } else {
            Edition::V6
        };
        let nodes = 1 + random.below(48);
        let tree: Vec<[u32; 2]> = (0..nodes)
            .map(|_| [random.record(nodes), random.record(nodes)])
            .collect();
        let mut file = encode(&tree, edition, round % 3 != 0);
        if round % 5 == 0 {
            // Flip one bit anywhere, the trailer included.
            let at = usize::try_from(random.below(u32::try_from(file.len()).unwrap())).unwrap();
            file[at] ^= 1 << random.below(8);
        }
        let (source, counters) = counted(file);
        let Ok(mut db) = CountryDb::open(source, edition) else {
            continue;
        };
        opened += 1;
        let after_open = counters.reads();
        for _ in 0..16 {
            if let Some(country) = db.lookup(random.address()) {
                assert!(known.contains(&country), "{country:?}");
            }
            lookups += 1;
        }
        node_reads += counters.reads() - after_open;
    }
    // Bytes that are random throughout, of any length, are opened safely too.
    for round in 0..2000 {
        let length = random.below(200);
        let file: Vec<u8> = (0..length)
            .map(|_| u8::try_from(random.below(256)).unwrap())
            .collect();
        let edition = if round % 2 == 0 {
            Edition::V4
        } else {
            Edition::V6
        };
        if let Ok(mut db) = open(file, edition) {
            for _ in 0..16 {
                if let Some(country) = db.lookup(random.address()) {
                    assert!(known.contains(&country), "{country:?}");
                }
                lookups += 1;
            }
        }
    }
    assert!(opened > 1500, "{opened} of 4000 opened");
    assert!(lookups > 25_000, "{lookups} lookups");
    assert!(node_reads > 20_000, "{node_reads} node reads");
}
