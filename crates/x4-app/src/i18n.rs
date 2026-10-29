//! Localized text: the Fluent messages of `i18n/` and the helpers that turn the values of
//! `x4-core` (sizes, speeds, ETAs, subtitles, states, the connection) into strings.
//!
//! Every user-visible string is a message in `i18n/en/x4.ftl`, fetched with [`t!`]: the macro
//! checks the message id against that file at compile time, and the argument names too when the
//! call passes any (a call without arguments is not checked for them). So a message with
//! variables is fetched only by the helpers here, which have tests. Another language is one more
//! `i18n/<code>/x4.ftl`; the `parity` test checks that it defines the same messages with the same
//! variables.
//!
//! Fluent does not format numbers by the rules of a language, so sizes, speeds and ratios are
//! formatted here ([`decimal`]) and passed as strings, with the decimal separator taken from the
//! message `number-decimal-separator`. Counts go in as numbers, so that the plural rules of the
//! language pick the wording.

use std::sync::LazyLock;

use gpui_kit::SharedString;
use i18n_embed::fluent::{FluentLanguageLoader, fluent_language_loader};
use i18n_embed::{I18nAssets, LanguageLoader as _};
use rust_embed::RustEmbed;
use x4_core::{
    ActionError, ColumnId, ConnectionError, Direction, DisplayState, EmptyTable, EtaView, Filter,
    HumanSize, HumanSpeed, Indicator, KibPerSecond, LimitError, MAX_LIMIT_KIB, MIN_LIMIT_KIB,
    QueueKind, Ratio, RemovalSummary, SizeUnit, SortDirection, SpeedControls, SpeedLimit,
    SpeedUnit, Subtitle, TimeUnit, UnsupportedReason, kbps_to_kib,
};
use x4_rpc::RemoveData;

use crate::actions::{ActionKind, PendingAction};
use crate::app::ConfigProblem;
use crate::clipboard::MagnetFailure;
use crate::speed_limits::LimitFeedback;

#[cfg(test)]
mod parity;
#[cfg(test)]
mod tests;

/// The message files, embedded into release builds and read from disk in debug builds.
#[derive(RustEmbed)]
#[folder = "i18n/"]
struct Localizations;

/// The messages of the app. English is the fallback, and the only language for now.
///
/// If the messages cannot be loaded, the error is logged and every message reads as the loader's
/// placeholder text, so the app still starts.
///
/// Once the language is chosen at run time, `set_use_isolating(false)` (see `load`) must be
/// applied again after every `load_languages` or `select_languages`: bundles loaded later get the
/// isolation marks around arguments again.
pub(crate) static LOADER: LazyLock<FluentLanguageLoader> = LazyLock::new(|| load(&Localizations));

/// Fetches a message from [`LOADER`]: `t!("id")` or `t!("id", name = value, ...)`.
///
/// The id is checked against `i18n/en/x4.ftl` when the crate compiles, and so are the argument
/// names, but only when the call passes at least one argument. Counts go in as numbers; sizes, speeds and ratios as strings made by [`decimal`].
macro_rules! t {
    ($($arguments:tt)*) => {
        ::i18n_embed_fl::fl!($crate::i18n::LOADER, $($arguments)*)
    };
}
pub(crate) use t;

/// Builds a loader on the fallback language of `assets`.
fn load(assets: &dyn I18nAssets) -> FluentLanguageLoader {
    let loader = fluent_language_loader!();
    if let Err(error) = loader.load_fallback_language(assets) {
        tracing::error!(%error, "cannot load the messages; texts show placeholders");
    }
    // The languages we ship are written left to right, so the invisible marks Fluent puts around
    // arguments are not needed, and they might be drawn as boxes.
    loader.set_use_isolating(false);
    loader
}

/// Formats `value` with `fraction_digits` decimals and the decimal separator of the language
/// (the message `number-decimal-separator`).
pub(crate) fn decimal(value: f64, fraction_digits: usize) -> String {
    with_separator(value, fraction_digits, &t!("number-decimal-separator"))
}

fn with_separator(value: f64, fraction_digits: usize, separator: &str) -> String {
    format!("{value:.fraction_digits$}").replacen('.', separator, 1)
}

