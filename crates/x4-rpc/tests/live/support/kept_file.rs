//! The data file a test keeps, and deletes itself, after removing its torrent without the data.

use std::path::Path;

use super::LocalData;

/// The data file of a test torrent that was removed while the daemon kept its data. The daemon no
/// longer knows the file, so deleting it is the test's job: [`KeptFile::delete`] deletes it and
/// the directory if this run created it, and dropping it deletes them too, so a failed check
/// leaves nothing behind.
#[derive(Debug)]
pub(crate) struct KeptFile(pub(crate) LocalData);

impl KeptFile {
    /// The file, as mounted on this machine.
    pub(crate) fn path(&self) -> &Path {
        &self.0.path
    }

    /// Deletes the file, and the directory if this run created it.
    pub(crate) fn delete(mut self) {
        let problems = self.0.delete();
        assert!(
            problems.is_empty(),
            "deleting the kept file {} failed: {problems:#?}",
            self.0.path.display()
        );
    }
}
