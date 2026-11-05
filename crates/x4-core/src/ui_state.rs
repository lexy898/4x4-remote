//! What the app remembers between runs: the window, the columns, the sort order, the filter, the
//! theme, the details panel and the folders torrents were added to, kept in `state.toml`.
//!
//! This file is the app's own, so it is rewritten whenever something changes, and nobody is
//! expected to edit it. It must still never keep the app from starting: unknown keys, unknown
//! columns and values that a newer version wrote are ignored, and what is left is repaired (see
//! [`UiState::parse`] and [`UiState::normalized`]). Only a value of the wrong type is an error,
//! and the caller then moves the file aside and starts from the defaults.
//!
//! Reading and writing the file is the app's job; this module only turns its text into a
//! [`UiState`] and back, and applies the changes the user makes to the table's columns (the
//! `columns` module).

mod add;
mod columns;
mod details;
mod wire;

use serde::{Deserialize, Serialize};

use crate::filter::Filter;
use crate::sort::{SortDirection, SortKey};
use crate::toml_error::describe;
use crate::view::{ColumnId, ViewState};

pub use add::{AddState, MAX_RECENT_FOLDERS};
pub use columns::{move_shown_column, set_column_shown, set_shown_widths, shown_columns};
pub use details::{DetailsState, DetailsTab, MAX_DETAILS_HEIGHT, MIN_DETAILS_HEIGHT};

/// The version of the state file this build writes. A file with a higher version was written by a
/// newer build and is left alone.
pub const UI_STATE_VERSION: u32 = 1;

/// The window size when there is none to restore, or the one restored is unusable.
const DEFAULT_WINDOW_WIDTH: f32 = 1200.0;
const DEFAULT_WINDOW_HEIGHT: f32 = 720.0;
/// The narrowest the main window can be, in logical pixels: below it the window cannot be used.
///
/// The app gives the window this minimum, so the compositor never makes it smaller and a size
/// the window reports is never reset by [`WindowState::normalized`]; that lower bound only guards
/// a hand-edited file.
pub const MIN_WINDOW_WIDTH: f32 = 400.0;
/// The lowest the main window can be, in logical pixels; see [`MIN_WINDOW_WIDTH`].
pub const MIN_WINDOW_HEIGHT: f32 = 300.0;
/// Above this size in either direction the window is not real: no screen is that large, and the
/// graphics stack fails on textures beyond it.
const MAX_WINDOW_SIZE: f32 = 16384.0;
/// The narrowest a column can be, in logical pixels, so that it never vanishes.
pub const MIN_COLUMN_WIDTH: f32 = 40.0;
/// The widest a column can be, in logical pixels, so that it never fills the screen.
pub const MAX_COLUMN_WIDTH: f32 = 1200.0;

/// Everything stored in `state.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiState {
    /// The version of the file format; see [`UI_STATE_VERSION`].
    pub version: u32,
    /// The size of the main window.
    pub window: WindowState,
    /// The table's columns, in the order shown. After [`UiState::parse`] every column is in the
    /// list exactly once; edit the list only in ways that keep it so, or call
    /// [`UiState::normalized`] again.
    #[serde(deserialize_with = "wire::columns")]
    pub columns: Vec<ColumnState>,
    /// How the list is sorted.
    pub sort: SortState,
    /// The selected filter of the sidebar.
    #[serde(deserialize_with = "wire::or_default")]
    pub filter: Filter,
    /// The theme the user chose.
    #[serde(deserialize_with = "wire::or_default")]
    pub theme: ThemeChoice,
    /// The details panel under the table. A file from before the panel existed has none, and the
    /// default applies.
    pub details: DetailsState,
    /// What the add dialog remembers. A file from before it existed has none, and the history is
    /// empty.
    pub add: AddState,
}

impl Default for UiState {
    /// The first run: [`default_columns`], sorted by name, every torrent, following the system
    /// theme, with the details panel open.
    fn default() -> Self {
        Self {
            version: UI_STATE_VERSION,
            window: WindowState::default(),
            columns: default_columns(),
            sort: SortState::default(),
            filter: Filter::default(),
            theme: ThemeChoice::default(),
            details: DetailsState::default(),
            add: AddState::default(),
        }
    }
}

/// The size of the main window.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    /// The width in logical pixels.
    pub width: f32,
    /// The height in logical pixels.
    pub height: f32,
    /// Whether the window was maximized.
    pub maximized: bool,
}

