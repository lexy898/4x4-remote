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

mod format;
mod model;
mod state;

pub use format::{
    DurationParts, HumanSize, HumanSpeed, SizeUnit, SpeedUnit, duration_parts, human_size,
    human_speed,
};
pub use model::{Eta, ModelError, Ratio, Torrent};
pub use state::{Attention, DisplayState, QueueKind, attention, classify, is_active};
