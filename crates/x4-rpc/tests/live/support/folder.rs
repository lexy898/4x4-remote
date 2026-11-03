//! The folder a test torrent downloads into, and whether a folder exists on the server.

use x4_rpc::{Client, DownloadDir, RpcError, UreqTransport};

use super::NO_SUCH_FOLDER;

/// The folder a test torrent downloads into, as the daemon sees it: the test directory itself,
/// or a folder directly in it that this run names (`4x4-live-new-<hex>`), which must not exist
/// before the test: the daemon creates it when the torrent starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TestFolder {
    /// The test directory, normalized.
    pub(super) test_dir: DownloadDir,
    /// The name of this run's folder in it; `None` for the test directory itself.
    subfolder: Option<String>,
    /// The folder as it is sent and compared: the test directory, or this run's folder in it.
    remote: DownloadDir,
}

impl TestFolder {
    /// The test directory itself.
    pub(crate) fn test_dir(test_dir: &DownloadDir) -> Self {
        Self {
            test_dir: test_dir.clone(),
            subfolder: None,
            remote: test_dir.clone(),
        }
    }

    /// The folder named after `seed`, `4x4-live-new-<seed in hex>`, directly in the test
    /// directory: one path component of this run's own, with nothing to normalize away.
    #[expect(
        clippy::panic,
        reason = "a live test stops at the first failure; nothing was added yet"
    )]
    pub(crate) fn new_subfolder(test_dir: &DownloadDir, seed: u64) -> Self {
        let name = format!("4x4-live-new-{seed:016x}");
        let path = format!("{}/{name}", test_dir.as_str());
        let remote = DownloadDir::new(&path)
            .unwrap_or_else(|error| panic!("{path:?} is not a folder ({error})"));
        assert_eq!(
            remote.parent().as_ref(),
            Some(test_dir),
            "{remote:?} is not directly in the test directory"
        );
        Self {
            test_dir: test_dir.clone(),
            subfolder: Some(name),
            remote,
        }
    }

    /// The folder, as it is sent.
    pub(crate) fn remote(&self) -> &DownloadDir {
        &self.remote
    }

    /// The name of this run's folder in the test directory; `None` for the test directory
    /// itself.
    pub(crate) fn subfolder(&self) -> Option<&str> {
        self.subfolder.as_deref()
    }

    /// Whether a torrent's `download_dir`, as the daemon reports it, is exactly this folder,
    /// both in their normalized forms: the guard of every removal. A folder with a control
    /// character anywhere is not it, even at either end, where normalizing would trim the
    /// character; nor is a folder that does not normalize. So the data of such a torrent is never
    /// deleted.
    pub(crate) fn is(&self, download_dir: &str) -> bool {
        !download_dir.contains(char::is_control)
            && DownloadDir::new(download_dir).is_ok_and(|dir| dir == self.remote)
    }
}

/// Whether `folder` exists on the server, as `free_space` tells: an answer means it does, and
/// [`NO_SUCH_FOLDER`] that it does not. A read, so it changes nothing.
///
/// # Errors
///
/// Any other error of the call: whether the folder is there is then unknown.
pub(crate) fn folder_exists(
    client: &mut Client<UreqTransport>,
    folder: &TestFolder,
) -> Result<bool, String> {
    match client.free_space(folder.remote().as_str()) {
        Ok(_) => Ok(true),
        Err(RpcError::Server { code, .. }) if code == NO_SUCH_FOLDER => Ok(false),
        Err(error) => Err(format!(
            "free_space of {:?} failed: {error}",
            folder.remote()
        )),
    }
}
