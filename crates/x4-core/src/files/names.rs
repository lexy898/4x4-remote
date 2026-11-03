//! How the names of a torrent's files are read: which files only pad, and in which order a
//! folder lists its entries.

use std::cmp::Ordering;

/// The start of the name of a padding file that most clients write.
const PADDING_PREFIX: &str = "_____padding_file_";

/// The folder below the torrent's top folder that holds padding files (BEP 47).
const PAD_FOLDER: &str = ".pad";

/// Whether the file with these path components, the last one being its name, only pads the
/// pieces to a boundary: its name starts with `_____padding_file_`, or its first folder below the
/// torrent's top folder is `.pad`.
///
/// Transmission lists such a file like any other, so the details hold it, but a user has nothing
/// to do with it. The components are read one by one, and not kept: a path may have millions.
pub(super) fn is_padding<'a>(parts: impl Iterator<Item = &'a str>) -> bool {
    let (mut count, mut second, mut name) = (0_usize, None, None);
    for part in parts {
        count = count.saturating_add(1);
        if count == 2 {
            second = Some(part);
        }
        name = Some(part);
    }
    let Some(name) = name else {
        return false;
    };
    // A `.pad` below the top folder has to be a folder, so the path goes on after it.
    name.starts_with(PADDING_PREFIX) || (count >= 3 && second == Some(PAD_FOLDER))
}

/// The most runs of digits, or of other characters, that two names are compared by in their
/// natural order; what follows them is compared byte by byte. A real name has far fewer.
const NATURAL_RUNS: usize = 32;

/// The most bytes at the start of a name that are compared in their natural order and without
/// case; what follows them is compared byte by byte. A file system allows 255 for a name.
const NATURAL_BYTES: usize = 256;

/// How two names in the same folder are ordered: digits compare by number, so `E2` comes before
/// `E10`, and letters compare ignoring case.
///
/// Names that this treats as equal (`a1` and `a01`, `A` and `a`) are ordered by their text
/// without case, and then exactly, so the order is the same for the same names whatever order
/// they came in.
///
/// Only the first [`NATURAL_RUNS`] runs of digits and of other characters within the first
/// [`NATURAL_BYTES`] of a name compare that way; the rest of it compares byte by byte. So one
/// comparison takes a bounded time, however long the names: 14 000 crafted names of 600 runs
/// each took seconds to sort. The order stays total and the same for the same names, and real
/// names, shorter than both bounds, are ordered as if there were none.
pub(super) fn name_order(a: &str, b: &str) -> Ordering {
    natural_order(a, b)
        .then_with(|| folded_order(head(a), head(b)))
        .then_with(|| a.cmp(b))
}

/// The start of `name` that is compared in its natural order: its first [`NATURAL_BYTES`],
/// fewer when they would cut a character.
fn head(name: &str) -> &str {
    name.get(..name.floor_char_boundary(NATURAL_BYTES))
        .unwrap_or(name)
}

/// The characters of `text` in lower case.
fn folded(text: &str) -> impl Iterator<Item = char> {
    text.chars().flat_map(char::to_lowercase)
}

/// How `a` and `b` compare in lower case: as their [`folded`] characters do, but byte by byte as
/// long as both are ASCII, which most names are throughout.
fn folded_order(a: &str, b: &str) -> Ordering {
    let differ = a
        .bytes()
        .zip(b.bytes())
        .position(|(x, y)| !(x.is_ascii() && y.is_ascii() && x.eq_ignore_ascii_case(&y)));
    let Some(at) = differ else {
        // One is the start of the other, and each character has a lower case.
        return a.len().cmp(&b.len());
    };
    match (a.as_bytes().get(at), b.as_bytes().get(at)) {
        (Some(x), Some(y)) if x.is_ascii() && y.is_ascii() => {
            x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase())
        }
        // Every byte before is ASCII, so both texts have a character boundary here; a character
        // beyond ASCII may have an ASCII lower case, or more than one character.
        _ => match (a.get(at..), b.get(at..)) {
            (Some(rest_a), Some(rest_b)) => folded(rest_a).cmp(folded(rest_b)),
            // Not reached; the whole texts compare as their rests do.
            _ => folded(a).cmp(folded(b)),
        },
    }
}

