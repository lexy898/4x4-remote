//! Test helper: directories the owner cannot write to. Unix only.
#![cfg(unix)]

use std::fs::{self, Permissions};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

/// A directory made read-only; writable again when dropped, so the temporary directory around it
/// can be removed.
pub(crate) struct ReadOnly(PathBuf);

impl Drop for ReadOnly {
    fn drop(&mut self) {
        if let Err(error) = fs::set_permissions(&self.0, Permissions::from_mode(0o755)) {
            eprintln!("cannot make {} writable again: {error}", self.0.display());
        }
    }
}

/// Makes `dir` read-only. `None` when that has no effect, because the tests run as root (which
/// ignores the mode); the test should then return, as it cannot show anything.
pub(crate) fn make_read_only(dir: &Path) -> Option<ReadOnly> {
    fs::set_permissions(dir, Permissions::from_mode(0o555)).expect("chmod");
    let guard = ReadOnly(dir.to_path_buf());
    let probe = dir.join(".probe");
    if fs::File::create(&probe).is_ok() {
        fs::remove_file(&probe).expect("the probe file is removed");
        eprintln!("skipped: the mode of a directory does not stop this user (root?)");
        return None;
    }
    Some(guard)
}
