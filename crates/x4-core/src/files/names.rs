//! How the names of a torrent's files are read: which files only pad, and in which order a
//! folder lists its entries.

use std::char::ToLowercase;
use std::cmp::Ordering;
use std::sync::OnceLock;

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

/// The most characters that UTF-8 writes in three bytes or four at the start of a name that are
/// compared in their natural order and without case; the rest of the name, from the next such
/// character on, is compared byte by byte. Folding one of them to lower case takes a search of
/// Unicode's tables, some 25 ns, where the characters of one byte or two take a few, on every
/// path ([`lower_case`], [`folded`]); so the letters of Latin, Greek, Cyrillic and the like are
/// bounded by [`NATURAL_BYTES`] alone, and a comparison searches the tables for no more than this
/// many characters of each name.
///
/// The bound is each name's, not each comparison's, so the order stays total: a bound on what
/// two names fold between them would order three names in a circle, and a sort may panic then.
const SLOWLY_FOLDED_CHARACTERS: usize = 64;

/// How two names in the same folder are ordered: digits compare by number, so `E2` comes before
/// `E10`, and letters compare ignoring case.
///
/// Names that this treats as equal (`a1` and `a01`, `A` and `a`) are ordered by their text
/// without case, and then exactly, so the order is the same for the same names whatever order
/// they came in.
///
/// Only the first [`NATURAL_RUNS`] runs of digits and of other characters, within the first
/// [`NATURAL_BYTES`] of a name and before its 65th character of three bytes or more
/// ([`SLOWLY_FOLDED_CHARACTERS`]), compare that way; the rest of it compares byte by byte. Of
/// those, only the characters where two names differ are folded to lower case, and, where they
/// differ at an `İ`, which folds into two characters, the rest of that run of text. So one
/// comparison takes a bounded time, however long the names: 14 000 crafted names of 600 runs
/// each, or of 128 letters beyond ASCII each, took seconds to sort. The order stays total and
/// the same for the same names, and real names, shorter than the bounds, are ordered as if
/// there were none.
pub(super) fn name_order(a: &str, b: &str) -> Ordering {
    counted_comparison();
    let (head_a, head_b) = (head(a), head(b));
    let tie = match natural_order((a, head_a), (b, head_b)) {
        Natural::Decided(order) => return order,
        Natural::Tie(tie) => tie,
    };
    // As the heads compare without case, `folded_order(head_a, head_b)`, without folding them
    // again.
    let folded = match tie {
        Tie::FromDigits => match digits_written_otherwise(head_a, head_b) {
            Some((from_a, from_b)) => folded_order(from_a, from_b),
            // Not reached; the heads compare as their rests from those digits do.
            None => folded_order(head_a, head_b),
        },
        Tie::Left(left_a, left_b) => left_a.cmp(&left_b),
    };
    folded.then_with(|| a.cmp(b))
}

/// What [`natural_order`] found.
enum Natural {
    /// The names are in this order, which is not `Equal`.
    Decided(Ordering),
    /// They are equal in it, and their heads compare without case as this tells.
    Tie(Tie),
}

/// How the heads of two names that are equal in their natural order compare without case, as
/// the walk over their runs found out.
///
/// Every text run compared folds alike, and the rest after the runs is the same in both names.
/// So the heads fold alike up to the first run of digits written otherwise (`01` and `1`), and
/// compare from there. Without one, they differ only in how much of that rest each head holds:
/// the head that holds less folds into the start of the other.
enum Tie {
    /// As the heads from their first runs of digits written otherwise
    /// ([`digits_written_otherwise`]).
    FromDigits,
    /// As the bytes left of each head after the runs compared.
    Left(usize, usize),
}

/// The heads `head_a` and `head_b`, of names equal in their natural order, from their first runs
/// of digits that are written otherwise, among the runs compared; `None` when there are none.
/// Only such names walk their runs again, and nothing is folded.
fn digits_written_otherwise<'a>(head_a: &'a str, head_b: &'a str) -> Option<(&'a str, &'a str)> {
    let (mut rest_a, mut rest_b) = (head_a, head_b);
    for _ in 0..NATURAL_RUNS {
        let ((run_a, next_a), (run_b, next_b)) = (next_run(rest_a)?, next_run(rest_b)?);
        if let (Run::Digits(x), Run::Digits(y)) = (run_a, run_b)
            && x.len() != y.len()
        {
            return Some((rest_a, rest_b));
        }
        (rest_a, rest_b) = (next_a, next_b);
    }
    None
}

