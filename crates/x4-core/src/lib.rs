//! Application model and logic for 4x4, with no UI dependencies.
//!
//! [`Torrent`] is a torrent as the views see it: [`Torrent::from_record`] turns the raw
//! [`x4_rpc::TorrentRecord`] of the server into display values. [`classify`] picks the
//! [`DisplayState`] a torrent is shown in, [`attention`] the [`Attention`] it needs, if any, and
//! [`is_active`] tells whether data moves. The wire sentinels become [`Eta`] and [`Ratio`].
//!
//! The model holds values, never text: sizes, speeds and durations come as [`HumanSize`],
//! [`HumanSpeed`] and [`DurationParts`] (see [`human_size`], [`human_speed`] and
//! [`duration_parts`]), and the UI renders them through its i18n layer. Sizes use binary IEC
//! units.
//!
//! The decisions behind the torrent list live here too, so the views only draw their result:
//! [`Filter`] and [`counts`] drive the sidebar, [`matches_search`] the search box, [`sort()`] the
//! column headers, and [`playable_prefix`] tells how much of a file can already be played from
//! its start while it downloads.
//!
//! The [`Store`] holds the torrents by hash, which survives the daemon renumbering its ids;
//! [`torrents_from_records`] fills it from a `torrent_get` answer. A [`Snapshot`] is what the UI
//! reads: the torrents, the [`SessionSummary`], the totals and the [`ConnectionState`], whose
//! [`ConnectionError`] says in the user's terms why the daemon cannot be used.

mod filter;
mod format;
mod model;
mod playable;
mod snapshot;
mod sort;
mod state;
mod store;
#[cfg(test)]
mod test_support;

pub use filter::{Filter, FilterCounts, counts, matches_search};
pub use format::{
    DurationParts, HumanSize, HumanSpeed, SizeUnit, SpeedUnit, duration_parts, human_size,
    human_speed,
};
pub use model::{Eta, ModelError, Ratio, Torrent};
pub use playable::{file_offsets, playable_prefix};
pub use snapshot::{ConnectionError, ConnectionState, SessionSummary, Snapshot};
pub use sort::{SortDirection, SortKey, sort};
pub use state::{Attention, DisplayState, QueueKind, attention, classify, is_active};
pub use store::{Store, torrents_from_records};