impl Default for WindowState {
    /// 1200 by 720, not maximized.
    fn default() -> Self {
        Self {
            width: DEFAULT_WINDOW_WIDTH,
            height: DEFAULT_WINDOW_HEIGHT,
            maximized: false,
        }
    }
}

impl WindowState {
    /// The window as it is, or with the default size if it is too small or too large to use or
    /// the size is not a number. Whether it is maximized stays.
    ///
    /// [`UiState::normalized`] does this for a whole state; this is for a size the window reports
    /// while it runs, before it is remembered.
    #[must_use]
    pub fn normalized(self) -> Self {
        let usable = (MIN_WINDOW_WIDTH..=MAX_WINDOW_SIZE).contains(&self.width)
            && (MIN_WINDOW_HEIGHT..=MAX_WINDOW_SIZE).contains(&self.height);
        if usable {
            self
        } else {
            Self {
                width: DEFAULT_WINDOW_WIDTH,
                height: DEFAULT_WINDOW_HEIGHT,
                maximized: self.maximized,
            }
        }
    }
}

/// How one column of the table is shown.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ColumnState {
    /// Which column. Written as [`ColumnId::key`].
    #[serde(serialize_with = "wire::column_id")]
    pub id: ColumnId,
    /// Whether the column is shown.
    pub visible: bool,
    /// The width in logical pixels.
    pub width: f32,
}

/// How the list is sorted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SortState {
    /// The column sorted by.
    #[serde(deserialize_with = "wire::or_default")]
    pub key: SortKey,
    /// Which way.
    #[serde(deserialize_with = "wire::or_default")]
    pub direction: SortDirection,
}

/// The theme the user chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeChoice {
    /// Follow the system.
    #[default]
    System,
    /// Always light.
    Light,
    /// Always dark.
    Dark,
}

/// Why the state file could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UiStateError {
    /// The text is not valid TOML, or a value has the wrong type. Holds the position and the
    /// message of the parser.
    #[error("cannot parse the UI state: {0}")]
    Parse(String),
    /// The state could not be turned into TOML. Holds the message of the serializer.
    #[error("cannot serialize the UI state: {0}")]
    Serialize(String),
}

/// The result of [`UiState::parse`].
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedUiState {
    /// What the file holds, normalized, or the defaults if it is [`newer`](Self::newer).
    pub state: UiState,
    /// Whether the file was written by a newer version of the app. Then `state` is the defaults,
    /// and the caller must not save it, or the newer file would be overwritten.
    pub newer: bool,
}

/// The part of the file that tells which version wrote it.
#[derive(Deserialize)]
struct Header {
    /// Read wide, so that any number a newer version might write still counts as newer.
    version: Option<u64>,
}

/// The columns of the table on the first run: all of them, shown, in their natural order.
#[must_use]
pub fn default_columns() -> Vec<ColumnState> {
    ColumnId::ALL.map(default_column).to_vec()
}

/// A shown column of the default width.
fn default_column(id: ColumnId) -> ColumnState {
    ColumnState {
        id,
        visible: true,
        width: default_width(id),
    }
}

/// How wide a column is before the user resizes it: room for its widest usual text
/// ("Downloading", a two-part ETA such as "23 h 59 m", a sorted header with its mark) in a 14 px
/// sans-serif font plus the cell's 16 px of padding. Together they take 978 px, which fits the
/// default 1200 px window next to the sidebar.
///
/// The font is the one GPUI really draws with, not the one it asks for. On Linux GPUI asks for IBM
/// Plex Sans, which desktops rarely have, and takes the first installed family of its fallback
/// list: Adwaita Sans (`adwaita-fonts` on Arch), then Cantarell, Noto Sans and `DejaVu Sans`. In
/// tabular digits "23 h 59 m" measures 68.1 px in Adwaita Sans, 62.1 px in Cantarell, 64.7 px in
/// Noto Sans and 71.5 px in `DejaVu Sans`, so the ETA column holds 72 px of text, which fits all
/// four. At 82 px it held 66 and cut "37 m 30 s" (67.2 px in Adwaita Sans). A time with three
/// digits of days, such as "100 d 23 h" (73.5 px in Adwaita Sans), is still wider; the user can
/// widen the column, and the width is kept.
fn default_width(id: ColumnId) -> f32 {
    match id {
        ColumnId::Name => 279.0,
        ColumnId::Size => 80.0,
        ColumnId::Progress => 120.0,
        ColumnId::State => 105.0,
        ColumnId::Down | ColumnId::Up | ColumnId::Eta => 88.0,
        ColumnId::Ratio => 64.0,
        ColumnId::Peers => 66.0,
    }
}

