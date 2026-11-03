//! Support for the live tests: the environment, a self-created test torrent that cleans up after
//! itself (one file, or a folder of a few small files), and a fingerprint of every other torrent
//! on the daemon.
//!
//! The safety rules (AGENTS.md, Hard Rules) live here:
//! - a [`TestTorrent`] exists only for a torrent `torrent_add` reported as newly added under the
//!   info hash of the metainfo this run built (or, when the reply was lost, that the daemon lists
//!   under that hash), and every selection it builds holds nothing but that hash;
//! - [`TestTorrent::remove_with_data`], [`TestTorrent::remove_keeping_data`] and the drop guard
//!   share one removal path, the only `torrent_remove` of the live tests. It removes a torrent
//!   only after the daemon has confirmed that the torrent lives in the test directory, and asks
//!   for `RemoveData::Delete` only for `remove_with_data` and the guard;
//! - only files and directories this run created are deleted locally. When the daemon was told to
//!   keep the data, the file is handed to the test as a [`KeptFile`], which deletes it, on every
//!   path.
//!
//! x4-core's live tests include this file and `metainfo.rs` too, so that their torrents follow the
//! same rules. The offline tests of both are in `support_tests.rs` and `metainfo_tests.rs`, which
//! only x4-rpc's live tests declare, so that they run once.

use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use x4_rpc::{
    AddOutcome, AddSource, Client, DEFAULT_TIMEOUT, DownloadDir, FilePriority, GetIds, HashString,
    RemoveData, Status, TorrentAdd, TorrentField, TorrentId, TorrentRecord, TorrentSelection,
    UreqTransport,
};

use crate::metainfo::{
    Metainfo, multi_file_metainfo, random_bytes, random_seed, single_file_metainfo,
};

/// The size of the test torrent's file.
pub(crate) const FILE_SIZE: usize = 3 * 1024 * 1024;
/// The piece length of the test torrent: 12 pieces for [`FILE_SIZE`].
const PIECE_LENGTH: usize = 262_144;
/// The files of the multi-file test torrent, in its order, with their sizes: a few KiB each, 27 KiB
/// in all.
pub(crate) const MULTI_FILES: [(&str, usize); 3] = [
    ("first.bin", 9 * 1024),
    ("second.bin", 13 * 1024),
    ("third.bin", 5 * 1024),
];
/// The piece length of the multi-file test torrent: 2 pieces for [`MULTI_FILES`], each across a
/// boundary between two files.
pub(crate) const MULTI_PIECE_LENGTH: usize = 16 * 1024;
/// How often waits poll the daemon; it runs on a Raspberry Pi, so keep the rate modest.
const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// How long a new torrent may take to verify its data and start seeding.
const SEEDING_AFTER_ADD: Duration = Duration::from_secs(60);
/// How long the daemon may take to drop a removed torrent and delete its file.
const REMOVAL_TIMEOUT: Duration = Duration::from_secs(10);
/// How long the daemon's temporary folder of a deletion may keep the test directory non-empty.
const DIR_REMOVAL_TIMEOUT: Duration = Duration::from_secs(5);
/// The fields a test reads about its own torrent.
const OWN_FIELDS: &[TorrentField] = &[
    TorrentField::Id,
    TorrentField::HashString,
    TorrentField::Status,
    TorrentField::PercentDone,
    TorrentField::Error,
    TorrentField::ErrorString,
    TorrentField::DownloadDir,
    TorrentField::SequentialDownload,
    TorrentField::FileStats,
];

/// Where the live tests run: the daemon, and one test directory seen from both sides.
#[derive(Debug, Clone)]
pub(crate) struct LiveEnv {
    /// `X4_LIVE_URL`: the daemon's RPC endpoint.
    pub(crate) url: String,
    /// `X4_LIVE_REMOTE_DIR`: the test directory as the daemon sees it, in the normalized form
    /// in which it is sent and compared.
    pub(crate) remote_dir: DownloadDir,
    /// `X4_LIVE_LOCAL_DIR`: the same directory as mounted on this machine.
    pub(crate) local_dir: PathBuf,
}

