//! Support for the live tests: the environment, a self-created test torrent that cleans up after
//! itself, and a fingerprint of every other torrent on the daemon.
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

use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use x4_rpc::{
    AddOutcome, AddSource, Client, DEFAULT_TIMEOUT, FilePriority, GetIds, HashString, RemoveData,
    Status, TorrentAdd, TorrentField, TorrentId, TorrentRecord, TorrentSelection, UreqTransport,
};

use crate::metainfo::{random_bytes, random_seed, single_file_metainfo};

/// The size of the test torrent's file.
pub(crate) const FILE_SIZE: usize = 3 * 1024 * 1024;
/// The piece length of the test torrent: 12 pieces for [`FILE_SIZE`].
const PIECE_LENGTH: usize = 262_144;
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
    /// `X4_LIVE_REMOTE_DIR`: the test directory as the daemon sees it.
    pub(crate) remote_dir: String,
    /// `X4_LIVE_LOCAL_DIR`: the same directory as mounted on this machine.
    pub(crate) local_dir: PathBuf,
}

impl LiveEnv {
    /// Reads the environment `just test-live` sets.
    pub(crate) fn from_env() -> Self {
        let env = Self {
            url: required_var("X4_LIVE_URL"),
            remote_dir: required_var("X4_LIVE_REMOTE_DIR"),
            local_dir: PathBuf::from(required_var("X4_LIVE_LOCAL_DIR")),
        };
        assert!(
            env.remote_dir.starts_with('/') && !env.remote_dir.trim_end_matches('/').is_empty(),
            "X4_LIVE_REMOTE_DIR must be an absolute directory below the root: {:?}",
            env.remote_dir
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

/// A torrent this test created: a random file in the test directory, added to the daemon.
///
/// Dropping it without [`TestTorrent::remove_with_data`] or [`TestTorrent::remove_keeping_data`]
/// (a failed assertion, a panic) still removes the torrent with its data and deletes the local
/// file and the directory this run created; problems are reported on stderr, since a guard cannot
/// fail a test that is already failing.
#[derive(Debug)]
pub(crate) struct TestTorrent {
    hash: HashString,
    id: TorrentId,
    add: TorrentAdd,
    url: String,
    remote_dir: String,
    file: LocalFile,
    /// Set once the daemon no longer has the torrent.
    removed: bool,
}

impl TestTorrent {
    /// Writes a random 3 MiB file into the test directory, adds it as a trackerless single-file
    /// torrent (started, so the daemon verifies the data) and waits until it seeds.
    #[expect(
        clippy::expect_used,
        clippy::panic,
        reason = "a live test stops at the first failure; the guards clean up"
    )]
    pub(crate) fn create(client: &mut Client<UreqTransport>, env: &LiveEnv) -> Self {
        assert_test_dir_is_dedicated(client, &env.remote_dir);
        let seed = random_seed();
        let name = format!("4x4-live-{seed:016x}.bin");
        let data = random_bytes(seed, FILE_SIZE);
        let metainfo = single_file_metainfo(&name, &data, PIECE_LENGTH);
        let hash = metainfo.info_hash;
        let file = LocalFile::write(&env.local_dir, &name, &data)
            .expect("cannot write the test file into X4_LIVE_LOCAL_DIR");
        let add = TorrentAdd {
            source: AddSource::Metainfo(metainfo.bytes),
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
                let _guard = listed_id.map(|id| Self::guard(hash.clone(), id, add, env, file));
                panic!("torrent_add of {hash} failed: {error}; {what_now} (lookup: {lookup:?})");
            }
        };
        // The guard is armed before anything else can fail.
        let torrent = Self::guard(hash, id, add, env, file);
        torrent.announce();
        torrent.wait_for(client, Status::Seeding, SEEDING_AFTER_ADD);
        torrent
    }

