//! What 4x4 was launched with: each argument is a magnet link, a `.torrent` file to read, or
//! something it refuses (see [`classify`]).
//!
//! Desktop launchers pass `Exec=4x4 %U` a plain path, a `file://` URI or a magnet link, and a
//! second launch forwards its arguments with its working directory to the first one. Both go
//! through [`classify`], which never touches the file system: whether the file exists, and what
//! it is, is checked when it is read.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use url::Url;
use x4_rpc::{MagnetError, MagnetLink};

/// What one launch argument asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchArg {
    /// A valid magnet link.
    Magnet(MagnetLink),
    /// A local file to read as a `.torrent`, by an absolute path. A path is kept as the user
    /// named it: `..` and symlinks are not resolved. In a `file:` URI, the URL rules resolve `.`
    /// and `..` segments, also when they are written `%2e%2e`.
    File(PathBuf),
    /// Neither: the window says why.
    Rejected(RejectedArg),
}

/// An argument 4x4 does not open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RejectedArg {
    /// Why it is refused.
    pub kind: RejectKind,
}

/// Why a launch argument is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectKind {
    /// A URL that is neither a magnet link nor a local file: any scheme but `magnet` and `file`
    /// followed by `//` (`http(s)`, `ftp` and the like), or a `file:` URI that
    /// - does not parse, or is not UTF-8;
    /// - has a control character, or ends with a space, which `Url::parse` would drop;
    /// - has a backslash, which the URL rules read as `/` (launchers write `%5C`);
    /// - has a host other than `localhost`;
    /// - has a query or a fragment.
    ///
    /// 4x4 never has the daemon fetch a URL.
    Url,
    /// An option (it starts with `-`) that 4x4 does not know; `--version` is answered before the
    /// arguments get here.
    Option,
    /// A bare info hash: only the explicit `magnet:?` form is taken.
    BareHash,
    /// An empty argument.
    Empty,
    /// A `magnet:` argument that is not UTF-8.
    NotUtf8Magnet,
    /// A `magnet:` argument that is not a magnet link the daemon can add.
    InvalidMagnet(MagnetError),
    /// A path that cannot name a file: one with a NUL byte, or a relative one when the sender's
    /// working directory is not absolute.
    Other,
}

/// Tells what the launch argument `arg` asks for; a relative path is joined to `cwd`, the
/// working directory of the process that was launched with it.
///
/// - `magnet:` in any case, with white space around it or not, is a magnet link, validated by
///   [`MagnetLink::parse`].
/// - `file:/…` or `file://…` with an empty host or `localhost`, and no query or fragment, is the
///   local path it names, percent-decoded.
/// - Any other scheme followed by `//` is a [`RejectKind::Url`]. A scheme is told from a file
///   name such as `a:b.torrent` by RFC 3986's scheme grammar and the slashes after it.
/// - An empty argument, one starting with `-`, and a bare info hash are refused.
/// - Anything else is a path, joined to `cwd` when it is relative.
#[must_use]
pub fn classify(arg: &OsStr, cwd: &Path) -> LaunchArg {
    let bytes = arg.as_encoded_bytes();
    if bytes.is_empty() {
        return reject(RejectKind::Empty);
    }
    if bytes.starts_with(b"-") {
        return reject(RejectKind::Option);
    }
    // A magnet link is trimmed, as `MagnetLink::parse` trims it; a path is not, since a file name
    // may start or end with a space.
    if split_scheme(bytes.trim_ascii_start())
        .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case(b"magnet"))
    {
        return magnet(arg);
    }
    if let Some((scheme, rest)) = split_scheme(bytes) {
        // RFC 8089 allows `file:/path` as well as `file:///path`.
        if scheme.eq_ignore_ascii_case(b"file") && rest.starts_with(b"/") {
            return file_uri(arg);
        }
        if rest.starts_with(b"//") {
            return reject(RejectKind::Url);
        }
    }
    if arg
        .to_str()
        .is_some_and(|text| MagnetLink::parse(text) == Err(MagnetError::BareHash))
    {
        return reject(RejectKind::BareHash);
    }
    local_file(cwd.join(arg))
}

fn reject(kind: RejectKind) -> LaunchArg {
    LaunchArg::Rejected(RejectedArg { kind })
}