/// A stretch of a name that is all digits or has none.
enum Run<'a> {
    Digits(&'a str),
    Text(&'a str),
}

/// The first run of `text` and what follows it; `None` for an empty text.
fn next_run(text: &str) -> Option<(Run<'_>, &str)> {
    // Digits are ASCII, and no byte of a character beyond ASCII is one, so the bytes tell where a
    // run ends, at a character boundary.
    let digits = text.as_bytes().first()?.is_ascii_digit();
    let end = text
        .bytes()
        .position(|byte| byte.is_ascii_digit() != digits)
        .unwrap_or(text.len());
    let (run, rest) = text.split_at_checked(end)?;
    Some((
        if digits {
            Run::Digits(run)
        } else {
            Run::Text(run)
        },
        rest,
    ))
}

/// Compares the [`head`]s of two names run by run, for [`NATURAL_RUNS`] runs at most: numbers by
/// value, text without regard to case, and a number before text. A name whose head has fewer runs
/// than the other's, and is alike as far as it goes, comes first. Names alike in all those runs
/// compare by what follows them, byte by byte.
///
/// Each name is cut into its runs on its own, so this orders the names as their runs and the rest
/// after them would be ordered: a total order of names that it treats as equal.
fn natural_order(a: &str, b: &str) -> Ordering {
    let (mut rest_a, mut rest_b) = (head(a), head(b));
    for _ in 0..NATURAL_RUNS {
        let (Some((run_a, next_a)), Some((run_b, next_b))) = (next_run(rest_a), next_run(rest_b))
        else {
            if rest_a.is_empty() && rest_b.is_empty() {
                break;
            }
            // One head is used up, and the one that has more runs left comes later.
            return rest_a.len().cmp(&rest_b.len());
        };
        let order = match (run_a, run_b) {
            (Run::Digits(x), Run::Digits(y)) => number_order(x, y),
            (Run::Text(x), Run::Text(y)) => folded_order(x, y),
            (Run::Digits(_), Run::Text(_)) => Ordering::Less,
            (Run::Text(_), Run::Digits(_)) => Ordering::Greater,
        };
        if order != Ordering::Equal {
            return order;
        }
        (rest_a, rest_b) = (next_a, next_b);
    }
    after_runs(a, rest_a).cmp(after_runs(b, rest_b))
}

/// What follows the runs of `name` that were compared, of which `rest` is what is left in its
/// head: the rest of the head and all after it.
fn after_runs<'a>(name: &'a str, rest: &str) -> &'a str {
    let compared = head(name).len().saturating_sub(rest.len());
    name.get(compared..).unwrap_or_default()
}