/// The start of `name` that is compared in its natural order: its first [`NATURAL_BYTES`],
/// fewer when they would cut a character, and only up to their character of three bytes or more
/// after the first [`SLOWLY_FOLDED_CHARACTERS`] when they hold more.
fn head(name: &str) -> &str {
    // A shorter name has no room for more such characters; most names are, and most longer
    // ones are ASCII.
    if name.len() < 3 * (SLOWLY_FOLDED_CHARACTERS + 1) {
        return name;
    }
    let head = name
        .get(..name.floor_char_boundary(NATURAL_BYTES))
        .unwrap_or(name);
    // A character of three bytes or more has two at least after its first, so a head with fewer
    // such bytes, of all its characters, than twice the bound and two holds no more than the
    // bound. Counting characters is quick, even in a debug build.
    let after_firsts = || head.len().saturating_sub(head.chars().count());
    if head.is_ascii() || after_firsts() < 2 * (SLOWLY_FOLDED_CHARACTERS + 1) {
        return head;
    }
    within_slowly_folded_characters(head)
}

/// The start of `head` before its character of three bytes or more after the first
/// [`SLOWLY_FOLDED_CHARACTERS`]; all of it when it has no more.
fn within_slowly_folded_characters(head: &str) -> &str {
    /// The bytes counted at once.
    const CHUNK: usize = 64;
    // Such a character starts with a byte of 0xE0 or more, and no other byte does.
    let starts = |byte: u8| byte >= 0xE0;
    let mut seen = 0;
    for (index, chunk) in head.as_bytes().chunks(CHUNK).enumerate() {
        // A chunk has fewer starts than a byte can count.
        let here = chunk.iter().fold(0_u8, |count, &byte| {
            count.wrapping_add(u8::from(starts(byte)))
        });
        let here = usize::from(here);
        if seen + here > SLOWLY_FOLDED_CHARACTERS {
            let within = chunk
                .iter()
                .enumerate()
                .filter(|&(_, &byte)| starts(byte))
                .nth(SLOWLY_FOLDED_CHARACTERS - seen)
                .map_or(chunk.len(), |(at, _)| at);
            return head.get(..index * CHUNK + within).unwrap_or(head);
        }
        seen += here;
    }
    head
}

/// The characters of `text` in lower case, as [`char::to_lowercase`] gives them.
///
/// Each character is folded through [`lower_case`], so that Unicode's tables are searched only
/// for the characters of three bytes or more, which a [`head`] bounds. `İ`, the one character
/// whose lower case is several, is folded by hand: it is where [`folded_order`] has to fold the
/// rest of two texts, and searching for every character of the rest took 2.5 µs a comparison.
fn folded(text: &str) -> Folded<'_> {
    Folded {
        chars: text.chars(),
        next: None,
        last: None,
    }
}

/// The capital I with a dot above, the one character whose lower case is several: `i` and a
/// combining dot above.
const CAPITAL_I_WITH_DOT: char = '\u{130}';

/// The characters of a text in lower case ([`folded`]).
struct Folded<'a> {
    chars: std::str::Chars<'a>,
    /// The characters of the lower case of the last character read that are still to come: a
    /// lower case of [`char::to_lowercase`] is never more than three.
    next: Option<char>,
    last: Option<char>,
}

impl Iterator for Folded<'_> {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        if let Some(next) = self.next.take() {
            self.next = self.last.take();
            return Some(next);
        }
        let c = self.chars.next()?;
        counted_fold();
        Some(match lower_case(c) {
            Some(lower) => lower,
            None if c == CAPITAL_I_WITH_DOT => {
                self.next = Some('\u{307}');
                'i'
            }
            // Not reached with the Unicode of this Rust; a newer one may fold another character
            // so.
            None => {
                counted_lookup();
                let mut lower = c.to_lowercase();
                let first = lower.next().unwrap_or(c);
                (self.next, self.last) = (lower.next(), lower.next());
                first
            }
        })
    }
}

