//! A [`Frame`] and a [`Reply`] as bytes on the socket.
//!
//! ```text
//! launch       "4x4-open\0" "1\0" ("<key>=<value>\0")*     then it closes its side
//! running 4x4  "ok\n" | "closing\n" | "bad\n"
//! ```
//!
//! The keys are `cwd` (once), `activation-token` and `desktop-startup-id` (at most once each) and
//! `arg` (once per argument, in order): the names of `platform_data` in D-Bus's
//! `org.freedesktop.Application`, so that a D-Bus front end could map onto them. A value is the
//! raw bytes of an `OsStr`; arguments, environment values and paths never hold a NUL byte, so
//! the format keeps a file name that is not UTF-8. A key 4x4 does not know is skipped, so that a
//! newer 4x4 may add some; a frame of another version is refused. A frame takes at most
//! [`MAX_FRAME_BYTES`], above Linux's default limit of 2 MiB on a program's arguments, so that the
//! arguments of any real launch fit. Its `cwd` is empty (unknown) or an absolute path of at most
//! [`MAX_CWD_BYTES`]: each relative argument told apart is joined to it. glibc's `getcwd` may give
//! a longer one, which a launch sends as unknown.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt as _;
use std::path::Path;

pub(crate) use super::MAX_CWD_BYTES;
use super::{Frame, Reply};

/// The most bytes a frame takes: 4 MiB.
pub(crate) const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
/// The most bytes an answer takes.
pub(crate) const MAX_REPLY_BYTES: usize = 16;

const MAGIC: &[u8] = b"4x4-open\0";
const VERSION: &[u8] = b"1\0";
const CWD: &[u8] = b"cwd";
const ACTIVATION_TOKEN: &[u8] = b"activation-token";
const DESKTOP_STARTUP_ID: &[u8] = b"desktop-startup-id";
const ARG: &[u8] = b"arg";

/// Why a frame cannot be written or read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum WireError {
    /// It would take, or takes, more than [`MAX_FRAME_BYTES`].
    #[error("the frame has more than {MAX_FRAME_BYTES} bytes")]
    TooLarge,
    /// A value holds a NUL byte, which ends a field.
    #[error("a value holds a NUL byte")]
    Nul,
    /// It does not start as a frame of 4x4 does.
    #[error("not a frame of 4x4")]
    NotAFrame,
    /// It is a frame of another version.
    #[error("a frame of another version")]
    OtherVersion,
    /// A field has no `=`, or the last one no NUL at its end.
    #[error("a field without a key, or without a NUL at its end")]
    Malformed,
    /// A field that is given at most once is given again.
    #[error("the field {0} is given more than once")]
    Repeated(&'static str),
    /// There is no `cwd`.
    #[error("the frame has no working directory")]
    NoCwd,
    /// The `cwd` is relative, or longer than [`MAX_CWD_BYTES`].
    #[error("the working directory is relative or longer than {MAX_CWD_BYTES} bytes")]
    BadCwd,
}

/// `frame` as the bytes a launch sends.
///
/// # Errors
///
/// [`WireError::BadCwd`] for a working directory that is relative or too long, [`WireError::Nul`]
/// for a value with a NUL byte, and [`WireError::TooLarge`] for a frame of more than
/// [`MAX_FRAME_BYTES`].
pub(crate) fn encode(frame: &Frame) -> Result<Vec<u8>, WireError> {
    check_cwd(frame.cwd.as_os_str())?;
    let mut bytes = [MAGIC, VERSION].concat();
    put(&mut bytes, CWD, frame.cwd.as_os_str())?;
    if let Some(token) = &frame.activation_token {
        put(&mut bytes, ACTIVATION_TOKEN, token)?;
    }
    if let Some(id) = &frame.desktop_startup_id {
        put(&mut bytes, DESKTOP_STARTUP_ID, id)?;
    }
    for arg in &frame.args {
        put(&mut bytes, ARG, arg)?;
    }
    Ok(bytes)
}

