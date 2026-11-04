//! A made-up torrent and what it is doing: its kind, rates and progress, and the actions that
//! change them. Its files and pieces are in `files`, its metainfo in `metainfo`, its trackers in
//! `trackers` and its peers in `peers`.

use serde_json::{Value, json};

use super::files::{FileState, Plan};
use super::peers::leechers;
use super::trackers::{self, Trackers};
use super::{Activity, Clock, DOWNLOAD_DIR, Fields, KIB, Lcg, VERIFY_POLLS, files};

/// The added dates are spread over the year after this moment (2026-06-01, Unix time).
const FIRST_ADDED: i64 = 1_780_272_000;
/// The span of the added dates, in seconds.
const ADDED_SPAN: u32 = 365 * 24 * 3600;
/// The sizes of the torrents, in KiB: from 50 MiB to 60 GiB.
const SIZE_KIB: (u32, u32) = (50 * 1024, 60 * 1024 * 1024);
/// The highest download rate of a torrent, in bytes per second (8 MiB/s).
const MAX_RATE_DOWN: u32 = 8 * 1024 * 1024;
/// The highest upload rate of a torrent, in bytes per second (2 MiB/s).
const MAX_RATE_UP: u32 = 2 * 1024 * 1024;
/// The engine asks for the recently active torrents every 1.5 s while the window is visible;
/// a download moves on by 1.5 s of its rate on each of those polls.
const POLL_TENTHS_OF_A_SECOND: u32 = 15;

/// What a made-up torrent is doing, which decides its status, its error and whether it moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Downloading,
    /// Seeding to peers that download from it.
    Uploading,
    /// Seeding with nobody downloading.
    Seeding,
    Paused,
    Checking,
    CheckWait,
    DownloadWait,
    SeedWait,
    /// Stopped by missing data.
    LocalError,
    /// Seeding to peers while the tracker warns.
    TrackerWarning,
    /// Seeding while the tracker reports an error.
    TrackerError,
}

impl Kind {
    /// How many of every 100 torrents are of each kind.
    const MIX: [(Self, u32); 11] = [
        (Self::Downloading, 12),
        (Self::Uploading, 10),
        (Self::Seeding, 40),
        (Self::Paused, 18),
        (Self::Checking, 2),
        (Self::CheckWait, 1),
        (Self::DownloadWait, 5),
        (Self::SeedWait, 3),
        (Self::LocalError, 3),
        (Self::TrackerWarning, 3),
        (Self::TrackerError, 3),
    ];

    /// The kind for a roll from 0 to 99.
    fn pick(roll: u32) -> Self {
        let mut below = 0;
        for (kind, share) in Self::MIX {
            below += share;
            if roll < below {
                return kind;
            }
        }
        Self::Seeding
    }

    /// The wire `status`.
    pub(super) fn status(self) -> u8 {
        match self {
            Self::Paused | Self::LocalError => 0,
            Self::CheckWait => 1,
            Self::Checking => 2,
            Self::DownloadWait => 3,
            Self::Downloading => 4,
            Self::SeedWait => 5,
            Self::Uploading | Self::Seeding | Self::TrackerWarning | Self::TrackerError => 6,
        }
    }

    /// The wire `error` and `error_string`, the daemon's own words.
    fn error(self) -> (u8, &'static str) {
        match self {
            Self::TrackerWarning => (1, "Could not connect to tracker"),
            Self::TrackerError => (2, "Tracker gave an error: unregistered torrent"),
            Self::LocalError => (
                3,
                "No data found! Ensure your drives are connected or use \"Set Location\".",
            ),
            _ => (0, ""),
        }
    }

    /// Whether the torrent's rates or progress change from poll to poll.
    pub(super) fn moves(self) -> bool {
        matches!(
            self,
            Self::Downloading | Self::Uploading | Self::TrackerWarning | Self::Checking
        )
    }

    /// Whether the torrent still lacks some of its data.
    pub(super) fn incomplete(self) -> bool {
        matches!(
            self,
            Self::Downloading | Self::DownloadWait | Self::Checking | Self::CheckWait
        )
    }
}

