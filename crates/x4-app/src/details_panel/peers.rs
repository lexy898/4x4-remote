//! The Peers tab: who the torrent is connected to, one line each, under a summary of how many
//! there are, how they were found and how many send to us and receive from us.
//!
//! x4-core decides all of it: the order of the rows ([`peer_rows`], by the [`PeerSort`] a click
//! on a header changes), the summary ([`peers_summary`]), why the tab is empty ([`peers_empty`]),
//! each peer's whole percent and whether data flows each way. The tab keeps the newest
//! [`PeerList`] the service sent of the panel's torrent while it is open, the rows made from it
//! and the order. A list of another torrent is never taken, and the panel's own goes the moment
//! the panel is about another torrent, so the rows of one torrent never show under another, not
//! for a frame. Left, or with the panel hidden, the tab keeps nothing: no list, no rows, and the
//! order goes back to the default, the busiest first. The order is not remembered between runs.
//!
//! Each row shows the peer's flag and country code (a muted globe when there is none), its
//! address (the port only for screen readers), its client, its progress as a bar in the torrent's
//! state color and in percent, both rates, in the state's color while data moves that way, and
//! the icons of its connection: a lock when it is encrypted, a badge when it runs over uTP, and
//! an arrow when the peer connected to us, each named in its tooltip. While the data is stale,
//! the summary and the rows are dimmed, as the table is.
//!
//! The list is the shared [`grid`](super::grid): no tab stop, no keys and no menu.

use std::collections::HashMap;
use std::mem;
use std::ops::Range;
use std::sync::Arc;

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, Theme, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, ElementId, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    TestSupportExt as _, UniformListScrollHandle, div, px, relative,
};
use x4_core::{
    DetailsTab, DetailsTarget, DisplayState, PeerColumn, PeerList, PeerRow, PeerSort, PeersSummary,
    SortDirection, Torrent, human_speed, peer_rows, peers_empty, peers_summary,
};

use super::grid::{self, Align, Column, Width};
use super::{DetailsPanel, is_of, note};
use crate::flags::flag;
use crate::i18n::{
    peer_address_label, peers_empty_text, peers_summary_texts, percent_text, speed_text, t,
};
use crate::theme::{
    PROGRESS_TRACK_OPACITY, STALE_OPACITY, rate_color, state_color, tabular_digits,
};
use crate::torrent_table::BAR_HEIGHT;

/// The element id of what the tab says instead of the peers.
pub(super) const PEERS_EMPTY_ID: &str = "peers-empty";
/// The element id of the summary line above the list.
pub(super) const PEERS_SUMMARY_ID: &str = "peers-summary";
/// The element id of the list.
const PEER_LIST_ID: &str = "peer-list";
/// The element id of the list's header row.
pub(super) const PEER_HEADER_ID: &str = "peer-header";

/// The widths of the columns: the country (the flag and its code), the address and the client
/// (each at least this wide, sharing the rest), the progress (the bar and the percent beside it),
/// the rates and the icons of the connection.
const COUNTRY_WIDTH: f32 = 52.;
const ADDRESS_MIN_WIDTH: f32 = 140.;
const CLIENT_MIN_WIDTH: f32 = 120.;
const BAR_WIDTH: f32 = 120.;
const PERCENT_WIDTH: f32 = 40.;
const RATE_WIDTH: f32 = 76.;
const TRAITS_WIDTH: f32 = 64.;
/// The room of each icon of the connection, so that each kind stays in its place from row to
/// row: the lock, the badge of uTP and the arrow.
const ICON_ROOM: f32 = 12.;
const BADGE_ROOM: f32 = 26.;
/// The side of an icon of the connection.
const ICON_SIZE: f32 = 12.;
/// The text of the badge of uTP, smaller than the row's.
const BADGE_TEXT: f32 = 10.;

