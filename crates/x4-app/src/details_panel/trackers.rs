//! The Trackers tab: one row for each tracker of the torrent, in the order the daemon keeps them,
//! with what it is doing, when it was last announced to and will be next, and the counts it knows.
//!
//! x4-core decides all of it: the rows and their status ([`tracker_rows`]), how far the last and
//! next announce are from now ([`relative_time`](x4_core::relative_time)) and why the tab is
//! empty ([`trackers_empty`]). The trackers arrive inside the details the service sends (it reads
//! them every 5 seconds while the tab is open, and the panel keeps the newest details for the
//! General tab), so the tab keeps only the rows made from them and the hash of the torrent they
//! belong to. They are made when the tab opens and with each new details of the panel's torrent,
//! and they go the moment the panel is about another torrent, the tab is left or the panel hides:
//! the rows of one torrent never show under another, not for a frame. The tab is not sortable,
//! as the order of the tiers means something.
//!
//! Each row is 44 px high, like a row of the torrent table, and has two lines in the Tracker
//! column. The first is the address, with the site name muted beside it when the address does not
//! say it. The second is the status in words, or the daemon's own text when an announce failed
//! (cleaned and clipped to the line by x4-core, the whole text in a tooltip), then how many peers
//! the last announce brought and how long ago it was; a failed scrape is a muted note after them,
//! only when the tracker reported neither seeders nor leechers, while the dot stays the working
//! color. The Peers column is not a column of its own: with the others the table needs more width
//! than the panel has at its least.
//!
//! The times are counted from the clock at each draw. The service sends the details again only
//! when a tracker changed, so while the tab shows trackers it draws itself again every few
//! seconds, and a "27 m" counts down on an idle torrent too. While the data is stale the rows are
//! dimmed, as the table is.
//!
//! The list is the shared [`grid`](super::grid): no tab stop, no keys and no menu.

use std::collections::HashMap;
use std::mem;
use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Theme, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, ElementId, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, Role, SharedString, StatefulInteractiveElement as _, Styled, Task,
    TestSupportExt as _, UniformListScrollHandle, div, px,
};
use x4_core::{
    DetailsTab, SortDirection, TorrentDetails, TrackerRow, TrackersEmpty, tracker_rows,
    trackers_empty,
};
use x4_rpc::{HashString, TrackerId};

use super::grid::{self, Align, Column, Width};
use super::{DetailsPanel, note};
use crate::i18n::{
    t, tracker_count_text, tracker_last_text, tracker_next_text, tracker_peers_text,
    tracker_row_label, tracker_status_text, tracker_status_words, tracker_tier_text,
    trackers_empty_text,
};
use crate::theme::{problem_note_color, stale_opacity, tabular_digits, tracker_color};

/// The element id of what the tab says instead of the trackers.
pub(super) const TRACKERS_EMPTY_ID: &str = "trackers-empty";
/// The element id of the list.
const TRACKER_LIST_ID: &str = "tracker-list";
/// The element id of the list's header row.
pub(super) const TRACKER_HEADER_ID: &str = "tracker-header";

/// The height of a row: two lines, as in the torrent table.
const ROW_HEIGHT: f32 = 44.;
/// How often the tab draws itself again, so that the times on its rows move: the service sends
/// the details again only when a tracker changed, which on an idle torrent is every few minutes.
/// It is the pace of the service's reads, so a time is never much older than the data it is
/// counted from.
const REFRESH_EVERY: Duration = Duration::from_secs(5);

/// The widths of the columns: the dot, the tier, the tracker (at least this wide, taking the
/// rest), the last and next announce, and the seeders, leechers and downloads the tracker knows.
const DOT_WIDTH: f32 = 12.;
const TIER_WIDTH: f32 = 40.;
const TRACKER_MIN_WIDTH: f32 = 160.;
const ANNOUNCE_WIDTH: f32 = 110.;
const SEEDERS_WIDTH: f32 = 64.;
const LEECHERS_WIDTH: f32 = 64.;
const DOWNLOADS_WIDTH: f32 = 76.;
/// The side of the status dot.
const DOT_SIZE: f32 = 8.;