/// Formats the whole number `value` with its digits in groups of three, apart by the separator
/// of the language (the message `number-group-separator`), such as "1,000,000".
pub(crate) fn integer(value: u32) -> String {
    with_groups(value, &t!("number-group-separator"))
}

fn with_groups(value: u32, separator: &str) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(digits.len() * 2);
    for (index, digit) in digits.chars().enumerate() {
        // `index` is below the length, so this counts the digits still to come, this one included.
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push_str(separator);
        }
        grouped.push(digit);
    }
    grouped
}

/// A size such as "22.9 GiB".
pub(crate) fn size_text(size: HumanSize) -> SharedString {
    size_string(size).into()
}

fn size_string(size: HumanSize) -> String {
    let unit = match size.unit {
        SizeUnit::B => t!("unit-b"),
        SizeUnit::KiB => t!("unit-kib"),
        SizeUnit::MiB => t!("unit-mib"),
        SizeUnit::GiB => t!("unit-gib"),
        SizeUnit::TiB => t!("unit-tib"),
    };
    t!(
        "quantity",
        value = decimal(size.value, size.fraction_digits()),
        unit = unit
    )
}

/// A transfer rate such as "180 KiB/s".
pub(crate) fn speed_text(speed: HumanSpeed) -> SharedString {
    speed_string(speed).into()
}

fn speed_string(speed: HumanSpeed) -> String {
    let unit = match speed.unit {
        SpeedUnit::BytesPerSec => t!("unit-b-per-s"),
        SpeedUnit::KiBPerSec => t!("unit-kib-per-s"),
        SpeedUnit::MiBPerSec => t!("unit-mib-per-s"),
        SpeedUnit::GiBPerSec => t!("unit-gib-per-s"),
    };
    t!(
        "quantity",
        value = decimal(speed.value, speed.fraction_digits()),
        unit = unit
    )
}

/// The total download speed in the status bar, such as "↓ 2.4 MiB/s".
pub(crate) fn download_rate_text(speed: HumanSpeed) -> SharedString {
    t!("status-download", speed = speed_string(speed)).into()
}

/// The total upload speed in the status bar, such as "↑ 180 KiB/s".
pub(crate) fn upload_rate_text(speed: HumanSpeed) -> SharedString {
    t!("status-upload", speed = speed_string(speed)).into()
}

/// A speed in whole KiB/s, as the speed limits are shown, such as "500 KiB/s".
fn kib_speed_string(speed: KibPerSecond) -> String {
    t!(
        "quantity",
        value = integer(speed.0),
        unit = t!("unit-kib-per-s")
    )
}

/// A normal speed limit as the status bar shows it: its value in whole KiB/s while it is on, "no
/// limit" while it is off.
fn limit_string(limit: SpeedLimit) -> String {
    if limit.enabled {
        kib_speed_string(kbps_to_kib(limit.kbps))
    } else {
        t!("limit-none")
    }
}

/// What the limits button of the status bar says: both normal limits of `speed`, such as
/// "↓ 500 KiB/s · ↑ no limit", or "Speed limits" while the daemon's settings are not known.
pub(crate) fn limits_button_text(speed: Option<SpeedControls>) -> SharedString {
    match speed {
        Some(speed) => t!(
            "speed-limits-button",
            down = limit_string(speed.limit_down),
            up = limit_string(speed.limit_up)
        ),
        None => t!("speed-limits"),
    }
    .into()
}

/// The tooltip of the turtle: the alternative limits of `speed` it applies, such as "Alternative
/// speed limits: ↓ 50 KiB/s ↑ 50 KiB/s", or its name alone while they are not known.
pub(crate) fn alt_speed_tooltip_text(speed: Option<SpeedControls>) -> SharedString {
    match speed {
        Some(speed) => t!(
            "alt-speed-tooltip",
            down = kib_speed_string(speed.alt_down),
            up = kib_speed_string(speed.alt_up)
        ),
        None => t!("alt-speed"),
    }
    .into()
}

/// What the line under a speed limit's field says when the limit cannot be applied: why its text
/// is not a limit, naming the range for a number out of it, or that it was switched on without a
/// speed.
pub(crate) fn limit_feedback_text(feedback: LimitFeedback) -> SharedString {
    match feedback {
        LimitFeedback::Invalid(LimitError::NotANumber) => t!("limit-not-a-number"),
        LimitFeedback::Invalid(LimitError::OutOfRange) => t!(
            "limit-out-of-range",
            min = integer(MIN_LIMIT_KIB.0),
            max = kib_speed_string(MAX_LIMIT_KIB)
        ),
        LimitFeedback::NeedsValue => t!("limit-needs-value"),
    }
    .into()
}