/// What the Peers tab shows and how the user ordered it.
pub(super) struct PeersTab {
    /// The newest peers the service sent of the panel's torrent while the tab is open; `None`
    /// before the first, after the service let them go, and while the tab is closed, the panel
    /// hidden or about no single torrent.
    list: Option<Arc<PeerList>>,
    /// The rows of `list`, in `sort`'s order, each with its element id.
    rows: Vec<Shown>,
    /// The summary of `list`.
    summary: PeersSummary,
    /// The order the user chose with the headers.
    sort: PeerSort,
    /// The scroll position of the list.
    scroll: UniformListScrollHandle,
}

/// A row as the list draws it.
struct Shown {
    /// The element id of the row.
    id: ElementId,
    /// What it shows.
    row: PeerRow,
}

impl PeersTab {
    /// A tab with no peers, in the default order.
    pub(super) fn new() -> Self {
        Self {
            list: None,
            rows: Vec::new(),
            summary: PeersSummary::default(),
            sort: PeerSort::default(),
            scroll: UniformListScrollHandle::new(),
        }
    }

    /// Takes `list`, which the panel has checked is of its torrent, and orders its rows.
    fn take(&mut self, list: Arc<PeerList>) {
        self.summary = peers_summary(&list);
        self.list = Some(list);
        self.order();
    }

    /// Makes the rows of the list, in the order chosen.
    fn order(&mut self) {
        let rows = self
            .list
            .as_deref()
            .map(|list| peer_rows(list, self.sort))
            .unwrap_or_default();
        let ids = row_ids(&rows);
        self.rows = ids
            .into_iter()
            .zip(rows)
            .map(|(id, row)| Shown { id, row })
            .collect();
    }

    /// Lets go of the list and its rows, with the room they took, and scrolls back to the top;
    /// the order stays. Returns whether there was a list.
    pub(super) fn clear(&mut self) -> bool {
        let had = self.list.take().is_some();
        drop(mem::take(&mut self.rows));
        self.summary = PeersSummary::default();
        self.scroll = UniformListScrollHandle::new();
        had
    }

    /// Lets go of everything, as the tab closes or the panel hides: the list, its rows and the
    /// order the user chose.
    pub(super) fn leave(&mut self) {
        self.clear();
        self.sort = PeerSort::default();
    }

    /// Lets go of the list if it is not of the one torrent of `target`, as the panel turns to it.
    pub(super) fn follow(&mut self, target: &DetailsTarget) {
        if self
            .list
            .as_ref()
            .is_some_and(|list| !is_of(target, &list.hash))
        {
            self.clear();
        }
    }
}

/// The element ids of `rows`, in their order: each keyed by the peer's address and port, so that
/// a row keeps its id when the order changes, and by how many rows of the same address and port
/// come before it, as the daemon can send one twice ("Invalid address" with the same port).
fn row_ids(rows: &[PeerRow]) -> Vec<ElementId> {
    let mut seen: HashMap<&(String, u16), u64> = HashMap::new();
    rows.iter()
        .map(|row| {
            let before = seen.entry(&row.key).or_insert(0);
            let id = row_id(&row.key, *before);
            *before += 1;
            id
        })
        .collect()
}

/// The element id of the row of the peer `key`, its address and port, with `before` rows of the
/// same key above it. The count comes first, up to a space: it has no space, so no address can
/// make two rows share an id.
fn row_id(key: &(String, u16), before: u64) -> ElementId {
    let (address, port) = key;
    ElementId::NamedChild(
        Arc::new(ElementId::NamedInteger("peer-row".into(), u64::from(*port))),
        format!("{before} {address}").into(),
    )
}

impl DetailsPanel {
    /// Takes the peers the service sent, if the Peers tab is open and they are of the target's
    /// torrent: a list that was on its way when the target changed, the tab closed or the panel
    /// hid is dropped.
    pub(crate) fn take_peers(&mut self, list: Arc<PeerList>, cx: &mut Context<'_, Self>) {
        if self.tab != DetailsTab::Peers || !is_of(&self.target, &list.hash) {
            tracing::debug!("peers of a torrent or a tab the panel does not show; dropped");
            return;
        }
        self.peers.take(list);
        cx.notify();
    }

