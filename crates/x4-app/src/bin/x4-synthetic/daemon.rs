//! A synthetic Transmission daemon: a [`Transport`] that answers the engine's requests from
//! torrents it makes up, in the shapes a real Transmission 4.1 daemon sends, without a network.
//!
//! It answers the reads the engine sends in this stage (`session_get`, `session_stats`,
//! `free_space` and `torrent_get` of every torrent or of the recently active ones) and refuses
//! every other method with a JSON-RPC error, so nothing can ever change through it.

use std::sync::{Mutex, MutexGuard, PoisonError};

use serde_json::{Value, json};
use x4_rpc::{HttpRequest, HttpResponse, Transport, TransportError};

/// The seed of the torrents, so that every run shows the same ones.
const SEED: u64 = 0x0004_0004_0004_0004;
/// Where every torrent downloads to.
const DOWNLOAD_DIR: &str = "/data/torrents";
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
/// The JSON-RPC error code of a method the daemon does not have.
const METHOD_NOT_FOUND: i64 = -32601;

/// How the made-up torrents behave between polls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Activity {
    /// The rates of the active torrents change on every poll of the recently active torrents,
    /// and the downloads and checks progress.
    #[default]
    Ticking,
    /// Every rate stays at 0 and no torrent is ever recently active: an idle daemon.
    Static,
}

/// The daemon: its torrents behind a lock, because [`Transport::post`] takes `&self`.
pub(crate) struct SyntheticDaemon {
    state: Mutex<State>,
}

impl SyntheticDaemon {
    /// A daemon of `count` torrents named "Synthetic torrent 0001" and on, in a fixed mix of
    /// states, that behave as `activity` says.
    pub(crate) fn new(count: u32, activity: Activity) -> Self {
        let mut random = Lcg(SEED);
        let torrents = (1..=count)
            .map(|id| Torrent::new(id, activity, &mut random))
            .collect();
        Self {
            state: Mutex::new(State {
                torrents,
                activity,
                random,
            }),
        }
    }
}

impl Transport for SyntheticDaemon {
    fn post(&self, request: &HttpRequest<'_>) -> Result<HttpResponse, TransportError> {
        let envelope: Value = serde_json::from_slice(request.body).map_err(|error| {
            TransportError::Io(format!(
                "synthetic daemon: the request is not JSON: {error}"
            ))
        })?;
        let id = envelope.get("id").cloned().unwrap_or(Value::Null);
        let method = envelope
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let params = envelope.get("params");
        let mut state = lock(&self.state);
        let result = match method {
            "session_get" => Some(session()),
            "session_stats" => Some(state.stats()),
            "free_space" => Some(free_space()),
            "torrent_get" => state.torrent_get(params),
            _ => None,
        };
        let body = match result {
            Some(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            None => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": METHOD_NOT_FOUND,
                    "message": format!("the synthetic daemon only reads; it has no {method:?}"),
                },
            }),
        };
        Ok(HttpResponse {
            status: 200,
            session_id: None,
            body: body.to_string().into_bytes(),
        })
    }
}

/// The torrents and what moves them.
struct State {
    torrents: Vec<Torrent>,
    activity: Activity,
    random: Lcg,
}

impl State {
    /// The answer to `torrent_get`: every torrent without `ids`, the recently active ones with
    /// `"ids": "recently_active"` (after moving them on), and `None`, a refusal, for a list of
    /// ids: the details of chosen torrents are not made up.
    fn torrent_get(&mut self, params: Option<&Value>) -> Option<Value> {
        match params.and_then(|params| params.get("ids")) {
            None => {
                let torrents: Vec<Value> = self.torrents.iter().map(Torrent::record).collect();
                Some(json!({ "torrents": torrents }))
            }
            Some(ids) if ids == "recently_active" => {
                let torrents = self.tick();
                Some(json!({ "torrents": torrents, "removed": [] }))
            }
            Some(_) => None,
        }
    }

    /// Moves every active torrent on and returns the records of those that changed; none when
    /// the daemon is static.
    fn tick(&mut self) -> Vec<Value> {
        if self.activity == Activity::Static {
            return Vec::new();
        }
        self.torrents
            .iter_mut()
            .filter(|torrent| torrent.kind.moves())
            .map(|torrent| {
                torrent.tick(&mut self.random);
                torrent.record()
            })
            .collect()
    }

    /// The answer to `session_stats`: the totals of the torrents' rates and states.
    fn stats(&self) -> Value {
        let rate = |of: fn(&Torrent) -> u32| -> u64 {
            self.torrents
                .iter()
                .map(|torrent| u64::from(of(torrent)))
                .sum()
        };
        let paused = self
            .torrents
            .iter()
            .filter(|torrent| torrent.kind.status() == 0)
            .count();
        let block = json!({
            "downloaded_bytes": 230_319_039_386_u64,
            "files_added": self.torrents.len(),
            "seconds_active": 352_231,
            "session_count": 1,
            "uploaded_bytes": 1_695_567_953_398_u64,
        });
        json!({
            "active_torrent_count": self.torrents.len() - paused,
            "paused_torrent_count": paused,
            "torrent_count": self.torrents.len(),
            "download_speed": rate(|torrent| torrent.rate_down),
            "upload_speed": rate(|torrent| torrent.rate_up),
            "current_stats": block,
            "cumulative_stats": block,
        })
    }
}