/// Appends the field `key=value` and its NUL to `bytes`, if the frame stays within
/// [`MAX_FRAME_BYTES`].
fn put(bytes: &mut Vec<u8>, key: &[u8], value: &OsStr) -> Result<(), WireError> {
    let value = value.as_bytes();
    if value.contains(&0) {
        return Err(WireError::Nul);
    }
    let length = bytes
        .len()
        .saturating_add(key.len())
        .saturating_add(value.len())
        .saturating_add(2);
    if length > MAX_FRAME_BYTES {
        return Err(WireError::TooLarge);
    }
    bytes.extend_from_slice(key);
    bytes.push(b'=');
    bytes.extend_from_slice(value);
    bytes.push(0);
    Ok(())
}

/// A frame as it was read: views of the bytes it was read from, which [`decode`] checked. The
/// arguments are split from them again as they are taken ([`args`](Self::args)), so a frame of
/// 4 MiB of one-byte arguments costs its bytes and nothing for each argument, where an owned
/// [`Frame`] takes some 70 MiB for them. [`to_frame`](Self::to_frame) makes the owned one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Received<'a> {
    /// The working directory: empty (unknown) or absolute, and at most [`MAX_CWD_BYTES`].
    pub(crate) cwd: &'a OsStr,
    /// `XDG_ACTIVATION_TOKEN`, the token of a Wayland launch.
    pub(crate) activation_token: Option<&'a OsStr>,
    /// `DESKTOP_STARTUP_ID`, the token of an X11 launch.
    pub(crate) desktop_startup_id: Option<&'a OsStr>,
    /// How many arguments the frame holds.
    pub(crate) arg_count: usize,
    /// The fields of the frame, between its version and its last NUL: each `key=value`, with a
    /// NUL between two.
    fields: &'a [u8],
}

impl<'a> Received<'a> {
    /// The working directory, which each relative argument is relative to.
    pub(crate) fn cwd(self) -> &'a Path {
        Path::new(self.cwd)
    }

    /// The arguments, in order, each a view of the frame's bytes.
    pub(crate) fn args(self) -> impl Iterator<Item = &'a OsStr> + use<'a> {
        self.fields
            .split(|&byte| byte == 0)
            .filter_map(field)
            .filter_map(|(key, value)| (key == ARG).then_some(value))
    }

    /// The frame as an owned [`Frame`].
    pub(crate) fn to_frame(self) -> Frame {
        Frame {
            cwd: self.cwd().to_path_buf(),
            activation_token: self.activation_token.map(OsStr::to_os_string),
            desktop_startup_id: self.desktop_startup_id.map(OsStr::to_os_string),
            args: self.args().map(OsStr::to_os_string).collect(),
        }
    }
}

/// The frame `bytes` hold, checked in one pass over them, without copying any of it.
///
/// # Errors
///
/// The [`WireError`] of bytes that are not a whole frame of this version.
pub(crate) fn decode(bytes: &[u8]) -> Result<Received<'_>, WireError> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(WireError::TooLarge);
    }
    let rest = bytes.strip_prefix(MAGIC).ok_or(WireError::NotAFrame)?;
    let rest = rest.strip_prefix(VERSION).ok_or(WireError::OtherVersion)?;
    let fields = rest.strip_suffix(b"\0").ok_or(WireError::Malformed)?;
    let (mut cwd, mut activation_token, mut desktop_startup_id) = (None, None, None);
    let mut arg_count = 0;
    for bytes in fields.split(|&byte| byte == 0) {
        let (key, value) = field(bytes).ok_or(WireError::Malformed)?;
        match key {
            CWD => once(&mut cwd, value, "cwd")?,
            ACTIVATION_TOKEN => once(&mut activation_token, value, "activation-token")?,
            DESKTOP_STARTUP_ID => once(&mut desktop_startup_id, value, "desktop-startup-id")?,
            ARG => arg_count += 1,
            // A field of a newer 4x4.
            _ => {}
        }
    }
    let cwd = cwd.ok_or(WireError::NoCwd)?;
    check_cwd(cwd)?;
    Ok(Received {
        cwd,
        activation_token,
        desktop_startup_id,
        arg_count,
        fields,
    })
}

/// The key and the value of the field `bytes`, which holds no NUL; `None` without an `=`.
fn field(bytes: &[u8]) -> Option<(&[u8], &OsStr)> {
    let equals = bytes.iter().position(|&byte| byte == b'=')?;
    let (key, value) = bytes.split_at_checked(equals)?;
    Some((key, OsStr::from_bytes(value.get(1..).unwrap_or_default())))
}

