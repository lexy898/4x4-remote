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
//! What the main window shows is decided here as well: [`table_model`] turns a [`Snapshot`] and
//! the user's [`ViewState`] into the [`Row`]s of the table and the sidebar counts (and, for an
//! empty table, the [`EmptyTable`] reason it gives), [`status_model`] builds the status bar,
//! whose [`Indicator`] says which of the [`ServiceStatus`] and the [`ConnectionState`] the
//! connection dot shows, and [`visibility`] tells whether the user can see the window.
//!
//! What the app remembers between runs is described here too: [`UiState`] is the `state.toml`
//! the app writes (window, columns, sort order, filter, theme), read tolerantly and repaired by
//! [`UiState::parse`] (see also [`UiState::normalized`]); [`set_column_shown`],
//! [`move_shown_column`] and [`set_shown_widths`] apply what the user does to its columns, which
//! the table counts by [`shown_columns`]. [`Config`] is the owner's `config.toml`, which the app
//! only reads. Both turn text into values and back; reading and writing the files is left to the
//! app. A [`ConfigError`] names the [`TextPosition`] of a parse error as numbers, so the app can
//! word it.
//!
//! The [`Store`] holds the torrents by hash, which survives the daemon renumbering its ids;
//! [`torrents_from_records`] turns a `torrent_get` answer into [`Converted`] torrents and the
//! [`SkippedRecord`]s that could not be. A [`Snapshot`] is what the UI reads: the torrents, the
//! [`SessionSummary`], the totals and the [`ConnectionState`], whose [`ConnectionError`] says in
//! the user's terms why the daemon cannot be used (a daemon whose version is too old is a
//! [`ConnectionState::Unsupported`] with an [`UnsupportedReason`], which is re-exported from
//! `x4-rpc`).
//!
//! The [`Engine`] runs the polling without doing any I/O itself: given the current time it
//! names the [`Request`]s that are due, takes each [`Response`] and each [`Command`] of the UI,
//! and hands out a new [`Snapshot`] only when something changed. It carries out [`Action`]s
//! only while the daemon is connected, refuses those that name no torrent or no file, and reads
//! the [`TorrentDetails`] of the selected torrent every tick.
//!
//! [`service::spawn`] runs the engine on a worker thread, which does all the network I/O through
//! an [`x4_rpc::Client`]. The UI talks to it through the [`service::ServiceHandle`]: it sends
//! [`Command`]s, which never blocks, awaits the [`Event`]s on its own executor, and drops the
//! handle to stop the thread without waiting for it, so it never waits on the network.
//!
//! The `test-support` feature adds the `test_support` module with builders for tests of
//! dependent crates (a complete [`Torrent`], a recorded fixture's `result`) and turns on
//! `x4-rpc`'s `test-support`, whose recorded fixtures the builders work with.

mod action;
mod config;
mod details;
mod engine;
mod filter;
mod format;
mod model;
mod playable;
pub mod service;
mod snapshot;
mod sort;
mod state;
mod store;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
mod toml_error;
mod ui_state;
mod view;

pub use action::{Action, ActionError, ActionId};
pub use config::{Config, ConfigError, ConnectionConfig, DEFAULT_RPC_URL};
pub use details::{FileEntry, TorrentDetails};
pub use engine::{Command, Engine, EngineConfig, Event, Request, Response, Visibility};
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
pub use store::{Converted, SkipReason, SkippedRecord, Store, torrents_from_records};
pub use toml_error::TextPosition;
pub use ui_state::{
    ColumnState, MAX_COLUMN_WIDTH, MIN_COLUMN_WIDTH, ParsedUiState, SortState, ThemeChoice,
    UI_STATE_VERSION, UiState, UiStateError, WindowState, default_columns, move_shown_column,
    set_column_shown, set_shown_widths, shown_columns,
};
pub use view::{
    ColumnId, CompactDuration, EmptyTable, EtaView, Indicator, Row, ServiceStatus, StatusModel,
    Subtitle, TableModel, TimeUnit, ViewState, status_model, table_model, visibility,
};
// The reason inside `ConnectionState::Unsupported`, so that the UI can name it without depending on
// x4-rpc.
pub use x4_rpc::UnsupportedReason;