/// The answer to `session_get`, for the server check and for the session settings.
fn session() -> Value {
    json!({
        "alt_speed_down": 50,
        "alt_speed_enabled": false,
        "alt_speed_up": 50,
        "download_dir": DOWNLOAD_DIR,
        "rpc_version_semver": "6.0.1",
        "sequential_download": false,
        "speed_limit_down": 100,
        "speed_limit_down_enabled": false,
        "speed_limit_up": 100,
        "speed_limit_up_enabled": false,
        "version": "4.1.3 (synthetic)",
    })
}

/// The answer to `free_space`: 800 GiB free of 1 TiB.
fn free_space() -> Value {
    json!({
        "path": DOWNLOAD_DIR,
        "size_bytes": 800_u64 << 30,
        "total_size": 1_u64 << 40,
    })
}

/// What a made-up torrent is doing, which decides its status, its error and whether it moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
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
    fn status(self) -> u8 {
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
    fn moves(self) -> bool {
        matches!(
            self,
            Self::Downloading | Self::Uploading | Self::TrackerWarning | Self::Checking
        )
    }

    /// Whether the torrent still lacks some of its data.
    fn incomplete(self) -> bool {
        matches!(
            self,
            Self::Downloading | Self::DownloadWait | Self::Checking | Self::CheckWait
        )
    }
}

/// A made-up torrent. Sizes are in KiB and rates in bytes per second, small enough for `u32`,
/// so that every conversion to the wire's floats is exact.
struct Torrent {
    id: u32,
    hash: String,
    name: String,
    kind: Kind,
    total_kib: u32,
    left_kib: u32,
    rate_down: u32,
    rate_up: u32,
    peers_connected: u32,
    peers_getting_from_us: u32,
    peers_sending_to_us: u32,
    ratio_thousandths: u32,
    added_after: u32,
    recheck_thousandths: u32,
}

impl Torrent {
    fn new(id: u32, activity: Activity, random: &mut Lcg) -> Self {
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
        };
        if activity == Activity::Ticking {
            torrent.tick(random);
        }
        torrent
    }

    /// New rates, peers and progress, as on the next poll of a busy daemon.
    fn tick(&mut self, random: &mut Lcg) {
        match self.kind {
            Kind::Downloading => {
                self.rate_down = random.below(MAX_RATE_DOWN);
                self.rate_up = random.below(MAX_RATE_UP / 4);
                self.peers_sending_to_us = random.between(1, self.peers_connected);
                self.peers_getting_from_us = random.below(self.peers_connected);
                let done_kib = self.rate_down / 1024 * POLL_TENTHS_OF_A_SECOND / 10;
                // A download comes close to the end and stays there, so that it keeps moving.
                self.left_kib = self
                    .left_kib
                    .saturating_sub(done_kib)
                    .max(self.total_kib / 100);
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
    }

    /// The torrent as `torrent_get` sends it, with every field of the app's list.
    fn record(&self) -> Value {
        let (error, error_string) = self.kind.error();
        let total = u64::from(self.total_kib) * 1024;
        let left = u64::from(self.left_kib) * 1024;
        let added = FIRST_ADDED + i64::from(self.added_after);
        let eta = if self.rate_down > 0 {
            i64::try_from(left / u64::from(self.rate_down)).unwrap_or(i64::MAX)
        } else {
            -1
        };
        json!({
            "added_date": added,
            "done_date": if left == 0 { added + 3600 } else { 0 },
            "download_dir": DOWNLOAD_DIR,
            "error": error,
            "error_string": error_string,
            "eta": eta,
            "hash_string": self.hash,
            "id": self.id,
            "is_finished": false,
            "labels": [],
            "left_until_done": left,
            "metadata_percent_complete": 1.0,
            "name": self.name,
            "peers_connected": self.peers_connected,
            "peers_getting_from_us": self.peers_getting_from_us,
            "peers_sending_to_us": self.peers_sending_to_us,
            "percent_done": f64::from(self.total_kib - self.left_kib) / f64::from(self.total_kib),
            "queue_position": self.id - 1,
            "rate_download": self.rate_down,
            "rate_upload": self.rate_up,
            "recheck_progress": f64::from(self.recheck_thousandths) / 1000.0,
            "sequential_download": false,
            "size_when_done": total,
            "status": self.kind.status(),
            "total_size": total,
            "upload_ratio": f64::from(self.ratio_thousandths) / 1000.0,
        })
    }
}

/// A tiny linear congruential generator (Knuth's MMIX constants): varied torrents, the same on
/// every run, without a dependency.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        // The high half is the better mixed one, and it always fits.
        u32::try_from(self.0 >> 32).unwrap_or(u32::MAX)
    }

    /// A number from 0 to `bound - 1`; 0 when `bound` is 0.
    fn below(&mut self, bound: u32) -> u32 {
        self.next() % bound.max(1)
    }

    /// A number from `low` to `high - 1`; `low` when the range is empty.
    fn between(&mut self, low: u32, high: u32) -> u32 {
        low + self.below(high.saturating_sub(low))
    }
}

/// A poisoned lock only means a request panicked on another thread; the torrents are still fine.
fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests;