/// How `a` and `b` compare in lower case: as their [`folded`] characters do.
///
/// The bytes that are alike, the same or ASCII letters that differ only in case, are skipped,
/// since they fold alike. Only the characters where the texts differ otherwise are decoded and
/// folded, one pair at a time, up to the next pair that is the same. A character that folds into
/// several, `İ`, shifts the rest, which then compares as a whole, folded through the same table.
fn folded_order(a: &str, b: &str) -> Ordering {
    if a == b {
        return Ordering::Equal;
    }
    let (mut a, mut b) = (a, b);
    loop {
        // The bytes before are alike in pairs, each the same or both ASCII, so the boundary of
        // the character they end in is the same in both.
        let start = a.floor_char_boundary(alike_bytes(a.as_bytes(), b.as_bytes()));
        let (Some(rest_a), Some(rest_b)) = (a.get(start..), b.get(start..)) else {
            // Not reached; the whole texts compare as their rests do.
            return folded(a).cmp(folded(b));
        };
        let (mut chars_a, mut chars_b) = (rest_a.chars(), rest_b.chars());
        loop {
            let (from_a, from_b) = (chars_a.as_str(), chars_b.as_str());
            let (x, y) = match (chars_a.next(), chars_b.next()) {
                (Some(x), Some(y)) => (x, y),
                (None, None) => return Ordering::Equal,
                // Each character has a lower case, so the one that goes on comes later.
                (None, Some(_)) => return Ordering::Less,
                (Some(_), None) => return Ordering::Greater,
            };
            if x == y {
                break;
            }
            counted_fold();
            counted_fold();
            let (Some(lower_x), Some(lower_y)) = (lower_case(x), lower_case(y)) else {
                return folded(from_a).cmp(folded(from_b));
            };
            let order = lower_x.cmp(&lower_y);
            if order != Ordering::Equal {
                return order;
            }
        }
        (a, b) = (chars_a.as_str(), chars_b.as_str());
    }
}

/// The lower case of `c`, as [`char::to_lowercase`] gives it, when it is one character; `None`
/// when it is several, as for `İ`.
///
/// The characters that UTF-8 writes in two bytes, Latin, Greek, Cyrillic and others, are looked
/// up in a table made once: searching Unicode's tables for each of them took most of the time of
/// sorting names of them.
fn lower_case(c: char) -> Option<char> {
    if c.is_ascii() {
        return Some(c.to_ascii_lowercase());
    }
    let index = u32::from(c)
        .checked_sub(TWO_BYTES_START)
        .and_then(|index| usize::try_from(index).ok());
    if let Some(&lower) = index.and_then(|index| two_byte_lower_cases().get(index)) {
        // No character of the table has the lower case U+0000, which marks several.
        return (lower != '\0').then_some(lower);
    }
    counted_lookup();
    one_character(c.to_lowercase())
}

/// The first character that UTF-8 writes in two bytes.
const TWO_BYTES_START: u32 = 0x80;

/// The character after the last that UTF-8 writes in two bytes.
const TWO_BYTES_END: u32 = 0x800;

/// The lower case of each character from U+0080 to U+07FF, at its distance from U+0080: the
/// character, or U+0000 when it is several.
fn two_byte_lower_cases() -> &'static [char] {
    static TABLE: OnceLock<Vec<char>> = OnceLock::new();
    TABLE.get_or_init(|| {
        (TWO_BYTES_START..TWO_BYTES_END)
            .map(|code| {
                char::from_u32(code)
                    .and_then(|c| one_character(c.to_lowercase()))
                    .unwrap_or('\0')
            })
            .collect()
    })
}

/// The lower case `lower` when it is one character.
fn one_character(mut lower: ToLowercase) -> Option<char> {
    if lower.len() == 1 { lower.next() } else { None }
}

/// How many bytes `a` and `b` start with that are alike in pairs: the same, or ASCII letters
/// that differ only in case. Runs of the same bytes are compared 16 at a time.
fn alike_bytes(a: &[u8], b: &[u8]) -> usize {
    /// The bytes compared at once.
    const CHUNK: usize = 16;
    let mut at = 0;
    while let (Some(x), Some(y)) = (a.get(at..at + CHUNK), b.get(at..at + CHUNK)) {
        if x != y {
            break;
        }
        at += CHUNK;
    }
    let (Some(a), Some(b)) = (a.get(at..), b.get(at..)) else {
        return at;
    };
    // Beyond ASCII, a byte is alike only to itself.
    at + a
        .iter()
        .zip(b)
        .take_while(|(x, y)| x.eq_ignore_ascii_case(y))
        .count()
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
/// after them would be ordered: a total order of names that it treats as equal. Each name comes
/// with its head. For names it treats as equal, it also tells how their heads compare without
/// case ([`Tie`]).
fn natural_order((a, head_a): (&str, &str), (b, head_b): (&str, &str)) -> Natural {
    let (mut rest_a, mut rest_b) = (head_a, head_b);
    let mut written_otherwise = false;
    for _ in 0..NATURAL_RUNS {
        let (Some((run_a, next_a)), Some((run_b, next_b))) = (next_run(rest_a), next_run(rest_b))
        else {
            if rest_a.is_empty() && rest_b.is_empty() {
                break;
            }
            // One head is used up, and the one that has more runs left comes later.
            return Natural::Decided(rest_a.len().cmp(&rest_b.len()));
        };
        let order = match (run_a, run_b) {
            (Run::Digits(x), Run::Digits(y)) => {
                // Numbers of the same value differ only in their zeros, and so in length.
                written_otherwise |= x.len() != y.len();
                number_order(x, y)
            }
            (Run::Text(x), Run::Text(y)) => folded_order(x, y),
            (Run::Digits(_), Run::Text(_)) => Ordering::Less,
            (Run::Text(_), Run::Digits(_)) => Ordering::Greater,
        };
        if order != Ordering::Equal {
            return Natural::Decided(order);
        }
        (rest_a, rest_b) = (next_a, next_b);
    }
    match after_runs(a, head_a, rest_a).cmp(after_runs(b, head_b, rest_b)) {
        Ordering::Equal if written_otherwise => Natural::Tie(Tie::FromDigits),
        Ordering::Equal => Natural::Tie(Tie::Left(rest_a.len(), rest_b.len())),
        order => Natural::Decided(order),
    }
}

/// What follows the runs of `name` that were compared, of which `rest` is what is left in its
/// `head`: the rest of the head and all after it.
fn after_runs<'a>(name: &'a str, head: &str, rest: &str) -> &'a str {
    let compared = head.len().saturating_sub(rest.len());
    name.get(compared..).unwrap_or_default()
}

