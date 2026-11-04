//! The trackers of the made-up torrents, in the shape `torrent_get` sends `tracker_stats`: what
//! each tracker last answered and what its tier is doing now.
//!
//! Which trackers a torrent has, and which of them fail, is decided once for the torrent
//! ([`Trackers::of`]), so that what they last said does not change when it is stopped. What the
//! tiers do follows the torrent and the clock. While the torrent runs, each tier announces every
//! [`ANNOUNCE_INTERVAL`] at a moment of its own: its last announce is the latest such moment and
//! its next is due one interval later, so the times move with the clock as a running daemon's
//! do; a few tiers wait for a free announce slot instead, or announce right now. While the
//! torrent is stopped nothing is due, and its trackers keep what they said before, or have said
//! nothing when it has not run since the daemon started. A backup tracker is never announced to.
//!
//! One torrent, the busy one, has a tracker in every state the Trackers tab shows
//! ([`Trackers::EveryState`]).

use serde_json::{Value, json};

use super::{Clock, Kind, Lcg, Torrent};

/// The random stream of a torrent's trackers.
pub(super) const TRACKERS_STREAM: u8 = 4;
/// How many trackers a public torrent lists: eleven tiers, the first of two trackers.
const MANY_TRACKERS: usize = 12;
/// Where the backup is among a public torrent's trackers: second, in the first tier, whose first
/// tracker is the one in use.
const BACKUP: usize = 1;
/// How often a tier is announced to, in seconds: what most trackers ask for.
const ANNOUNCE_INTERVAL: i64 = 1800;
/// How long before the daemon started a stopped torrent's trackers answered last, at most, in
/// seconds: the made-up daemon has run for four days.
const STOPPED_BEFORE: u32 = 4 * 24 * 3600;
/// The daemon's text for a scrape the tracker did not take.
const SCRAPE_FAILED: &str = "Could not connect to tracker";

/// What one tracker went wrong with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Trouble {
    /// "Could not connect to tracker".
    Unreachable,
    /// "Tracker did not respond": the announce timed out.
    NoAnswer,
    /// The tracker's own refusal, "Unregistered torrent".
    Refused,
    /// The announces work, but the scrapes fail: many trackers take no scrapes.
    ScrapeFails,
}

impl Trouble {
    /// The daemon's text for the last announce.
    fn text(self) -> &'static str {
        match self {
            Self::Unreachable => "Could not connect to tracker",
            Self::NoAnswer => "Tracker did not respond",
            Self::Refused => "Unregistered torrent",
            Self::ScrapeFails => "Success",
        }
    }

    /// Whether the last announce succeeded despite the trouble.
    fn announces(self) -> bool {
        self == Self::ScrapeFails
    }
}

/// What a tier does while its torrent runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Turn {
    /// It waits for the time of its next announce.
    Waiting,
    /// Its time has come and it waits for a free announce slot.
    Queued,
    /// It announces now.
    Announcing,
    /// It waits for a slot for its first announce since the torrent started: it has not been
    /// contacted yet.
    FirstQueued,
}

/// What the tiers of [`Trackers::EveryState`] went through, tier by tier: every status of the
/// Trackers tab, a scrape that failed alone and a tier queued after it announced. The rarer sights
/// come first, so that the tab shows most of them, the backup in the first tier among them,
/// without scrolling.
const EVERY_STATE: [(Option<Trouble>, Turn); MANY_TRACKERS - 1] = [
    (None, Turn::Waiting),
    (None, Turn::Announcing),
    (Some(Trouble::Unreachable), Turn::Waiting),
    (Some(Trouble::NoAnswer), Turn::Waiting),
    (Some(Trouble::ScrapeFails), Turn::Waiting),
    (None, Turn::FirstQueued),
    (None, Turn::Queued),
    (None, Turn::Waiting),
    (None, Turn::Waiting),
    (None, Turn::Waiting),
    (None, Turn::Waiting),
];

/// The trackers of a torrent and which of them fail. Decided once for a torrent, so that what the
/// trackers last said does not change when the torrent is stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Trackers {
    /// None: peers come from the DHT only.
    None,
    /// One tracker, which fails if there is a trouble.
    One(Option<Trouble>),
    /// A public torrent's twelve trackers, the second of them a backup that is never announced to;
    /// the one at the index fails.
    Twelve(Option<(usize, Trouble)>),
    /// Twelve trackers in every state, the second a backup: see [`EVERY_STATE`].
    EveryState,
}

impl Trackers {
    /// The trackers of a torrent of `kind`: the torrents that show a tracker problem in the list
    /// have the tracker that causes it.
    pub(super) fn of(kind: Kind, random: &mut Lcg) -> Self {
        match kind {
            Kind::TrackerWarning => Self::One(Some(Trouble::Unreachable)),
            Kind::TrackerError => Self::One(Some(Trouble::Refused)),
            _ => match random.below(100) {
                0..12 => Self::None,
                12..58 => Self::One(None),
                58..62 => Self::One(Some(Trouble::ScrapeFails)),
                62..70 => Self::One(Some(Trouble::NoAnswer)),
                70..85 => Self::Twelve(None),
                _ => {
                    // Never the backup, which is not announced to.
                    let index = usize::try_from(random.below(11)).unwrap_or(0);
                    let index = index + usize::from(index >= BACKUP);
                    let trouble = if random.below(2) == 0 {
                        Trouble::Unreachable
                    } else {
                        Trouble::NoAnswer
                    };
                    Self::Twelve(Some((index, trouble)))
                }
            },
        }
    }