/// The scheme an argument starts with and what follows its colon, by RFC 3986's grammar: a
/// letter, then letters, digits, `+`, `-` and `.`.
fn split_scheme(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
    let colon = bytes.iter().position(|&byte| byte == b':')?;
    let (scheme, colon_and_rest) = bytes.split_at_checked(colon)?;
    let (first, others) = scheme.split_first()?;
    let valid = first.is_ascii_alphabetic()
        && others
            .iter()
            .all(|&byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'));
    valid.then(|| (scheme, colon_and_rest.get(1..).unwrap_or_default()))
}

fn magnet(arg: &OsStr) -> LaunchArg {
    match arg.to_str().map(MagnetLink::parse) {
        Some(Ok(link)) => LaunchArg::Magnet(link),
        Some(Err(error)) => reject(RejectKind::InvalidMagnet(error)),
        None => reject(RejectKind::NotUtf8Magnet),
    }
}

/// The local file a `file:` URI names, through [`Url::to_file_path`], which refuses a host other
/// than `localhost`.
fn file_uri(arg: &OsStr) -> LaunchArg {
    let Some(text) = arg.to_str() else {
        return reject(RejectKind::Url);
    };
    // `Url::parse` drops tabs and line breaks inside and control characters and spaces at the
    // end, and reads a backslash as `/`, so such a URI would name another file than the one given.
    if text.contains(|character: char| character.is_ascii_control() || character == '\\')
        || text.ends_with(' ')
    {
        return reject(RejectKind::Url);
    }
    let Ok(url) = Url::parse(text) else {
        return reject(RejectKind::Url);
    };
    if url.query().is_some() || url.fragment().is_some() {
        return reject(RejectKind::Url);
    }
    url.to_file_path()
        .map_or_else(|()| reject(RejectKind::Url), local_file)
}

/// `path`, if it can name a local file: absolute, with no NUL byte.
fn local_file(path: PathBuf) -> LaunchArg {
    if path.is_absolute() && !path.as_os_str().as_encoded_bytes().contains(&0) {
        LaunchArg::File(path)
    } else {
        reject(RejectKind::Other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "0123456789abcdef0123456789abcdef01234567";
    /// [`HASH`] in base32.
    const HASH_BASE32: &str = "AERUKZ4JVPG66AJDIVTYTK6N54ASGRLH";
    /// The working directory of the launches: a folder of this system's kind.
    #[cfg(unix)]
    const CWD: &str = "/home/user/Downloads";
    #[cfg(windows)]
    const CWD: &str = r"C:\Users\user\Downloads";

    fn launched(arg: &str) -> LaunchArg {
        classify(OsStr::new(arg), Path::new(CWD))
    }

    fn file(path: &str) -> LaunchArg {
        LaunchArg::File(PathBuf::from(path))
    }

    fn rejected(kind: RejectKind) -> LaunchArg {
        LaunchArg::Rejected(RejectedArg { kind })
    }

    fn magnet(arg: &str) -> MagnetLink {
        match launched(arg) {
            LaunchArg::Magnet(link) => link,
            other => panic!("{arg:?} is not a magnet: {other:?}"),
        }
    }

    #[test]
    fn classify_takes_magnets_file_uris_and_paths() {
        // A magnet as KIO passes it: `%20` decoded to a raw space, the delimiters still encoded.
        let kio = magnet(&format!(
            "magnet:?xt=urn:btih:{HASH}&dn=a b&tr=udp%3A%2F%2Ft.example%3A80"
        ));
        assert_eq!(
            kio.as_str(),
            format!("magnet:?xt=urn:btih:{HASH}&dn=a b&tr=udp%3A%2F%2Ft.example%3A80")
        );
        assert_eq!(kio.hash().as_str(), HASH);
        assert_eq!(kio.name(), Some("a b"));
        assert_eq!(
            magnet(&format!("magnet:?xt=urn:btih:{HASH}&dn=Фильм+1%23")).name(),
            Some("Фильм+1#")
        );
        // The scheme in any case, and white space around the link, as a shell may pass it.
        for arg in [
            format!("MAGNET:?xt=urn:btih:{HASH}"),
            format!(" magnet:?xt=urn:btih:{HASH}"),
            format!("\tMagnet:?xt=urn:btih:{HASH}\n"),
        ] {
            assert_eq!(
                magnet(&arg).as_str(),
                format!("magnet:?xt=urn:btih:{HASH}"),
                "{arg:?}"
            );
        }

        #[cfg(unix)]
        for (arg, path) in [
            (
                "file:///home/user/my%20file%20%C3%BC%20%231.torrent",
                "/home/user/my file ü #1.torrent",
            ),
            (
                "file://localhost/home/user/a.torrent",
                "/home/user/a.torrent",
            ),
            ("FILE:///home/user/a.torrent", "/home/user/a.torrent"),
            // Pasted into a shell, with a raw space.
            ("file:///home/user/my a.torrent", "/home/user/my a.torrent"),
            // One slash (RFC 8089).
            ("file:/home/user/a.torrent", "/home/user/a.torrent"),
            ("FILE:/home/user/a.torrent", "/home/user/a.torrent"),
            // A backslash as launchers write it.
            ("file:///home/user/a%5Cb.torrent", "/home/user/a\\b.torrent"),
            // The URL rules resolve `..`, also written `%2e%2e`.
            ("file:///home/user/x/../a.torrent", "/home/user/a.torrent"),
            (
                "file:///home/user/x/%2e%2e/a.torrent",
                "/home/user/a.torrent",
            ),
            (
                "/home/user/my file ü #1.torrent",
                "/home/user/my file ü #1.torrent",
            ),
            ("/home/user/file://x.torrent", "/home/user/file://x.torrent"),
            // No `//` after the colon: a file name, not a scheme.
            ("a:b.torrent", "/home/user/Downloads/a:b.torrent"),
            ("file:a.torrent", "/home/user/Downloads/file:a.torrent"),
            ("http:a.torrent", "/home/user/Downloads/http:a.torrent"),
            ("1http://a", "/home/user/Downloads/1http://a"),
            ("magnet.torrent", "/home/user/Downloads/magnet.torrent"),
            // A path is not trimmed: a file name may start or end with a space.
            (" a.torrent ", "/home/user/Downloads/ a.torrent "),
            (&HASH[1..], &format!("{CWD}/{}", &HASH[1..])),
        ] {
            assert_eq!(launched(arg), file(path), "{arg:?}");
        }

        // The paths and the `file:` URIs of Windows.
        #[cfg(windows)]
        for (arg, path) in [
            // An absolute path is kept as it is.
            (r"C:\Users\user\a.torrent", r"C:\Users\user\a.torrent"),
            // A relative one joins the working directory.
            ("a.torrent", r"C:\Users\user\Downloads\a.torrent"),
            // So is a path on a network share.
            (r"\\server\share\a.torrent", r"\\server\share\a.torrent"),
            (
                "file:///C:/Users/user/a.torrent",
                r"C:\Users\user\a.torrent",
            ),
            // Percent-encoded UTF-8: the folder of a user named in Cyrillic.
            ("file:///C:/Users/%D0%90/a.torrent", r"C:\Users\А\a.torrent"),
        ] {
            assert_eq!(launched(arg), file(path), "{arg:?}");
        }

        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt as _;

            let not_utf8 = OsStr::from_bytes(b"/home/user/\xff.torrent");
            assert_eq!(
                classify(not_utf8, Path::new(CWD)),
                LaunchArg::File(PathBuf::from(not_utf8))
            );
        }
    }

    #[test]
    fn classify_rejects_urls_options_bare_hashes_and_hosts() {
        for arg in [
            "http://example.com/a.torrent",
            "https://example.com/a.torrent",
            "HTTPS://example.com/a.torrent",
            "ftp://example.com/a.torrent",
            "sftp://example.com/a.torrent",
            "file://example.com/home/user/a.torrent",
            "file://192.168.1.5/home/user/a.torrent",
            // A network share, as Windows names one in a URI.
            "file://server/share/a.torrent",
            "file:///home/user/a.torrent?x=1",
            "file:///home/user/a.torrent?",
            "file:///home/user/a.torrent#top",
            // `Url::parse` would drop these and read another path.
            "file:///home/user/a\n.torrent",
            "file:///home/user/a\t.torrent",
            "file:///home/user/a.torrent ",
            "file://[::1/a.torrent",
            // The URL rules read a backslash as `/`, which would name another file.
            "file:///home/user/a\\b.torrent",
            "file:/home/user/a\\b.torrent",
        ] {
            assert_eq!(launched(arg), rejected(RejectKind::Url), "{arg:?}");
        }
        for arg in ["-x", "--", "--help", "-", "-/a.torrent"] {
            assert_eq!(launched(arg), rejected(RejectKind::Option), "{arg:?}");
        }
        for arg in [HASH, &HASH.to_uppercase(), HASH_BASE32] {
            assert_eq!(launched(arg), rejected(RejectKind::BareHash), "{arg:?}");
        }
        assert_eq!(launched(""), rejected(RejectKind::Empty));
        for (arg, error) in [
            (
                format!("magnet:?xt=urn:btmh:1220{}", "a".repeat(64)),
                MagnetError::V2Only,
            ),
            ("magnet:?dn=x".to_owned(), MagnetError::NoHash),
            (format!("magnet:xt=urn:btih:{HASH}"), MagnetError::NotMagnet),
            (
                format!("magnet:?xt=urn:btih:{HASH}\nmagnet:?xt=urn:btih:{HASH}"),
                MagnetError::ControlCharacter { offset: 60 },
            ),
        ] {
            assert_eq!(
                launched(&arg),
                rejected(RejectKind::InvalidMagnet(error)),
                "{arg:?}"
            );
        }
        // A NUL can come from a URI, and from a forwarded argument.
        #[cfg(unix)]
        let nul_in_a_uri = "file:///home/user/a%00b.torrent";
        #[cfg(windows)]
        let nul_in_a_uri = "file:///C:/Users/user/a%00b.torrent";
        assert_eq!(launched(nul_in_a_uri), rejected(RejectKind::Other));
        assert_eq!(launched("a\0b.torrent"), rejected(RejectKind::Other));

        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt as _;

            let launched_bytes = |bytes: &[u8]| classify(OsStr::from_bytes(bytes), Path::new(CWD));
            let mut not_utf8 = format!("magnet:?xt=urn:btih:{HASH}&dn=").into_bytes();
            not_utf8.push(0xff);
            assert_eq!(
                launched_bytes(&not_utf8),
                rejected(RejectKind::NotUtf8Magnet)
            );
            assert_eq!(
                launched_bytes(b"file:///home/user/\xff.torrent"),
                rejected(RejectKind::Url)
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn relative_paths_join_the_senders_cwd() {
        let in_dir = |arg: &str, cwd: &str| classify(OsStr::new(arg), Path::new(cwd));

        assert_eq!(
            in_dir("a.torrent", CWD),
            file("/home/user/Downloads/a.torrent")
        );
        assert_eq!(in_dir("sub/a.torrent", "/srv"), file("/srv/sub/a.torrent"));
        // Opened as the user named it: `..` is not resolved, and `~` is the shell's to expand.
        assert_eq!(
            in_dir("../x/a.torrent", CWD),
            file("/home/user/Downloads/../x/a.torrent")
        );
        assert_eq!(
            in_dir("~/a.torrent", CWD),
            file("/home/user/Downloads/~/a.torrent")
        );
        // An absolute path does not depend on the cwd.
        assert_eq!(in_dir("/srv/a.torrent", CWD), file("/srv/a.torrent"));
        assert_eq!(in_dir("/srv/a.torrent", ""), file("/srv/a.torrent"));
        // A cwd that is not absolute cannot place a relative path; the file is not looked for
        // relative to this process instead.
        assert_eq!(in_dir("a.torrent", ""), rejected(RejectKind::Other));
        assert_eq!(
            in_dir("a.torrent", "relative/dir"),
            rejected(RejectKind::Other)
        );
    }

    #[cfg(windows)]
    #[test]
    fn relative_paths_join_the_senders_cwd() {
        let in_dir = |arg: &str, cwd: &str| classify(OsStr::new(arg), Path::new(cwd));

        assert_eq!(
            in_dir("a.torrent", CWD),
            file(r"C:\Users\user\Downloads\a.torrent")
        );
        assert_eq!(
            in_dir(r"sub\a.torrent", r"D:\srv"),
            file(r"D:\srv\sub\a.torrent")
        );
        // Opened as the user named it: `..` is not resolved.
        assert_eq!(
            in_dir(r"..\x\a.torrent", CWD),
            file(r"C:\Users\user\Downloads\..\x\a.torrent")
        );
        // An absolute path does not depend on the cwd, nor does a path on a network share.
        assert_eq!(in_dir(r"D:\srv\a.torrent", CWD), file(r"D:\srv\a.torrent"));
        assert_eq!(in_dir(r"D:\srv\a.torrent", ""), file(r"D:\srv\a.torrent"));
        assert_eq!(
            in_dir(r"\\server\share\a.torrent", ""),
            file(r"\\server\share\a.torrent")
        );
        // A cwd that is not absolute cannot place a relative path; the file is not looked for
        // relative to this process instead.
        assert_eq!(in_dir("a.torrent", ""), rejected(RejectKind::Other));
        assert_eq!(
            in_dir("a.torrent", r"relative\dir"),
            rejected(RejectKind::Other)
        );
    }
}
