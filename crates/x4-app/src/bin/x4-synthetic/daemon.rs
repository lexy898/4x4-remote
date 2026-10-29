//! A synthetic Transmission daemon: a [`Transport`] that answers the engine's requests from
//! torrents it makes up, in the shapes a real Transmission 4.1 daemon sends, without a network.
//!
//! It answers the reads (`session_get`, `session_stats`, `free_space`, and `torrent_get` of every
//! torrent, of the recently active ones or of a list of torrents, with only the fields asked for)
//! and carries out the actions on its own made-up torrents and settings, so that they can be tried
//! in the demo:
//!
//! - `torrent_stop` pauses a torrent; `torrent_start` runs a stopped one, downloading or seeding
//!   by how much it has; `torrent_start_now` also takes a queued one out of its queue;
//! - `torrent_verify` checks a torrent for [`VERIFY_POLLS`] polls of the recently active torrents,
//!   then puts it back as it was; `torrent_reannounce` changes nothing;
//! - `torrent_remove` takes torrents out, and the next poll of the recently active torrents reports
//!   them removed; `torrent_set` keeps the sequential download (the torrents have no files);
//! - `session_set` keeps the speed settings, and `session_get` answers with them.
//!
//! As a real daemon does, an action without `ids` reaches every torrent, an unknown torrent is
//! skipped, and a torrent an action changed is among the recently active ones at the next poll.
//! It adds no torrents: `torrent_add`, like any other method, is refused with a JSON-RPC error. It
//! never touches the network or a file.

use std::collections::{HashMap, HashSet};
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
/// The JSON-RPC error code of parameters the daemon cannot use.
const INVALID_PARAMS: i64 = -32602;
/// How many polls of the recently active torrents a verify takes.
pub(crate) const VERIFY_POLLS: u32 = 3;

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
                removed: Vec::new(),
                session: Session::default(),
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
            "session_get" => Ok(state.session.answer()),
            "session_set" => state.session.set(params).map(|()| json!({})),
            "session_stats" => Ok(state.stats()),
            "free_space" => Ok(free_space()),
            "torrent_get" => state.torrent_get(params),
            "torrent_start" => state.act(params, |torrent| torrent.start(false)),
            "torrent_start_now" => state.act(params, |torrent| torrent.start(true)),
            "torrent_stop" => state.act(params, Torrent::stop),
            "torrent_verify" => state.act(params, Torrent::verify),
            "torrent_reannounce" => state.act(params, |_| {}),
            "torrent_remove" => state.remove(params),
            "torrent_set" => state.torrent_set(params),
            _ => Err(Refusal {
                code: METHOD_NOT_FOUND,
                message: format!("the synthetic daemon has no {method:?}"),
            }),
        };
        let body = match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(refusal) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": refusal.code, "message": refusal.message },
            }),
        };
        Ok(HttpResponse {
            status: 200,
            session_id: None,
            body: body.to_string().into_bytes(),
        })
    }
}

/// Why the daemon refused a request: the JSON-RPC error it answers with.
#[derive(Debug)]
struct Refusal {
    code: i64,
    message: String,
}

impl Refusal {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: INVALID_PARAMS,
            message: message.into(),
        }
    }
}

/// The torrents and what moves them, and the settings.
struct State {
    torrents: Vec<Torrent>,
    /// The ids of the torrents removed since the last poll of the recently active ones.
    removed: Vec<u32>,
    session: Session,
    activity: Activity,
    random: Lcg,
}

impl State {
    /// The answer to `torrent_get`, with the `fields` asked for: every torrent without `ids`, the
    /// recently active ones with `"ids": "recently_active"` (after moving them on), and the
    /// torrents of a list of ids or hashes, in its order.
    fn torrent_get(&mut self, params: Option<&Value>) -> Result<Value, Refusal> {
        let fields = Fields::of(params)?;
        match params.and_then(|params| params.get("ids")) {
            None => {
                let torrents: Vec<Value> = self
                    .torrents
                    .iter()
                    .map(|torrent| torrent.record(&fields))
                    .collect();
                Ok(json!({ "torrents": torrents }))
            }
            Some(ids) if ids == "recently_active" => {
                let torrents = self.tick(&fields);
                let removed = std::mem::take(&mut self.removed);
                Ok(json!({ "torrents": torrents, "removed": removed }))
            }
            Some(ids) => {
                let torrents: Vec<Value> = self
                    .listed(ids)?
                    .into_iter()
                    .filter_map(|index| self.torrents.get(index))
                    .map(|torrent| torrent.record(&fields))
                    .collect();
                Ok(json!({ "torrents": torrents }))
            }
        }
    }

    /// Moves every active torrent on and returns the records of the recently active ones: those
    /// that moved, those being verified and those an action changed. With a static daemon only
    /// the last two move.
    fn tick(&mut self, fields: &Fields) -> Vec<Value> {
        let ticking = self.activity == Activity::Ticking;
        let mut records = Vec::new();
        for torrent in &mut self.torrents {
            let verifying = torrent.verify_poll();
            let moves = ticking && !verifying && torrent.kind.moves();
            if moves {
                torrent.tick(&mut self.random);
            }
            let touched = std::mem::take(&mut torrent.touched);
            if moves || verifying || touched {
                records.push(torrent.record(fields));
            }
        }
        records
    }