impl LiveEnv {
    /// Reads the environment `just test-live` sets.
    #[expect(
        clippy::panic,
        reason = "a live test cannot run without its environment"
    )]
    pub(crate) fn from_env() -> Self {
        let remote_dir = required_var("X4_LIVE_REMOTE_DIR");
        let env = Self {
            url: required_var("X4_LIVE_URL"),
            remote_dir: DownloadDir::new(&remote_dir).unwrap_or_else(|error| {
                panic!("X4_LIVE_REMOTE_DIR is not a folder ({error}): {remote_dir:?}")
            }),
            local_dir: PathBuf::from(required_var("X4_LIVE_LOCAL_DIR")),
        };
        assert!(
            env.remote_dir.as_str() != "/",
            "X4_LIVE_REMOTE_DIR must be an absolute directory below the root: {remote_dir:?}"
        );
        assert!(
            env.local_dir.is_absolute() && env.local_dir.parent().is_some(),
            "X4_LIVE_LOCAL_DIR must be an absolute directory below the root: {}",
            env.local_dir.display()
        );
        env
    }

    /// A client connected directly to the daemon.
    pub(crate) fn client(&self) -> Client<UreqTransport> {
        live_client(&self.url)
    }
}

#[expect(
    clippy::panic,
    reason = "a live test cannot run without its environment"
)]
fn required_var(name: &str) -> String {
    match env::var(name) {
        Ok(value) if !value.is_empty() => value,
        _ => panic!("{name} is not set; run the live tests with `just test-live`"),
    }
}

fn live_client(url: &str) -> Client<UreqTransport> {
    Client::new(UreqTransport::new(url, DEFAULT_TIMEOUT))
}

/// A torrent this test created: a random file, or a folder of random files, in the test
/// directory, added to the daemon.
///
/// Dropping it without [`TestTorrent::remove_with_data`] or [`TestTorrent::remove_keeping_data`]
/// (a failed assertion, a panic) still removes the torrent with its data and deletes the local
/// files and the directories this run created; problems are reported on stderr, since a guard
/// cannot fail a test that is already failing.
#[derive(Debug)]
pub(crate) struct TestTorrent {
    hash: HashString,
    id: TorrentId,
    add: TorrentAdd,
    url: String,
    remote_dir: DownloadDir,
    data: LocalData,
    /// Set once the daemon no longer has the torrent.
    removed: bool,
}

