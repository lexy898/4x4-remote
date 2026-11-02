//! Hostile input: the reader refuses it with an error, and never panics, overflows the stack or
//! holds much more memory than the file.

use std::thread;

use x4_core::{MAX_TORRENT_FILE_BYTES, MetainfoError, ParsedTorrent, parse_torrent};

use super::writer::{Bencode, bytes, dict, int, list, raw};
use super::{PIECE, file, folder_info, pad, single_info, torrent};

/// A stack far smaller than the default, so that reading with recursion would overflow it.
const SMALL_STACK: usize = 64 * 1024;

/// Reads `file` on a thread with [`SMALL_STACK`]. Overflowing that stack aborts the whole test
/// binary, so a reader that recursed with the nesting would fail the test, not pass it by luck.
#[expect(
    clippy::expect_used,
    reason = "a test helper: a failed spawn or a panic in the reader fails the test"
)]
fn parse_on_small_stack(file: Vec<u8>) -> Result<ParsedTorrent, MetainfoError> {
    thread::Builder::new()
        .stack_size(SMALL_STACK)
        .spawn(move || parse_torrent(&file))
        .expect("the thread starts")
        .join()
        .expect("the reader does not panic")
}

/// `prefix`, then `depth` times `open`, then `depth` times `e`, then `suffix`.
fn nested(prefix: &[u8], open: &[u8], depth: usize, suffix: &[u8]) -> Vec<u8> {
    let mut file = prefix.to_vec();
    file.extend(open.repeat(depth));
    file.extend(b"e".repeat(depth));
    file.extend_from_slice(suffix);
    file
}

#[test]
fn deep_nesting_is_refused_without_overflow() {
    const DEPTH: usize = 100_000;
    let cases = [
        ("lists", nested(b"", b"l", DEPTH, b"")),
        ("dictionaries", nested(b"", b"d1:a", DEPTH, b"")),
        ("lists left open", b"l".repeat(DEPTH)),
        ("inside info", nested(b"d4:infod1:x", b"l", DEPTH, b"ee")),
        (
            "inside an entry",
            nested(b"d4:infod5:filesld4:pathl", b"l", DEPTH, b"eeee"),
        ),
    ];
    for (case, file) in cases {
        let result = parse_on_small_stack(file);
        assert!(
            matches!(result, Err(MetainfoError::TooDeep { .. })),
            "{case}: {result:?}"
        );
    }

    // A good torrent reads on the same small stack.
    let good = torrent(folder_info("Top", vec![file(&["a"], 1)], 1));
    assert!(parse_on_small_stack(good).unwrap().files.is_some());
}

#[test]
fn nesting_up_to_31_levels_is_read() {
    // The top dictionary is the first level; `x` holds the other 30, or 31 to go too deep.
    let with_depth = |levels: usize| {
        let info = single_info("a.bin", 10).encode();
        let mut file = b"d4:info".to_vec();
        file.extend_from_slice(&info);
        file.extend_from_slice(b"1:x");
        file.extend(nested(b"", b"l", levels, b"e"));
        file
    };

    assert!(parse_torrent(&with_depth(30)).unwrap().files.is_some());
    assert!(matches!(
        parse_torrent(&with_depth(31)),
        Err(MetainfoError::TooDeep { .. })
    ));
}

#[test]
fn huge_lengths_and_overflowing_integers_are_refused() {
    let not_bencode = |value: &[u8], case: &str| {
        let info = folder_info(
            "Top",
            vec![dict([("length", raw(value)), ("path", list([bytes("a")]))])],
            1,
        );
        let result = parse_torrent(&torrent(info));
        assert!(
            matches!(result, Err(MetainfoError::NotBencode { .. })),
            "{case}: {result:?}"
        );
    };
    not_bencode(b"i9223372036854775808e", "one past i64::MAX");
    not_bencode(b"i-9223372036854775809e", "one below i64::MIN");
    not_bencode(b"i99999999999999999999999999e", "far beyond i64");
    not_bencode(b"i03e", "a leading zero");
    not_bencode(b"i-03e", "a negative leading zero");
    not_bencode(b"i-e", "a sign without digits");
    not_bencode(b"ie", "no digits");
    not_bencode(b"i+3e", "a plus sign");
    not_bencode(b"99999999999999999999999:", "a string length beyond usize");
    not_bencode(b"134217728:", "a string of 128 MiB");
    not_bencode(b"1000:abc", "a string longer than the file");

    // Lengths that fit in an i64 but not, added up, in a u64.
    let huge = vec![
        file(&["a"], i64::MAX),
        file(&["b"], i64::MAX),
        file(&["c"], i64::MAX),
    ];
    let info = dict([
        ("files", Bencode::List(huge)),
        ("name", bytes("Top")),
        ("piece length", int(PIECE)),
        ("pieces", bytes([0; 20])),
    ]);
    assert!(matches!(
        parse_torrent(&torrent(info)),
        Err(MetainfoError::Invalid(_))
    ));

    // A huge length that no `pieces` within the size limit can cover.
    let info = dict([
        ("length", int(i64::MAX)),
        ("name", bytes("a.bin")),
        ("piece length", int(i64::from(u32::MAX))),
        ("pieces", bytes([0; 20])),
    ]);
    assert!(matches!(
        parse_torrent(&torrent(info)),
        Err(MetainfoError::Invalid(_))
    ));
}