    /// The indices of the torrents an action's `params` name: every torrent without `ids`, and
    /// otherwise those of the list, by id or hash, in its order. Unknown ones are skipped.
    fn targets(&self, params: Option<&Value>) -> Result<Vec<usize>, Refusal> {
        match params.and_then(|params| params.get("ids")) {
            None => Ok((0..self.torrents.len()).collect()),
            Some(ids) => self.listed(ids),
        }
    }

    /// The indices of the torrents of the list `ids`, by id or hash, in its order; unknown ones
    /// are skipped.
    fn listed(&self, ids: &Value) -> Result<Vec<usize>, Refusal> {
        let ids = ids
            .as_array()
            .ok_or_else(|| Refusal::invalid("ids must be a list of ids or hashes"))?;
        // Indexed first, so that a list of every torrent is not looked up one by one.
        let by_id: HashMap<u64, usize> = self
            .torrents
            .iter()
            .enumerate()
            .map(|(index, torrent)| (u64::from(torrent.id), index))
            .collect();
        let by_hash: HashMap<&str, usize> = self
            .torrents
            .iter()
            .enumerate()
            .map(|(index, torrent)| (torrent.hash.as_str(), index))
            .collect();
        ids.iter()
            .map(|id| match id {
                Value::Number(number) => Ok(number
                    .as_u64()
                    .and_then(|number| by_id.get(&number).copied())),
                Value::String(hash) => Ok(by_hash.get(hash.as_str()).copied()),
                _ => Err(Refusal::invalid("an id is a number or a hash")),
            })
            .filter_map(Result::transpose)
            .collect()
    }

    /// Applies `action` to the torrents `params` name; the answer has no arguments.
    fn act(
        &mut self,
        params: Option<&Value>,
        action: impl Fn(&mut Torrent),
    ) -> Result<Value, Refusal> {
        for index in self.targets(params)? {
            if let Some(torrent) = self.torrents.get_mut(index) {
                action(torrent);
            }
        }
        Ok(json!({}))
    }

    /// The answer to `torrent_remove`: the torrents are gone, with their data if there were any,
    /// and the next poll of the recently active torrents reports them removed.
    fn remove(&mut self, params: Option<&Value>) -> Result<Value, Refusal> {
        let targets: HashSet<usize> = self.targets(params)?.into_iter().collect();
        let mut index = 0;
        let removed = &mut self.removed;
        self.torrents.retain(|torrent| {
            let keep = !targets.contains(&index);
            index += 1;
            if !keep {
                removed.push(torrent.id);
            }
            keep
        });
        Ok(json!({}))
    }

    /// The answer to `torrent_set`: the sequential download is kept; the file lists are taken and
    /// change nothing, as the torrents have no files.
    fn torrent_set(&mut self, params: Option<&Value>) -> Result<Value, Refusal> {
        let sequential = match params.and_then(|params| params.get("sequential_download")) {
            None => None,
            Some(Value::Bool(sequential)) => Some(*sequential),
            Some(_) => return Err(Refusal::invalid("sequential_download must be a boolean")),
        };
        let targets = self.targets(params)?;
        if let Some(sequential) = sequential {
            for index in targets {
                if let Some(torrent) = self.torrents.get_mut(index) {
                    torrent.sequential = sequential;
                    torrent.touched = true;
                }
            }
        }
        Ok(json!({}))
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

/// The fields a `torrent_get` asks for; every field when it names none.
struct Fields(Option<Vec<String>>);

impl Fields {
    fn of(params: Option<&Value>) -> Result<Self, Refusal> {
        let Some(fields) = params.and_then(|params| params.get("fields")) else {
            return Ok(Self(None));
        };
        fields
            .as_array()
            .and_then(|fields| {
                fields
                    .iter()
                    .map(|field| field.as_str().map(str::to_owned))
                    .collect::<Option<Vec<_>>>()
            })
            .map(|fields| Self(Some(fields)))
            .ok_or_else(|| Refusal::invalid("fields must be a list of names"))
    }

    fn wants(&self, field: &str) -> bool {
        self.0
            .as_ref()
            .is_none_or(|fields| fields.iter().any(|wanted| wanted == field))
    }
}

/// The settings `session_set` changes and `session_get` reports: the speeds, in kB/s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Session {
    alt_speed_enabled: bool,
    alt_speed_down: u64,
    alt_speed_up: u64,
    speed_limit_down: u64,
    speed_limit_down_enabled: bool,
    speed_limit_up: u64,
    speed_limit_up_enabled: bool,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            alt_speed_enabled: false,
            alt_speed_down: 50,
            alt_speed_up: 50,
            speed_limit_down: 100,
            speed_limit_down_enabled: false,
            speed_limit_up: 100,
            speed_limit_up_enabled: false,
        }
    }
}

