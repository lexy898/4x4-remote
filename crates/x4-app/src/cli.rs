//! The one option of the command line that 4x4 answers itself: `--version` (or `-V`), which prints
//! the product name and version and starts nothing. The `4x4` binary asks [`wants_version`] before
//! it touches the allocator, the log, the lock or the window; every other argument is a file or a
//! link to add, or an option that [`launch_requests`](crate::adding::launch_requests) refuses.

use std::ffi::OsStr;

use crate::PRODUCT_NAME;

/// The option that asks for the version.
const VERSION_LONG: &str = "--version";
/// Its short form.
const VERSION_SHORT: &str = "-V";

/// Whether `args`, the arguments 4x4 was started with (without the program's name), ask for the
/// version: `--version` or `-V`, as a whole argument, anywhere before the first `--`.
///
/// Anything else is no such request: a file or link that merely contains the text, an argument
/// that is not Unicode, a different option (those are refused later, as unknown), and every
/// argument after a `--`, which shells and launchers pass before the files.
#[must_use]
pub fn wants_version<I, A>(args: I) -> bool
where
    I: IntoIterator<Item = A>,
    A: AsRef<OsStr>,
{
    for arg in args {
        let arg = arg.as_ref();
        if arg == OsStr::new("--") {
            return false;
        }
        if arg == OsStr::new(VERSION_LONG) || arg == OsStr::new(VERSION_SHORT) {
            return true;
        }
    }
    false
}

/// The line `4x4 --version` prints: the product name and the version of this build, such as
/// `4x4 Remote 1.0.0`. It has no words of its own, so it needs no message in `i18n/`.
#[must_use]
pub fn version_line() -> String {
    format!("{PRODUCT_NAME} {}", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_version_options_ask_for_the_version() {
        for args in [
            &["--version"][..],
            &["-V"],
            &["a.torrent", "--version"],
            &[
                "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
                "-V",
            ],
        ] {
            assert!(wants_version(args), "{args:?}");
        }
    }

    #[test]
    fn other_arguments_are_no_version_request() {
        for args in [
            &[][..],
            &["-v"],
            &["--versions"],
            &["--Version"],
            &["--", "--version"],
            &["--version.torrent"],
            &["magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=--version"],
        ] {
            assert!(!wants_version(args), "{args:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn an_argument_that_is_not_unicode_is_no_version_request() {
        use std::os::unix::ffi::OsStrExt;
        assert!(!wants_version([OsStr::from_bytes(b"--version\xff")]));
    }

    #[test]
    fn the_version_line_is_the_product_name_and_the_package_version() {
        assert_eq!(
            version_line(),
            format!("4x4 Remote {}", env!("CARGO_PKG_VERSION"))
        );
    }
}
