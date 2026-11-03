//! The test torrent's data on this machine, of which only what this run created is deleted.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{file_is_gone, wait_until};

/// How long the daemon's temporary folder of a deletion may keep the test directory non-empty.
const DIR_REMOVAL_TIMEOUT: Duration = Duration::from_secs(5);

/// The test torrent's data in the local test directory: the file of a single-file torrent, or the
/// folder of a multi-file one and the files in it.
///
/// Deletes only what this run created: each file once it was created, the folder once it was
/// created and its files are gone, and the test directory if this run created it and nothing else
/// is in it. In a folder of this run's own in the test directory, which the daemon creates, all
/// that the daemon makes there for the torrent is this run's too ([`LocalData::unwritten`]).
/// Dropping it deletes them too, so a test that fails before its torrent is added leaves nothing
/// behind either.
#[derive(Debug)]
pub(crate) struct LocalData {
    /// What the daemon names after the torrent: its file, or its folder.
    pub(super) path: PathBuf,
    /// The test directory.
    dir: PathBuf,
    /// This run's folder directly in the test directory, which holds `path`; `None` when `path`
    /// is in the test directory itself.
    subfolder: Option<PathBuf>,
    /// The files this run created and has not deleted yet.
    files: Vec<PathBuf>,
    /// Set from creating the folder of a multi-file torrent until it is removed.
    owns_folder: bool,
    /// Set for `subfolder` until it is removed.
    owns_subfolder: bool,
    /// Set from creating the test directory until it is removed.
    owns_dir: bool,
}

impl LocalData {
    /// Creates `dir/name` (never overwriting a file) holding `data`, flushed to the server.
    ///
    /// `dir` itself is created if missing, but not its parents: a missing parent means the
    /// mount is not there.
    pub(crate) fn write_file(dir: &Path, name: &str, data: &[u8]) -> io::Result<Self> {
        let mut local = Self::in_dir(dir, name)?;
        local.create_file(local.path.clone(), data)?;
        Ok(local)
    }

    /// Creates the folder `dir/name`, which must not exist yet, and in it each of `files`, a name
    /// and its data, flushed to the server.
    ///
    /// `dir` itself is created if missing, but not its parents, as in
    /// [`LocalData::write_file`].
    pub(crate) fn write_folder(
        dir: &Path,
        name: &str,
        files: &[(&str, &[u8])],
    ) -> io::Result<Self> {
        let mut local = Self::in_dir(dir, name)?;
        fs::create_dir(&local.path)?;
        local.owns_folder = true;
        for (file, data) in files {
            local.create_file(local.path.join(file), data)?;
        }
        Ok(local)
    }

    /// Where the daemon puts the data `name` of a test torrent whose data this run did not write:
    /// the folder `name` holding `files` (names directly in it), or the file `name` when `files`
    /// is empty.
    ///
    /// In the test directory `dir` itself, none of it is this run's to delete: what the daemon
    /// makes there is the daemon's to delete with the torrent. In `subfolder`, this run's own
    /// folder directly in `dir`, which does not exist yet (the caller asked the daemon), all the
    /// daemon makes is for this run's torrent: the files, the torrent's folder and `subfolder`
    /// itself are deleted, once the daemon has removed the torrent, wherever they are still
    /// there. Nothing is looked up there before then: over NFS, a lookup of a missing folder could
    /// hide it for a while once the daemon has made it.
    ///
    /// `dir` itself is created if missing, but not its parents, as in [`LocalData::write_file`].
    pub(crate) fn unwritten(
        dir: &Path,
        subfolder: Option<&str>,
        name: &str,
        files: &[&str],
    ) -> io::Result<Self> {
        let mut local = Self::in_dir(dir, name)?;
        if let Some(subfolder) = subfolder {
            let subfolder = dir.join(subfolder);
            local.path = subfolder.join(name);
            if files.is_empty() {
                local.files = vec![local.path.clone()];
            } else {
                local.files = files.iter().map(|file| local.path.join(file)).collect();
                local.owns_folder = true;
            }
            local.subfolder = Some(subfolder);
            local.owns_subfolder = true;
        }
        Ok(local)
    }