/// A made-up torrent. Sizes are in KiB and rates in bytes per second, small enough for `u32`,
/// so that every conversion to the wire's floats is exact.
pub(super) struct Torrent {
    pub(super) id: u32,
    pub(super) hash: String,
    pub(super) name: String,
    pub(super) kind: Kind,
    pub(super) total_kib: u32,
    pub(super) left_kib: u32,
    pub(super) rate_down: u32,
    pub(super) rate_up: u32,
    /// How many peers it is connected to; `peers` makes up who they are.
    pub(super) peers_connected: u32,
    /// How many of them it sends to now: those its upload rate is shared out among.
    pub(super) peers_getting_from_us: u32,
    /// How many of them send to it now: those its download rate is shared out among.
    pub(super) peers_sending_to_us: u32,
    pub(super) ratio_thousandths: u32,
    added_after: u32,
    recheck_thousandths: u32,
    pub(super) sequential: bool,
    /// What its files are, before their sizes.
    pub(super) plan: Plan,
    /// The trackers it has and which of them fail.
    pub(super) trackers: Trackers,
    /// Whether it has run since the daemon started, so that its trackers have been announced to.
    pub(super) ran: bool,
    /// Its files and pieces, made when a read or a `torrent_set` first needs them.
    pub(super) files: Option<FileState>,
    /// Whether an action changed it since the last poll of the recently active torrents.
    pub(super) touched: bool,
    /// A verify the torrent is going through.
    verify: Option<Verify>,
    /// The folder a client chose when it added the torrent; `None` is [`DOWNLOAD_DIR`].
    pub(super) dir: Option<String>,
}

/// What a `torrent_add` gives a new torrent: what its content says, and what the client asked.
pub(super) struct NewTorrent {
    pub(super) id: u32,
    pub(super) hash: String,
    pub(super) name: String,
    /// Its files, or none for a magnet link without its metadata.
    pub(super) plan: Plan,
    /// The size of its files, in KiB; 0 without metadata.
    pub(super) total_kib: u32,
    /// Where it downloads to; `None` is the session's folder.
    pub(super) dir: Option<String>,
    pub(super) paused: bool,
}

/// A verify under way: how many more polls of the recently active torrents it takes, and what the
/// torrent goes back to afterwards.
#[derive(Debug, Clone, Copy)]
struct Verify {
    polls_left: u32,
    then: Kind,
}

impl Torrent {
    pub(super) fn new(id: u32, activity: Activity, random: &mut Lcg) -> Self {
        let kind = Kind::pick(random.below(100));
        let total_kib = random.between(SIZE_KIB.0, SIZE_KIB.1);
        let left_kib = if kind.incomplete() || (kind == Kind::Paused && random.below(2) == 0) {
            random.between(1, total_kib)
        } else {
            0
        };
        let peers_connected = match kind {
            Kind::Downloading => random.between(5, 50),
            Kind::Uploading | Kind::TrackerWarning => random.between(1, 30),
            Kind::Seeding | Kind::TrackerError => random.below(10),
            _ => 0,
        };
        let mut torrent = Self {
            id,
            // Unique through the id at the end; the rest only looks like a hash.
            hash: format!(
                "{:08x}{:08x}{:08x}{:08x}{id:08x}",
                random.next(),
                random.next(),
                random.next(),
                random.next(),
            ),
            name: format!("Synthetic torrent {id:04}"),
            kind,
            total_kib,
            left_kib,
            rate_down: 0,
            rate_up: 0,
            peers_connected,
            peers_getting_from_us: 0,
            peers_sending_to_us: 0,
            ratio_thousandths: random.below(5000),
            added_after: random.below(ADDED_SPAN),
            recheck_thousandths: if kind == Kind::Checking {
                random.below(1000)
            } else {
                0
            },
            sequential: false,
            // What a torrent holds is decided by streams of its own, so that the torrents and
            // their numbers stay what they were before they had files.
            plan: Plan::pick(&mut Lcg::stream(id, files::PLAN_STREAM)),
            trackers: Trackers::of(kind, &mut Lcg::stream(id, trackers::TRACKERS_STREAM)),
            // A stopped torrent may not have run since the daemon started; one that stopped on
            // an error did, and so did every other.
            ran: kind != Kind::Paused || id.is_multiple_of(2),
            files: None,
            touched: false,
            verify: None,
            dir: None,
        };
        if activity == Activity::Ticking {
            torrent.tick(random);
        }
        torrent
    }