/// What the Trackers tab shows.
pub(super) struct TrackersTab {
    /// The torrent the rows belong to; `None` before the first details, after the tab let them
    /// go, and while the tab is closed, the panel hidden or about no single torrent.
    hash: Option<HashString>,
    /// Why there are no rows, if there are none. Before the first details it is
    /// [`TrackersEmpty::Reading`].
    empty: Option<TrackersEmpty>,
    /// The rows of the tab, in the daemon's order, each with its element id.
    rows: Vec<Shown>,
    /// The scroll position of the list.
    scroll: UniformListScrollHandle,
    /// What the times are counted from. The wall clock; the tests set a fixed one.
    clock: fn() -> SystemTime,
    /// The task that draws the tab again every few seconds, running while there are rows and the
    /// window is shown.
    refresh: Option<Task<()>>,
}

/// A row as the list draws it.
struct Shown {
    /// The element id of the row.
    id: ElementId,
    /// What it shows.
    row: TrackerRow,
}

impl TrackersTab {
    /// A tab with no trackers.
    pub(super) fn new() -> Self {
        Self {
            hash: None,
            empty: Some(TrackersEmpty::Reading),
            rows: Vec::new(),
            scroll: UniformListScrollHandle::new(),
            clock: SystemTime::now,
            refresh: None,
        }
    }

    /// Shows the trackers of `details`, or the wait for them without any. Returns whether
    /// something on the tab changed.
    fn follow(&mut self, details: Option<&TorrentDetails>) -> bool {
        let hash = details.map(|details| details.hash.clone());
        let rows = details.map(tracker_rows).unwrap_or_default();
        let empty = trackers_empty(details);
        let unchanged = hash == self.hash
            && empty == self.empty
            && self.rows.iter().map(|shown| &shown.row).eq(&rows);
        if unchanged {
            return false;
        }
        if hash != self.hash {
            // Another torrent's list starts at the top.
            self.scroll = UniformListScrollHandle::new();
        }
        let ids = row_ids(&rows);
        self.rows = ids
            .into_iter()
            .zip(rows)
            .map(|(id, row)| Shown { id, row })
            .collect();
        self.hash = hash;
        self.empty = empty;
        true
    }

    /// Whether the task that draws the tab again every few seconds runs.
    pub(super) fn redraws(&self) -> bool {
        self.refresh.is_some()
    }

    /// Lets go of everything, with the room the rows took, as the tab closes or the panel hides.
    /// Returns whether there was anything.
    pub(super) fn leave(&mut self) -> bool {
        let had = self.hash.is_some() || self.refresh.is_some();
        self.hash = None;
        self.empty = Some(TrackersEmpty::Reading);
        drop(mem::take(&mut self.rows));
        self.scroll = UniformListScrollHandle::new();
        self.refresh = None;
        had
    }
}

/// The element ids of `rows`, in their order: each keyed by the daemon's id of the tracker, so
/// that a row keeps its id when the daemon rotates a tier, and by how many rows of the same id come
/// before it, as an id the daemon sent in a form we cannot read is 0 for every such tracker.
fn row_ids(rows: &[TrackerRow]) -> Vec<ElementId> {
    let mut seen: HashMap<TrackerId, u64> = HashMap::new();
    rows.iter()
        .map(|row| {
            let before = seen.entry(row.id).or_insert(0);
            let id = row_id(row.id, *before);
            *before += 1;
            id
        })
        .collect()
}

/// The element id of the row of the tracker `id`, with `before` rows of the same id above it.
fn row_id(id: TrackerId, before: u64) -> ElementId {
    ElementId::NamedChild(
        Arc::new(ElementId::NamedInteger(
            "tracker-row".into(),
            u64::from(id.0),
        )),
        before.to_string().into(),
    )
}