#[test]
fn truncated_input_is_an_error() {
    let file = torrent(folder_info(
        "Top",
        vec![file(&["a"], 1), pad(PIECE - 1), file(&["b"], 2)],
        PIECE + 2,
    ));
    assert!(parse_torrent(&file).unwrap().files.is_some());

    for end in 0..file.len() {
        let result = parse_torrent(&file[..end]);
        assert!(
            matches!(result, Err(MetainfoError::NotBencode { offset }) if offset <= end),
            "cut at {end}: {result:?}"
        );
    }
}

#[test]
fn a_file_over_16_mib_is_refused_before_parsing() {
    let limit = usize::try_from(MAX_TORRENT_FILE_BYTES).unwrap();

    // Exactly the limit is read: what follows the top dictionary is ignored, as the daemon does.
    let mut file = torrent(single_info("a.bin", 10));
    file.resize(limit, b'x');
    assert!(parse_torrent(&file).unwrap().files.is_some());

    file.push(b'x');
    assert_eq!(
        parse_torrent(&file),
        Err(MetainfoError::TooLarge {
            size: MAX_TORRENT_FILE_BYTES + 1
        })
    );
}

/// `SplitMix64`: a small deterministic generator, so a failing mutation can be found again.
struct SplitMix(u64);

impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    /// A number below `bound`, which must not be 0.
    fn below(&mut self, bound: usize) -> usize {
        usize::try_from(self.next() % u64::try_from(bound).unwrap_or(u64::MAX)).unwrap_or(0)
    }
}

/// The bytes that change the structure of bencode the most.
const STRUCTURAL: &[u8] = b"deil0123456789:-";

/// `original` with one to four random changes: a byte replaced (by any byte or a structural
/// one), removed or inserted, a run copied elsewhere, two bytes swapped, or the end cut off.
fn mutate(original: &[u8], random: &mut SplitMix) -> Vec<u8> {
    let mut file = original.to_vec();
    for _ in 0..=random.below(4) {
        if file.is_empty() {
            break;
        }
        let at = random.below(file.len());
        let structural = STRUCTURAL
            .get(random.below(STRUCTURAL.len()))
            .copied()
            .unwrap_or(b'e');
        match random.below(7) {
            0 => {
                if let Some(byte) = file.get_mut(at) {
                    *byte = random.next().to_le_bytes()[0];
                }
            }
            1 => {
                if let Some(byte) = file.get_mut(at) {
                    *byte = structural;
                }
            }
            2 => {
                let end = (at + 1 + random.below(8)).min(file.len());
                file.drain(at..end);
            }
            3 => file.insert(at, structural),
            4 => {
                let end = (at + 1 + random.below(16)).min(file.len());
                let run = file.get(at..end).map(<[u8]>::to_vec).unwrap_or_default();
                let to = random.below(file.len());
                file.splice(to..to, run);
            }
            5 => {
                let other = random.below(file.len());
                file.swap(at, other);
            }
            _ => file.truncate(at),
        }
    }
    file
}

