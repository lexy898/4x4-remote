//! Support for the live tests: the environment, a self-created test torrent that cleans up after
//! itself, and a fingerprint of every other torrent on the daemon.
//!
//! The safety rules (AGENTS.md, Hard Rules) live here:
//! - a [`TestTorrent`] exists only for a torrent `torrent_add` reported as newly added, and every
//!   selection it builds holds nothing but the hash `torrent_add` returned;
//! - [`TestTorrent::remove_with_data`] and the drop guard share one removal path, the only use of
//!   `RemoveData::Delete` in the tests, and it deletes data only after the daemon has confirmed
//!   that the torrent lives in the test directory;
//! - only files and directories this run created are deleted locally.

use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File};
use std::hash::{BuildHasher, RandomState};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sha1::{Digest, Sha1};
use x4_rpc::{
    AddOutcome, AddSource, Client, DEFAULT_TIMEOUT, FilePriority, GetIds, HashString, RemoveData,
    Status, TorrentAdd, TorrentField, TorrentId, TorrentRecord, TorrentSelection, UreqTransport,
};

/// The size of the test torrent's file.
const FILE_SIZE: usize = 3 * 1024 * 1024;
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
/// Dropping it without [`TestTorrent::remove_with_data`] (a failed assertion, a panic) still
/// removes the torrent with its data and deletes the local file and the directory this run
/// created; problems are reported on stderr, since a guard cannot fail a test that is already
/// failing.
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
        let added = match client.torrent_add(&add) {
            Ok(AddOutcome::Added(added)) => added,
            Ok(AddOutcome::Duplicate(duplicate)) => panic!(
                "the daemon already has torrent {}, which this run did not add; not touching it",
                duplicate.hash
            ),
            Err(error) => panic!(
                "torrent_add failed: {error}; if the daemon added it anyway, remove torrent {} \
                 by hash",
                metainfo.info_hash
            ),
        };
        // The guard is armed before anything else can fail.
        let torrent = Self {
            hash: added.hash,
            id: added.id,
            add,
            url: env.url.clone(),
            remote_dir: env.remote_dir.clone(),
            file,
            removed: false,
        };
        assert_eq!(
            torrent.hash, metainfo.info_hash,
            "the daemon reports another info hash than the metainfo has"
        );
        torrent.wait_for(client, Status::Seeding, SEEDING_AFTER_ADD);
        torrent
    }

    /// The torrent's info hash, as `torrent_add` returned it.
    pub(crate) fn hash(&self) -> &HashString {
        &self.hash
    }

    /// The torrent's id, as `torrent_add` returned it.
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

    /// The one removal path, shared by [`TestTorrent::remove_with_data`] and the drop guard;
    /// returns what went wrong. Never panics, so the guard can run while a test unwinds.
    fn clean_up(&mut self, client: &mut Client<UreqTransport>) -> Vec<String> {
        let mut problems = Vec::new();
        if !self.removed {
            match self.remove_from_daemon(client) {
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

    /// Removes the torrent with its data, then waits until the daemon no longer lists it.
    fn remove_from_daemon(&self, client: &mut Client<UreqTransport>) -> Result<(), String> {
        let Some(record) = torrent_by_hash(client, &self.hash)? else {
            return Ok(());
        };
        // Data is deleted only for a torrent in the test directory, whatever happened before.
        let download_dir = record.download_dir.as_deref().unwrap_or_default();
        if download_dir.trim_end_matches('/') != self.remote_dir.trim_end_matches('/') {
            return Err(format!(
                "test torrent {} is in {download_dir:?}, not in the test directory {:?}; \
                 not deleting its data",
                self.hash, self.remote_dir
            ));
        }
        client
            .torrent_remove(&self.selection(), RemoveData::Delete)
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

/// Whether nothing is at `path` any more.
///
/// Opening asks the server, whereas a `stat` over NFS may answer from a cache for a while.
pub(crate) fn file_is_gone(path: &Path) -> bool {
    matches!(File::open(path), Err(error) if error.kind() == io::ErrorKind::NotFound)
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

/// What the tests must leave untouched on every other torrent: status, sequential download and
/// file priorities (`None` while a magnet link has no metadata).
pub(crate) type Fingerprint = BTreeMap<HashString, (Status, bool, Option<Vec<FilePriority>>)>;

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
                record
                    .file_stats
                    .map(|stats| stats.iter().map(|stat| stat.priority).collect()),
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

/// A seed that differs between runs: the standard library's per-process random hash keys,
/// mixed with the time and the process id.
fn random_seed() -> u64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    RandomState::new().hash_one((nanos, process::id()))
}

/// `len` pseudo-random bytes from `SplitMix64` seeded with `seed`: fresh content (so a fresh info
/// hash) for every run, without a dependency.
fn random_bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed;
    let mut bytes = Vec::with_capacity(len + 8);
    while bytes.len() < len {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        bytes.extend_from_slice(&(value ^ (value >> 31)).to_le_bytes());
    }
    bytes.truncate(len);
    bytes
}

/// A bencode value: as much of the format as single-file metainfo needs.
#[derive(Debug)]
enum Bencode<'a> {
    Int(usize),
    Bytes(&'a [u8]),
    /// Encoded with its keys sorted, as bencode requires.
    Dict(Vec<(&'a str, Bencode<'a>)>),
}

impl Bencode<'_> {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode_into(&mut out);
        out
    }

    fn encode_into(&self, out: &mut Vec<u8>) {
        match self {
            Self::Int(value) => out.extend_from_slice(format!("i{value}e").as_bytes()),
            Self::Bytes(bytes) => {
                out.extend_from_slice(format!("{}:", bytes.len()).as_bytes());
                out.extend_from_slice(bytes);
            }
            Self::Dict(entries) => {
                let mut sorted: Vec<_> = entries.iter().collect();
                // `str` orders by bytes, which is the order bencode wants.
                sorted.sort_by_key(|(key, _)| *key);
                out.push(b'd');
                for (key, value) in sorted {
                    Bencode::Bytes(key.as_bytes()).encode_into(out);
                    value.encode_into(out);
                }
                out.push(b'e');
            }
        }
    }
}

/// A `.torrent` file and the info hash the daemon will report for it.
#[derive(Debug)]
struct Metainfo {
    bytes: Vec<u8>,
    info_hash: HashString,
}

/// The metainfo of a single-file torrent `name` holding `data`, without trackers.
#[expect(
    clippy::expect_used,
    reason = "a SHA-1 digest in hex is always a valid hash"
)]
fn single_file_metainfo(name: &str, data: &[u8], piece_length: usize) -> Metainfo {
    let pieces: Vec<u8> = data.chunks(piece_length).flat_map(Sha1::digest).collect();
    let info = Bencode::Dict(vec![
        ("length", Bencode::Int(data.len())),
        ("name", Bencode::Bytes(name.as_bytes())),
        ("piece length", Bencode::Int(piece_length)),
        ("pieces", Bencode::Bytes(&pieces)),
    ]);
    let info_hash = HashString::parse(&to_hex(&Sha1::digest(info.encode())))
        .expect("a SHA-1 digest in hex is a valid hash");
    let bytes = Bencode::Dict(vec![("info", info)]).encode();
    Metainfo { bytes, info_hash }
}

