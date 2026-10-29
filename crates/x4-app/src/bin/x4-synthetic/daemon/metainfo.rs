//! What the made-up torrents say about themselves and their trackers: the comment, the creator,
//! the source tag, and what each tracker last answered, in the shapes `torrent_get` sends them.
//!
//! None of it changes while the demo runs, so it is worked out from the torrent's id whenever a
//! read asks for it, and the same torrent has the same metainfo on every run.

use serde_json::{Value, json};

use super::{Kind, Lcg};

/// The random stream of a torrent's metainfo.
pub(super) const METAINFO_STREAM: u8 = 3;
/// The random stream of a torrent's trackers.
pub(super) const TRACKERS_STREAM: u8 = 4;
/// How many trackers a public torrent lists.
const MANY_TRACKERS: usize = 12;
/// How long before it was added a torrent was created, at most, in seconds (30 days).
const CREATED_BEFORE_ADDED: u32 = 30 * 24 * 3600;

/// The metainfo fields of the general tab.
pub(super) struct Metainfo {
    pub(super) comment: String,
    pub(super) creator: &'static str,
    /// Unix seconds; 0 when the metainfo has no date.
    pub(super) date_created: i64,
    /// The source tag of a private tracker; empty for most torrents.
    pub(super) source: &'static str,
    pub(super) is_private: bool,
}

impl Metainfo {
    /// The metainfo of torrent `id`, which was added at `added` (Unix seconds).
    pub(super) fn of(id: u32, added: i64) -> Self {
        let mut random = Lcg::stream(id, METAINFO_STREAM);
        let comment = match random.below(10) {
            0..4 => format!("https://example.org/torrents/{id}"),
            4 => "Mirrored from example.net".to_owned(),
            _ => String::new(),
        };
        let creator = match random.below(10) {
            0..7 => "mktorrent 1.1",
            7..9 => "qBittorrent v4.6.7",
            _ => "",
        };
        // A client that leaves its name out leaves the date out too.
        let date_created = if creator.is_empty() {
            0
        } else {
            added - i64::from(random.between(3600, CREATED_BEFORE_ADDED))
        };
        let source = if random.below(100) < 6 { "EXAMPLE" } else { "" };
        Self {
            comment,
            creator,
            date_created,
            source,
            // A tag belongs to a private tracker; a few others are private without one.
            is_private: !source.is_empty() || random.below(20) == 0,
        }
    }
}

/// What one tracker went wrong with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Trouble {
    /// "Could not connect to tracker".
    Unreachable,
    /// "Tracker did not respond": the announce timed out.
    NoAnswer,
    /// The tracker's own refusal, "Unregistered torrent".
    Refused,
}

impl Trouble {
    /// The daemon's text for the last announce.
    fn text(self) -> &'static str {
        match self {
            Self::Unreachable => "Could not connect to tracker",
            Self::NoAnswer => "Tracker did not respond",
            Self::Refused => "Unregistered torrent",
        }
    }
}

/// The trackers of a torrent and which of them fail. Decided once for a torrent, so that what the
/// trackers last said does not change when the torrent is stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Trackers {
    /// None: peers come from the DHT only.
    None,
    /// One tracker, which fails if there is a trouble.
    One(Option<Trouble>),
    /// A public torrent's twelve trackers, the last of them a backup that is never announced to;
    /// the one at the index fails.
    Twelve(Option<(usize, Trouble)>),
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
                12..62 => Self::One(None),
                62..70 => Self::One(Some(Trouble::NoAnswer)),
                70..85 => Self::Twelve(None),
                _ => {
                    // Never the backup, which is not announced to.
                    let index = usize::try_from(random.below(11)).unwrap_or(0);
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

    /// The `tracker_stats` of torrent `id`, which was added at `added` (Unix seconds).
    pub(super) fn stats(self, id: u32, added: i64) -> Vec<Value> {
        let count = match self {
            Self::None => 0,
            Self::One(_) => 1,
            Self::Twelve(_) => MANY_TRACKERS,
        };
        (0..count)
            .map(|index| self.tracker(index, id, added))
            .collect()
    }

    /// The tracker at `index`.
    fn tracker(self, index: usize, id: u32, added: i64) -> Value {
        let trouble = match self {
            Self::None => None,
            Self::One(trouble) => trouble,
            Self::Twelve(failing) => failing
                .filter(|&(at, _)| at == index)
                .map(|(_, trouble)| trouble),
        };
        let backup = matches!(self, Self::Twelve(_)) && index + 1 == MANY_TRACKERS;
        // Public trackers listed in turn on UDP, HTTPS and HTTP, each on its usual port.
        let (scheme, domain, port) = match index % 3 {
            0 => ("udp", "example.org", 6969),
            1 => ("https", "example.net", 443),
            _ => ("http", "example.org", 80),
        };
        let number = usize::try_from(id % 5).unwrap_or(0) + index + 1;
        let host = format!("tracker{number}.{domain}:{port}");
        let announced = if backup {
            0
        } else {
            added + 600 + 7 * i64::try_from(index).unwrap_or(0)
        };
        let working = !backup && trouble.is_none();
        let (result, timed_out) = match (backup, trouble) {
            (true, _) => ("", false),
            (false, None) => ("Success", false),
            (false, Some(trouble)) => (trouble.text(), trouble == Trouble::NoAnswer),
        };
        // Counts the tracker reports: they come with a working one and are -1 otherwise.
        let count = |scale: u32| {
            if working {
                i64::from((id * scale + u32::try_from(index).unwrap_or(0) * 13) % 300)
            } else {
                -1
            }
        };
        json!({
            "announce": format!("{scheme}://{host}/announce"),
            "announce_state": u8::from(!backup),
            "download_count": if working { count(11) * 3 } else { -1 },
            "has_announced": !backup,
            "has_scraped": working,
            "host": host,
            "id": index,
            "is_backup": backup,
            "last_announce_peer_count": if working { count(5) / 4 } else { 0 },
            "last_announce_result": result,
            "last_announce_start_time": announced - 1,
            "last_announce_succeeded": working,
            "last_announce_time": announced,
            "last_announce_timed_out": timed_out,
            "last_scrape_result": if working { "Success" } else { "" },
            "last_scrape_start_time": if working { announced - 1 } else { 0 },
            "last_scrape_succeeded": working,
            "last_scrape_time": if working { announced } else { 0 },
            "last_scrape_timed_out": false,
            "leecher_count": count(3),
            "next_announce_time": if backup { 0 } else { announced + 1800 },
            "next_scrape_time": if working { announced + 1800 } else { 0 },
            "scrape": format!("{scheme}://{host}/scrape"),
            "scrape_state": u8::from(!backup),
            "seeder_count": count(7),
            "sitename": domain,
            // The twelve trackers are twelve tiers, but the backup shares the last one's.
            "tier": if backup { index - 1 } else { index },
        })
    }
}