/// Compares two runs of digits by value, however long they are.
fn number_order(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.trim_start_matches('0'), b.trim_start_matches('0'));
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

/// Counts a comparison of two names, in the tests of this crate ([`work`]).
fn counted_comparison() {
    #[cfg(test)]
    work::add(|work| work.comparisons += 1);
}

/// Counts a lower case looked up through [`char::to_lowercase`], a search of Unicode's tables, in
/// the tests of this crate ([`work`]).
fn counted_lookup() {
    #[cfg(test)]
    work::add(|work| work.to_lowercase += 1);
}

/// Counts a character folded to lower case, in the tests of this crate ([`work`]).
fn counted_fold() {
    #[cfg(test)]
    work::add(|work| work.folded += 1);
}

/// The work that comparing names does on a thread, counted in the tests of this crate, so that
/// they bound it per comparison: a count is the same on a busy machine, where a time is not.
/// Outside those tests nothing is counted, and the counting functions are empty.
#[cfg(test)]
pub(super) mod work {
    use std::cell::Cell;

    /// What comparing names did.
    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
    pub(in crate::files) struct Work {
        /// Comparisons of two names ([`name_order`](super::name_order)).
        pub(in crate::files) comparisons: usize,
        /// Lower cases looked up through [`char::to_lowercase`].
        pub(in crate::files) to_lowercase: usize,
        /// Characters folded to lower case.
        pub(in crate::files) folded: usize,
    }

    thread_local! {
        static WORK: Cell<Work> = Cell::new(Work::default());
    }

    /// The work counted on this thread so far.
    pub(in crate::files) fn so_far() -> Work {
        WORK.with(Cell::get)
    }