    /// A torrent a client has just added, with none of its data: stopped if it was added paused,
    /// and otherwise started, which for a magnet link means looking for its metadata.
    ///
    /// It is newer than every torrent made up at the start, and the next poll of the recently
    /// active torrents reports it.
    pub(super) fn from_add(new: NewTorrent) -> Self {
        let NewTorrent {
            id,
            hash,
            name,
            plan,
            total_kib,
            dir,
            paused,
        } = new;
        let mut torrent = Self {
            id,
            hash,
            name,
            kind: Kind::Paused,
            total_kib,
            left_kib: total_kib,
            rate_down: 0,
            rate_up: 0,
            peers_connected: 0,
            peers_getting_from_us: 0,
            peers_sending_to_us: 0,
            ratio_thousandths: 0,
            added_after: ADDED_SPAN.saturating_add(id),
            recheck_thousandths: 0,
            sequential: false,
            plan,
            trackers: Trackers::of(
                Kind::Paused,
                &mut Lcg::stream(id, trackers::TRACKERS_STREAM),
            ),
            ran: false,
            files: None,
            touched: true,
            verify: None,
            dir,
        };
        if !paused {
            torrent.start(false);
        }
        torrent
    }

    /// Whether the torrent's rates or progress change from poll to poll. A magnet link without its
    /// metadata has nothing to download yet, so it does not.
    pub(super) fn moves(&self) -> bool {
        self.kind.moves() && self.plan.has_metadata()
    }

    /// The folder the torrent downloads to.
    pub(super) fn download_dir(&self) -> &str {
        self.dir.as_deref().unwrap_or(DOWNLOAD_DIR)
    }

    /// The torrent as `torrent_add` reports it: its id, name and hash.
    pub(super) fn summary(&self) -> Value {
        json!({ "id": self.id, "name": self.name, "hash_string": self.hash })
    }

    /// New rates, peers and progress, as on the next poll of a busy daemon.
    pub(super) fn tick(&mut self, random: &mut Lcg) {
        match self.kind {
            Kind::Downloading => {
                self.rate_down = random.below(MAX_RATE_DOWN);
                self.rate_up = random.below(MAX_RATE_UP / 4);
                self.peers_sending_to_us = random.between(1, self.peers_connected);
                self.peers_getting_from_us = random.below(self.peers_connected);
                self.download(self.rate_down / 1024 * POLL_TENTHS_OF_A_SECOND / 10);
            }
            Kind::Uploading | Kind::TrackerWarning => {
                self.rate_up = random.below(MAX_RATE_UP);
                self.peers_getting_from_us = random.between(1, self.peers_connected);
            }
            Kind::Checking => {
                self.recheck_thousandths = (self.recheck_thousandths + 7) % 1000;
            }
            _ => {}
        }
        // The counts are those of the peers the rates are shared out among (`peers`): the draws
        // above stay as they were, so that the other torrents' numbers do not move.
        self.peers_sending_to_us = sharing(
            &mut self.rate_down,
            self.peers_sending_to_us,
            self.peers_connected,
        );
        self.peers_getting_from_us = sharing(
            &mut self.rate_up,
            self.peers_getting_from_us,
            leechers(self.kind, self.peers_connected),
        );
    }

    /// Starts a stopped torrent, downloading or seeding by how much it has, and with `now` also a
    /// queued one, which leaves its queue. A running torrent, or one still queued without `now`,
    /// stays as it is, as on a real daemon.
    pub(super) fn start(&mut self, now: bool) {
        let kind = match self.kind {
            Kind::Paused | Kind::LocalError => self.running(),
            Kind::DownloadWait | Kind::SeedWait if now => self.running(),
            Kind::CheckWait if now => Kind::Checking,
            _ => return,
        };
        self.kind = kind;
        self.ran = true;
        // As many peers as a torrent of the kind has when the demo starts.
        self.peers_connected = match kind {
            Kind::Downloading => 12,
            Kind::Seeding => 3,
            _ => 0,
        };
        self.touched = true;
    }

    /// What a torrent runs as: downloading while it lacks data, seeding once it has it all. A
    /// magnet link without its metadata lacks data, though nothing is left to download as yet.
    fn running(&self) -> Kind {
        if self.left_kib > 0 || !self.plan.has_metadata() {
            Kind::Downloading
        } else {
            Kind::Seeding
        }
    }

    /// Stops the torrent; a stopped one stays as it is. A verify under way stops too.
    pub(super) fn stop(&mut self) {
        if matches!(self.kind, Kind::Paused | Kind::LocalError) {
            return;
        }
        self.kind = Kind::Paused;
        self.verify = None;
        self.recheck_thousandths = 0;
        self.rate_down = 0;
        self.rate_up = 0;
        self.peers_connected = 0;
        self.peers_getting_from_us = 0;
        self.peers_sending_to_us = 0;
        self.touched = true;
    }

