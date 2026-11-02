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
/// to do with it.
pub(super) fn is_padding(parts: &[&str]) -> bool {
    let Some(name) = parts.last() else {
        return false;
    };
    // A `.pad` below the top folder has to be a folder, so the path goes on after it.
    name.starts_with(PADDING_PREFIX) || (parts.len() >= 3 && parts.get(1) == Some(&PAD_FOLDER))
}

/// How two names in the same folder are ordered: digits compare by number, so `E2` comes before
/// `E10`, and letters compare ignoring case.
///
/// Names that this treats as equal (`a1` and `a01`, `A` and `a`) are ordered by their text
/// without case, and then exactly, so the order is the same for the same names whatever order
/// they came in.
pub(super) fn name_order(a: &str, b: &str) -> Ordering {
    natural_order(a, b)
        .then_with(|| folded(a).cmp(folded(b)))
        .then_with(|| a.cmp(b))
}

/// The characters of `text` in lower case.
fn folded(text: &str) -> impl Iterator<Item = char> {
    text.chars().flat_map(char::to_lowercase)
}

/// A stretch of a name that is all digits or has none.
enum Run<'a> {
    Digits(&'a str),
    Text(&'a str),
}

/// The first run of `text` and what follows it; `None` for an empty text.
fn next_run(text: &str) -> Option<(Run<'_>, &str)> {
    let digits = text.chars().next()?.is_ascii_digit();
    let end = text
        .find(|c: char| c.is_ascii_digit() != digits)
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

/// Compares run by run: numbers by value, text without regard to case, and a number before text.
/// A name that is the start of another one comes first.
fn natural_order(mut a: &str, mut b: &str) -> Ordering {
    loop {
        let (Some((run_a, rest_a)), Some((run_b, rest_b))) = (next_run(a), next_run(b)) else {
            // One name is used up, and the one that has more left comes later.
            return a.len().cmp(&b.len());
        };
        let order = match (run_a, run_b) {
            (Run::Digits(x), Run::Digits(y)) => number_order(x, y),
            (Run::Text(x), Run::Text(y)) => folded(x).cmp(folded(y)),
            (Run::Digits(_), Run::Text(_)) => Ordering::Less,
            (Run::Text(_), Run::Digits(_)) => Ordering::Greater,
        };
        if order != Ordering::Equal {
            return order;
        }
        (a, b) = (rest_a, rest_b);
    }
}

/// Compares two runs of digits by value, however long they are.
fn number_order(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.trim_start_matches('0'), b.trim_start_matches('0'));
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts(path: &str) -> Vec<&str> {
        path.split('/').collect()
    }

    fn sorted<'a>(names: &[&'a str]) -> Vec<&'a str> {
        let mut names = names.to_vec();
        names.sort_by(|a, b| name_order(a, b));
        names
    }

    #[test]
    fn a_file_named_like_a_padding_file_is_padding() {
        assert!(is_padding(&parts("T/_____padding_file_0")));
        assert!(is_padding(&parts("T/sub/_____padding_file_12_extra")));
        assert!(is_padding(&parts("_____padding_file_0")));
        assert!(!is_padding(&parts("T/padding_file_0")));
        assert!(!is_padding(&parts("T/_____padding_file_0/real.mkv")));
    }

    #[test]
    fn a_file_in_the_pad_folder_below_the_top_folder_is_padding() {
        assert!(is_padding(&parts("T/.pad/0")));
        assert!(is_padding(&parts("T/.pad/sub/1024")));
        // Elsewhere, the name is an ordinary one.
        assert!(!is_padding(&parts("T/.pad")));
        assert!(!is_padding(&parts("T/sub/.pad/0")));
        assert!(!is_padding(&parts(".pad")));
        assert!(!is_padding(&parts("T/.padding/0")));
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
    fn non_ascii_letters_compare_ignoring_case() {
        assert_eq!(
            sorted(&["Яблоко", "арбуз", "Арбуз"]),
            ["Арбуз", "арбуз", "Яблоко"]
        );
    }
}
