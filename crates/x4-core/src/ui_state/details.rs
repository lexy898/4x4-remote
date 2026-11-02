//! The state of the details panel under the table: whether it is shown, how high it is and which
//! tab is open.

use serde::{Deserialize, Serialize};

use super::wire;

/// The lowest the panel can be, in logical pixels. Not exactly 100, which the resizable split
/// would take for its own minimum and treat as "no room".
pub const MIN_DETAILS_HEIGHT: f32 = 110.0;
/// The highest the panel can be, in logical pixels, so that a hand-edited file cannot make it
/// taller than any screen. The window shrinks the panel to fit when it is shown.
pub const MAX_DETAILS_HEIGHT: f32 = 2000.0;
/// The height on the first run, in logical pixels: the General tab of a torrent with a source
/// and a tracker problem, every row the tab has but an error, fits without scrolling.
const DEFAULT_DETAILS_HEIGHT: f32 = 273.0;

/// The panel under the table.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DetailsState {
    /// Whether it is shown.
    pub open: bool,
    /// Its height in logical pixels; [`UiState::normalized`](super::UiState::normalized) keeps it
    /// from [`MIN_DETAILS_HEIGHT`] to [`MAX_DETAILS_HEIGHT`].
    pub height: f32,
    /// The tab shown.
    #[serde(deserialize_with = "wire::or_default")]
    pub tab: DetailsTab,
}

impl Default for DetailsState {
    /// The first run: open, 273 pixels high, on the General tab.
    fn default() -> Self {
        Self {
            open: true,
            height: DEFAULT_DETAILS_HEIGHT,
            tab: DetailsTab::default(),
        }
    }
}

impl DetailsState {
    /// The state with a height that fits: from [`MIN_DETAILS_HEIGHT`] to [`MAX_DETAILS_HEIGHT`],
    /// or the default one if it is not a number.
    #[must_use]
    pub(super) fn normalized(self) -> Self {
        let height = if self.height.is_finite() {
            self.height.clamp(MIN_DETAILS_HEIGHT, MAX_DETAILS_HEIGHT)
        } else {
            DEFAULT_DETAILS_HEIGHT
        };
        Self { height, ..self }
    }

    /// How high to draw the panel when `room` pixels are free for it under the table: the
    /// remembered height, lowered to fit, and never below [`MIN_DETAILS_HEIGHT`]. The remembered
    /// height itself stays, so that the panel grows back when the window does.
    #[must_use]
    pub fn fitted_height(self, room: f32) -> f32 {
        // `max` takes the minimum for a room that is not a number.
        let room = room.max(MIN_DETAILS_HEIGHT);
        self.height.min(room).max(MIN_DETAILS_HEIGHT)
    }
}

/// A tab of the details panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetailsTab {
    /// The torrent's path, sizes, hash, dates and trackers.
    #[default]
    General,
    /// The torrent's files.
    Files,
}