    /// The `tracker_stats` of `torrent` at `clock`.
    pub(super) fn stats(self, torrent: &Torrent, clock: Clock) -> Vec<Value> {
        let count = match self {
            Self::None => 0,
            Self::One(_) => 1,
            Self::Twelve(_) | Self::EveryState => MANY_TRACKERS,
        };
        (0..count)
            .map(|index| self.tracker(index, torrent, clock))
            .collect()
    }

    /// Whether the tracker at `index` is a backup.
    fn is_backup(self, index: usize) -> bool {
        matches!(self, Self::Twelve(_) | Self::EveryState) && index == BACKUP
    }

    /// The tier of the tracker at `index`: each tracker has one of its own, but the backup shares
    /// the first.
    fn tier(self, index: usize) -> usize {
        match self {
            Self::Twelve(_) | Self::EveryState => index - usize::from(index >= BACKUP),
            Self::None | Self::One(_) => index,
        }
    }

    /// What went wrong with the tracker at `index` of torrent `id`, and what its tier does while
    /// the torrent runs: one tier in forty waits for a slot and one announces.
    fn plan(self, index: usize, id: u32) -> (Option<Trouble>, Turn) {
        let trouble = match self {
            Self::None => None,
            Self::One(trouble) => trouble,
            Self::Twelve(failing) => failing
                .filter(|&(at, _)| at == index)
                .map(|(_, trouble)| trouble),
            Self::EveryState => {
                return EVERY_STATE
                    .get(self.tier(index))
                    .copied()
                    .unwrap_or((None, Turn::Waiting));
            }
        };
        let index = u64::try_from(index).unwrap_or(0);
        let turn = match (u64::from(id) * 7 + index * 13) % 40 {
            0 => Turn::Announcing,
            1 => Turn::Queued,
            _ => Turn::Waiting,
        };
        (trouble, turn)
    }

    /// The tracker at `index` of `torrent`, at `clock`.
    fn tracker(self, index: usize, torrent: &Torrent, clock: Clock) -> Value {
        let id = torrent.id;
        let (trouble, turn) = self.plan(index, id);
        let backup = self.is_backup(index);
        // Public trackers listed in turn on UDP, HTTPS and HTTP, each on its usual port.
        let (scheme, domain, port) = match index % 3 {
            0 => ("udp", "example.org", 6969),
            1 => ("https", "example.net", 443),
            _ => ("http", "example.org", 80),
        };
        let number = usize::try_from(id % 5).unwrap_or(0) + index + 1;
        let host = format!("tracker{number}.{domain}:{port}");
        let wide = u32::try_from(index).unwrap_or(0);
        // The moment of its own at which the tier announces, every interval.
        let phase = (i64::from(id) * 613 + i64::from(wide) * 277) % ANNOUNCE_INTERVAL;
        let (announced, last, state, next) = if backup {
            (false, 0, 0, 0)
        } else if torrent.kind.status() != 0 {
            let last = clock.now - (clock.now - phase).rem_euclid(ANNOUNCE_INTERVAL);
            match turn {
                Turn::Waiting => (true, last, 1, last + ANNOUNCE_INTERVAL),
                Turn::Queued => (true, last, 2, 0),
                Turn::Announcing => (true, last, 3, 0),
                Turn::FirstQueued => (false, 0, 2, 0),
            }
        } else if torrent.ran {
            let before = 600 + (id * 7919 + wide * 7) % STOPPED_BEFORE;
            (true, clock.started - i64::from(before), 0, 0)
        } else {
            (false, 0, 0, 0)
        };
        let working = announced && trouble.is_none_or(Trouble::announces);
        let scraped = working && trouble.is_none();
        let (result, timed_out) = match (announced, trouble) {
            (false, _) => ("", false),
            (true, None) => ("Success", false),
            (true, Some(trouble)) => (trouble.text(), trouble == Trouble::NoAnswer),
        };
        // Counts the tracker reports: they come with a working one and are -1 otherwise; the
        // downloads only with a scrape.
        let count = |scale: u32| {
            if working {
                i64::from((id * scale + wide * 13) % 300)
            } else {
                -1
            }
        };
        let running = torrent.kind.status() != 0 && !backup;
        json!({
            "announce": format!("{scheme}://{host}/announce"),
            "announce_state": state,
            "download_count": if scraped { count(11) * 3 } else { -1 },
            "has_announced": announced,
            "has_scraped": working,
            "host": host,
            "id": index,
            "is_backup": backup,
            "last_announce_peer_count": if working { count(5) / 4 } else { 0 },
            "last_announce_result": result,
            "last_announce_start_time": if announced { last - 1 } else { 0 },
            "last_announce_succeeded": working,
            "last_announce_time": last,
            "last_announce_timed_out": timed_out,
            "last_scrape_result": match (working, scraped) {
                (false, _) => "",
                (true, true) => "Success",
                (true, false) => SCRAPE_FAILED,
            },
            "last_scrape_start_time": if working { last - 1 } else { 0 },
            "last_scrape_succeeded": scraped,
            "last_scrape_time": if working { last } else { 0 },
            "last_scrape_timed_out": false,
            "leecher_count": count(3),
            "next_announce_time": next,
            "next_scrape_time": if working && running { last + ANNOUNCE_INTERVAL } else { 0 },
            "scrape": format!("{scheme}://{host}/scrape"),
            "scrape_state": u8::from(running),
            "seeder_count": count(7),
            "sitename": domain,
            "tier": self.tier(index),
        })
    }
}