    /// Starts checking the torrent's data, for [`VERIFY_POLLS`] polls; then it goes back to what it
    /// was. A verify of a torrent being verified starts over.
    pub(super) fn verify(&mut self) {
        let then = self.verify.map_or(self.kind, |verify| verify.then);
        self.verify = Some(Verify {
            polls_left: VERIFY_POLLS,
            then,
        });
        self.kind = Kind::Checking;
        self.recheck_thousandths = 0;
        self.rate_down = 0;
        self.rate_up = 0;
        self.peers_getting_from_us = 0;
        self.peers_sending_to_us = 0;
        self.touched = true;
    }

    /// Moves a verify on by a poll of the recently active torrents; returns whether the torrent was
    /// being verified.
    pub(super) fn verify_poll(&mut self) -> bool {
        let Some(verify) = &mut self.verify else {
            return false;
        };
        verify.polls_left = verify.polls_left.saturating_sub(1);
        if verify.polls_left == 0 {
            self.kind = verify.then;
            self.verify = None;
            self.recheck_thousandths = 0;
        } else {
            self.recheck_thousandths = 1000 * (VERIFY_POLLS - verify.polls_left) / VERIFY_POLLS;
        }
        true
    }

    /// A magnet link of the torrent, with its hash and its name.
    fn magnet_link(&self) -> String {
        format!(
            "magnet:?xt=urn:btih:{}&dn={}",
            self.hash,
            self.name.replace(' ', "%20")
        )
    }

    /// When the torrent was added, in Unix seconds.
    pub(super) fn added(&self) -> i64 {
        FIRST_ADDED + i64::from(self.added_after)
    }

    /// The torrent as `torrent_get` sends it at `clock`: the `fields` asked for, among every
    /// field of the app's list, the magnet link, the details and the peers.
    pub(super) fn record(&mut self, fields: &Fields, clock: Clock) -> Value {
        // The files settle what is left, which the list fields report, so they come first.
        if Self::needs_files(fields) {
            self.files();
        }
        let mut record = self.list_record();
        if let Value::Object(record) = &mut record {
            record.retain(|field, _| fields.wants(field));
            if fields.wants("magnet_link") {
                record.insert("magnet_link".to_owned(), self.magnet_link().into());
            }
            self.add_file_fields(fields, record);
            self.add_metainfo(fields, clock, record);
            self.add_peers(fields, record);
        }
        record
    }

    /// The torrent with every field of the app's list.
    fn list_record(&self) -> Value {
        let (error, error_string) = self.kind.error();
        let metadata = self.plan.has_metadata();
        let total = u64::from(self.total_kib) * KIB;
        let wanted_kib = self.size_when_done_kib();
        let left = u64::from(self.left_kib) * KIB;
        let added = self.added();
        let eta = if self.rate_down > 0 {
            i64::try_from(left / u64::from(self.rate_down)).unwrap_or(i64::MAX)
        } else {
            -1
        };
        json!({
            "added_date": added,
            "done_date": if left == 0 && metadata { added + 3600 } else { 0 },
            "download_dir": self.download_dir(),
            "error": error,
            "error_string": error_string,
            "eta": eta,
            "hash_string": self.hash,
            "id": self.id,
            "is_finished": false,
            "labels": [],
            "left_until_done": left,
            "metadata_percent_complete": if metadata { 1.0 } else { 0.0 },
            "name": self.name,
            "peers_connected": self.peers_connected,
            "peers_getting_from_us": self.peers_getting_from_us,
            "peers_sending_to_us": self.peers_sending_to_us,
            // Nothing wanted is nothing left to do, so it is all done; but of a torrent whose files
            // are not known yet nothing is done.
            "percent_done": if !metadata {
                0.0
            } else if wanted_kib == 0 {
                1.0
            } else {
                f64::from(wanted_kib.saturating_sub(self.left_kib)) / f64::from(wanted_kib)
            },
            "queue_position": self.id - 1,
            "rate_download": self.rate_down,
            "rate_upload": self.rate_up,
            "recheck_progress": f64::from(self.recheck_thousandths) / 1000.0,
            "sequential_download": self.sequential,
            "size_when_done": u64::from(wanted_kib) * KIB,
            "status": self.kind.status(),
            "total_size": total,
            "upload_ratio": f64::from(self.ratio_thousandths) / 1000.0,
        })
    }
}

/// How many peers share `rate` when `drawn` were drawn to and `eligible` can: none without a rate,
/// otherwise at least one and at most as many as can, and no more than the bytes per second, so
/// that each gets one. A rate that no peer can take is no rate.
fn sharing(rate: &mut u32, drawn: u32, eligible: u32) -> u32 {
    if eligible == 0 {
        *rate = 0;
    }
    if *rate == 0 {
        0
    } else {
        drawn.clamp(1, eligible).min(*rate)
    }
}