impl TestTorrent {
    /// Writes a random 3 MiB file into the test directory, adds it as a trackerless single-file
    /// torrent (started, so the daemon verifies the data) and waits until it seeds.
    #[expect(
        clippy::expect_used,
        reason = "a live test stops at the first failure; the guards clean up"
    )]
    pub(crate) fn create(client: &mut Client<UreqTransport>, env: &LiveEnv) -> Self {
        assert_test_dir_is_dedicated(client, &env.remote_dir);
        let seed = random_seed();
        let name = format!("4x4-live-{seed:016x}.bin");
        let data = random_bytes(seed, FILE_SIZE);
        let metainfo = single_file_metainfo(&name, &data, PIECE_LENGTH);
        let local = LocalData::write_file(&env.local_dir, &name, &data)
            .expect("cannot write the test file into X4_LIVE_LOCAL_DIR");
        Self::add(client, env, metainfo, local)
    }

    /// Writes the [`MULTI_FILES`], random, into a new folder of the test directory, adds the
    /// folder as a trackerless multi-file torrent (started, so the daemon verifies the data) and
    /// waits until it seeds.
    #[expect(
        clippy::expect_used,
        reason = "a live test stops at the first failure; the guards clean up"
    )]
    pub(crate) fn create_multi_file(client: &mut Client<UreqTransport>, env: &LiveEnv) -> Self {
        assert_test_dir_is_dedicated(client, &env.remote_dir);
        let seed = random_seed();
        let name = format!("4x4-live-{seed:016x}");
        let total = MULTI_FILES.iter().map(|(_, size)| size).sum();
        let data = random_bytes(seed, total);
        let mut rest = data.as_slice();
        let files: Vec<(&str, &[u8])> = MULTI_FILES
            .iter()
            .map(|&(file, size)| {
                let (this, after) = rest.split_at(size);
                rest = after;
                (file, this)
            })
            .collect();
        let metainfo = multi_file_metainfo(&name, &files, MULTI_PIECE_LENGTH);
        let local = LocalData::write_folder(&env.local_dir, &name, &files)
            .expect("cannot write the test files into X4_LIVE_LOCAL_DIR");
        Self::add(client, env, metainfo, local)
    }

    /// Adds `metainfo`, the torrent whose data this run wrote as `local`, to download into the
    /// test directory, and waits until it seeds.
    #[expect(
        clippy::panic,
        reason = "a live test stops at the first failure; the guards clean up"
    )]
    fn add(
        client: &mut Client<UreqTransport>,
        env: &LiveEnv,
        metainfo: Metainfo,
        local: LocalData,
    ) -> Self {
        let hash = metainfo.info_hash;
        let add = TorrentAdd {
            source: AddSource::Metainfo(Arc::from(metainfo.bytes)),
            download_dir: Some(env.remote_dir.clone()),
            paused: false,
            files_unwanted: Vec::new(),
            priority_high: Vec::new(),
            priority_low: Vec::new(),
            sequential_download: None,
        };
        let id = match client.torrent_add(&add) {
            Ok(outcome) => {
                accept_added(&outcome, &hash).unwrap_or_else(|problem| panic!("{problem}"))
            }
            Err(error) => {
                // The reply may be lost after the daemon added the torrent. This run's random
                // data makes the hash this run's alone, so a torrent listed under it is ours.
                let lookup = torrent_by_hash(client, &hash);
                let listed_id = match &lookup {
                    Ok(Some(record)) => record.id,
                    Ok(None) | Err(_) => None,
                };
                let what_now = match &lookup {
                    Ok(Some(_)) if listed_id.is_some() => {
                        "the daemon added it anyway; the guard removes it"
                    }
                    Ok(None) => "the daemon does not have it",
                    Ok(Some(_)) | Err(_) => {
                        "if the daemon has it, remove it by hash together with its data"
                    }
                };
                // Unwinding drops the guard, which removes the torrent.
                let _guard = listed_id.map(|id| Self::guard(hash.clone(), id, add, env, local));
                panic!("torrent_add of {hash} failed: {error}; {what_now} (lookup: {lookup:?})");
            }
        };
        // The guard is armed before anything else can fail.
        let torrent = Self::guard(hash, id, add, env, local);
        torrent.announce();
        torrent.wait_for(client, Status::Seeding, SEEDING_AFTER_ADD);
        torrent
    }

    /// The guard of the torrent `hash` the daemon added from `add`, with the local `data`.
    fn guard(
        hash: HashString,
        id: TorrentId,
        add: TorrentAdd,
        env: &LiveEnv,
        data: LocalData,
    ) -> Self {
        Self {
            hash,
            id,
            add,
            url: env.url.clone(),
            remote_dir: env.remote_dir.clone(),
            data,
            removed: false,
        }
    }

    /// Names the torrent and its file on stderr as soon as the daemon has it: an interrupted run
    /// (Ctrl-C, a killed process) skips every guard, and this line says what it left behind.
    #[expect(
        clippy::print_stderr,
        reason = "test support: the tests install no tracing subscriber, and `just test-live` \
                  streams stderr so the line survives an interrupted run"
    )]
    fn announce(&self) {
        eprintln!(
            "live test: added torrent {} with its data at {}; if this run is interrupted, remove \
             that torrent together with its data",
            self.hash,
            self.data.path.display()
        );
    }

    /// The torrent's info hash: the one of the metainfo this run built, as `torrent_add`
    /// confirmed it.
    pub(crate) fn hash(&self) -> &HashString {
        &self.hash
    }

    /// The torrent's id, as the daemon reported it.
    pub(crate) fn id(&self) -> TorrentId {
        self.id
    }

    /// The torrent's data as mounted on this machine: its file, or its folder.
    pub(crate) fn local_data(&self) -> &Path {
        &self.data.path
    }

    /// The request that added the torrent.
    pub(crate) fn add_request(&self) -> &TorrentAdd {
        &self.add
    }

    /// A selection of this torrent only.
    #[expect(clippy::expect_used, reason = "one hash is never an empty selection")]
    pub(crate) fn selection(&self) -> TorrentSelection {
        TorrentSelection::from_hashes([self.hash.clone()]).expect("one hash is a selection")
    }

    /// The daemon's current record of the torrent.
    pub(crate) fn record(&self, client: &mut Client<UreqTransport>) -> TorrentRecord {
        self.read(client, OWN_FIELDS)
    }

    /// The daemon's current record of the torrent, with `fields`.
    #[expect(
        clippy::panic,
        reason = "a live test stops at the first failure; the guard cleans up"
    )]
    pub(crate) fn read(
        &self,
        client: &mut Client<UreqTransport>,
        fields: &[TorrentField],
    ) -> TorrentRecord {
        match torrent_fields_by_hash(client, &self.hash, fields) {
            Ok(Some(record)) => record,
            Ok(None) => panic!("the daemon no longer has test torrent {}", self.hash),
            Err(error) => panic!("cannot read test torrent {}: {error}", self.hash),
        }
    }

    /// Waits until the torrent reports `status` and returns its record.
    pub(crate) fn wait_for(
        &self,
        client: &mut Client<UreqTransport>,
        status: Status,
        timeout: Duration,
    ) -> TorrentRecord {
        let deadline = Instant::now() + timeout;
        loop {
            let record = self.record(client);
            if record.status == Some(status) {
                return record;
            }
            assert!(
                Instant::now() < deadline,
                "test torrent {} did not reach {status:?} within {timeout:?}; last seen: \
                 {record:?}",
                self.hash
            );
            thread::sleep(POLL_INTERVAL);
        }
    }

    /// Removes the torrent and its data from the daemon, waits until the daemon has dropped the
    /// torrent and deleted the file, then removes the test directory if this run created it.
    pub(crate) fn remove_with_data(mut self, client: &mut Client<UreqTransport>) {
        let problems = self.clean_up(client);
        assert!(
            problems.is_empty(),
            "cleaning up test torrent {} failed: {problems:#?}",
            self.hash
        );
    }

    /// Removes the torrent from the daemon but keeps its data, waits until the daemon no longer
    /// lists it, and hands the data file over to the test, which checks that it is still there
    /// and deletes it. If the removal fails, the guard removes the torrent with its data.
    #[expect(
        clippy::panic,
        reason = "a live test stops at the first failure; the guard cleans up"
    )]
    pub(crate) fn remove_keeping_data(mut self, client: &mut Client<UreqTransport>) -> KeptFile {
        if let Err(problem) = self.remove_from_daemon(client, RemoveData::Keep) {
            panic!(
                "removing test torrent {} and keeping its data failed: {problem}",
                self.hash
            );
        }
        self.removed = true;
        KeptFile(self.data.hand_over())
    }

    /// The cleanup of [`TestTorrent::remove_with_data`] and the drop guard, around the one
    /// removal path; returns what went wrong. Never panics, so the guard can run while a test
    /// unwinds.
    fn clean_up(&mut self, client: &mut Client<UreqTransport>) -> Vec<String> {
        let mut problems = Vec::new();
        if !self.removed {
            match self.remove_from_daemon(client, RemoveData::Delete) {
                Ok(()) => self.removed = true,
                Err(problem) => problems.push(problem),
            }
        }
        if self.removed {
            // The daemon deletes the files itself; the local deletion below is only a fallback.
            if !wait_until(REMOVAL_TIMEOUT, || self.data.files_are_gone()) {
                problems.push(format!(
                    "the daemon did not delete the files of {} within {REMOVAL_TIMEOUT:?}",
                    self.data.path.display()
                ));
            }
        }
        problems.extend(self.data.delete());
        problems
    }

    /// The one removal path: removes the torrent, with its data or keeping it as `data` says,
    /// then waits until the daemon no longer lists it.
    fn remove_from_daemon(
        &self,
        client: &mut Client<UreqTransport>,
        data: RemoveData,
    ) -> Result<(), String> {
        let Some(record) = torrent_by_hash(client, &self.hash)? else {
            return Ok(());
        };
        // Only a torrent in the test directory is removed, and its data deleted, whatever
        // happened before.
        let download_dir = record.download_dir.as_deref().unwrap_or_default();
        if !is_the_test_dir(download_dir, &self.remote_dir) {
            return Err(format!(
                "test torrent {} is in {download_dir:?}, not in the test directory {:?}; \
                 not deleting its data",
                self.hash, self.remote_dir
            ));
        }
        client
            .torrent_remove(&self.selection(), data)
            .map_err(|error| format!("torrent_remove of {} failed: {error}", self.hash))?;
        let mut last_error = None;
        let gone = wait_until(REMOVAL_TIMEOUT, || {
            match torrent_by_hash(client, &self.hash) {
                Ok(record) => record.is_none(),
                Err(error) => {
                    last_error = Some(error);
                    false
                }
            }
        });
        if gone {
            Ok(())
        } else {
            Err(format!(
                "the daemon still lists test torrent {} {REMOVAL_TIMEOUT:?} after removing it \
                 (last error: {last_error:?})",
                self.hash
            ))
        }
    }
}