    /// The guard of the torrent `hash` the daemon added from `add`, with the local `file`.
    fn guard(
        hash: HashString,
        id: TorrentId,
        add: TorrentAdd,
        env: &LiveEnv,
        file: LocalFile,
    ) -> Self {
        Self {
            hash,
            id,
            add,
            url: env.url.clone(),
            remote_dir: env.remote_dir.clone(),
            file,
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
            "live test: added torrent {} with data file {}; if this run is interrupted, remove \
             that torrent together with its data",
            self.hash,
            self.file.path.display()
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

    /// The torrent's data file, as mounted on this machine.
    pub(crate) fn local_file(&self) -> &Path {
        &self.file.path
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
    #[expect(
        clippy::panic,
        reason = "a live test stops at the first failure; the guard cleans up"
    )]
    pub(crate) fn record(&self, client: &mut Client<UreqTransport>) -> TorrentRecord {
        match torrent_by_hash(client, &self.hash) {
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
        KeptFile(self.file.hand_over())
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
            // The daemon deletes the file itself; the local deletion below is only a fallback.
            if !wait_until(REMOVAL_TIMEOUT, || file_is_gone(&self.file.path)) {
                problems.push(format!(
                    "the daemon did not delete {} within {REMOVAL_TIMEOUT:?}",
                    self.file.path.display()
                ));
            }
        }
        problems.extend(self.file.delete());
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
        if download_dir.trim_end_matches('/') != self.remote_dir.trim_end_matches('/') {
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
        if self.removed && self.file.is_clean() {
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

/// The test torrent's data file in the local test directory.
///
/// Deletes only what this run created: the file once it was created, and the directory if this
/// run created it and nothing else is in it. Dropping it deletes them too, so a test that fails
/// before its torrent is added leaves nothing behind either.
#[derive(Debug)]
struct LocalFile {
    path: PathBuf,
    dir: PathBuf,
    /// Set from creating the file until it is deleted.
    owns_file: bool,
    /// Set from creating the directory until it is removed.
    owns_dir: bool,
}

impl LocalFile {
    /// Creates `dir/name` (never overwriting a file) holding `data`, flushed to the server.
    ///
    /// `dir` itself is created if missing, but not its parents: a missing parent means the
    /// mount is not there.
    fn write(dir: &Path, name: &str, data: &[u8]) -> io::Result<Self> {
        let owns_dir = match fs::create_dir(dir) {
            Ok(()) => true,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => false,
            Err(error) => return Err(error),
        };
        let mut local = Self {
            path: dir.join(name),
            dir: dir.to_owned(),
            owns_file: false,
            owns_dir,
        };
        let mut file = File::create_new(&local.path)?;
        local.owns_file = true;
        file.write_all(data)?;
        // On NFS, make sure the server has the data before the daemon reads it.
        file.sync_all()?;
        Ok(local)
    }

    fn is_clean(&self) -> bool {
        !self.owns_file && !self.owns_dir
    }

    /// Moves what this run owns into a new `LocalFile`, which deletes it from then on; this one
    /// is left clean.
    fn hand_over(&mut self) -> Self {
        let taken = Self {
            path: self.path.clone(),
            dir: self.dir.clone(),
            owns_file: self.owns_file,
            owns_dir: self.owns_dir,
        };
        self.owns_file = false;
        self.owns_dir = false;
        taken
    }

    /// Deletes the file and the directory this run created; returns what went wrong.
    fn delete(&mut self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.owns_file {
            match fs::remove_file(&self.path) {
                Ok(()) => self.owns_file = false,
                Err(error) if error.kind() == io::ErrorKind::NotFound => self.owns_file = false,
                Err(error) => {
                    problems.push(format!("cannot delete {}: {error}", self.path.display()));
                }
            }
        }
        if self.owns_dir && !self.owns_file {
            let mut last_error = None;
            let removed = wait_until(DIR_REMOVAL_TIMEOUT, || match fs::remove_dir(&self.dir) {
                Ok(()) => true,
                Err(error) if error.kind() == io::ErrorKind::NotFound => true,
                Err(error) => {
                    last_error = Some(error);
                    false
                }
            });
            if removed {
                self.owns_dir = false;
            } else if let Some(error) = last_error {
                problems.push(format!("cannot remove {}: {error}", self.dir.display()));
            }
        }
        problems
    }
}

impl Drop for LocalFile {
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

/// The data file of a test torrent that was removed while the daemon kept its data. The daemon no
/// longer knows the file, so deleting it is the test's job: [`KeptFile::delete`] deletes it and
/// the directory if this run created it, and dropping it deletes them too, so a failed check
/// leaves nothing behind.
#[derive(Debug)]
pub(crate) struct KeptFile(LocalFile);

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
fn accept_added(outcome: &AddOutcome, expected: &HashString) -> Result<TorrentId, String> {
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
    let selection =
        TorrentSelection::from_hashes([hash.clone()]).map_err(|error| error.to_string())?;
    let response = client
        .torrent_get(&GetIds::Selection(selection), OWN_FIELDS)
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
fn assert_test_dir_is_dedicated(client: &mut Client<UreqTransport>, remote_dir: &str) {
    let test_dir = remote_dir.trim_end_matches('/');
    let response = client
        .torrent_get(&GetIds::All, &[TorrentField::DownloadDir])
        .expect("torrent_get of every download directory failed");
    let in_test_dir = response
        .torrents
        .iter()
        .filter_map(|record| record.download_dir.as_deref())
        .filter(|dir| {
            let dir = dir.trim_end_matches('/');
            dir == test_dir || dir.starts_with(&format!("{test_dir}/"))
        })
        .count();
    assert_eq!(
        in_test_dir, 0,
        "{in_test_dir} torrent(s) already download into the test directory {remote_dir:?}; \
         remove leftovers by hash or choose an unused X4_LIVE_REMOTE_DIR"
    );
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

#[cfg(test)]
mod tests {
    use x4_rpc::AddedTorrent;

    use super::*;

    fn hash(digit: char) -> HashString {
        HashString::parse(&digit.to_string().repeat(40)).unwrap()
    }

    fn torrent(hash: HashString) -> AddedTorrent {
        AddedTorrent {
            id: TorrentId(9),
            name: "4x4-live-0.bin".to_owned(),
            hash,
        }
    }

    #[test]
    fn a_new_torrent_with_the_built_hash_is_accepted() {
        let outcome = AddOutcome::Added(torrent(hash('a')));

        assert_eq!(accept_added(&outcome, &hash('a')), Ok(TorrentId(9)));
    }

    #[test]
    fn a_new_torrent_with_another_hash_is_rejected() {
        let outcome = AddOutcome::Added(torrent(hash('b')));

        assert!(accept_added(&outcome, &hash('a')).is_err());
    }

    #[test]
    fn a_duplicate_is_rejected_even_with_the_built_hash() {
        let outcome = AddOutcome::Duplicate(torrent(hash('a')));

        assert!(accept_added(&outcome, &hash('a')).is_err());
    }

    /// A local test file in a directory of its own, which the file creates.
    fn local_file() -> (PathBuf, LocalFile) {
        let dir = env::temp_dir().join(format!("x4-live-support-{:016x}", random_seed()));
        let file = LocalFile::write(&dir, "kept.bin", b"data").unwrap();
        (dir, file)
    }

    #[test]
    fn a_kept_file_outlives_the_torrent_and_is_deleted_by_the_test() {
        let (dir, mut file) = local_file();

        let kept = KeptFile(file.hand_over());
        drop(file);

        assert!(
            !file_is_gone(kept.path()),
            "the torrent's guard deleted the kept file"
        );
        let path = kept.path().to_owned();
        kept.delete();
        assert!(file_is_gone(&path), "{} is still there", path.display());
        assert!(!dir.exists(), "{} is left behind", dir.display());
    }

    #[test]
    fn a_kept_file_is_deleted_when_the_test_fails_before_deleting_it() {
        let (dir, mut file) = local_file();
        let kept = KeptFile(file.hand_over());
        drop(file);

        drop(kept);

        assert!(!dir.exists(), "{} is left behind", dir.display());
    }
}