    /// Counts `change` on this thread.
    pub(super) fn add(change: impl FnOnce(&mut Work)) {
        WORK.with(|work| {
            let mut counted = work.get();
            change(&mut counted);
            work.set(counted);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::work::Work;
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
    fn names_compare_without_case_for_their_first_64_characters_of_three_bytes_or_more() {
        // From the 65th such character on, byte by byte: "Ａ" (EF BC A1) comes before "ａ"
        // (EF BD 81), and "ａ1" before "ａ9".
        let many = "ａ".repeat(64);
        let (upper_b, lower_a) = (format!("{many}Ａb"), format!("{many}ａa"));
        assert_eq!(
            sorted(&[lower_a.as_str(), upper_b.as_str()]),
            [upper_b.as_str(), lower_a.as_str()]
        );
        let (nine, ten) = (format!("{many}ａ9"), format!("{many}ａ10"));
        assert_eq!(
            sorted(&[nine.as_str(), ten.as_str()]),
            [ten.as_str(), nine.as_str()]
        );
        // Up to the 64th, and the ASCII after it, they compare as any name does.
        let fewer = "ａ".repeat(63);
        let (upper_b, lower_a) = (format!("{fewer}Ａb"), format!("{fewer}ａa"));
        assert_eq!(
            sorted(&[upper_b.as_str(), lower_a.as_str()]),
            [lower_a.as_str(), upper_b.as_str()]
        );
        let (nine, ten) = (format!("{many}9"), format!("{many}10"));
        assert_eq!(
            sorted(&[ten.as_str(), nine.as_str()]),
            [nine.as_str(), ten.as_str()]
        );
    }

    #[test]
    fn a_head_ends_before_its_65th_character_of_three_bytes_or_more() {
        // Characters of two bytes, three and four, after ASCII that puts them at every place
        // among eight, before 256 bytes and after them.
        for lead in ["", "a", "ab", "abc", "abcd", "abcde", "abcdef", "abcdefg"] {
            for (wide, count) in [
                ("ａ", 85),
                ("ａ", 65),
                ("ａ", 64),
                ("ａb", 64),
                ("ａ\u{1f600}", 40),
                ("\u{1f600}", 63),
                ("\u{1f600}", 70),
                ("é", 128),
                ("éａ", 60),
                ("éａ", 70),
            ] {
                let name = format!("{lead}{}", wide.repeat(count));
                let within = &name[..name.floor_char_boundary(NATURAL_BYTES)];
                let expected = within
                    .char_indices()
                    .filter(|(_, c)| c.len_utf8() >= 3)
                    .nth(SLOWLY_FOLDED_CHARACTERS)
                    .map_or(within.len(), |(at, _)| at);
                assert_eq!(head(&name).len(), expected, "{name:?}");
            }
        }
    }

    #[test]
    fn long_names_in_latin_greek_or_cyrillic_compare_as_any_name_for_their_first_256_bytes() {
        // Their letters take two bytes, and are not bounded but by the bytes: 120 of them and a
        // number, 240 bytes and more.
        let many = "я".repeat(120);
        let (upper_b, lower_a) = (format!("{many}Яb"), format!("{many}яa"));
        assert_eq!(
            sorted(&[upper_b.as_str(), lower_a.as_str()]),
            [lower_a.as_str(), upper_b.as_str()]
        );
        let (nine, ten) = (format!("{many}9"), format!("{many}10"));
        assert_eq!(
            sorted(&[ten.as_str(), nine.as_str()]),
            [nine.as_str(), ten.as_str()]
        );
        let title = "Длинное название аудиокниги — Глава ".repeat(3);
        let (nine, ten) = (format!("{title}9.mp3"), format!("{title}10.mp3"));
        assert_eq!(
            sorted(&[ten.as_str(), nine.as_str()]),
            [nine.as_str(), ten.as_str()]
        );
        let mixed = format!("{}{}", "e".repeat(50), "é".repeat(100));
        let (upper_b, lower_a) = (format!("{mixed}Éb"), format!("{mixed}éa"));
        assert_eq!(
            sorted(&[upper_b.as_str(), lower_a.as_str()]),
            [lower_a.as_str(), upper_b.as_str()]
        );
    }

    /// Names of up to 40 pieces, drawn with the generator of `seed` from letters in both cases,
    /// digits, signs and characters beyond ASCII that fold in odd ways, often after a shared
    /// start, so that comparisons go deep; one start has 55 Cyrillic letters. Each comes with
    /// variants of it that differ only in case and in the zeros before their numbers, and some go
    /// on past 256 bytes. None has more than 64 characters of three bytes or more in its first
    /// 256 bytes.
    fn random_names(count: usize, mut seed: u64) -> Vec<String> {
        const PIECES: [&str; 34] = [
            "a", "A", "b", "B", "z", "0", "1", "9", "10", "007", " ", "-", ".", "é", "É", "ß", "ẞ",
            "İ", "ı", "\u{212a}", "k", "Σ", "σ", "ς", "я", "Я", "Ё", "中", "ﬀ", "Ａ", "ａ", "ǅ",
            "ǆ", "Ǆ",
        ];
        const STARTS: [&str; 6] = [
            "",
            "Track ",
            "Песня ",
            "éÉé",
            "Season 1 Episode ",
            "Длинное название аудиокниги — Глава 1 — Длинное название аудиокниги ",
        ];
        let mut next = move |below: usize| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            usize::try_from(seed >> 33).unwrap_or(0) % below
        };
        let mut names = Vec::new();
        while names.len() < count {
            let mut name = STARTS[next(STARTS.len())].to_owned();
            for _ in 0..next(41) {
                name.push_str(PIECES[next(PIECES.len())]);
            }
            if next(4) == 0 {
                name.push_str(&"x".repeat(150 + next(150)));
            }
            for _ in 0..3 {
                let variant: String = name
                    .chars()
                    .flat_map(|c| {
                        let flip = next(2) == 0;
                        let zero = c.is_ascii_digit() && next(3) == 0;
                        let cased = match c {
                            'a'..='z' | 'A'..='Z' if flip => c.to_ascii_uppercase(),
                            'é' if flip => 'É',
                            'É' if flip => 'é',
                            'я' if flip => 'Я',
                            'а' if flip => 'А',
                            'k' if flip => '\u{212a}',
                            _ => c,
                        };
                        zero.then_some('0').into_iter().chain([cased])
                    })
                    .collect();
                names.push(variant);
            }
            names.push(name);
        }
        names
    }

    /// Whether the first 256 bytes of `name` hold no more than 64 characters of three bytes or
    /// more, the bounds within which the order is what it was.
    fn within_the_bounds(name: &str) -> bool {
        let window = &name[..name.floor_char_boundary(NATURAL_BYTES)];
        window.chars().filter(|c| c.len_utf8() >= 3).count() <= SLOWLY_FOLDED_CHARACTERS
    }

    /// The characters of `text` in lower case, as Unicode gives them, one by one.
    fn unicode_folded(text: &str) -> impl Iterator<Item = char> {
        text.chars().flat_map(char::to_lowercase)
    }

    /// The order as it was before the characters of three bytes or more were bounded: every
    /// character of the first 256 bytes folded, as Unicode folds it.
    fn previous_order(a: &str, b: &str) -> Ordering {
        fn head(name: &str) -> &str {
            name.get(..name.floor_char_boundary(NATURAL_BYTES))
                .unwrap_or(name)
        }
        fn natural(a: &str, b: &str) -> Ordering {
            let (mut rest_a, mut rest_b) = (head(a), head(b));
            for _ in 0..NATURAL_RUNS {
                let (Some((run_a, next_a)), Some((run_b, next_b))) =
                    (next_run(rest_a), next_run(rest_b))
                else {
                    if rest_a.is_empty() && rest_b.is_empty() {
                        break;
                    }
                    return rest_a.len().cmp(&rest_b.len());
                };
                let order = match (run_a, run_b) {
                    (Run::Digits(x), Run::Digits(y)) => number_order(x, y),
                    (Run::Text(x), Run::Text(y)) => unicode_folded(x).cmp(unicode_folded(y)),
                    (Run::Digits(_), Run::Text(_)) => Ordering::Less,
                    (Run::Text(_), Run::Digits(_)) => Ordering::Greater,
                };
                if order != Ordering::Equal {
                    return order;
                }
                (rest_a, rest_b) = (next_a, next_b);
            }
            let after = |name: &str, rest: &str| name[head(name).len() - rest.len()..].to_owned();
            after(a, rest_a).cmp(&after(b, rest_b))
        }
        natural(a, b)
            .then_with(|| unicode_folded(head(a)).cmp(unicode_folded(head(b))))
            .then_with(|| a.cmp(b))
    }

    #[test]
    fn names_within_the_bounds_are_ordered_as_before() {
        let mut names = random_names(800, 11);
        // Heads cut at different bytes: the Kelvin sign takes three bytes, and folds to "k".
        let runs = "a1".repeat(16);
        for start in ["\u{212a}", "k", "K"] {
            names.push(format!("{start}{runs}{}", "x".repeat(300)));
        }
        // Names that differ at an `İ`, and fold alike there or not.
        names.extend(dotted_i_by_bits().into_iter().step_by(97));
        names.extend(dotted_i_first().into_iter().step_by(97));
        let mut ties = 0;
        for a in &names {
            assert!(within_the_bounds(a), "{a:?}");
            for b in &names {
                assert_eq!(name_order(a, b), previous_order(a, b), "{a:?} {b:?}");
                let (head_a, head_b) = (head(a), head(b));
                ties += usize::from(
                    a != b && matches!(natural_order((a, head_a), (b, head_b)), Natural::Tie(_)),
                );
            }
        }
        // Enough pairs that only the case or the zeros tell apart: some 1 500.
        assert!(ties > 1_000, "{ties}");
    }

    /// Names beyond ASCII that fold slowly or in odd ways: many characters alike, case variants,
    /// characters that fold into several, and names on both sides of the bound of 64 characters
    /// of three bytes or more.
    fn names_beyond_ascii() -> Vec<String> {
        let mut names: Vec<String> = ["İ", "i\u{307}", "\u{212a}", "k", "K", "ß", "ẞ", "ss", "ﬀ"]
            .map(str::to_owned)
            .to_vec();
        for (letter, capital, counts) in
            [("é", "É", &[64, 128][..]), ("ａ", "Ａ", &[63, 64, 65, 85])]
        {
            for &count in counts {
                let many = letter.repeat(count);
                for end in [
                    "", "1", "10", "01", "A", "İ", "é", "É", "ａ", "Ａ", "Éb", "ａa",
                ] {
                    names.push(format!("{many}{end}"));
                }
                names.push(format!("{capital}{}", &many[letter.len()..]));
            }
        }
        for variant in [0_u32, 1, 2, 5, 1 << 15, 0xffff] {
            names.push(
                (0..128)
                    .map(|bit| {
                        if bit < 16 && (variant >> bit) & 1 == 1 {
                            'É'
                        } else {
                            'é'
                        }
                    })
                    .collect(),
            );
        }
        // Names that differ at an `İ` and fold alike there, or differ further on.
        let by_bits = dotted_i_by_bits();
        let first = dotted_i_first();
        for n in [0, 1, 2, 3, 5, 1_024, 1_999] {
            names.push(by_bits[n].clone());
            names.push(first[n].clone());
        }
        names
    }

    #[test]
    fn the_order_of_names_beyond_ascii_is_total() {
        let names = names_beyond_ascii();
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
    fn the_table_of_lower_cases_is_unicode_s() {
        for code in (0..0x3_0000).chain(0xe_0000..0xe_0100) {
            let Some(c) = char::from_u32(code) else {
                continue;
            };
            let lower: Vec<char> = c.to_lowercase().collect();
            let expected = (lower.len() == 1).then(|| lower[0]);
            assert_eq!(lower_case(c), expected, "{c:?}");
            assert_eq!(folded(&c.to_string()).collect::<Vec<_>>(), lower, "{c:?}");
        }
        assert_eq!(two_byte_lower_cases().len(), 0x780);
        assert_eq!(lower_case('İ'), None, "two characters");
    }

    #[test]
    fn only_the_capital_i_with_a_dot_has_a_lower_case_of_several_characters() {
        // So [`folded`] folds it by hand, and looks up no other character in Unicode's tables
        // but those the table of two-byte letters does not hold. A new version of Unicode that
        // gives another character such a lower case fails here.
        let several: Vec<char> = (0..=u32::from(char::MAX))
            .filter_map(char::from_u32)
            .filter(|c| c.to_lowercase().len() > 1)
            .collect();
        assert_eq!(several, ['İ']);
        assert_eq!('İ'.to_lowercase().collect::<String>(), "i\u{307}");
    }

    /// `names` in a shuffled order, the same every time.
    fn shuffled(mut names: Vec<String>) -> Vec<String> {
        let mut seed: u64 = 42;
        for at in (1..names.len()).rev() {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let other = usize::try_from(seed >> 33).unwrap() % (at + 1);
            names.swap(at, other);
        }
        names
    }

    /// The number of names of each crafted shape that is sorted.
    const CRAFTED: u32 = 2_000;

    /// [`CRAFTED`] names of `letter` `times` times and each its own number.
    fn alike_but_for_a_number(letter: &str, times: usize) -> Vec<String> {
        (0..CRAFTED)
            .map(|n| format!("{}{n}", letter.repeat(times)))
            .collect()
    }

    /// [`CRAFTED`] names of `times` letters, `lower` or `upper` by the bits of each name's own
    /// number in the first 16.
    fn alike_but_for_their_case([lower, upper]: [char; 2], times: u32) -> Vec<String> {
        (0..CRAFTED)
            .map(|n| {
                (0..times)
                    .map(|bit| {
                        if bit < 16 && (n >> bit) & 1 == 1 {
                            upper
                        } else {
                            lower
                        }
                    })
                    .collect()
            })
            .collect()
    }

    /// [`CRAFTED`] names of `times` letters, each `lower` or `upper` at random.
    fn in_random_case([lower, upper]: [char; 2], times: usize) -> Vec<String> {
        let mut seed: u64 = 7;
        (0..CRAFTED)
            .map(|_| {
                (0..times)
                    .map(|_| {
                        seed = seed
                            .wrapping_mul(6_364_136_223_846_793_005)
                            .wrapping_add(1_442_695_040_888_963_407);
                        if seed >> 63 == 1 { upper } else { lower }
                    })
                    .collect()
            })
            .collect()
    }

    /// [`CRAFTED`] names of `lower` or `upper` by the parity of their own number, `times` times,
    /// and the number: two names of either kind differ in case at every letter.
    fn in_one_case_or_the_other([lower, upper]: [char; 2], times: usize) -> Vec<String> {
        (0..CRAFTED)
            .map(|n| {
                let letter = if n % 2 == 0 { lower } else { upper };
                format!("{}{n}", String::from(letter).repeat(times))
            })
            .collect()
    }

    /// [`CRAFTED`] names of 120 `İ`, of which each of the first 16 is `i̇` (`i` and a combining
    /// dot above, which `İ` folds into) by the bits of the name's own number: two names differ at
    /// an `İ`, and fold alike there.
    fn dotted_i_by_bits() -> Vec<String> {
        (0..CRAFTED)
            .map(|n| {
                (0..120)
                    .map(|bit| {
                        if bit < 16 && (n >> bit) & 1 == 1 {
                            "i\u{307}"
                        } else {
                            "İ"
                        }
                    })
                    .collect()
            })
            .collect()
    }

    /// [`CRAFTED`] names of `İ` or `i̇` by the parity of their own number, then `é` 120 times
    /// and the number: two names of either kind differ at their first character.
    fn dotted_i_first() -> Vec<String> {
        (0..CRAFTED)
            .map(|n| {
                let first = if n % 2 == 0 { "İ" } else { "i\u{307}" };
                format!("{first}{}{n}", "é".repeat(120))
            })
            .collect()
    }

    /// The most work that one comparison did while `names` were sorted, shuffled.
    fn most_work_per_comparison(names: Vec<String>) -> Work {
        let mut names = shuffled(names);
        let mut most = Work::default();
        names.sort_by(|a, b| {
            let before = work::so_far();
            let order = name_order(a, b);
            let after = work::so_far();
            most.comparisons = most.comparisons.max(after.comparisons - before.comparisons);
            most.to_lowercase = most
                .to_lowercase
                .max(after.to_lowercase - before.to_lowercase);
            most.folded = most.folded.max(after.folded - before.folded);
            order
        });
        most
    }

    #[test]
    fn a_comparison_of_crafted_names_does_bounded_work() {
        // Each shape, with the most characters a comparison of two of them folds and the most
        // lower cases it looks up in Unicode's tables. Before the bounds, a comparison of two
        // names folded every character of both, and again for the tie of their case: some 30 µs
        // each, and seconds for a sort of 36 000 names, at every read of the details.
        let shapes: [(&str, Vec<String>, usize, usize); 9] = [
            // Alike in their first 256 bytes, which are skipped.
            ("ASCII", alike_but_for_a_number("a", 256), 0, 0),
            ("é128", alike_but_for_a_number("é", 128), 0, 0),
            // 600 runs, of which 32 compare naturally, each alike.
            ("a1×300", alike_but_for_a_number("a1", 300), 0, 0),
            // Only where they differ, in their first 16 letters.
            (
                "é/É by bits",
                alike_but_for_their_case(['é', 'É'], 128),
                32,
                0,
            ),
            // Every letter may differ, and each is looked up in a table of two-byte letters.
            ("é/É at random", in_random_case(['é', 'É'], 128), 256, 0),
            // Three bytes each, so looked up in Unicode's tables: the first 64 of the 85 of each
            // name, 255 bytes, and the rest byte by byte.
            (
                "Ａ/ａ at random",
                in_random_case(['ａ', 'Ａ'], 85),
                2 * SLOWLY_FOLDED_CHARACTERS,
                2 * SLOWLY_FOLDED_CHARACTERS,
            ),
            // Every letter differs, and only the first 64 of each name fold.
            (
                "Ａ or ａ throughout",
                in_one_case_or_the_other(['ａ', 'Ａ'], 85),
                2 * SLOWLY_FOLDED_CHARACTERS,
                2 * SLOWLY_FOLDED_CHARACTERS,
            ),
            // `İ` folds into two characters, so the rest of both names folds, through the table.
            ("İ by bits", dotted_i_by_bits(), 2 * NATURAL_BYTES, 0),
            ("İ first", dotted_i_first(), 2 * NATURAL_BYTES, 0),
        ];
        for (shape, names, folded, to_lowercase) in shapes {
            let most = most_work_per_comparison(names);

            assert_eq!(most.comparisons, 1, "{shape}");
            assert!(most.folded <= folded, "{shape}: {most:?}");
            assert!(most.to_lowercase <= to_lowercase, "{shape}: {most:?}");
            // However the names are crafted.
            assert!(most.folded <= 2 * NATURAL_BYTES, "{shape}: {most:?}");
            assert!(
                most.to_lowercase <= 2 * SLOWLY_FOLDED_CHARACTERS,
                "{shape}: {most:?}"
            );
        }
    }

    #[test]
    fn non_ascii_letters_compare_ignoring_case() {
        assert_eq!(
            sorted(&["Яблоко", "арбуз", "Арбуз"]),
            ["Арбуз", "арбуз", "Яблоко"]
        );
    }
}