impl UiState {
    /// Reads the contents of `state.toml`.
    ///
    /// The reading is tolerant: an empty text gives the defaults, a missing key takes its
    /// default, unknown keys and tables are ignored, a filter, sort key, direction, theme or
    /// details tab this version does not know becomes the default, and a column it does not know
    /// is dropped. What is left is then repaired by [`UiState::normalized`], so the state returned
    /// can be used as it is.
    ///
    /// A file from a newer version (`version` above [`UI_STATE_VERSION`]) is not read at all: the
    /// result holds the defaults and has [`ParsedUiState::newer`] set, so that the caller can
    /// leave the file alone. Older versions would be migrated here; there is none yet.
    ///
    /// # Errors
    ///
    /// [`UiStateError::Parse`] if `text` is not valid TOML or a value has the wrong type, such
    /// as a width that is a string.
    pub fn parse(text: &str) -> Result<ParsedUiState, UiStateError> {
        let parse_error = |error: toml::de::Error| UiStateError::Parse(describe(&error, text));
        // The version is read on its own first: a newer file may have changed a type that the
        // full read below would reject, and it must not look like a corrupt file.
        let header: Header = toml::from_str(text).map_err(parse_error)?;
        if header
            .version
            .is_some_and(|version| version > u64::from(UI_STATE_VERSION))
        {
            return Ok(ParsedUiState {
                state: Self::default(),
                newer: true,
            });
        }
        let mut state: Self = toml::from_str(text).map_err(parse_error)?;
        // Older versions would be migrated here. Nothing differs between them yet, so a file
        // that says it is older is simply the current one.
        state.version = UI_STATE_VERSION;
        Ok(ParsedUiState {
            state: state.normalized(),
            newer: false,
        })
    }

    /// Writes the state as the TOML text of `state.toml`.
    ///
    /// # Errors
    ///
    /// [`UiStateError::Serialize`] if the state cannot be written as TOML.
    pub fn to_toml(&self) -> Result<String, UiStateError> {
        toml::to_string_pretty(self).map_err(|error| UiStateError::Serialize(error.to_string()))
    }

    /// Repairs a state read from a file, so that the rest of the app can rely on it.
    /// [`UiState::parse`] does this already; it is public for states changed afterwards.
    ///
    /// - Every column is in the list exactly once: a repeat is dropped (the first one counts) and
    ///   a column that is missing is added at the end, shown, with its default width.
    /// - The name column is always shown.
    /// - A width is clamped to 40 to 1200 pixels; one that is not a number takes the default of
    ///   its column.
    /// - A window smaller than 400 by 300 pixels, larger than 16384 pixels in either direction,
    ///   or with a size that is not a number, takes the default size.
    /// - The details panel is 110 to 2000 pixels high; a height that is not a number takes the
    ///   default one.
    /// - The history of folders holds at most 10 folders the daemon can use, normalized, each
    ///   once; the others are dropped.
    #[must_use]
    pub fn normalized(mut self) -> Self {
        self.window = self.window.normalized();
        self.details = self.details.normalized();
        self.add = self.add.normalized();

        let mut columns: Vec<ColumnState> = Vec::with_capacity(ColumnId::ALL.len());
        for column in self.columns {
            if columns.iter().all(|kept| kept.id != column.id) {
                columns.push(column);
            }
        }
        for id in ColumnId::ALL {
            if columns.iter().all(|kept| kept.id != id) {
                columns.push(default_column(id));
            }
        }
        for column in &mut columns {
            column.width = if column.width.is_finite() {
                column.width.clamp(MIN_COLUMN_WIDTH, MAX_COLUMN_WIDTH)
            } else {
                default_width(column.id)
            };
            column.visible |= !column.id.can_hide();
        }
        self.columns = columns;
        self
    }

    /// The filter and the sort order as a [`ViewState`] with an empty search.
    #[must_use]
    pub fn view_state(&self) -> ViewState {
        ViewState {
            filter: self.filter,
            search: String::new(),
            sort_key: self.sort.key,
            sort_direction: self.sort.direction,
        }
    }
}

#[cfg(test)]
mod tests;