/// The free space in the status bar, such as "22.9 GiB free".
pub(crate) fn free_space_text(size: HumanSize) -> SharedString {
    t!("status-free-space", size = size_string(size)).into()
}

/// Why `config.toml` is not used, as the tooltip of the notice in the status bar says it.
pub(crate) fn config_problem_text(problem: ConfigProblem) -> SharedString {
    match problem {
        ConfigProblem::Unreadable => t!("config-problem-unreadable"),
        ConfigProblem::Invalid { line, column } => {
            t!("config-problem-invalid", line = line, column = column)
        }
        ConfigProblem::Malformed => t!("config-problem-malformed"),
        ConfigProblem::UnsupportedUrl => t!("config-problem-unsupported-url"),
    }
    .into()
}

/// The name of a filter in the sidebar, such as "Seeding".
pub(crate) fn filter_text(filter: Filter) -> SharedString {
    match filter {
        Filter::All => t!("filter-all"),
        Filter::Downloading => t!("filter-downloading"),
        Filter::Seeding => t!("filter-seeding"),
        Filter::Paused => t!("filter-paused"),
        Filter::Active => t!("filter-active"),
        Filter::Error => t!("filter-error"),
    }
    .into()
}

/// The number of torrents a filter matches, as the sidebar shows it next to the filter's name.
pub(crate) fn filter_count_text(count: usize) -> SharedString {
    t!("filter-count", count = count).into()
}

/// What a screen reader says for a filter in the sidebar: its name and the number of torrents it
/// matches, such as "Seeding, 3 torrents", or the name alone while the number is not known.
pub(crate) fn filter_label_text(filter: Filter, count: Option<usize>) -> SharedString {
    match count {
        Some(count) => t!(
            "filter-label",
            filter = filter_text(filter).to_string(),
            count = count
        )
        .into(),
        None => filter_text(filter),
    }
}

/// What the empty search box says: "Search (Ctrl+F)".
pub(crate) fn search_placeholder_text() -> SharedString {
    t!("search-placeholder").into()
}

/// How many torrents the list shows, such as "3 torrents".
pub(crate) fn torrent_count_text(count: usize) -> SharedString {
    t!("torrent-count", count = count).into()
}

/// What the empty list says: "No torrents", or "No torrents match" when the filter and the search
/// hide them all. Before the daemon's list has arrived it says nothing; the status bar tells how
/// the connection is doing.
pub(crate) fn empty_table_text(empty: EmptyTable) -> Option<SharedString> {
    match empty {
        EmptyTable::Waiting => None,
        EmptyTable::NoTorrents => Some(t!("torrent-list-empty").into()),
        EmptyTable::NoMatches => Some(t!("torrent-list-no-matches").into()),
    }
}

/// The header of a column, such as "Size"; the speed columns have only an arrow.
pub(crate) fn column_title_text(column: ColumnId) -> SharedString {
    match column {
        ColumnId::Name => t!("column-name"),
        ColumnId::Size => t!("column-size"),
        ColumnId::Progress => t!("column-progress"),
        ColumnId::State => t!("column-state"),
        ColumnId::Down => t!("column-down"),
        ColumnId::Up => t!("column-up"),
        ColumnId::Eta => t!("column-eta"),
        ColumnId::Ratio => t!("column-ratio"),
        ColumnId::Peers => t!("column-peers"),
    }
    .into()
}

/// The full name of a column, for the column menu and screen readers: the header, or the name a
/// header of only an arrow stands for, such as "Download speed".
pub(crate) fn column_name_text(column: ColumnId) -> SharedString {
    match column {
        ColumnId::Down => t!("column-down-long").into(),
        ColumnId::Up => t!("column-up-long").into(),
        ColumnId::Name
        | ColumnId::Size
        | ColumnId::Progress
        | ColumnId::State
        | ColumnId::Eta
        | ColumnId::Ratio
        | ColumnId::Peers => column_title_text(column),
    }
}

