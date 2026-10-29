//! The status bar at the bottom of the main window: how the connection is doing, a notice about
//! the config file, whether the alternative speed limits apply, the free space and the total
//! speeds.
//!
//! [`StatusModel`] from `x4-core` decides the values; this module words them and lays them out.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Sizable as _, Theme, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, ClickEvent, InteractiveElement as _, IntoElement, ParentElement as _, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, TestSupportExt as _, Window, div, px,
};
use x4_core::{ConnectionState, StatusModel};

use crate::i18n::{
    self, config_problem_detail_text, connection_reason_detail_text, download_rate_text,
    free_space_text, t, upload_rate_text,
};
use crate::theme::{connection_color, tabular_digits};

/// The diameter of the connection dot.
const DOT_SIZE: f32 = 8.;

/// Whether the service thread is there to send snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServiceState {
    /// It runs; the snapshots tell how the connection is doing.
    Running,
    /// It stopped or panicked: its events channel closed, so nothing will change any more.
    Stopped,
    /// It could not be started.
    Failed,
}

/// What the status bar says about the connection: the state of the link while the service runs,
/// and that the service is gone otherwise. Empty while connected, as the bar then shows only the
/// dot.
pub(crate) fn connection_label(status: &StatusModel, service: ServiceState) -> SharedString {
    match service {
        ServiceState::Running => i18n::connection_text(&status.connection),
        ServiceState::Stopped => t!("connection-service-stopped").into(),
        ServiceState::Failed => t!("connection-service-failed").into(),
    }
}

/// The tooltip of the connection indicator, when there is more to say than its text: what the
/// dot alone means, or why the last try to reconnect failed.
fn connection_tooltip(status: &StatusModel, service: ServiceState) -> Option<SharedString> {
    match (service, &status.connection) {
        (ServiceState::Running, ConnectionState::Connected) => {
            Some(t!("connection-connected").into())
        }
        (ServiceState::Running, ConnectionState::Reconnecting { last_error, .. }) => {
            Some(connection_reason_detail_text(last_error))
        }
        _ => None,
    }
}

/// What the status bar shows, beyond the [`StatusModel`].
pub(crate) struct StatusBarParts<'a> {
    /// The values from `x4-core`.
    pub status: &'a StatusModel,
    /// Whether the service runs.
    pub service: ServiceState,
    /// Why `config.toml` is not used, until the user dismisses the notice.
    pub config_problem: Option<&'a SharedString>,
}

/// The status bar for `parts`, in the colors of `theme`. `on_dismiss` handles a click on the
/// button that dismisses the config notice.
pub(crate) fn status_bar(
    parts: &StatusBarParts<'_>,
    theme: &Theme,
    on_dismiss: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> StatusBar {
    let StatusBarParts {
        status,
        service,
        config_problem,
    } = *parts;
    let mut bar = StatusBar::new().left(connection_indicator(status, service, theme));
    if let Some(problem) = config_problem {
        bar = bar.left(config_notice(problem, on_dismiss));
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
    if let Some(free_space) = status.free_space {
        bar = bar.right(number("free-space", free_space_text(free_space)));
    }
    bar.right(number("rate-down", download_rate_text(status.rate_down)))
        .right(number("rate-up", upload_rate_text(status.rate_up)))
}

/// The dot and the text that tell how the connection is doing.
fn connection_indicator(
    status: &StatusModel,
    service: ServiceState,
    theme: &Theme,
) -> impl IntoElement {
    let label = connection_label(status, service);
    let tooltip = connection_tooltip(status, service);
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
                .bg(connection_color(service, &status.connection, theme)),
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
    problem: &SharedString,
    on_dismiss: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let detail = config_problem_detail_text(problem);
    h_flex()
        .gap_1()
        .child(
            div()
                .id("config-problem")
                .child(t!("status-config-problem"))
                .tooltip(move |window, cx| Tooltip::new(detail.clone()).build(window, cx))
                .role(Role::Status)
                .aria_label(SharedString::from(t!("status-config-problem")))
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

/// A number that changes every tick, in tabular digits so the bar does not jitter.
fn number(id: &'static str, text: SharedString) -> impl IntoElement {
    div()
        .id(id)
        .font_features(tabular_digits())
        .child(text.clone())
        .aria_label(text)
        .test_support()
}

#[cfg(test)]
mod tests {
    use x4_core::{ConnectionError, UnsupportedReason};

    use super::*;

    fn status(connection: ConnectionState) -> StatusModel {
        StatusModel {
            connection,
            ..StatusModel::default()
        }
    }

    #[test]
    fn a_running_service_shows_the_connection() {
        let reconnecting = status(ConnectionState::Reconnecting {
            attempt: 2,
            last_error: ConnectionError::Timeout,
        });

        assert_eq!(
            connection_label(&StatusModel::default(), ServiceState::Running),
            "Connecting…"
        );
        assert_eq!(
            connection_label(&status(ConnectionState::Connected), ServiceState::Running),
            ""
        );
        assert_eq!(
            connection_label(&reconnecting, ServiceState::Running),
            "Reconnecting (attempt 2)… timed out"
        );
        assert_eq!(
            connection_label(
                &status(ConnectionState::Unsupported {
                    reason: UnsupportedReason::NoVersion
                }),
                ServiceState::Running
            ),
            "Transmission 4.1 or newer required (server did not report a version)"
        );
    }

    #[test]
    fn a_service_that_is_gone_says_so_whatever_the_last_snapshot_said() {
        for connection in [ConnectionState::Connected, ConnectionState::Connecting] {
            assert_eq!(
                connection_label(&status(connection.clone()), ServiceState::Stopped),
                "Service stopped"
            );
            assert_eq!(
                connection_label(&status(connection), ServiceState::Failed),
                "Service could not start"
            );
        }
    }

    #[test]
    fn the_tooltip_explains_the_dot_and_the_reconnect_reason() {
        let reconnecting = status(ConnectionState::Reconnecting {
            attempt: 1,
            last_error: ConnectionError::Unauthorized,
        });

        assert_eq!(
            connection_tooltip(&status(ConnectionState::Connected), ServiceState::Running),
            Some("Connected".into())
        );
        assert_eq!(
            connection_tooltip(&reconnecting, ServiceState::Running),
            Some("The server rejected the credentials".into())
        );
        assert_eq!(
            connection_tooltip(&StatusModel::default(), ServiceState::Running),
            None
        );
        assert_eq!(
            connection_tooltip(&status(ConnectionState::Connected), ServiceState::Stopped),
            None,
            "a stopped service is not connected"
        );
    }
}