impl Drop for TestTorrent {
    fn drop(&mut self) {
        if self.removed && self.data.is_clean() {
            return;
        }
        let problems = self.clean_up(&mut live_client(&self.url));
        if !problems.is_empty() {
            // A failed write to stderr leaves nowhere else to report to.
            drop(writeln!(
                io::stderr(),
                "live test cleanup of torrent {} left something behind: {problems:#?}",
                self.hash
            ));
        }
    }
}

/// The test torrent's data in the local test directory: the file of a single-file torrent, or the
/// folder of a multi-file one and the files in it.
///
/// Deletes only what this run created: each file once it was created, the folder once it was
/// created and its files are gone, and the test directory if this run created it and nothing else
/// is in it. Dropping it deletes them too, so a test that fails before its torrent is added leaves
/// nothing behind either.
#[derive(Debug)]
pub(crate) struct LocalData {
    /// What the daemon names after the torrent: its file, or its folder.
    path: PathBuf,
    /// The test directory.
    dir: PathBuf,
    /// The files this run created and has not deleted yet.
    files: Vec<PathBuf>,
    /// Set from creating the folder of a multi-file torrent until it is removed.
    owns_folder: bool,
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
            files: Vec::new(),
            owns_folder: false,
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
        self.files.is_empty() && !self.owns_folder && !self.owns_dir
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
            files: std::mem::take(&mut self.files),
            owns_folder: std::mem::take(&mut self.owns_folder),
            owns_dir: std::mem::take(&mut self.owns_dir),
        }
    }

    /// Deletes the files, the folder and the directory this run created; returns what went wrong.
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
        if self.owns_dir && !self.owns_folder && self.files.is_empty() {
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

/// The id of the torrent `torrent_add` reported, if it is the one this run built: newly added,
/// under the info hash of this run's metainfo. Anything else is not this run's to remove.
pub(crate) fn accept_added(
    outcome: &AddOutcome,
    expected: &HashString,
) -> Result<TorrentId, String> {
    match outcome {
        AddOutcome::Added(added) if added.hash == *expected => Ok(added.id),
        AddOutcome::Added(added) => Err(format!(
            "the daemon added torrent {} for metainfo with info hash {expected}; not touching it",
            added.hash
        )),
        AddOutcome::Duplicate(duplicate) => Err(format!(
            "the daemon already has torrent {}, which this run did not add; not touching it",
            duplicate.hash
        )),
    }
}

/// Whether nothing is at `path` any more.
///
/// Opening asks the server, whereas a `stat` over NFS may answer from a cache for a while.
pub(crate) fn file_is_gone(path: &Path) -> bool {
    matches!(File::open(path), Err(error) if error.kind() == io::ErrorKind::NotFound)
}

/// The size of the file at `path` as the server has it: opening asks the server, as in
/// [`file_is_gone`], and the size comes with that answer.
///
/// # Errors
///
/// The file cannot be opened, or its size cannot be read.
pub(crate) fn file_size(path: &Path) -> io::Result<u64> {
    File::open(path)?.metadata().map(|metadata| metadata.len())
}

/// Calls `done` until it returns true or `timeout` passes; returns whether it did.
pub(crate) fn wait_until(timeout: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// The daemon's record of torrent `hash` (the [`OWN_FIELDS`]), or `None` if it has none.
///
/// # Errors
///
/// The call failed, or the daemon answered with another torrent than the one asked for.
pub(crate) fn torrent_by_hash(
    client: &mut Client<UreqTransport>,
    hash: &HashString,
) -> Result<Option<TorrentRecord>, String> {
    torrent_fields_by_hash(client, hash, OWN_FIELDS)
}

/// The daemon's record of torrent `hash` with `fields`, which must hold `hash_string`, or `None` if
/// it has none.
///
/// # Errors
///
/// The call failed, its answer does not parse, or the daemon answered with another torrent than
/// the one asked for.
pub(crate) fn torrent_fields_by_hash(
    client: &mut Client<UreqTransport>,
    hash: &HashString,
    fields: &[TorrentField],
) -> Result<Option<TorrentRecord>, String> {
    let selection =
        TorrentSelection::from_hashes([hash.clone()]).map_err(|error| error.to_string())?;
    let response = client
        .torrent_get(&GetIds::Selection(selection), fields)
        .map_err(|error| format!("torrent_get of {hash} failed: {error}"))?;
    let mut records = response.torrents.into_iter();
    let record = records.next();
    if records.next().is_some() {
        return Err(format!("torrent_get of {hash} returned several torrents"));
    }
    if let Some(found) = &record {
        let found_hash = found.hash_string.as_deref().map(HashString::parse);
        if found_hash != Some(Ok(hash.clone())) {
            return Err(format!(
                "torrent_get of {hash} returned another torrent: {:?}",
                found.hash_string
            ));
        }
    }
    Ok(record)
}

/// Fails if any torrent on the daemon already downloads into the test directory, so the only
/// data there is this run's.
#[expect(
    clippy::expect_used,
    reason = "a live test stops at the first failure; nothing was changed yet"
)]
fn assert_test_dir_is_dedicated(client: &mut Client<UreqTransport>, remote_dir: &DownloadDir) {
    let response = client
        .torrent_get(&GetIds::All, &[TorrentField::DownloadDir])
        .expect("torrent_get of every download directory failed");
    let in_test_dir = response
        .torrents
        .iter()
        .filter_map(|record| record.download_dir.as_deref())
        .filter(|dir| is_in_test_dir(dir, remote_dir))
        .count();
    assert_eq!(
        in_test_dir, 0,
        "{in_test_dir} torrent(s) already download into the test directory {remote_dir:?}; \
         remove leftovers by hash or choose an unused X4_LIVE_REMOTE_DIR"
    );
}