/// The mark after the header of the sorted column: "▲" from the smallest, "▼" from the largest.
pub(crate) fn sort_mark_text(direction: SortDirection) -> SharedString {
    match direction {
        SortDirection::Ascending => t!("sort-ascending"),
        SortDirection::Descending => t!("sort-descending"),
    }
    .into()
}

/// What the sort mark means, for screen readers: "sorted ascending" or "sorted descending".
pub(crate) fn sort_mark_label(direction: SortDirection) -> SharedString {
    match direction {
        SortDirection::Ascending => t!("sort-ascending-label"),
        SortDirection::Descending => t!("sort-descending-label"),
    }
    .into()
}

/// The peers column: the number of connected peers.
pub(crate) fn peer_count_text(count: u32) -> SharedString {
    t!("peer-count", count = count).into()
}

/// The caption of a progress bar, such as "63% of 22.9 GiB".
pub(crate) fn progress_text(percent: u8, size: HumanSize) -> SharedString {
    t!("progress-of", percent = percent, size = size_string(size)).into()
}

fn time_unit_text(unit: TimeUnit) -> String {
    match unit {
        TimeUnit::Days => t!("unit-day"),
        TimeUnit::Hours => t!("unit-hour"),
        TimeUnit::Minutes => t!("unit-minute"),
        TimeUnit::Seconds => t!("unit-second"),
    }
}

/// The ETA column: "1 d 2 h", "5 m", "∞" when the server cannot tell and "—" when there is none.
pub(crate) fn eta_text(eta: &EtaView) -> SharedString {
    match eta {
        EtaView::None => t!("eta-none"),
        EtaView::Unknown => t!("eta-unknown"),
        EtaView::In(duration) => {
            let (major, major_unit) = duration.major;
            let major_unit = time_unit_text(major_unit);
            match duration.minor {
                None => t!("eta-major", major = major, major_unit = major_unit),
                Some((minor, minor_unit)) => t!(
                    "eta-major-minor",
                    major = major,
                    major_unit = major_unit,
                    minor = minor,
                    minor_unit = time_unit_text(minor_unit)
                ),
            }
        }
    }
    .into()
}

/// The ratio column: two decimals, "∞" for infinity and "—" when there is no ratio yet.
pub(crate) fn ratio_text(ratio: Ratio) -> SharedString {
    match ratio {
        Ratio::NotAvailable => t!("ratio-none"),
        Ratio::Infinite => t!("ratio-infinite"),
        Ratio::Value(value) => decimal(value, 2),
    }
    .into()
}

/// The line under a torrent's name, such as "Seeding · 3 peers downloading from you".
///
/// The text of an error or a warning is the server's own and is shown as sent. When the server
/// sent none, a generic message stands in, so the line never ends in a bare "Error:".
pub(crate) fn subtitle_text(subtitle: &Subtitle) -> SharedString {
    match subtitle {
        Subtitle::Downloading {
            from_peers,
            connected,
        } => t!(
            "subtitle-downloading",
            from_peers = from_peers,
            connected = connected
        ),
        Subtitle::Seeding { to_peers } => t!("subtitle-seeding", peers = to_peers),
        Subtitle::Paused => t!("subtitle-paused"),
        Subtitle::Queued(QueueKind::Download) => t!("subtitle-queued-download"),
        Subtitle::Queued(QueueKind::Seed) => t!("subtitle-queued-seed"),
        Subtitle::Checking { percent } => t!("subtitle-checking", percent = percent),
        Subtitle::FetchingMetadata { percent } => {
            t!("subtitle-fetching-metadata", percent = percent)
        }
        Subtitle::Error(text) if has_text(text) => t!("subtitle-error", text = text.as_str()),
        Subtitle::Error(_) => t!("subtitle-error-generic"),
        Subtitle::TrackerWarning(text) if has_text(text) => {
            t!("subtitle-tracker-warning", text = text.as_str())
        }
        Subtitle::TrackerWarning(_) => t!("subtitle-tracker-warning-generic"),
        Subtitle::TrackerError(text) if has_text(text) => {
            t!("subtitle-tracker-error", text = text.as_str())
        }
        Subtitle::TrackerError(_) => t!("subtitle-tracker-error-generic"),
    }
    .into()
}

fn has_text(text: &str) -> bool {
    !text.trim().is_empty()
}