    /// Lets go of the peers, which the service no longer reads; the tab says "—" until it sends
    /// new ones.
    pub(crate) fn clear_peers(&mut self, cx: &mut Context<'_, Self>) {
        if self.peers.clear() {
            cx.notify();
        }
    }

    /// Handles a click on the header of `column`: sorts by it, or the other way round.
    fn sort_peers(&mut self, column: PeerColumn, cx: &mut Context<'_, Self>) {
        self.peers.sort.toggle(column);
        tracing::debug!(sort = ?self.peers.sort, "peers sorted");
        self.peers.order();
        cx.notify();
    }

    /// The Peers tab, for the panel's torrent `torrent`: why it shows no peers, or the summary
    /// over the list.
    pub(super) fn peers_tab(&self, torrent: &Torrent, cx: &mut Context<'_, Self>) -> AnyElement {
        if let Some(empty) = peers_empty(torrent.state, self.peers.list.as_deref()) {
            return note(PEERS_EMPTY_ID, peers_empty_text(empty), cx);
        }
        let sort = self.peers.sort;
        let sorted = sort.column.map(|column| {
            let direction = if sort.descending {
                SortDirection::Descending
            } else {
                SortDirection::Ascending
            };
            (column, direction)
        });
        let panel = cx.weak_entity();
        let header = grid::header(
            PEER_HEADER_ID,
            &columns(),
            sorted,
            move |column, _, cx| {
                if let Err(error) = panel.update(cx, |panel, cx| panel.sort_peers(column, cx)) {
                    tracing::debug!(%error, "a header was clicked after the panel closed");
                }
            },
            cx,
        );
        let list = grid::list(
            PEER_LIST_ID,
            t!("peers-list").into(),
            header,
            self.peers.rows.len(),
            &self.peers.scroll,
            cx.processor(|panel, range: Range<usize>, _window, cx| panel.peer_rows_in(range, cx)),
        );
        v_flex()
            .size_full()
            .child(self.peers_summary_line(cx))
            .child(list.flex_1().min_h_0().w_full().test_support())
            .into_any_element()
    }

    /// The line above the list: each part of the summary on its own, with a separator between.
    fn peers_summary_line(&self, cx: &App) -> impl IntoElement {
        let separator = SharedString::from(t!("peers-summary-separator"));
        let parts = peers_summary_texts(&self.peers.summary)
            .into_iter()
            .enumerate()
            .flat_map(|(index, part)| {
                let before = (index > 0).then(|| div().child(separator.clone()).into_any_element());
                let part = div()
                    .id(("summary-part", index))
                    .whitespace_nowrap()
                    .aria_label(part.clone())
                    .child(part)
                    .test_support()
                    .into_any_element();
                before.into_iter().chain([part])
            });
        h_flex()
            .id(PEERS_SUMMARY_ID)
            .flex_none()
            .px_3()
            .py_1()
            .gap_1p5()
            .overflow_hidden()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .when(self.stale, |line| line.opacity(STALE_OPACITY))
            .children(parts)
            .test_support()
    }

    /// The rows of the list at `range`, as drawn now, in the colors of the torrent's state.
    fn peer_rows_in(&self, range: Range<usize>, cx: &App) -> Vec<AnyElement> {
        let Some(torrent) = &self.torrent else {
            return Vec::new();
        };
        let columns = columns();
        let start = range.start;
        self.peers
            .rows
            .get(range)
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(offset, shown)| {
                peer_row(
                    shown,
                    start + offset,
                    &columns,
                    torrent.state,
                    self.stale,
                    cx,
                )
            })
            .collect()
    }
}