impl Session {
    /// The answer to `session_get`, for the server check and for the session settings.
    fn answer(&self) -> Value {
        json!({
            "alt_speed_down": self.alt_speed_down,
            "alt_speed_enabled": self.alt_speed_enabled,
            "alt_speed_up": self.alt_speed_up,
            "download_dir": DOWNLOAD_DIR,
            "rpc_version_semver": "6.0.1",
            "sequential_download": false,
            "speed_limit_down": self.speed_limit_down,
            "speed_limit_down_enabled": self.speed_limit_down_enabled,
            "speed_limit_up": self.speed_limit_up,
            "speed_limit_up_enabled": self.speed_limit_up_enabled,
            "version": "4.1.3 (synthetic)",
        })
    }

    /// Takes the speed settings of a `session_set`; the others are ignored, as a daemon ignores
    /// what it does not know. A value of the wrong type refuses the whole request and changes
    /// nothing.
    fn set(&mut self, params: Option<&Value>) -> Result<(), Refusal> {
        let Some(params) = params.and_then(Value::as_object) else {
            return Err(Refusal::invalid("session_set needs its settings"));
        };
        let mut next = *self;
        for (key, value) in params {
            let flag = |value: &Value| {
                value
                    .as_bool()
                    .ok_or_else(|| Refusal::invalid(format!("{key} must be a boolean")))
            };
            let speed = |value: &Value| {
                value
                    .as_u64()
                    .ok_or_else(|| Refusal::invalid(format!("{key} must be a number of kB/s")))
            };
            match key.as_str() {
                "alt_speed_enabled" => next.alt_speed_enabled = flag(value)?,
                "alt_speed_down" => next.alt_speed_down = speed(value)?,
                "alt_speed_up" => next.alt_speed_up = speed(value)?,
                "speed_limit_down" => next.speed_limit_down = speed(value)?,
                "speed_limit_down_enabled" => next.speed_limit_down_enabled = flag(value)?,
                "speed_limit_up" => next.speed_limit_up = speed(value)?,
                "speed_limit_up_enabled" => next.speed_limit_up_enabled = flag(value)?,
                _ => {}
            }
        }
        *self = next;
        Ok(())
    }
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
    sequential: bool,
    /// Whether an action changed it since the last poll of the recently active torrents.
    touched: bool,
    /// A verify the torrent is going through.
    verify: Option<Verify>,
}

/// A verify under way: how many more polls of the recently active torrents it takes, and what the
/// torrent goes back to afterwards.
#[derive(Debug, Clone, Copy)]
struct Verify {
    polls_left: u32,
    then: Kind,
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
            sequential: false,
            touched: false,
            verify: None,
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

    /// Starts a stopped torrent, downloading or seeding by how much it has, and with `now` also a
    /// queued one, which leaves its queue. A running torrent, or one still queued without `now`,
    /// stays as it is, as on a real daemon.
    fn start(&mut self, now: bool) {
        let kind = match self.kind {
            Kind::Paused | Kind::LocalError => self.running(),
            Kind::DownloadWait | Kind::SeedWait if now => self.running(),
            Kind::CheckWait if now => Kind::Checking,
            _ => return,
        };
        self.kind = kind;
        // As many peers as a torrent of the kind has when the demo starts.
        self.peers_connected = match kind {
            Kind::Downloading => 12,
            Kind::Seeding => 3,
            _ => 0,
        };
        self.touched = true;
    }

    /// What a torrent runs as: downloading while it lacks data, seeding once it has it all.
    fn running(&self) -> Kind {
        if self.left_kib > 0 {
            Kind::Downloading
        } else {
            Kind::Seeding
        }
    }

    /// Stops the torrent; a stopped one stays as it is. A verify under way stops too.
    fn stop(&mut self) {
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
    fn verify(&mut self) {
        let then = self.verify.map_or(self.kind, |verify| verify.then);
        self.verify = Some(Verify {
            polls_left: VERIFY_POLLS,
            then,
        });
        self.kind = Kind::Checking;
        self.recheck_thousandths = 0;
        self.rate_down = 0;
        self.rate_up = 0;
        self.touched = true;
    }

    /// Moves a verify on by a poll of the recently active torrents; returns whether the torrent was
    /// being verified.
    fn verify_poll(&mut self) -> bool {
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

    /// The torrent as `torrent_get` sends it: the `fields` asked for, among every field of the
    /// app's list and the magnet link.
    fn record(&self, fields: &Fields) -> Value {
        let mut record = self.list_record();
        if let Value::Object(record) = &mut record {
            record.retain(|field, _| fields.wants(field));
            if fields.wants("magnet_link") {
                record.insert("magnet_link".to_owned(), self.magnet_link().into());
            }
        }
        record
    }

    /// The torrent with every field of the app's list.
    fn list_record(&self) -> Value {
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
            "sequential_download": self.sequential,
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