/// The name of a torrent's state, as the state column shows it.
pub(crate) fn state_text(state: DisplayState) -> SharedString {
    match state {
        DisplayState::Downloading => t!("state-downloading"),
        DisplayState::Seeding => t!("state-seeding"),
        DisplayState::Paused => t!("state-paused"),
        DisplayState::Queued(_) => t!("state-queued"),
        DisplayState::Checking => t!("state-checking"),
        DisplayState::Error => t!("state-error"),
    }
    .into()
}

/// What the status bar says next to the connection dot; empty when connected, as the bar then
/// shows only the dot.
///
/// The status bar has little room, so a failed reconnect is named in a few words (see
/// [`connection_reason_text`]). The details the server gave for a protocol or server error are
/// not part of the text; they are for a tooltip, together with [`connection_reason_detail_text`].
pub(crate) fn connection_text(indicator: &Indicator) -> SharedString {
    match indicator {
        Indicator::ServiceFailed => t!("connection-service-failed"),
        Indicator::ServiceStopped => t!("connection-service-stopped"),
        Indicator::Unsupported(reason) => match reason {
            UnsupportedReason::Legacy => t!("connection-unsupported-legacy"),
            UnsupportedReason::NoVersion => t!("connection-unsupported-no-version"),
            UnsupportedReason::Version(version) => {
                t!("connection-unsupported-version", version = version.as_str())
            }
        },
        Indicator::Reconnecting {
            attempt,
            last_error,
        } => t!(
            "connection-reconnecting",
            attempt = attempt,
            reason = connection_reason_text(last_error).to_string()
        ),
        Indicator::Connecting => t!("connection-connecting"),
        Indicator::Connected => String::new(),
    }
    .into()
}

/// Why the connection failed, in a few words for the status bar, such as "timed out".
pub(crate) fn connection_reason_text(error: &ConnectionError) -> SharedString {
    match error {
        ConnectionError::Unreachable => t!("connection-reason-unreachable"),
        ConnectionError::Timeout => t!("connection-reason-timeout"),
        ConnectionError::Forbidden => t!("connection-reason-forbidden"),
        ConnectionError::Unauthorized => t!("connection-reason-unauthorized"),
        ConnectionError::Protocol(_) => t!("connection-reason-protocol"),
        ConnectionError::Server(_) => t!("connection-reason-server"),
    }
    .into()
}

/// Why the connection failed, as a sentence for a tooltip. Like [`connection_reason_text`], it
/// leaves out the details the server gave for a protocol or server error.
pub(crate) fn connection_reason_detail_text(error: &ConnectionError) -> SharedString {
    match error {
        ConnectionError::Unreachable => t!("connection-reason-unreachable-detail"),
        ConnectionError::Timeout => t!("connection-reason-timeout-detail"),
        ConnectionError::Forbidden => t!("connection-reason-forbidden-detail"),
        ConnectionError::Unauthorized => t!("connection-reason-unauthorized-detail"),
        ConnectionError::Protocol(_) => t!("connection-reason-protocol-detail"),
        ConnectionError::Server(_) => t!("connection-reason-server-detail"),
    }
    .into()
}