    /// Nothing yet but the test directory `dir`, created if missing, for the data `name`.
    fn in_dir(dir: &Path, name: &str) -> io::Result<Self> {
        let owns_dir = match fs::create_dir(dir) {
            Ok(()) => true,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => false,
            Err(error) => return Err(error),
        };
        Ok(Self {
            path: dir.join(name),
            dir: dir.to_owned(),
            subfolder: None,
            files: Vec::new(),
            owns_folder: false,
            owns_subfolder: false,
            owns_dir,
        })
    }

    /// Creates the file `path`, never overwriting one, with `data`.
    fn create_file(&mut self, path: PathBuf, data: &[u8]) -> io::Result<()> {
        let mut file = File::create_new(&path)?;
        self.files.push(path);
        file.write_all(data)?;
        // On NFS, make sure the server has the data before the daemon reads it.
        file.sync_all()
    }

    pub(crate) fn is_clean(&self) -> bool {
        self.files.is_empty() && !self.owns_folder && !self.owns_subfolder && !self.owns_dir
    }

    /// Whether every file this run created is gone, as when the daemon deleted them.
    pub(crate) fn files_are_gone(&self) -> bool {
        self.files.iter().all(|file| file_is_gone(file))
    }

    /// Moves what this run owns into a new `LocalData`, which deletes it from then on; this one
    /// is left clean.
    pub(crate) fn hand_over(&mut self) -> Self {
        Self {
            path: self.path.clone(),
            dir: self.dir.clone(),
            subfolder: self.subfolder.clone(),
            files: std::mem::take(&mut self.files),
            owns_folder: std::mem::take(&mut self.owns_folder),
            owns_subfolder: std::mem::take(&mut self.owns_subfolder),
            owns_dir: std::mem::take(&mut self.owns_dir),
        }
    }

    /// Deletes the files, the folders and the directory this run created; returns what went
    /// wrong.
    pub(crate) fn delete(&mut self) -> Vec<String> {
        let mut problems = Vec::new();
        self.files.retain(|file| match fs::remove_file(file) {
            Ok(()) => false,
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => {
                problems.push(format!("cannot delete {}: {error}", file.display()));
                true
            }
        });
        if self.owns_folder && self.files.is_empty() {
            match remove_empty_dir(&self.path) {
                Ok(()) => self.owns_folder = false,
                Err(problem) => problems.push(problem),
            }
        }
        if self.owns_subfolder && !self.owns_folder && self.files.is_empty() {
            match self.subfolder.as_deref().map_or(Ok(()), remove_empty_dir) {
                Ok(()) => self.owns_subfolder = false,
                Err(problem) => problems.push(problem),
            }
        }
        if self.owns_dir && !self.owns_subfolder && !self.owns_folder && self.files.is_empty() {
            match remove_empty_dir(&self.dir) {
                Ok(()) => self.owns_dir = false,
                Err(problem) => problems.push(problem),
            }
        }
        problems
    }
}

impl Drop for LocalData {
    fn drop(&mut self) {
        if self.is_clean() {
            return;
        }
        let problems = self.delete();
        if !problems.is_empty() {
            // A failed write to stderr leaves nowhere else to report to.
            drop(writeln!(
                io::stderr(),
                "live test cleanup left local files behind: {problems:#?}"
            ));
        }
    }
}

/// Removes the directory `dir` once it is empty, or finds it gone: the daemon may still be
/// deleting a torrent's data through a temporary folder in it.
///
/// # Errors
///
/// What still keeps it after [`DIR_REMOVAL_TIMEOUT`].
fn remove_empty_dir(dir: &Path) -> Result<(), String> {
    let mut last_error = None;
    let removed = wait_until(DIR_REMOVAL_TIMEOUT, || match fs::remove_dir(dir) {
        Ok(()) => true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => true,
        Err(error) => {
            last_error = Some(error);
            false
        }
    });
    match last_error {
        Some(error) if !removed => Err(format!("cannot remove {}: {error}", dir.display())),
        _ => Ok(()),
    }
}
