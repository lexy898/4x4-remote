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
//!   them removed;
//! - `torrent_set` keeps the sequential download and the wanted switch and the priority of the
//!   files, as Transmission 4.1 reads them: an empty list of file indices means every file, and an
//!   index out of range fails the call with error 6 (`files`);
//! - `session_set` keeps the speed settings, and `session_get` answers with them.
//!
//! Every torrent has files (most one, some a folder, a few dozens, one 2000, some with BEP 47
//! padding files), and the pieces it has follow how much of it is downloaded, from the start when
//! the download is sequential (`files`). A read that asks for them also gets the metainfo and the
//! trackers (`metainfo`): the comment, the creator, the source tag, and a tracker that may fail.
//! Those fields, being heavy, are sent only when `fields` names them.
//!
//! As a real daemon does, an action without `ids` reaches every torrent, an unknown torrent is
//! skipped, and a torrent an action changed is among the recently active ones at the next poll.
//! It adds no torrents: `torrent_add`, like any other method, is refused with a JSON-RPC error. It
//! never touches the network or a file.

mod files;
mod metainfo;
mod torrent;

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, MutexGuard, PoisonError};

use serde_json::{Value, json};
use x4_rpc::{HttpRequest, HttpResponse, Transport, TransportError};

use files::{FileOps, Plan};
use torrent::{Kind, Torrent};

/// The seed of the torrents, so that every run shows the same ones.
const SEED: u64 = 0x0004_0004_0004_0004;
/// Where every torrent downloads to.
const DOWNLOAD_DIR: &str = "/data/torrents";
/// A list of ids this long or shorter is looked up by scanning the torrents.
const SHORT_LIST: usize = 8;
/// One KiB, the unit of the torrents' sizes, in bytes.
const KIB: u64 = 1024;
/// The torrent with 2000 files is the first one that downloads and holds at least this much, in
/// KiB (8 GiB), so that each of its files is a few MiB.
const ALBUM_MIN_KIB: u32 = 8 * 1024 * 1024;
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
        let mut torrents: Vec<Torrent> = (1..=count)
            .map(|id| Torrent::new(id, activity, &mut random))
            .collect();
        // One torrent with 2000 files, for the details of a big torrent: a download, so that its
        // files and pieces move.
        if let Some(album) = torrents
            .iter_mut()
            .find(|torrent| torrent.kind == Kind::Downloading && torrent.total_kib >= ALBUM_MIN_KIB)
        {
            album.plan = Plan::album();
        }
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
        let body = body.to_string().into_bytes();
        // What each call costs, for counting the requests and the bytes the app asks of a daemon:
        // the bodies, as HTTP would carry them, without headers.
        tracing::debug!(
            method,
            request_bytes = request.body.len(),
            response_bytes = body.len(),
            "synthetic daemon answered"
        );
        Ok(HttpResponse {
            status: 200,
            session_id: None,
            body,
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
                    .iter_mut()
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
                let mut torrents = Vec::new();
                for index in self.listed(ids)? {
                    if let Some(torrent) = self.torrents.get_mut(index) {
                        torrents.push(torrent.record(&fields));
                    }
                }
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
        // A long list is indexed first, so that it is not looked up one by one; a short one, such
        // as the read of one torrent's details, is scanned, as indexing 2000 torrents costs more.
        let indexed = ids.len() > SHORT_LIST;
        let by_id: HashMap<u64, usize> = if indexed {
            (self.torrents.iter().enumerate())
                .map(|(index, torrent)| (u64::from(torrent.id), index))
                .collect()
        } else {
            HashMap::new()
        };
        let by_hash: HashMap<&str, usize> = if indexed {
            (self.torrents.iter().enumerate())
                .map(|(index, torrent)| (torrent.hash.as_str(), index))
                .collect()
        } else {
            HashMap::new()
        };
        ids.iter()
            .map(|id| match id {
                Value::Number(number) => Ok(number.as_u64().and_then(|number| {
                    if indexed {
                        by_id.get(&number).copied()
                    } else {
                        (self.torrents.iter()).position(|torrent| u64::from(torrent.id) == number)
                    }
                })),
                Value::String(hash) => Ok(if indexed {
                    by_hash.get(hash.as_str()).copied()
                } else {
                    (self.torrents.iter()).position(|torrent| torrent.hash == *hash)
                }),
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

    /// The answer to `torrent_set`: the sequential download and the wanted switch and the priority
    /// of the files are kept (see [`FileOps`]). A call that fails changes no torrent, not even
    /// those before the one with the index out of range.
    fn torrent_set(&mut self, params: Option<&Value>) -> Result<Value, Refusal> {
        let sequential = match params.and_then(|params| params.get("sequential_download")) {
            None => None,
            Some(Value::Bool(sequential)) => Some(*sequential),
            Some(_) => return Err(Refusal::invalid("sequential_download must be a boolean")),
        };
        let lists = FileOps::of(params)?;
        let targets = self.targets(params)?;
        if !lists.is_empty() {
            for &index in &targets {
                if let Some(torrent) = self.torrents.get_mut(index) {
                    lists.check(torrent.files().file_count())?;
                }
            }
        }
        for index in targets {
            if let Some(torrent) = self.torrents.get_mut(index) {
                torrent.set(&lists, sequential);
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

    /// Whether the request names `field`. Unlike [`Fields::wants`], a request that names no fields
    /// does not ask for it: the details are heavy, and only a details read wants them.
    fn asks(&self, field: &str) -> bool {
        self.0
            .as_ref()
            .is_some_and(|fields| fields.iter().any(|asked| asked == field))
    }

    /// Whether the request names any of `names`.
    fn asks_any(&self, names: &[&str]) -> bool {
        names.iter().any(|name| self.asks(name))
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

/// A tiny linear congruential generator (Knuth's MMIX constants): varied torrents, the same on
/// every run, without a dependency.
struct Lcg(u64);

impl Lcg {
    /// A generator of its own for one thing (`stream`) about torrent `id`: deciding more about a
    /// torrent never moves the numbers of the other torrents, or of its own other aspects.
    fn stream(id: u32, stream: u8) -> Self {
        let key = (u64::from(id) << 8) | u64::from(stream);
        let mut random = Self(SEED ^ key.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        // The first numbers of generators whose seeds are alike are alike.
        random.next();
        random.next();
        random
    }

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
