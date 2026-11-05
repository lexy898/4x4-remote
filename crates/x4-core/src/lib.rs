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
//! connection dot shows, and [`visibility`] tells whether the user can see the window. What the
//! user does to the rows is decided here too: a [`Selection`] holds the selected hashes and
//! follows clicks and arrow keys, its `targets` are the rows an action reaches, and
//! [`availability()`] and [`toggle_kind`] tell which actions those rows allow.
//! [`removal_summary`] gives the [`RemovalSummary`] the removal dialog shows.
//!
//! The details panel under the table is decided here as well. [`details_target`] says which
//! torrent it is about (a [`DetailsTarget`]), and [`general_model`] gives the values of its
//! General tab, with the [`TrackerSummary`] of [`tracker_summary`]. A [`FileTree`] turns the files
//! of the [`TorrentDetails`] into folders and files with their sums (or into none, for more than
//! [`MAX_CHOOSABLE_FILES`] files or a tree beyond [`TREE_BUDGET`]); a [`FileTreeView`] keeps
//! which folders are open and where the cursor is and lists the [`FileRow`]s to draw; and
//! [`FileTree::indices`] gives the files an action on a row reaches, as [`NonEmptyIndices`], which
//! cannot be empty because the daemon reads an empty list as all files.
//!
//! Adding a torrent starts here as well: [`parse_torrent`] reads a `.torrent` file of at most
//! [`MAX_TORRENT_FILE_BYTES`], strictly, into a [`ParsedTorrent`] with its info hash and its
//! [`MetaFile`]s at the indices the daemon will give them, or into no files at all when the daemon
//! could number them otherwise, and refuses with a [`MetainfoError`] what the daemon would.
//! [`launch::classify`] tells what each argument 4x4 is launched with asks for: a magnet link, a
//! `.torrent` file to read, or a [`launch::RejectedArg`] that the window explains. It never
//! touches the file system, and it never lets a URL through.
//!
//! The add dialog is decided here too. An [`AddQueue`] holds what the user asked to add, as
//! [`AddItem`]s (paths and magnet links, at most [`QUEUE_CAP`]), and its head is shown. The app
//! reads the head's file, and [`LoadedItem::torrent`] parses it into a [`LoadedTorrent`]. An
//! [`AddForm`] holds the folder, "Start immediately" and, for a `.torrent`, the choice of files,
//! in a [`FileTree`] made by [`FileTree::of_files`] whose checkboxes apply
//! [`FileTree::wanted_toggle`] to the form; a torrent whose files cannot be chosen from is added
//! with all of them, and [`NoChoiceReason`] says why.
//! [`AddForm::check`] gives the [`AddCheck`] that tells whether Add may send it, and the request
//! it sends. [`folder_picks`] lists at most [`MAX_FOLDER_PICKS`] folders to offer: the session's,
//! then the history of the [`AddState`], then those of the torrents in the list. Before anything
//! is queued, [`dropped_files`] keeps the `.torrent` files of a drop and counts the others,
//! [`clipboard_magnets`] takes the magnet links on the clipboard, no more than the queue holds
//! (and [`magnet_form_items`] what "Add magnet" queues), and [`chosen_files`] reads the files a
//! file dialog names by their URIs.
//!
//! The global speed limits are shown and typed in [`KibPerSecond`], while the daemon keeps them
//! in [`KbPerSecond`]: [`parse_limit`] reads what the user typed (or the [`LimitError`] why not),
//! and [`kib_to_kbps`] and [`kbps_to_kib`] convert between the two. The [`SessionSummary`] keeps
//! each [`SpeedLimit`] with its switch, so a limit that is off keeps its value, and the status
//! bar's [`SpeedControls`] carry them while the session is known. [`limit_prefill`] gives the
//! text a limit's input starts with, and [`limit_edit`] the [`LimitEdit`] that leaving it sends;
//! [`limit_needs_value`] tells when it was switched on without a speed.
//!
//! Where the daemon is, and the dialog that sets it, are decided here as well. A
//! [`ConnectionSettings`] is a host, a port, a path and the [`x4_rpc::Credentials`] of one daemon,
//! read from an `http://` address by [`ConnectionSettings::from_url`] (or refused with a
//! [`UrlProblem`]); its [`url`](ConnectionSettings::url) never holds the credentials. A
//! [`ConnectionForm`] is the text of the dialog's fields: [`ConnectionForm::check`] turns it into
//! settings or into [`FormProblems`] for each field, and [`ConnectionForm::spread`] splits an
//! address pasted into the host field over the fields.
//! The dialog's Test button ends in a [`CheckOutcome`]: the daemon's version, or the reason it
//! cannot be used. A window with no server set has no service; its status is
//! [`ServiceStatus::NotConfigured`], and its [`Indicator`] says so.
//!
//! What the app remembers between runs is described here too: [`UiState`] is the `state.toml`
//! the app writes (window, columns, sort order, filter, theme, details panel, and the folders
//! torrents were added to), read tolerantly and repaired by [`UiState::parse`] (see also
//! [`UiState::normalized`]); [`set_column_shown`], [`move_shown_column`] and [`set_shown_widths`]
//! apply what the user does to its columns, which the table counts by [`shown_columns`].
//! [`Config`] is the owner's `config.toml`: the [`ConnectionSettings`] it names, if any (there is
//! no built-in daemon), which [`config_with_connection`] writes back into a text with the owner's
//! comments, keys and tables kept. Both files turn text into values and back; reading and writing
//! them is left to the app. A [`ConfigError`] names the [`TextPosition`] of a parse error as
//! numbers, so the app can word it.
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
//! the [`TorrentDetails`] of the selected torrent: in full once the selection settles, then only
//! their progress and, once a minute, their trackers ([`DetailsRead`]), and nothing while the
//! window is hidden. While the Peers or the Trackers tab is shown ([`Command::ShowTab`]) it also
//! reads that torrent's peers every 3 s, which come as [`Event::Peers`] and go with
//! [`Event::PeersCleared`], or its trackers every 5 s, merged into the details. A speed limit is set one [`Direction`] at a time. Magnet links are read once,
//! on [`Command::ReadMagnetLinks`], under the same rules as an action. An [`Action::Add`] is
//! answered with [`Event::AddFinished`]: the torrent [`Added`] (or found already there), or the
//! [`AddError`] why not. An add that gets no answer within its long timeout is
//! [`AddError::Unconfirmed`], which does not drop the connection: the next lists tell whether it
//! was added. [`Command::FreeSpace`] asks how much room a folder on the server has, for the folder
//! the user is choosing; a folder that does not exist yet is answered with the room in its
//! nearest parent that does ([`FreeSpaceAnswer`]), with few calls to the daemon.
//!
//! [`service::spawn`] runs the engine on a worker thread, which does all the network I/O through
//! an [`x4_rpc::Client`]. The UI talks to it through the [`service::ServiceHandle`]: it sends
//! [`Command`]s, which never blocks, awaits the [`Event`]s on its own executor, and drops the
//! handle to stop the thread without waiting for it, so it never waits on the network. The
//! thread also fills in the countries of the peers, from the databases that the
//! [`service::OpenCountries`] the app hands it opens on the first peers.
//!
//! The countries of peers come from the legacy country files that the `geoip-database`
//! package installs. [`CountryDb::open`] checks a file of an [`Edition`], or says with a
//! [`GeoIpError`] why it cannot be used, and a [`GeoIp`] holds the IPv4 and the IPv6 database and
//! gives the [`CountryCode`] of an address as a [`CountryLookup`], reading a few bytes of a file
//! per lookup and never a whole file. [`build_country_file`] writes such a file for tests and the
//! synthetic demo, and [`known_countries`] lists every country a lookup can give.
//!
//! The peers of a torrent are modeled here as well. [`build_peers`] turns a `torrent_get` answer
//! into a [`PeerList`] of [`Peer`]s and the [`PeerSources`] they were found through; it renames the
//! daemon's choke and interest flags as [`PeerTraits`] and cleans the client name a peer chose.
//! [`locate`] fills in the countries through a [`CountryLookup`], and [`peer_flag`] names the
//! letters of a flag string. What the Peers tab shows is decided here too: [`peer_rows`] orders
//! the [`PeerRow`]s as a [`PeerSort`] asks (by a [`PeerColumn`], or the busiest first; a click on
//! a header changes it with [`PeerSort::toggle`]),
//! [`peers_summary`] gives the [`PeersSummary`] above them, and [`peers_empty`] the [`PeersEmpty`]
//! reason when there are none.
//!
//! What the Trackers tab shows is decided here as well: [`tracker_rows`] gives a [`TrackerRow`] for
//! each tracker of the [`TorrentDetails`] in the daemon's order, with the [`TrackerStatus`] that
//! [`tracker_status`] tells and the [`NextAnnounce`] of its tier, and [`relative_time`] tells how
//! far a time is from now as a [`RelativeTime`], for the UI to word as "3 min ago" or "in 27 min",
//! and [`trackers_empty`] tells why there are no rows ([`TrackersEmpty`]).
//!
//! The `test-support` feature adds the `test_support` module with builders for tests of
//! dependent crates (a complete [`Torrent`], a recorded fixture's `result`) and turns on
//! `x4-rpc`'s `test-support`, whose recorded fixtures the builders work with.