/// What a notification says when an action failed: what it was and for how many torrents
/// (`pending`, the action as it was sent; `None` when that is not known), or which speed setting
/// it changed, and what is known about it from `error`.
///
/// Refused because the daemon was not connected, it was not sent. When the daemon did not answer
/// in time, or the connection broke, it may have been carried out all the same, so the text says
/// so and leaves the result to the list. Otherwise it failed, for the short reason of the status
/// bar, or for the server's own text when it sent one.
pub(crate) fn action_failure_text(
    pending: Option<PendingAction>,
    error: &ActionError,
) -> SharedString {
    let (outcome, reason) = match error {
        ActionError::NotConnected => ("not-sent", String::new()),
        ActionError::Rpc(ConnectionError::Timeout | ConnectionError::Unreachable) => {
            ("no-answer", String::new())
        }
        error => ("failed", failure_reason(error)),
    };
    let Some(PendingAction { kind, count }) = pending else {
        return t!("action-failed-other", outcome = outcome, reason = reason).into();
    };
    match kind {
        ActionKind::Start => t!(
            "action-failed-start",
            outcome = outcome,
            count = count,
            reason = reason
        ),
        ActionKind::Pause => t!(
            "action-failed-pause",
            outcome = outcome,
            count = count,
            reason = reason
        ),
        ActionKind::StartNow => t!(
            "action-failed-start-now",
            outcome = outcome,
            count = count,
            reason = reason
        ),
        ActionKind::Verify => t!(
            "action-failed-verify",
            outcome = outcome,
            count = count,
            reason = reason
        ),
        ActionKind::Reannounce => t!(
            "action-failed-reannounce",
            outcome = outcome,
            count = count,
            reason = reason
        ),
        ActionKind::Remove(RemoveData::Keep) => t!(
            "action-failed-remove",
            outcome = outcome,
            count = count,
            reason = reason
        ),
        ActionKind::Remove(RemoveData::Delete) => t!(
            "action-failed-remove-with-data",
            outcome = outcome,
            count = count,
            reason = reason
        ),
        ActionKind::AltSpeed(enabled) => t!(
            "action-failed-alt-speed",
            outcome = outcome,
            state = if enabled { "on" } else { "off" },
            reason = reason
        ),
        ActionKind::SpeedLimit { direction, .. } => t!(
            "action-failed-speed-limit",
            outcome = outcome,
            direction = match direction {
                Direction::Down => "down",
                Direction::Up => "up",
            },
            reason = reason
        ),
    }
    .into()
}

/// Why a request to the daemon failed, in a few words: the server's own text when it sent one, the
/// short reason of the status bar otherwise. Nothing for a request that was not sent, which its
/// message words as such.
fn failure_reason(error: &ActionError) -> String {
    match error {
        ActionError::Rpc(ConnectionError::Server(text)) if has_text(text) => text.clone(),
        ActionError::Rpc(error) => connection_reason_text(error).to_string(),
        ActionError::EmptySelection => t!("action-reason-nothing-selected"),
        ActionError::NotConnected => String::new(),
    }
}

/// What a notification says when copying the magnet links of `count` torrents failed, for the
/// reason `failure`. Unlike an action, a read that got no answer changed nothing, so a timeout is
/// only a reason.
pub(crate) fn magnet_failure_text(count: usize, failure: &MagnetFailure) -> SharedString {
    let (outcome, reason) = match failure {
        MagnetFailure::Gone => ("gone", String::new()),
        MagnetFailure::Error(ActionError::NotConnected) => ("not-sent", String::new()),
        MagnetFailure::Error(error) => ("failed", failure_reason(error)),
    };
    t!(
        "copy-magnet-failed",
        outcome = outcome,
        count = count,
        reason = reason
    )
    .into()
}

/// What a notification says when only `copied` of the `asked` magnet links were copied, because
/// the daemon no longer has the other torrents.
pub(crate) fn magnet_partial_text(copied: usize, asked: usize) -> SharedString {
    let missing = asked.saturating_sub(copied);
    t!(
        "copy-magnet-partial",
        copied = copied,
        asked = asked,
        missing = missing
    )
    .into()
}

/// The title of a removal dialog, which counts the torrents of `summary`: "Remove 3 torrents from
/// the list?" when the files stay (`data`), "Delete 3 torrents and their files?" when they go.
pub(crate) fn removal_title_text(data: RemoveData, summary: &RemovalSummary) -> SharedString {
    let count = summary.count;
    match data {
        RemoveData::Keep => t!("remove-from-list-title", count = count),
        RemoveData::Delete => t!("remove-with-data-title", count = count),
    }
    .into()
}

/// What a removal dialog says after the names it lists, "and 2 more"; `None` when `summary`
/// names every torrent.
pub(crate) fn removal_more_text(summary: &RemovalSummary) -> Option<SharedString> {
    let more = summary.more;
    (more > 0).then(|| t!("remove-more", more = more).into())
}

/// What a removal dialog says becomes of the downloaded files: that they stay on the server when
/// `data` keeps them, or how much of them (`summary`) is deleted for good.
pub(crate) fn removal_files_text(data: RemoveData, summary: &RemovalSummary) -> SharedString {
    match data {
        RemoveData::Keep => t!("remove-from-list-keeps-files"),
        RemoveData::Delete => t!(
            "remove-with-data-deletes-files",
            size = size_string(summary.downloaded)
        ),
    }
    .into()
}