/// Whether `cwd` may be a frame's working directory: empty, or absolute and at most
/// [`MAX_CWD_BYTES`].
fn check_cwd(cwd: &OsStr) -> Result<(), WireError> {
    if cwd.len() > MAX_CWD_BYTES || !(cwd.is_empty() || Path::new(cwd).is_absolute()) {
        return Err(WireError::BadCwd);
    }
    Ok(())
}

/// Sets `slot`, the field `key`, to `value`, unless it was set before.
fn once<'a>(
    slot: &mut Option<&'a OsStr>,
    value: &'a OsStr,
    key: &'static str,
) -> Result<(), WireError> {
    if slot.replace(value).is_some() {
        return Err(WireError::Repeated(key));
    }
    Ok(())
}

/// `reply` as the bytes the running 4x4 sends.
pub(crate) fn reply_bytes(reply: Reply) -> &'static [u8] {
    match reply {
        Reply::Ok => b"ok\n",
        Reply::Closing => b"closing\n",
        Reply::Bad => b"bad\n",
    }
}

/// The answer `bytes` hold, if they hold one.
pub(crate) fn parse_reply(bytes: &[u8]) -> Option<Reply> {
    [Reply::Ok, Reply::Closing, Reply::Bad]
        .into_iter()
        .find(|reply| reply_bytes(*reply) == bytes)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;
    use std::path::PathBuf;

    use super::*;

    /// The owned frame `bytes` hold.
    fn decoded(bytes: &[u8]) -> Result<Frame, WireError> {
        decode(bytes).map(Received::to_frame)
    }

    fn os(bytes: &[u8]) -> OsString {
        OsString::from_vec(bytes.to_vec())
    }

    fn frame() -> Frame {
        Frame {
            cwd: PathBuf::from(os(b"/home/user/D\xf6wnloads")),
            activation_token: Some("kwin-7".into()),
            desktop_startup_id: Some("startup_id 1".into()),
            args: vec![
                os(b"Fr\xe9d\xe9ric.torrent"),
                "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=a b".into(),
                OsString::new(),
                "with=equals".into(),
            ],
        }
    }

    #[test]
    fn a_frame_round_trips_non_utf8_args_and_its_cwd() {
        let frame = frame();
        assert!(frame.cwd.to_str().is_none(), "the cwd is not UTF-8");

        let bytes = encode(&frame).expect("a frame");

        assert!(bytes.starts_with(b"4x4-open\x001\x00cwd=/home/user/D\xf6wnloads\x00"));
        assert_eq!(decoded(&bytes), Ok(frame));
        let bare = Frame {
            cwd: "/".into(),
            ..Frame::default()
        };
        assert_eq!(decoded(&encode(&bare).expect("a frame")), Ok(bare));
    }

    #[test]
    fn the_arguments_keep_their_order_and_unknown_keys_are_skipped() {
        let bytes = b"4x4-open\x001\x00arg=b\x00future=x\x00cwd=/w\x00arg=a\x00";

        assert_eq!(
            decoded(bytes),
            Ok(Frame {
                cwd: "/w".into(),
                args: vec!["b".into(), "a".into()],
                ..Frame::default()
            })
        );
    }

    #[test]
    fn a_frame_that_is_not_one_of_this_version_is_refused() {
        let cases: [(&[u8], WireError); 9] = [
            (b"", WireError::NotAFrame),
            (b"GET / HTTP/1.1\r\n\r\n", WireError::NotAFrame),
            (b"4x4-open\x002\x00cwd=/\x00", WireError::OtherVersion),
            (b"4x4-open\x001\x00", WireError::Malformed),
            (b"4x4-open\x001\x00cwd=/", WireError::Malformed),
            (
                b"4x4-open\x001\x00cwd=/\x00noequals\x00",
                WireError::Malformed,
            ),
            (b"4x4-open\x001\x00cwd=/\x00\x00", WireError::Malformed),
            (b"4x4-open\x001\x00arg=a\x00", WireError::NoCwd),
            (
                b"4x4-open\x001\x00cwd=/\x00cwd=/etc\x00",
                WireError::Repeated("cwd"),
            ),
        ];
        for (bytes, error) in cases {
            assert_eq!(decode(bytes), Err(error), "{:?}", bytes.escape_ascii());
        }
    }

    #[test]
    fn a_frame_over_four_mib_is_neither_written_nor_read() {
        let large = Frame {
            cwd: "/".into(),
            args: vec![os(&vec![b'a'; MAX_FRAME_BYTES])],
            ..Frame::default()
        };
        assert_eq!(encode(&large), Err(WireError::TooLarge));

        let mut bytes = encode(&Frame {
            cwd: "/".into(),
            ..Frame::default()
        })
        .expect("a frame");
        bytes.resize(MAX_FRAME_BYTES + 1, b'a');
        assert_eq!(decode(&bytes), Err(WireError::TooLarge));
    }

    #[test]
    fn a_working_directory_that_is_relative_or_too_long_is_refused() {
        let at = |cwd: &str| Frame {
            cwd: cwd.into(),
            ..Frame::default()
        };
        let longest = format!("/{}", "d".repeat(MAX_CWD_BYTES - 1));
        for fine in ["", "/", longest.as_str()] {
            let bytes = encode(&at(fine)).expect("a frame");
            assert_eq!(decoded(&bytes), Ok(at(fine)), "{} bytes", fine.len());
        }

        let too_long = format!("/{}", "d".repeat(MAX_CWD_BYTES));
        for refused in ["home/user", too_long.as_str()] {
            assert_eq!(encode(&at(refused)), Err(WireError::BadCwd));
            let mut bytes = b"4x4-open\x001\x00cwd=".to_vec();
            bytes.extend_from_slice(refused.as_bytes());
            bytes.push(0);
            assert_eq!(
                decode(&bytes),
                Err(WireError::BadCwd),
                "{} bytes",
                refused.len()
            );
        }
    }

    #[test]
    fn a_frame_is_read_as_views_of_its_bytes_whatever_its_count_of_arguments() {
        /// What a frame read holds is copied freely: it owns nothing, no list of arguments either.
        fn owns_nothing<T: Copy>(_: &T) {}
        // As many one-byte arguments as 4 MiB holds: each made an `OsString` of 32 bytes or more,
        // and the list of them more, some 70 MiB at the peak.
        let count = (MAX_FRAME_BYTES - 64) / 6;
        let frame = Frame {
            cwd: "/w".into(),
            activation_token: Some("kwin-7".into()),
            args: vec!["a".into(); count],
            ..Frame::default()
        };
        let bytes = encode(&frame).expect("a frame");
        assert!(count > 690_000, "{count}");

        let received = decode(&bytes).expect("a frame");

        owns_nothing(&received);
        let within = bytes.as_ptr_range();
        let in_bytes = |value: &OsStr| within.contains(&value.as_bytes().as_ptr());
        assert!(in_bytes(received.cwd));
        assert!(received.activation_token.is_some_and(in_bytes));
        assert_eq!(received.arg_count, count);
        let mut args = 0;
        for arg in received.args() {
            assert!(in_bytes(arg) && arg == "a");
            args += 1;
        }
        assert_eq!(args, count);
        assert_eq!(received.to_frame(), frame);
    }

    #[test]
    fn a_value_with_a_nul_byte_is_not_written() {
        let frame = Frame {
            cwd: "/".into(),
            args: vec![os(b"a\0arg=b")],
            ..Frame::default()
        };

        assert_eq!(encode(&frame), Err(WireError::Nul));
    }

    #[test]
    fn the_answers_are_lines_and_nothing_else_is_one() {
        for reply in [Reply::Ok, Reply::Closing, Reply::Bad] {
            let bytes = reply_bytes(reply);
            assert!(bytes.len() <= MAX_REPLY_BYTES);
            assert_eq!(parse_reply(bytes), Some(reply));
        }
        assert_eq!(reply_bytes(Reply::Ok), b"ok\n");
        assert_eq!(parse_reply(b"ok"), None);
        assert_eq!(parse_reply(b"ok\nok\n"), None);
        assert_eq!(parse_reply(b""), None);
    }
}
