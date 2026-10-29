//! The status bar at the bottom of the main window: how the connection is doing, notices about
//! the config file and the saved state, whether the alternative speed limits apply, the free space
//! and the total speeds.
//!
//! [`StatusModel`] from `x4-core` decides the values, down to which state the connection
//! indicator shows ([`Indicator`]); this module words them and lays them out.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Sizable as _, Theme, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ClickEvent, InteractiveElement as _, IntoElement, ParentElement as _, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, div, px,
};
use x4_core::{Indicator, StatusModel};

use crate::app::ConfigProblem;
use crate::i18n::{
    config_problem_text, connection_reason_detail_text, connection_text, download_rate_text,
    free_space_text, t, upload_rate_text,
};
use crate::theme::{STALE_OPACITY, connection_color, tabular_digits};

/// The diameter of the connection dot.
const DOT_SIZE: f32 = 8.;

/// The tooltip of the connection indicator, when there is more to say than its text: what the
/// dot alone means, or why the last try to reconnect failed.
fn connection_tooltip(indicator: &Indicator) -> Option<SharedString> {
    match indicator {
        Indicator::Connected => Some(t!("connection-connected").into()),
        Indicator::Reconnecting { last_error, .. } => {
            Some(connection_reason_detail_text(last_error))
        }
        Indicator::ServiceFailed
        | Indicator::ServiceStopped
        | Indicator::Unsupported(_)
        | Indicator::Connecting => None,
    }
}

/// The status bar for `status`, in the colors of `theme`. `config_problem` adds a notice whose
/// button calls `on_dismiss`, and `state_not_saved` one that the UI state cannot be saved.
pub(crate) fn status_bar(
    status: &StatusModel,
    config_problem: Option<ConfigProblem>,
    state_not_saved: bool,
    theme: &Theme,
    on_dismiss: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> StatusBar {
    let mut bar = StatusBar::new().left(connection_indicator(&status.indicator(), theme));
    if let Some(problem) = config_problem {
        bar = bar.left(config_notice(problem, on_dismiss));
    }
    if state_not_saved {
        bar = bar.left(state_notice());
    }
    if status.alt_speed {
        bar = bar.right(
            div()
                .id("alt-speed")
                .text_color(theme.warning)
                .child(t!("status-alt-speed"))
                .tooltip(|window, cx| {
                    Tooltip::new(t!("status-alt-speed-detail")).build(window, cx)
                }),
        );
    }
    // The engine keeps the last numbers while the daemon does not answer; they are dimmed then,
    // like the table.
    let stale = status.stale;
    if let Some(free_space) = status.free_space {
        bar = bar.right(number("free-space", free_space_text(free_space), stale));
    }
    bar.right(number(
        "rate-down",
        download_rate_text(status.rate_down),
        stale,
    ))
    .right(number("rate-up", upload_rate_text(status.rate_up), stale))
}

/// The dot and the text that tell how the connection is doing.
fn connection_indicator(indicator: &Indicator, theme: &Theme) -> impl IntoElement {
    let label = connection_text(indicator);
    let tooltip = connection_tooltip(indicator);
    // The dot alone says nothing to a screen reader, so the indicator is named either way.
    let name = if label.is_empty() {
        tooltip.clone().unwrap_or_default()
    } else {
        label.clone()
    };
    h_flex()
        .id("connection")
        .gap_1p5()
        .child(
            div()
                .size(px(DOT_SIZE))
                .flex_none()
                .rounded_full()
                .bg(connection_color(indicator, theme)),
        )
        .when(!label.is_empty(), |indicator| indicator.child(label))
        .when_some(tooltip, |indicator, tooltip| {
            indicator.tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        })
        .role(Role::Status)
        .aria_label(name)
        .test_support()
}

/// The notice that `config.toml` is not used, with the reason in its tooltip and a button that
/// dismisses it.
fn config_notice(
    problem: ConfigProblem,
    on_dismiss: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let detail = config_problem_text(problem);
    h_flex()
        .gap_1()
        .child(
            div()
                .id("config-problem")
                .child(t!("status-config-problem"))
                .tooltip({
                    let detail = detail.clone();
                    move |window, cx| Tooltip::new(detail.clone()).build(window, cx)
                })
                .role(Role::Status)
                .aria_label(SharedString::from(t!("status-config-problem")))
                .aria_description(detail)
                .test_support(),
        )
        .child(
            Button::new("config-problem-dismiss")
                .ghost()
                .xsmall()
                .label(t!("status-dismiss"))
                .on_click(on_dismiss),
        )
}

/// The notice that the UI state cannot be saved, with what that means in its tooltip. It stays
/// until a save works again; the log has the error.
fn state_notice() -> impl IntoElement {
    let detail = SharedString::from(t!("status-state-not-saved-detail"));
    div()
        .id("state-not-saved")
        .child(t!("status-state-not-saved"))
        .tooltip({
            let detail = detail.clone();
            move |window, cx| Tooltip::new(detail.clone()).build(window, cx)
        })
        .role(Role::Status)
        .aria_label(SharedString::from(t!("status-state-not-saved")))
        .aria_description(detail)
        .test_support()
}

/// A number that changes every tick, in tabular digits so the bar does not jitter, and dimmed
/// while `stale`.
fn number(id: &'static str, text: SharedString, stale: bool) -> impl IntoElement {
    div()
        .id(id)
        .font_features(tabular_digits())
        .when(stale, |number| number.opacity(STALE_OPACITY))
        .child(text.clone())
        .aria_label(text)
        .test_support()
}

#[cfg(test)]
mod tests {
    use x4_core::{ConnectionError, UnsupportedReason};

    use super::*;

    #[test]
    fn the_tooltip_explains_the_dot_and_the_reconnect_reason() {
        assert_eq!(
            connection_tooltip(&Indicator::Connected),
            Some("Connected".into())
        );
        assert_eq!(
            connection_tooltip(&Indicator::Reconnecting {
                attempt: 1,
                last_error: ConnectionError::Unauthorized,
            }),
            Some("The server rejected the credentials".into())
        );
        for indicator in [
            Indicator::Connecting,
            Indicator::ServiceStopped,
            Indicator::ServiceFailed,
            Indicator::Unsupported(UnsupportedReason::Legacy),
        ] {
            assert_eq!(
                connection_tooltip(&indicator),
                None,
                "the text says it all: {indicator:?}"
            );
        }
    }
}