/// The columns of the list, in order: the country, the address, the client, the progress, the
/// rates and the icons of the connection. All but the icons sort.
fn columns() -> [Column<PeerColumn>; 7] {
    let column = |width, align, header: String, sort| {
        let header = SharedString::from(header);
        Column {
            width,
            align,
            name: header.clone(),
            header,
            sort,
        }
    };
    [
        column(
            Width::Fixed(COUNTRY_WIDTH),
            Align::Start,
            t!("peers-column-country"),
            Some(PeerColumn::Country),
        ),
        column(
            Width::Flex(ADDRESS_MIN_WIDTH),
            Align::Start,
            t!("peers-column-address"),
            Some(PeerColumn::Address),
        ),
        column(
            Width::Flex(CLIENT_MIN_WIDTH),
            Align::Start,
            t!("peers-column-client"),
            Some(PeerColumn::Client),
        ),
        column(
            Width::Fixed(BAR_WIDTH + PERCENT_WIDTH),
            Align::Start,
            t!("peers-column-progress"),
            Some(PeerColumn::Progress),
        ),
        Column {
            name: t!("peers-column-down-long").into(),
            ..column(
                Width::Fixed(RATE_WIDTH),
                Align::End,
                t!("peers-column-down"),
                Some(PeerColumn::Down),
            )
        },
        Column {
            name: t!("peers-column-up-long").into(),
            ..column(
                Width::Fixed(RATE_WIDTH),
                Align::End,
                t!("peers-column-up"),
                Some(PeerColumn::Up),
            )
        },
        Column {
            width: Width::Fixed(TRAITS_WIDTH),
            align: Align::Start,
            header: SharedString::default(),
            name: t!("peers-column-connection").into(),
            sort: None,
        },
    ]
}

/// The row of `shown`, the row at `index`, in `columns`, colored for a torrent in `state`, and
/// `dimmed` while the data is stale.
fn peer_row(
    shown: &Shown,
    index: usize,
    columns: &[Column<PeerColumn>; 7],
    state: DisplayState,
    dimmed: bool,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let row = &shown.row;
    let [country, address, client, progress, down, up, traits] = columns;
    let address_label = peer_address_label(&row.address, row.key.1);
    grid::row(shown.id.clone(), index, address_label.clone())
        .when(dimmed, |line| line.opacity(STALE_OPACITY))
        .child(country_cell(country, row, cx))
        .child(
            grid::cell(address)
                .id("peer-address")
                .role(Role::Cell)
                .aria_label(address_label)
                .child(
                    div()
                        .truncate()
                        .child(SharedString::from(row.address.clone())),
                )
                .test_support(),
        )
        .child(client_cell(client, row, theme))
        .child(progress_cell(progress, row, state, theme))
        .child(rate_cell(
            down,
            "peer-down",
            row.rate_down,
            row.transferring_down,
            state,
            theme,
        ))
        .child(rate_cell(
            up,
            "peer-up",
            row.rate_up,
            row.transferring_up,
            state,
            theme,
        ))
        .child(traits_cell(traits, row, theme))
        .test_support()
        .into_any_element()
}

/// The flag, or the globe, and the country's code beside it, muted.
fn country_cell(column: &Column<PeerColumn>, row: &PeerRow, cx: &App) -> impl IntoElement {
    let code = row
        .country
        .map(|code| SharedString::from(code.as_str().to_owned()));
    grid::cell(column)
        .id("peer-country")
        .role(Role::Cell)
        .aria_label(
            code.clone()
                .unwrap_or_else(|| t!("peers-country-unknown").into()),
        )
        .gap_1()
        .child(flag(row.country, cx))
        .when_some(code, |cell, code| {
            cell.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(code),
            )
        })
        .test_support()
}

/// What the peer calls itself, as the daemon sent it, or a muted dash when it said nothing.
fn client_cell(column: &Column<PeerColumn>, row: &PeerRow, theme: &Theme) -> impl IntoElement {
    let named = !row.client.is_empty();
    let text = SharedString::from(if named {
        row.client.clone()
    } else {
        t!("peers-client-none")
    });
    grid::cell(column)
        .id("peer-client")
        .role(Role::Cell)
        .aria_label(text.clone())
        .when(!named, |cell| cell.text_color(theme.muted_foreground))
        .child(div().truncate().child(text))
        .test_support()
}

