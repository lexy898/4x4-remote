//! What the made-up torrents say about themselves: the comment, the creator, the source tag and
//! whether they are private, in the shapes `torrent_get` sends them, with the other fields of a
//! details read that the files do not answer (the trackers, from `trackers`, and the totals).
//!
//! The metainfo does not change while the demo runs, so it is worked out from the torrent's id
//! whenever a read asks for it, and the same torrent has the same metainfo on every run.

#[cfg(test)]
use std::cell::Cell;

use serde_json::{Map, Value, json};

use super::{Clock, Fields, KIB, Lcg, Torrent};

/// The random stream of a torrent's metainfo.
pub(super) const METAINFO_STREAM: u8 = 3;
/// The fields that come from the metainfo; the trackers and the totals do not.
const METAINFO_FIELDS: [&str; 5] = ["comment", "creator", "date_created", "source", "is_private"];
/// How long before it was added a torrent was created, at most, in seconds (30 days).
const CREATED_BEFORE_ADDED: u32 = 30 * 24 * 3600;

#[cfg(test)]
thread_local! {
    /// How many metainfos this thread has built, which a test reads to see that a read builds one
    /// only when it asks for a field of it. A daemon answers on the thread that posts to it.
    static BUILT: Cell<u32> = const { Cell::new(0) };
}

/// How many metainfos this thread has built so far.
#[cfg(test)]
pub(super) fn built() -> u32 {
    BUILT.with(Cell::get)
}

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
        #[cfg(test)]
        BUILT.with(|built| built.set(built.get().saturating_add(1)));
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

impl Torrent {
    /// Adds to `record` the fields of `fields` that only a details read asks for and that the
    /// files do not answer: the metainfo, the trackers at `clock`, and what was downloaded and
    /// uploaded.
    ///
    /// The metainfo is worked out only when a field of it is asked for. A read of the list names
    /// none, and would otherwise work it out for every torrent it reads.
    pub(super) fn add_metainfo(
        &self,
        fields: &Fields,
        clock: Clock,
        record: &mut Map<String, Value>,
    ) {
        let added = self.added();
        let mut put = |field: &str, value: &dyn Fn() -> Value| {
            if fields.asks(field) {
                record.insert(field.to_owned(), value());
            }
        };
        if fields.asks_any(&METAINFO_FIELDS) {
            let metainfo = Metainfo::of(self.id, added);
            put("comment", &|| json!(metainfo.comment));
            put("creator", &|| json!(metainfo.creator));
            put("date_created", &|| json!(metainfo.date_created));
            put("source", &|| json!(metainfo.source));
            put("is_private", &|| json!(metainfo.is_private));
        }
        put("tracker_stats", &|| json!(self.trackers.stats(self, clock)));
        // What came in is what is not left; what went out follows from the ratio.
        let downloaded = u64::from(self.total_kib.saturating_sub(self.left_kib)) * KIB;
        put("downloaded_ever", &|| json!(downloaded));
        put("uploaded_ever", &|| {
            json!(downloaded * u64::from(self.ratio_thousandths) / 1000)
        });
    }
}