impl DetailsPanel {
    /// Fits the Trackers tab to the panel's details while the tab is open, and lets it go
    /// otherwise; returns whether it shows something else now. Keeps the task that draws the tab
    /// again running while there are rows to count the times of and the window is shown, and
    /// only then.
    pub(super) fn follow_trackers(&mut self, cx: &mut Context<'_, Self>) -> bool {
        if self.tab != DetailsTab::Trackers {
            return self.trackers.leave();
        }
        let changed = self.trackers.follow(self.details.as_deref());
        if self.trackers.rows.is_empty() || !self.window_shown {
            self.trackers.refresh = None;
        } else if self.trackers.refresh.is_none() {
            self.trackers.refresh = Some(cx.spawn(async |panel, cx| {
                loop {
                    cx.background_executor().timer(REFRESH_EVERY).await;
                    if panel.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            }));
        }
        changed
    }

    /// The Trackers tab: why it shows no trackers, or the list.
    pub(super) fn trackers_tab(&self, cx: &mut Context<'_, Self>) -> AnyElement {
        if let Some(empty) = self.trackers.empty {
            return note(TRACKERS_EMPTY_ID, trackers_empty_text(empty), cx);
        }
        let header = grid::header(
            TRACKER_HEADER_ID,
            &columns(),
            None::<((), SortDirection)>,
            |(), _, _| {},
            cx,
        );
        grid::list(
            TRACKER_LIST_ID,
            t!("trackers-list").into(),
            header,
            self.trackers.rows.len(),
            &self.trackers.scroll,
            cx.processor(|panel, range: Range<usize>, _window, cx| {
                panel.tracker_rows_in(range, cx)
            }),
        )
        .size_full()
        .test_support()
        .into_any_element()
    }

    /// The rows of the list at `range`, as drawn now: the times counted from the clock.
    fn tracker_rows_in(&self, range: Range<usize>, cx: &App) -> Vec<AnyElement> {
        self.styled_tracker_rows(range, cx)
            .into_iter()
            .map(IntoElement::into_any_element)
            .collect()
    }

    /// The rows of the list at `range`, with their style open to the tests.
    fn styled_tracker_rows(
        &self,
        range: Range<usize>,
        cx: &App,
    ) -> Vec<impl IntoElement + Styled + use<>> {
        let columns = columns();
        let now = (self.trackers.clock)();
        let start = range.start;
        self.trackers
            .rows
            .get(range)
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(offset, shown)| {
                tracker_row(shown, start + offset, &columns, now, self.stale, cx)
            })
            .collect()
    }

    /// The opacity of each row of the Trackers tab, as drawn now. For the tests.
    #[cfg(test)]
    pub(crate) fn trackers_dimming(&self, cx: &App) -> Vec<Option<f32>> {
        self.styled_tracker_rows(0..self.trackers.rows.len(), cx)
            .iter_mut()
            .map(|row| row.style().opacity)
            .collect()
    }
}

/// The columns of the list, in order: the status dot, the tier, the tracker, the last and the next
/// announce, and the seeders, the leechers and the downloads. None of them sorts.
fn columns() -> [Column<()>; 8] {
    let column = |width, align, header: String| {
        let header = SharedString::from(header);
        Column {
            width,
            align,
            name: header.clone(),
            header,
            sort: None,
        }
    };
    [
        // The dots have no title, only a name for screen readers.
        Column {
            header: SharedString::default(),
            ..column(
                Width::Fixed(DOT_WIDTH),
                Align::Start,
                t!("trackers-column-status"),
            )
        },
        column(
            Width::Fixed(TIER_WIDTH),
            Align::End,
            t!("trackers-column-tier"),
        ),
        column(
            Width::Flex(TRACKER_MIN_WIDTH),
            Align::Start,
            t!("trackers-column-tracker"),
        ),
        column(
            Width::Fixed(ANNOUNCE_WIDTH),
            Align::Start,
            t!("trackers-column-last"),
        ),
        column(
            Width::Fixed(ANNOUNCE_WIDTH),
            Align::Start,
            t!("trackers-column-next"),
        ),
        column(
            Width::Fixed(SEEDERS_WIDTH),
            Align::End,
            t!("trackers-column-seeders"),
        ),
        column(
            Width::Fixed(LEECHERS_WIDTH),
            Align::End,
            t!("trackers-column-leechers"),
        ),
        column(
            Width::Fixed(DOWNLOADS_WIDTH),
            Align::End,
            t!("trackers-column-downloads"),
        ),
    ]
}

/// The row of `shown`, the row at `index`, in `columns`, with its times counted from `now`, and
/// `dimmed` while the data is stale.
fn tracker_row(
    shown: &Shown,
    index: usize,
    columns: &[Column<()>; 8],
    now: SystemTime,
    dimmed: bool,
    cx: &App,
) -> impl IntoElement + Styled + use<> {
    let theme = cx.theme();
    let row = &shown.row;
    let [dot, tier, tracker, last, next, seeders, leechers, downloads] = columns;
    grid::row(
        shown.id.clone(),
        index,
        tracker_row_label(&row.host, row.tier),
    )
    .h(px(ROW_HEIGHT))
    .when_some(stale_opacity(dimmed), Styled::opacity)
    .child(dot_cell(dot, row, theme))
    .child(text_cell(
        tier,
        "tracker-tier",
        tracker_tier_text(row.tier),
        Some(theme.muted_foreground),
    ))
    .child(tracker_cell(tracker, row, now, theme))
    .child(text_cell(
        last,
        "tracker-last",
        tracker_last_text(row.last, now),
        None,
    ))
    .child(text_cell(
        next,
        "tracker-next",
        tracker_next_text(row.next, now),
        None,
    ))
    .child(text_cell(
        seeders,
        "tracker-seeders",
        tracker_count_text(row.seeders),
        None,
    ))
    .child(text_cell(
        leechers,
        "tracker-leechers",
        tracker_count_text(row.leechers),
        None,
    ))
    .child(text_cell(
        downloads,
        "tracker-downloads",
        tracker_count_text(row.downloads),
        None,
    ))
    .test_support()
}