/// Compares two runs of digits by value, however long they are.
fn number_order(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.trim_start_matches('0'), b.trim_start_matches('0'));
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(path: &str) -> std::str::Split<'_, char> {
        path.split('/')
    }

    fn sorted<'a>(names: &[&'a str]) -> Vec<&'a str> {
        let mut names = names.to_vec();
        names.sort_by(|a, b| name_order(a, b));
        names
    }

    #[test]
    fn a_file_named_like_a_padding_file_is_padding() {
        assert!(is_padding(parts("T/_____padding_file_0")));
        assert!(is_padding(parts("T/sub/_____padding_file_12_extra")));
        assert!(is_padding(parts("_____padding_file_0")));
        assert!(!is_padding(parts("T/padding_file_0")));
        assert!(!is_padding(parts("T/_____padding_file_0/real.mkv")));
    }

    #[test]
    fn a_file_in_the_pad_folder_below_the_top_folder_is_padding() {
        assert!(is_padding(parts("T/.pad/0")));
        assert!(is_padding(parts("T/.pad/sub/1024")));
        // Elsewhere, the name is an ordinary one.
        assert!(!is_padding(parts("T/.pad")));
        assert!(!is_padding(parts("T/sub/.pad/0")));
        assert!(!is_padding(parts(".pad")));
        assert!(!is_padding(parts("T/.padding/0")));
    }

    #[test]
    fn digits_compare_by_number_and_letters_ignoring_case() {
        assert_eq!(
            sorted(&["E10.mkv", "E2.mkv", "e1.mkv", "E20.mkv"]),
            ["e1.mkv", "E2.mkv", "E10.mkv", "E20.mkv"]
        );
        assert_eq!(
            sorted(&["file 10", "File 9", "file 100", "file 1"]),
            ["file 1", "File 9", "file 10", "file 100"]
        );
        // Names that differ only in case come out in one fixed order: capitals first.
        assert_eq!(sorted(&["b", "A", "c", "B"]), ["A", "B", "b", "c"]);
    }

    #[test]
    fn digits_come_before_letters_and_a_prefix_before_its_extensions() {
        assert_eq!(sorted(&["a1", "1a", "a", "_a"]), ["1a", "_a", "a", "a1"]);
        // The number is the same, so the name that goes on after it comes second, whatever zeros
        // the numbers are written with.
        assert_eq!(sorted(&["a01b", "a1"]), ["a1", "a01b"]);
    }

    #[test]
    fn leading_zeros_and_huge_numbers_do_not_overflow() {
        let huge = format!("x{}", "9".repeat(60));
        let huger = format!("x{}", "9".repeat(61));
        assert_eq!(
            sorted(&[huger.as_str(), huge.as_str(), "x2"]),
            ["x2", huge.as_str(), huger.as_str()]
        );
        // Equal numbers are ordered by their text: fewer zeros after more, then exactly.
        assert_eq!(sorted(&["a1", "a01", "a001"]), ["a001", "a01", "a1"]);
    }

    #[test]
    fn names_that_differ_only_in_case_are_ordered_the_same_way_every_time() {
        assert_eq!(sorted(&["a", "A"]), ["A", "a"]);
        assert_eq!(sorted(&["A", "a"]), ["A", "a"]);
        assert_eq!(name_order("a", "a"), Ordering::Equal);
    }

    #[test]
    fn names_compare_by_number_for_32_runs_and_then_byte_by_byte() {
        // 32 runs alike, "a1" sixteen times, and then "-9" and "-10": byte by byte, "1" comes
        // before "9". A run less before them, and 9 and 10 are the 32nd: by number.
        let alike = "a1".repeat(16);
        let (nine, ten) = (format!("{alike}-9"), format!("{alike}-10"));
        assert_eq!(
            sorted(&[nine.as_str(), ten.as_str()]),
            [ten.as_str(), nine.as_str()]
        );
        let fewer = format!("{}a", "a1".repeat(15));
        let (nine, ten) = (format!("{fewer}9"), format!("{fewer}10"));
        assert_eq!(
            sorted(&[ten.as_str(), nine.as_str()]),
            [nine.as_str(), ten.as_str()]
        );
    }

    #[test]
    fn names_compare_by_number_and_without_case_for_their_first_256_bytes() {
        // Longer than any name a file system allows: past 256 bytes, byte by byte.
        let long = "y".repeat(300);
        let (nine, ten) = (format!("{long}9"), format!("{long}10"));
        assert_eq!(
            sorted(&[nine.as_str(), ten.as_str()]),
            [ten.as_str(), nine.as_str()]
        );
        let (upper, lower) = (format!("{long}B"), format!("{long}a"));
        assert_eq!(
            sorted(&[lower.as_str(), upper.as_str()]),
            [upper.as_str(), lower.as_str()]
        );
        // Within them, as for any name.
        let name = "y".repeat(250);
        let (nine, ten) = (format!("{name}9"), format!("{name}10"));
        assert_eq!(
            sorted(&[ten.as_str(), nine.as_str()]),
            [nine.as_str(), ten.as_str()]
        );
        let (upper, lower) = (format!("{name}B"), format!("{name}a"));
        assert_eq!(
            sorted(&[upper.as_str(), lower.as_str()]),
            [lower.as_str(), upper.as_str()]
        );
    }

    /// Names of every kind the order treats in its own way: case, numbers with and without
    /// zeros, letters beyond ASCII, a prefix of another, and names longer than 32 runs and than
    /// 256 bytes, alike up to there.
    fn odd_names() -> Vec<String> {
        let mut names: Vec<String> = [
            "", "a", "A", "a1", "a01", "A1", "a10", "a2", "1a", "_a", "é", "É", "e", "a b", "a-1",
        ]
        .map(str::to_owned)
        .to_vec();
        for stem in [
            "a1".repeat(16),
            "a1".repeat(15),
            "Y".repeat(255),
            "y".repeat(256),
        ] {
            for end in ["", "9", "10", "010", "a", "A", "é", "-9", "-10", "a1a1"] {
                names.push(format!("{stem}{end}"));
            }
        }
        names
    }

    #[test]
    fn the_order_is_total_and_the_same_whatever_order_the_names_come_in() {
        let names = odd_names();
        for a in &names {
            for b in &names {
                let order = name_order(a, b);
                assert_eq!(order, name_order(b, a).reverse(), "{a:?} {b:?}");
                assert_eq!(order == Ordering::Equal, a == b, "{a:?} {b:?}");
                for c in &names {
                    if order != Ordering::Greater && name_order(b, c) != Ordering::Greater {
                        assert_ne!(name_order(a, c), Ordering::Greater, "{a:?} {b:?} {c:?}");
                    }
                }
            }
        }
        let forward = sorted(&names.iter().map(String::as_str).collect::<Vec<_>>());
        let mut reversed: Vec<&str> = names.iter().map(String::as_str).rev().collect();
        reversed.sort_by(|a, b| name_order(a, b));
        assert_eq!(forward, reversed);
    }

    #[test]
    fn non_ascii_letters_compare_ignoring_case() {
        assert_eq!(
            sorted(&["Яблоко", "арбуз", "Арбуз"]),
            ["Арбуз", "арбуз", "Яблоко"]
        );
    }
}