mod action;
mod adding;
mod config;
mod connection;
mod details;
mod engine;
mod files;
mod filter;
mod format;
mod geoip;
pub mod launch;
mod limits;
mod metainfo;
mod model;
mod peers;
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

pub use action::{
    Action, ActionError, ActionId, AddError, Added, Direction, FileChange, FileTarget,
};
pub use adding::{
    AddCheck, AddForm, AddItem, AddNotice, AddOutcome, AddQueue, ChosenFiles, ClipboardItems,
    DroppedFiles, FREE_SPACE_DELAY, FormDefaults, FormStep, FreeSpaceCache, FreeSpaceFollow,
    ListChoices, LoadedItem, LoadedTorrent, MAX_FOLDER_PICKS, NoChoiceReason,
    PENDING_SELECTION_LIFE, PendingSelection, QUEUE_CAP, Reveal, SentAdd, TakenDefaults,
    add_outcome, chosen_files, clipboard_magnets, dropped_files, folder_picks, magnet_form_items,
    stopped_outcome,
};
pub use config::{Config, ConfigError, config_with_connection};
pub use connection::{
    CheckOutcome, ConnectionForm, ConnectionSettings, DEFAULT_PATH, DEFAULT_PORT, FormProblems,
    HostProblem, PathProblem, PortProblem, UrlProblem, UsernameProblem,
};
pub use details::{
    FileEntry, NextAnnounce, TorrentDetails, TrackerAnswer, TrackerOutcome, TrackerStat,
};
pub use engine::{
    Command, DetailsRead, Engine, EngineConfig, Event, FreeSpaceAnswer, Request, Response,
    Visibility,
};
pub use files::{
    CHEAP_TREE_FILES, FileRow, FileTree, FileTreeView, MAX_CHOOSABLE_FILES, NodeKey,
    NonEmptyIndices, PriorityView, RowKind, TREE_BUDGET, WantedState, next_priority,
};
pub use filter::{Filter, FilterCounts, counts, matches_search};
pub use format::{
    DurationParts, HumanSize, HumanSpeed, SizeUnit, SpeedUnit, duration_parts, human_size,
    human_speed,
};
pub use geoip::{
    CountryCode, CountryDb, CountryLookup, Edition, GeoIp, GeoIpError, Source, build_country_file,
    known_countries,
};
pub use limits::{
    KbPerSecond, KibPerSecond, LimitEdit, LimitError, MAX_LIMIT_KIB, MIN_LIMIT_KIB, kbps_to_kib,
    kib_to_kbps, limit_edit, limit_needs_value, limit_prefill, parse_limit,
};
pub use metainfo::{MAX_TORRENT_FILE_BYTES, MetaFile, MetainfoError, ParsedTorrent, parse_torrent};
pub use model::{Eta, ModelError, Ratio, Torrent};
pub use peers::{
    MAX_CLIENT_CHARS, Peer, PeerFlag, PeerList, PeerSources, PeerTraits, build_peers, locate,
    peer_flag,
};
pub use playable::{Playable, file_offsets, playable_prefix};
pub use snapshot::{ConnectionError, ConnectionState, SessionSummary, Snapshot, SpeedLimit};
pub use sort::{SortDirection, SortKey, sort};
pub use state::{Attention, DisplayState, QueueKind, attention, classify, is_active};
pub use store::{Converted, SkipReason, SkippedRecord, Store, torrents_from_records};
pub use toml_error::TextPosition;
pub use ui_state::{
    AddState, ColumnState, DetailsState, DetailsTab, MAX_COLUMN_WIDTH, MAX_DETAILS_HEIGHT,
    MAX_RECENT_FOLDERS, MIN_COLUMN_WIDTH, MIN_DETAILS_HEIGHT, MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH,
    ParsedUiState, SortState, ThemeChoice, UI_STATE_VERSION, UiState, UiStateError, WindowState,
    default_columns, move_shown_column, set_column_shown, set_shown_widths, shown_columns,
};
pub use view::{
    Availability, ColumnId, CompactDuration, DetailsTarget, Edge, EmptyTable, ErrorView, EtaView,
    GeneralModel, Indicator, PeerColumn, PeerRow, PeerSort, PeerSource, PeersEmpty, PeersSummary,
    PiecesView, RelativeTime, RemovalSummary, Row, Selection, ServiceStatus, SpeedControls,
    StatusModel, Subtitle, TableModel, TimeDirection, TimeUnit, ToggleKind, TrackerCount,
    TrackerProblem, TrackerProblemKind, TrackerRow, TrackerStatus, TrackerSummary, TrackersEmpty,
    ViewState, availability, details_target, general_model, peer_rows, peers_empty, peers_summary,
    relative_time, removal_summary, status_model, table_model, toggle_kind, tracker_rows,
    tracker_status, tracker_summary, trackers_empty, visibility,
};
// The reason inside `ConnectionState::Unsupported`, so that the UI can name it without depending on
// x4-rpc.
pub use x4_rpc::UnsupportedReason;