/// The dot in the color of what the tracker is doing; a screen reader hears the status in words.
fn dot_cell(column: &Column<()>, row: &TrackerRow, theme: &Theme) -> impl IntoElement {
    grid::cell(column)
        .id("tracker-dot")
        .role(Role::Cell)
        .aria_label(tracker_status_words(row.status))
        .justify_center()
        .child(
            div()
                .flex_none()
                .size(px(DOT_SIZE))
                .rounded_full()
                .bg(tracker_color(row.status, theme)),
        )
        .test_support()
}

/// A cell of one line of text in `color`, or in the row's own, in tabular digits so that the
/// figures of a column line up.
fn text_cell(
    column: &Column<()>,
    id: &'static str,
    text: SharedString,
    color: Option<Hsla>,
) -> impl IntoElement {
    grid::cell(column)
        .id(id)
        .role(Role::Cell)
        .aria_label(text.clone())
        .font_features(tabular_digits())
        .when_some(color, Styled::text_color)
        .child(div().truncate().child(text))
        .test_support()
}

/// The tracker's address, with its site name muted beside it when the address does not say it,
/// and under it the status line.
fn tracker_cell(
    column: &Column<()>,
    row: &TrackerRow,
    now: SystemTime,
    theme: &Theme,
) -> impl IntoElement {
    let host = SharedString::from(row.host.clone());
    let first_line = h_flex()
        .gap_1p5()
        .min_w_0()
        .overflow_hidden()
        .child(
            div()
                .id("tracker-host")
                .role(Role::Cell)
                .aria_label(host.clone())
                .truncate()
                .child(host)
                .test_support(),
        )
        .when_some(row.sitename.clone(), |line, sitename| {
            let sitename = SharedString::from(sitename);
            line.child(
                div()
                    .id("tracker-sitename")
                    .role(Role::Cell)
                    .aria_label(sitename.clone())
                    .truncate()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(sitename)
                    .test_support(),
            )
        });
    grid::cell(column)
        .id("tracker-name")
        .role(Role::Cell)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .child(first_line)
                .child(status_line(row, now, theme)),
        )
        .test_support()
}

/// The line under the address: the status in words, or the daemon's text, then how many peers
/// the last announce brought, how long ago it was, and a note when the last scrape failed. Each
/// part is its own element, with the separator between, so that a long text of the daemon is cut
/// and the parts after it are not.
fn status_line(row: &TrackerRow, now: SystemTime, theme: &Theme) -> impl IntoElement {
    let color = tracker_color(row.status, theme);
    let status = tracker_status_text(row);
    let separator = SharedString::from(t!("tracker-line-separator"));
    let status_part = div()
        .id("tracker-status")
        .role(Role::Cell)
        .aria_label(status.clone())
        .min_w_0()
        .truncate()
        .child(status.clone())
        .when(row.message.is_some(), |part| {
            // The line is cut at the cell's edge: the whole text is in the tooltip.
            let full = status.clone();
            part.tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx))
        })
        .test_support();
    let extras = [
        row.peers
            .map(|peers| ("tracker-peers", tracker_peers_text(peers), color)),
        row.last
            .map(|_| ("tracker-ago", tracker_last_text(row.last, now), color)),
        // A scrape's trouble is mild, and the General tab says it in the same color.
        row.scrape_problem.then(|| {
            (
                "tracker-scrape",
                SharedString::from(t!("tracker-scrape-problem")),
                problem_note_color(true, theme),
            )
        }),
    ]
    .into_iter()
    .flatten()
    .flat_map(|(id, text, color)| {
        [
            div()
                .flex_none()
                .child(separator.clone())
                .into_any_element(),
            div()
                .id(id)
                .role(Role::Cell)
                .aria_label(text.clone())
                .flex_none()
                .whitespace_nowrap()
                .text_color(color)
                .child(text)
                .test_support()
                .into_any_element(),
        ]
    });
    h_flex()
        .gap_1()
        .overflow_hidden()
        .text_xs()
        .text_color(color)
        .child(status_part)
        .children(extras)
}

#[cfg(test)]
mod tests;