fn to_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .filter_map(|nibble| char::from_digit(u32::from(nibble), 16))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn from_hex(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn bencode_sorts_keys_and_prefixes_strings_with_their_length() {
        let value = Bencode::Dict(vec![
            ("name", Bencode::Bytes(b"a")),
            ("length", Bencode::Int(11)),
            ("info", Bencode::Dict(Vec::new())),
        ]);

        assert_eq!(value.encode(), b"d4:infode6:lengthi11e4:name1:ae");
    }

    #[test]
    fn metainfo_hashes_every_piece_and_the_info_dictionary() {
        // Expected values computed independently with Python's hashlib.
        let pieces = from_hex(concat!(
            "a5cec7af5f7aab769cf0d4aa440e01c7bfc371b2", // "hell"
            "1085cd1a2f46b0902dac61890f31072561978deb", // "o wo"
            "c466b3a33e6f6dc170438d34fcddf7860d3658fb", // "rld"
        ));
        let mut info = b"d6:lengthi11e4:name5:a.bin12:piece lengthi4e6:pieces60:".to_vec();
        info.extend_from_slice(&pieces);
        info.push(b'e');
        let mut torrent = b"d4:info".to_vec();
        torrent.extend_from_slice(&info);
        torrent.push(b'e');

        let metainfo = single_file_metainfo("a.bin", b"hello world", 4);

        assert_eq!(metainfo.bytes, torrent);
        assert_eq!(
            metainfo.info_hash.as_str(),
            "420ae9d2244908beea4a3022d6a186dd308f0dca"
        );
    }

    #[test]
    fn random_bytes_have_the_requested_length_and_follow_the_seed() {
        let first = random_bytes(1, 13);

        assert_eq!(first.len(), 13);
        assert_eq!(random_bytes(1, 13), first);
        assert_ne!(random_bytes(2, 13), first);
    }
}