/// A thin bar of how much of the torrent the peer has, in the color of the torrent's state as in
/// the Files tab, and the percent beside it.
fn progress_cell(
    column: &Column<PeerColumn>,
    row: &PeerRow,
    state: DisplayState,
    theme: &Theme,
) -> impl IntoElement {
    let color = state_color(state, theme);
    let percent = percent_text(row.percent);
    grid::cell(column)
        .id("peer-progress")
        .role(Role::Cell)
        .aria_label(percent.clone())
        .child(
            div()
                .flex_none()
                .w(px(BAR_WIDTH))
                .h(px(BAR_HEIGHT))
                .rounded_full()
                .bg(color.opacity(PROGRESS_TRACK_OPACITY))
                .child(
                    div()
                        .h_full()
                        .w(relative(row.progress))
                        .rounded_full()
                        .bg(color),
                ),
        )
        .child(
            div()
                .flex_none()
                .w(px(PERCENT_WIDTH))
                .whitespace_nowrap()
                .text_right()
                .text_xs()
                .font_features(tabular_digits())
                .text_color(theme.muted_foreground)
                .child(percent),
        )
        .test_support()
}

/// A rate, as the table draws one: right-aligned in tabular digits, in the state's color and a
/// heavier weight while data moves that way (`active`), muted otherwise.
fn rate_cell(
    column: &Column<PeerColumn>,
    id: &'static str,
    rate: u64,
    active: bool,
    state: DisplayState,
    theme: &Theme,
) -> impl IntoElement {
    let text = speed_text(human_speed(rate));
    grid::cell(column)
        .id(id)
        .role(Role::Cell)
        .aria_label(text.clone())
        .font_features(tabular_digits())
        .text_color(rate_color(state, active, theme))
        .when(active, |cell| cell.font_weight(FontWeight::SEMIBOLD))
        .child(div().truncate().child(text))
        .test_support()
}

/// The icons of the connection, monochrome, each in its own room and named in its tooltip: a
/// lock when it is encrypted, a badge when it runs over uTP, and an arrow when the peer
/// connected to us.
fn traits_cell(column: &Column<PeerColumn>, row: &PeerRow, theme: &Theme) -> impl IntoElement {
    let room = |width: f32| div().flex_none().w(px(width)).flex().justify_center();
    let icon = |name: gpui_kit::assets::IconName| {
        Icon::new(name)
            .size(px(ICON_SIZE))
            .text_color(theme.muted_foreground)
    };
    let badge = div()
        .px_0p5()
        .rounded(px(3.))
        .border_1()
        .border_color(theme.border)
        .text_size(px(BADGE_TEXT))
        .line_height(px(ICON_SIZE))
        .text_color(theme.muted_foreground)
        .child(t!("peers-utp"));
    let traits = row.traits;
    grid::cell(column)
        .id("peer-traits")
        .role(Role::Cell)
        .gap_1()
        .child(room(ICON_ROOM).when(traits.encrypted, |room| {
            room.child(marked(
                "peer-encrypted",
                t!("peers-encrypted"),
                icon(gpui_kit::assets::IconName::Lock),
            ))
        }))
        .child(room(BADGE_ROOM).when(traits.utp, |room| {
            room.child(marked("peer-utp", t!("peers-utp-tooltip"), badge))
        }))
        .child(room(ICON_ROOM).when(traits.incoming, |room| {
            room.child(marked(
                "peer-incoming",
                t!("peers-incoming"),
                icon(gpui_kit::assets::IconName::ArrowDownLeft),
            ))
        }))
        .test_support()
}

/// `content`, an icon of the connection, as `id`, with `word` as its name and its tooltip.
fn marked(id: &'static str, word: String, content: impl IntoElement) -> impl IntoElement {
    let word = SharedString::from(word);
    div()
        .id(id)
        .flex()
        .items_center()
        .role(Role::Image)
        .aria_label(word.clone())
        .tooltip(move |window, cx| Tooltip::new(word.clone()).build(window, cx))
        .child(content)
        .test_support()
}

#[cfg(test)]
mod tests;
