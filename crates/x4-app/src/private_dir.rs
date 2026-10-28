//! Creating the app's own directories so that only the owner can use them.

use std::fs::DirBuilder;
use std::io;
use std::path::Path;

/// Creates `path` and the missing directories above it, readable and writable by the owner only
/// (mode 0700 on Unix, as the XDG base directory specification asks for directories it creates),
/// because logs at a verbose level may hold URLs. A directory that exists already keeps its mode.
///
/// # Errors
///
/// Any error of creating a directory, such as no permission.
pub(crate) fn create_private_dir(path: &Path) -> io::Result<()> {
    let mut builder = DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(path)
}

#[cfg(test)]
mod tests {
    // The mode bits are Unix's, so the tests are.
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt as _;

    #[cfg(unix)]
    use super::*;

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        fs::metadata(path).expect("metadata").permissions().mode() & 0o777
    }

    #[cfg(unix)]
    #[test]
    fn created_directories_are_private_to_the_owner() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let leaf = dir.path().join("a").join("b").join("c");

        create_private_dir(&leaf).expect("created");

        for created in [leaf.clone(), leaf.join(".."), leaf.join("../..")] {
            assert_eq!(mode(&created), 0o700, "{}", created.display());
        }
    }

    #[cfg(unix)]
    #[test]
    fn an_existing_directory_keeps_its_mode() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let existing = dir.path().join("existing");
        fs::create_dir(&existing).expect("created");
        fs::set_permissions(&existing, fs::Permissions::from_mode(0o755)).expect("chmod");

        create_private_dir(&existing).expect("an existing directory is fine");

        assert_eq!(mode(&existing), 0o755);
    }
}
