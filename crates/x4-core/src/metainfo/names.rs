//! The names libtransmission shows for a torrent and its files, made from the bytes of the
//! `.torrent` file by its rules for a Linux daemon. They only change how a name reads, never
//! which files there are or in which order.

use std::borrow::Cow;

/// A name as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Text {
    pub(super) text: String,
    /// Whether bytes that are not UTF-8 were replaced. libtransmission's repair may group them
    /// differently, so such a name might not read exactly as the daemon's.
    pub(super) repaired: bool,
}

/// `bytes` as text, the way libtransmission converts every name (`tr_strv_convert_utf8` and
/// `tr_strv_replace_invalid`, T/utils.cc:318-337): everything from the first NUL on is dropped,
/// and what is not UTF-8 becomes U+FFFD.
pub(super) fn display_text(bytes: &[u8]) -> Text {
    let before_nul = bytes.split(|&byte| byte == 0).next().unwrap_or(bytes);
    match String::from_utf8_lossy(before_nul) {
        Cow::Borrowed(text) => Text {
            text: text.to_owned(),
            repaired: false,
        },
        Cow::Owned(text) => Text {
            text,
            repaired: true,
        },
    }
}

/// `path` as a path inside the torrent (`tr_torrent_files::sanitize_subpath`,
/// T/torrent-files.cc:490-503): split again at each `/`, with a `_` before each part that is `.`
/// or `..`, and the trailing `/` dropped.
pub(super) fn sanitized_subpath(path: &str) -> String {
    let mut out = String::new();
    append_sanitized_subpath(path, &mut out);
    out
}

/// Appends one element of a file's `path` list to `subpath`, after a `/` if `subpath` is not
/// empty, as libtransmission builds a file's path (T/torrent-metainfo.cc:301-309). An empty
/// element takes back that `/`, so it adds nothing.
pub(super) fn push_path_element(subpath: &mut String, element: &str) {
    if !subpath.is_empty() {
        subpath.push('/');
    }
    append_sanitized_subpath(element, subpath);
}

/// The two-argument `sanitize_subpath`: appends the parts of `path`, each followed by a `/`, then
/// removes the last character of `out`. On Unix only `.` and `..` are reserved (`_` in front,
/// T/torrent-files.cc:349-357, 474-478), and the only reserved character, `/`, cannot occur in a
/// part. An empty `path` appends nothing, so the character removed is the one before it.
fn append_sanitized_subpath(path: &str, out: &mut String) {
    let mut rest = path;
    while !rest.is_empty() {
        let (part, tail) = rest.split_once('/').unwrap_or((rest, ""));
        if part == "." || part == ".." {
            out.push('_');
        }
        out.push_str(part);
        out.push('/');
        rest = tail;
    }
    out.pop();
}

#[cfg(test)]
mod tests {
    use super::{Text, display_text, push_path_element, sanitized_subpath};

    fn subpath(elements: &[&str]) -> String {
        let mut subpath = String::new();
        for element in elements {
            push_path_element(&mut subpath, element);
        }
        subpath
    }

    #[test]
    fn text_stops_at_a_nul_and_marks_repaired_bytes() {
        let text = |text: &str, repaired| Text {
            text: text.to_owned(),
            repaired,
        };
        assert_eq!(display_text(b"plain"), text("plain", false));
        assert_eq!(display_text("café".as_bytes()), text("café", false));
        assert_eq!(display_text(b"a\0b\xff"), text("a", false));
        assert_eq!(display_text(b"\0"), text("", false));
        assert_eq!(display_text(b"a\xffb"), text("a\u{fffd}b", true));
        assert_eq!(display_text(b"\xff\0\xff"), text("\u{fffd}", true));
    }

    #[test]
    fn dots_get_an_underscore_and_slashes_split() {
        assert_eq!(sanitized_subpath("a"), "a");
        assert_eq!(sanitized_subpath("."), "_.");
        assert_eq!(sanitized_subpath(".."), "_..");
        assert_eq!(sanitized_subpath("..."), "...");
        assert_eq!(sanitized_subpath(".hidden"), ".hidden");
        assert_eq!(sanitized_subpath("a/../b"), "a/_../b");
        assert_eq!(sanitized_subpath("a/"), "a");
        assert_eq!(sanitized_subpath("/"), "");
        assert_eq!(sanitized_subpath(""), "");
        // An empty part inside one element stays, as in libtransmission; a leading `/` too.
        assert_eq!(sanitized_subpath("a//b"), "a//b");
        assert_eq!(sanitized_subpath("/a"), "/a");
        // Windows names and characters are left alone on a Linux daemon.
        assert_eq!(sanitized_subpath("CON:*?.txt"), "CON:*?.txt");
    }

    #[test]
    fn path_elements_join_with_slashes_and_empty_ones_vanish() {
        assert_eq!(subpath(&["a", "b"]), "a/b");
        assert_eq!(subpath(&["", "a", "", "b", ""]), "a/b");
        assert_eq!(subpath(&["..", "."]), "_../_.");
        assert_eq!(subpath(&["a/b", "c"]), "a/b/c");
        assert_eq!(subpath(&["a", "/"]), "a/");
        assert_eq!(subpath(&["/", "a"]), "a");
        assert_eq!(subpath(&[]), "");
        assert_eq!(subpath(&["", ""]), "");
    }
}