/// Whether a torrent's `download_dir`, as the daemon reports it, is the test directory itself,
/// both in their normalized forms. A folder that does not normalize is not it, so the data of
/// such a torrent is never deleted.
pub(crate) fn is_the_test_dir(download_dir: &str, test_dir: &DownloadDir) -> bool {
    DownloadDir::new(download_dir).is_ok_and(|dir| dir == *test_dir)
}

/// Whether a torrent's `download_dir`, as the daemon reports it, is the test directory or a
/// folder inside it, both in their normalized forms. A folder that does not normalize (a `.` or
/// `..` part, say) is compared by its text, without trailing slashes, so it is counted rather
/// than missed.
pub(crate) fn is_in_test_dir(download_dir: &str, test_dir: &DownloadDir) -> bool {
    let normalized = DownloadDir::new(download_dir);
    let dir = normalized
        .as_ref()
        .map_or(download_dir.trim_end_matches('/'), DownloadDir::as_str);
    let test_dir = test_dir.as_str();
    dir.strip_prefix(test_dir)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// What the tests must leave untouched on every other torrent: status, sequential download, and
/// the priority and wanted flag of every file (`None` while a magnet link has no metadata).
pub(crate) type Fingerprint =
    BTreeMap<HashString, (Status, bool, Option<Vec<(FilePriority, bool)>>)>;

/// Reads the [`Fingerprint`] of every torrent on the daemon except `own`, the test's torrent.
#[expect(
    clippy::expect_used,
    reason = "a live test stops at the first failure; the guard cleans up"
)]
pub(crate) fn others_fingerprint(
    client: &mut Client<UreqTransport>,
    own: Option<&HashString>,
) -> Fingerprint {
    let response = client
        .torrent_get(
            &GetIds::All,
            &[
                TorrentField::HashString,
                TorrentField::Status,
                TorrentField::SequentialDownload,
                TorrentField::FileStats,
            ],
        )
        .expect("torrent_get of every torrent failed");
    response
        .torrents
        .into_iter()
        .filter_map(|record| {
            let hash = record
                .hash_string
                .as_deref()
                .map(HashString::parse)
                .expect("a torrent without hash_string")
                .expect("a torrent with an invalid hash_string");
            let fingerprint = (
                record.status.expect("a torrent without status"),
                record
                    .sequential_download
                    .expect("a torrent without sequential_download"),
                record.file_stats.map(|stats| {
                    stats
                        .iter()
                        .map(|stat| (stat.priority, stat.wanted))
                        .collect()
                }),
            );
            (Some(&hash) != own).then_some((hash, fingerprint))
        })
        .collect()
}

/// Fails if any torrent but `own` changed since `before` was taken, naming the changed torrents
/// by the first 8 digits of their hash only.
pub(crate) fn assert_others_unchanged(
    client: &mut Client<UreqTransport>,
    own: Option<&HashString>,
    before: &Fingerprint,
) {
    let after = others_fingerprint(client, own);
    let short = |hash: &HashString| hash.as_str().get(..8).unwrap_or_default().to_owned();
    let changes: Vec<String> = before
        .keys()
        .chain(after.keys().filter(|hash| !before.contains_key(*hash)))
        .filter(|hash| before.get(*hash) != after.get(*hash))
        .map(|hash| {
            format!(
                "{}: {:?} -> {:?}",
                short(hash),
                before.get(hash),
                after.get(hash)
            )
        })
        .collect();
    assert!(
        changes.is_empty(),
        "STOP: other torrents changed during a live test: {changes:#?}"
    );
}