#[test]
fn random_mutations_never_panic() {
    const MUTATIONS: usize = 25_000;
    let entries = vec![
        file(&["b.txt"], 3),
        pad(PIECE - 3),
        dict([
            ("attr", bytes("x")),
            ("length", int(5000)),
            ("md5sum", bytes("0123456789abcdef0123456789abcdef")),
            ("path", list([bytes("sub"), bytes("a.txt")])),
            ("path.utf-8", list([bytes("sub"), bytes("ä.txt")])),
        ]),
        file(&["c.txt"], 0),
    ];
    let info = dict([
        ("files", Bencode::List(entries)),
        ("name", bytes("Top")),
        ("name.utf-8", bytes("Töp")),
        ("piece length", int(PIECE)),
        ("pieces", bytes([0xab; 40])),
        ("private", int(1)),
    ]);
    let original = dict([
        ("announce", bytes("http://tracker.test/announce")),
        (
            "announce-list",
            list([
                list([bytes("http://a.test/")]),
                list([bytes("http://b.test/")]),
            ]),
        ),
        ("creation date", int(1_700_000_000)),
        ("info", info),
    ])
    .encode();
    assert!(parse_torrent(&original).unwrap().files.is_some());

    let mut random = SplitMix(0x4b34_7834_2d73_3561);
    let (mut with_choice, mut without_choice, mut refused) = (0, 0, 0);
    for round in 0..MUTATIONS {
        let file = mutate(&original, &mut random);
        match parse_torrent(&file) {
            Ok(parsed) => {
                let hash = parsed.hash.as_str();
                assert!(
                    hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
                    "round {round}: {hash}"
                );
                assert_eq!(hash, hash.to_ascii_lowercase(), "round {round}");
                if let Some(files) = &parsed.files {
                    assert!(!files.is_empty(), "round {round}");
                    let sum = files.iter().map(|file| file.length).sum::<u64>();
                    assert_eq!(parsed.total_size, Some(sum), "round {round}");
                    with_choice += 1;
                } else {
                    assert_eq!(parsed.total_size, None, "round {round}");
                    without_choice += 1;
                }
            }
            Err(_) => refused += 1,
        }
    }

    // The mutations reach every outcome, so they exercise more than the first syntax check.
    assert!(
        with_choice > 0 && without_choice > 0 && refused > 0,
        "{with_choice} with a choice, {without_choice} without, {refused} refused"
    );
}

#[test]
fn the_path_limit_stops_the_reader_at_the_file_that_passes_it() {
    // 4 MiB that are not UTF-8 make a 12 MiB name, so the sixth path passes 64 MiB. The reader
    // stops there, before it builds any path: the entry after it, which would leave no choice of
    // files, is never read.
    let name = vec![0xff; 4 * 1024 * 1024];
    let mut entries: Vec<_> = (0..6).map(|index| file(&[&index.to_string()], 1)).collect();
    entries.push(bytes("not an entry"));
    let info = dict([
        ("files", Bencode::List(entries)),
        ("name", bytes(&name)),
        ("piece length", int(PIECE)),
        ("pieces", bytes([0; 20])),
    ]);

    // Only the file count is compared, so a failure does not print the 12 MiB name.
    assert_eq!(
        parse_torrent(&torrent(info)).map(|parsed| parsed.files.map(|files| files.len())),
        Err(MetainfoError::Invalid(
            "the name and the file paths are too long"
        ))
    );
}

#[test]
fn a_long_name_over_many_files_is_refused_before_it_bloats() {
    // The name is the folder of every file: 4 MiB in front of 20 paths would be 80 MiB of text
    // from a file of 4 MiB.
    let name = "n".repeat(4 * 1024 * 1024);
    let entries = (0..20)
        .map(|index| file(&[&index.to_string()], 1))
        .collect();
    let result = parse_torrent(&torrent(folder_info(&name, entries, 20)));
    assert!(
        matches!(result, Err(MetainfoError::Invalid(_))),
        "{:?}",
        result.map(|parsed| parsed.files.map(|files| files.len()))
    );

    // A single file is named by the name too: 12 MiB that are not UTF-8 make 36 MiB of name
    // and 36 MiB of path.
    let info = dict([
        ("length", int(1)),
        ("name", bytes(vec![0xff; 12 * 1024 * 1024])),
        ("piece length", int(PIECE)),
        ("pieces", bytes([0; 20])),
    ]);
    assert_eq!(
        parse_torrent(&torrent(info)).map(|parsed| parsed.files.map(|files| files.len())),
        Err(MetainfoError::Invalid(
            "the name and the file paths are too long"
        ))
    );

    // The same name over a few files is read.
    let entries = (0..10)
        .map(|index| file(&[&index.to_string()], 1))
        .collect();
    let parsed = parse_torrent(&torrent(folder_info(&name, entries, 10))).unwrap();
    assert_eq!(parsed.files.map(|files| files.len()), Some(10));
}
